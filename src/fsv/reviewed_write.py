"""Single-attempt mutation primitive for captured internal contracts.

Builders supply exact methods/paths and independent verification. This module
is not an arbitrary API command and never retries a mutation.
"""
from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any, Callable

from fsv import config, journal
from fsv.errors import APIError, server_message


class ReviewedWriteError(RuntimeError):
    pass


@dataclass(frozen=True)
class WritePlan:
    action: str
    target: int
    tenant: str
    method: str
    path: str
    payload_json: str
    context_hash: str

    @property
    def payload(self) -> Any:
        return json.loads(self.payload_json)

    @property
    def hash(self) -> str:
        return journal.plan_hash({"action": self.action, "target": self.target, "tenant": self.tenant,
                                  "method": self.method, "path": self.path, "payload": self.payload,
                                  "context": self.context_hash}) or ""

    def review(self) -> dict[str, Any]:
        return {"action": self.action, "method": self.method, "path": "/api/_/" + self.path,
                "payload": self.payload, "plan_hash": self.hash,
                "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."}


def execute(plan: WritePlan, client: Any, *, reviewed_hash: str | None,
            rebuild: Callable[[], WritePlan], verify: Callable[[Any], dict[str, Any]],
            rejection: Callable[[APIError], str] | None = None) -> dict[str, Any]:
    if reviewed_hash is None:
        return {"status": "cancelled", "verified": False}
    if not plan.tenant or plan.tenant != config.DOMAIN or reviewed_hash != plan.hash:
        raise ReviewedWriteError("tenant or full review hash changed; review again")
    events = [e for e in journal.read() if e.get("kind") == "plan:" + plan.action and e.get("plan") == plan.hash]
    refusal = journal.replay_refusal(events, "write was sent or outcome unknown; inspect target and journal before retry")
    if refusal:
        raise ReviewedWriteError(refusal)
    if rebuild().hash != plan.hash:
        raise ReviewedWriteError("target or metadata changed since review; review again")
    try:
        csrf = client._get_csrf()
    except Exception:
        raise ReviewedWriteError("could not prepare session; no write attempted") from None
    journal.record_plan("started", plan.hash, action=plan.action, target=plan.target)
    events = [e for e in journal.read() if e.get("kind") == "plan:" + plan.action and e.get("plan") == plan.hash]
    if not events or events[-1].get("event") != "started":
        raise ReviewedWriteError("could not persist ambiguity journal; no write attempted")
    kwargs: dict[str, Any] = {"headers": {"X-CSRF-Token": csrf, "Content-Type": "application/json"}}
    if plan.payload is not None:
        kwargs["content"] = plan.payload_json.encode("utf-8")
        kwargs["headers"]["Content-Type"] = "application/json"
    try:
        response = client._request(plan.method, f"{config.API_INT}/{plan.path}", **kwargs)
    except APIError as exc:
        if 400 <= exc.status < 500 and exc.status not in (403, 408, 429):
            journal.record_plan("failed", plan.hash, action=plan.action, target=plan.target)
            if rejection:
                raise ReviewedWriteError(rejection(exc)) from None
            # Name the server's reason (e.g. a lifecycle edge, criterion or lock).
            reason = server_message(exc.body)
            raise ReviewedWriteError(f"write rejected (HTTP {exc.status})" + (f": {reason}" if reason else "")
                                     + "; no retry") from None
        raise ReviewedWriteError("write outcome unknown; inspect target and journal before retry") from None
    except Exception:
        raise ReviewedWriteError("write outcome unknown; inspect target and journal before retry") from None
    journal.record_plan("done", plan.hash, action=plan.action, target=plan.target)
    result = {"status": "submitted", "plan_hash": plan.hash, "verified": False}
    try:
        result.update(verify(response))
    except Exception:
        result["warning"] = "Independent readback failed; do not resend."
    if not result.get("verified"):
        result.setdefault("warning", "Readback was not fully verified; do not resend.")
    return result
