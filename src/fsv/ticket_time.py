"""Captured manual, nonbillable, self-owned ticket time-entry CRUD."""
from __future__ import annotations

import json
import re
from typing import Any

from fsv import config, journal
from fsv.change_tasks import _date
from fsv.resources import TICKETS, parse_id
from fsv.readback import duration_seconds, same_duration, same_instant
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute
from fsv.views import collect_pages
from fsv.watchers import positive_id, self_id


def inventory(client: Any, target: int) -> list[dict[str, Any]]:
    rows, meta, _ = collect_pages(client, f"tickets/{target}/time_entries", "time_entries",
                                  {"page": 1, "per_page": 30}, what="time entries")
    if not meta.get("complete"):
        raise ReviewedWriteError("complete time-entry inventory is unavailable")
    ids = [positive_id(row.get("id")) for row in rows]
    if len(ids) != len(set(ids)):
        raise ReviewedWriteError("time-entry inventory contains duplicate identities")
    return sorted(rows, key=lambda row: positive_id(row["id"]))


def _duration(value: str) -> str:
    if not isinstance(value, str) or not re.fullmatch(r"\d{2,}:[0-5]\d", value) or int(value.replace(":", "")) == 0:
        raise ValueError("manual time must be a positive HH:MM duration")
    return value


def build_plan(client: Any, target: str | int, *, action: str, entry_id: int | None = None,
               spent: str | None = None, executed_at: str | None = None, note: str | None = None) -> WritePlan:
    if action not in {"create", "update", "delete"}:
        raise ValueError("unsupported time-entry action")
    cid = positive_id(parse_id(str(target), TICKETS))
    user = self_id(client)
    rows = inventory(client, cid)
    if note is not None and (not isinstance(note, str) or not note.strip()):
        raise ValueError("note must be nonempty text when provided")
    if action == "create":
        if entry_id is not None or spent is None or executed_at is None:
            raise ValueError("create requires --spent and --executed-at, without an entry ID")
        when = _date(executed_at)
        if when is None:
            raise ValueError("executed-at needs an explicit offset")
        payload = {"executed_at": when, "timer_running": False, "billable": False,
                   "time_spent": _duration(spent), "note": note or "", "agent_id": user}
        method, path = "POST", f"tickets/{cid}/time_entries"
    else:
        eid = positive_id(entry_id)
        row = next((row for row in rows if positive_id(row["id"]) == eid), None)
        if row is None or str(row.get("agent_id")) != str(user) or row.get("billable") is not False or row.get("timer_running") is not False:
            raise ReviewedWriteError("exact self-owned nonbillable manual entry is unavailable")
        if executed_at is not None:
            raise ValueError("changing executed-at is not a captured update contract")
        path = f"tickets/{cid}/time_entries/{eid}"
        if action == "delete":
            if spent is not None or note is not None:
                raise ValueError("delete does not accept time or note changes")
            method, payload = "DELETE", None
        else:
            if spent is None and note is None:
                raise ValueError("update requires --spent or --note")
            if "time_spent" not in row or "note" not in row:
                raise ReviewedWriteError("entry time/note is unavailable")
            method = "PUT"
            # The server returns HH:MM:SS even for minute-only writes.
            current = row["time_spent"]
            if spent is None:
                seconds = duration_seconds(current)
                if seconds % 60:
                    raise ReviewedWriteError("entry has sub-minute precision; provide --spent explicitly")
                current = f"{seconds // 3600:02d}:{seconds // 60 % 60:02d}"
            payload = {"time_spent": _duration(spent if spent is not None else current),
                       "note": note if note is not None else row["note"]}
    context = journal.plan_hash({"self": user, "entries": rows}) or ""
    return WritePlan(f"ticket_time_{action}", cid, config.DOMAIN, method, path,
                     json.dumps(payload, sort_keys=True, ensure_ascii=False), context)


def send(plan: WritePlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    action = plan.action.removeprefix("ticket_time_")
    payload = plan.payload
    eid = None if action == "create" else positive_id(plan.path.rsplit("/", 1)[1])

    def rebuild() -> WritePlan:
        return build_plan(client, plan.target, action=action, entry_id=eid,
                          spent=payload.get("time_spent") if payload else None,
                          note=payload.get("note") or None if payload else None,
                          executed_at=payload.get("executed_at") if payload else None)

    def verify(response: Any) -> dict[str, Any]:
        exact_id = eid
        if action == "create":
            raw = response.get("time_entry", response) if isinstance(response, dict) else {}
            exact_id = positive_id(raw.get("id"))
        rows = inventory(client, plan.target)
        row = next((row for row in rows if str(row.get("id")) == str(exact_id)), None)
        if action == "delete":
            return {"entry_id": exact_id, "verified": row is None}
        expected = {k: v for k, v in payload.items()}
        def matches(key: str, actual: Any, value: Any) -> bool:
            if key == "time_spent":
                return same_duration(actual, value)
            if key == "executed_at":
                return same_instant(actual, value)
            return type(actual) is type(value) and actual == value

        fields = {k: "unavailable" if row is None or k not in row else
                  "matched" if matches(k, row[k], value) else "mismatched" for k, value in expected.items()}
        return {"entry_id": exact_id, "fields": fields, "verified": row is not None and all(v == "matched" for v in fields.values())}

    return execute(plan, client, reviewed_hash=reviewed_hash, rebuild=rebuild, verify=verify)
