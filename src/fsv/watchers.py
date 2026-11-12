"""Captured ticket/change self-watch contracts; other subscribers are unsupported."""
from __future__ import annotations

import json
from typing import Any

from fsv import config, journal
from fsv.resources import CHANGES, TICKETS, parse_id
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute


def positive_id(value: Any) -> int:
    if isinstance(value, bool) or not str(value).isdigit() or int(value) <= 0:
        raise ReviewedWriteError("expected a positive numeric identity")
    return int(value)


def self_id(client: Any) -> int:
    # Do not use a stale UI identity cache to authorize self-only writes.
    me = client.me()
    if not isinstance(me, dict):
        raise ReviewedWriteError("signed-in agent identity is unavailable")
    return positive_id(me.get("id"))


def inventory(client: Any, module: str, target: int) -> list[int]:
    raw = client.int_get(f"{module}/{target}/watchers")
    if (not isinstance(raw, dict) or not isinstance(raw.get("watcher_ids"), list) or
            raw.get("has_more") or raw.get("has_next") or raw.get("next_page")):
        raise ReviewedWriteError("complete watcher membership is unavailable")
    ids = [positive_id(value) for value in raw["watcher_ids"]]
    if len(ids) != len(set(ids)):
        raise ReviewedWriteError("watcher membership contains duplicate identities")
    if raw.get("total") is not None and raw["total"] != len(ids):
        raise ReviewedWriteError("watcher membership is incomplete")
    return sorted(ids)


def build_plan(client: Any, module: str, target: str | int, *, watch: bool) -> WritePlan:
    if module not in {"tickets", "changes"} or not isinstance(watch, bool):
        raise ReviewedWriteError("only ticket/change self-watch contracts are proven")
    cid = positive_id(parse_id(str(target), TICKETS if module == "tickets" else CHANGES))
    user = self_id(client)
    before = inventory(client, module, cid)
    if module == "tickets":
        method, path, payload = ("POST" if watch else "PUT"), f"tickets/{cid}/{'watch' if watch else 'unwatch'}", {"user_id": user}
    elif watch:
        method, path, payload = "POST", f"changes/{cid}/watchers", {"user_id": user}
    else:
        method, path, payload = "DELETE", f"changes/{cid}/unwatch?user_id={user}", None
    context = {"self": user, "before": before, "watch": watch}
    return WritePlan(f"{module}_self_{'watch' if watch else 'unwatch'}", cid, config.DOMAIN,
                     method, path, json.dumps(payload, sort_keys=True), journal.snapshot_hash(context) or "")


def send(plan: WritePlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    module = plan.path.split("/")[0]
    watch = plan.action.endswith("_watch")
    user = self_id(client)
    before = inventory(client, module, plan.target)
    # Rebuild and hash-check even no-ops, so an old review is never silently accepted.
    fresh = build_plan(client, module, plan.target, watch=watch)
    if reviewed_hash is None:
        return {"status": "cancelled", "verified": False}
    if fresh.hash != plan.hash or reviewed_hash != plan.hash or plan.tenant != config.DOMAIN:
        raise ReviewedWriteError("target or review changed; review again")
    if (user in before) == watch:
        return {"status": "already_in_requested_state", "verified": True, "plan_hash": plan.hash}

    def verify(response: Any) -> dict[str, Any]:
        after = inventory(client, module, plan.target)
        expected = sorted(set(before) | {user}) if watch else [v for v in before if v != user]
        return {"watcher_ids": after, "verified": after == expected}

    return execute(plan, client, reviewed_hash=reviewed_hash,
                   rebuild=lambda: build_plan(client, module, plan.target, watch=watch), verify=verify)
