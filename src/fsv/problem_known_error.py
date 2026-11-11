"""Captured problem known-error mark; reversal is deliberately unsupported."""
from __future__ import annotations

import json
from typing import Any

from fsv import config, journal
from fsv.resources import PROBLEMS, parse_id
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute
from fsv.watchers import positive_id


def record(client: Any, target: int) -> dict[str, Any]:
    raw = client.int_get(f"problems/{target}")
    row = raw.get("problem", raw) if isinstance(raw, dict) else None
    if not isinstance(row, dict) or str(row.get("id")) != str(target) or not isinstance(row.get("known_error"), bool):
        raise ReviewedWriteError("exact problem identity/known-error state is unavailable")
    return row


def build_plan(client: Any, target: str | int) -> WritePlan:
    cid = positive_id(parse_id(str(target), PROBLEMS))
    row = record(client, cid)
    return WritePlan("problem_known_error_mark", cid, config.DOMAIN, "PUT", f"problems/{cid}",
                     json.dumps({"known_error": True}), journal.plan_hash(row) or "")


def send(plan: WritePlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    fresh = build_plan(client, plan.target)
    if reviewed_hash is None:
        return {"status": "cancelled", "verified": False}
    if reviewed_hash != plan.hash or fresh.hash != plan.hash or plan.tenant != config.DOMAIN:
        raise ReviewedWriteError("problem or review changed; review again")
    if record(client, plan.target)["known_error"] is True:
        return {"status": "already_in_requested_state", "verified": True}
    return execute(plan, client, reviewed_hash=reviewed_hash,
                   rebuild=lambda: build_plan(client, plan.target),
                   verify=lambda response: {"problem_id": plan.target, "verified": record(client, plan.target)["known_error"] is True})
