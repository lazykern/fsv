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
    """Safe user-facing failure. ``http_status``/``server_message`` carry the
    server's answer (sanitized: one line, tags stripped, bounded) when a
    write got a non-2xx response, so callers never report a bare failure."""

    def __init__(self, message: str, *, http_status: int | None = None, server_message: str | None = None) -> None:
        super().__init__(message)
        self.http_status = http_status
        self.server_message = server_message

    def as_dict(self) -> dict[str, Any]:
        out: dict[str, Any] = {"error": str(self)}
        if self.http_status is not None:
            out["http_status"] = self.http_status
            out["server_message"] = self.server_message or ""
        return out


def http_detail(exc: APIError) -> tuple[str, str]:
    """``("HTTP 503: <message>", "<message>")`` for a failed write."""
    reason = server_message(exc.body)
    return f"HTTP {exc.status}" + (f": {reason}" if reason else " (empty response body)"), reason


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
        # Never a bare failure: name the server's status and reason (e.g. a
        # lifecycle edge, criterion, lock, rate limit or server error).
        detail, reason = http_detail(exc)
        if 400 <= exc.status < 500 and exc.status not in (403, 408, 429):
            journal.record_plan("failed", plan.hash, action=plan.action, target=plan.target)
            if rejection:
                raise ReviewedWriteError(rejection(exc), http_status=exc.status, server_message=reason) from None
            raise ReviewedWriteError(f"write rejected (HTTP {exc.status})" + (f": {reason}" if reason else "")
                                     + "; no retry", http_status=exc.status, server_message=reason) from None
        raise ReviewedWriteError(f"write outcome unknown ({detail}); inspect target and journal before retry",
                                 http_status=exc.status, server_message=reason) from None
    except Exception as exc:
        raise ReviewedWriteError(f"write outcome unknown ({type(exc).__name__}); inspect target and journal "
                                 "before retry") from None
    journal.record_plan("done", plan.hash, action=plan.action, target=plan.target)
    result = {"status": "submitted", "plan_hash": plan.hash, "verified": False}
    try:
        result.update(verify(response))
    except Exception:
        result["warning"] = "Independent readback failed; do not resend."
    if not result.get("verified"):
        result.setdefault("warning", "Readback was not fully verified; do not resend.")
    return result
