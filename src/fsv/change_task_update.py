"""Reviewed change-task updates; PUT contains only explicitly supplied fields."""
from __future__ import annotations
import html
import json
from dataclasses import dataclass
from datetime import datetime
from typing import Any
from fsv import config, journal, task_status
from fsv.change_tasks import _date, _effort, _note, resolve_group, resolve_owner
from fsv.readback import same_instant, submitted_value_matches
from fsv.render import strip_html
from fsv.resources import CHANGES, parse_id
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute
from fsv.ticket_tasks import custom_values
from fsv.views import collect_pages
from fsv.watchers import positive_id


@dataclass(frozen=True)
class TaskUpdatePlan(WritePlan):
    task_id: int
    inputs_json: str


def inventory(client: Any, target: int) -> list[dict[str, Any]]:
    rows, meta, missing = collect_pages(client, f"changes/{target}/tasks", "tasks", {"page": 1, "per_page": 100}, what="change tasks")
    if missing or not meta.get("complete"):
        raise ReviewedWriteError("complete task inventory unavailable")
    ids = [positive_id(r.get("id")) for r in rows]
    if len(set(ids)) != len(ids):
        raise ReviewedWriteError("duplicate task identities")
    return rows


def reminder(value: str, fields: list[dict[str, Any]]) -> int:
    matches = [f for f in fields if f.get("name") == "notify_before" and not f.get("deleted")]
    if len(matches) != 1:
        raise ReviewedWriteError("runtime reminder metadata unavailable")
    choices = [c for c in matches[0].get("choices", []) if isinstance(c, dict) and
               not any(c.get(k) for k in ("deleted", "deleted_at", "deactivated")) and
               str(value).casefold() in (str(c.get("id")), str(c.get("value") or c.get("label")).casefold())]
    if len(choices) != 1 or isinstance(choices[0].get("id"), bool) or not str(choices[0].get("id")).isascii() or not str(choices[0].get("id")).isdigit():
        raise ValueError("notify-before must match a runtime reminder label or seconds value")
    return int(choices[0]["id"])


def build_plan(client: Any, target: str | int, task: int, inputs: dict[str, Any]) -> TaskUpdatePlan:
    target = positive_id(parse_id(str(target), CHANGES))
    task = positive_id(task)
    allowed = {"title", "status", "owner", "group", "note", "planned_start", "planned_end", "planned_effort", "notify_before", "values"}
    if not inputs or set(inputs) - allowed:
        raise ValueError("provide captured task fields; unsupported fields cannot be sent")
    before = next((r for r in inventory(client, target) if positive_id(r["id"]) == task), None)
    if before is None:
        raise ReviewedWriteError("exact task unavailable")
    fields = task_status.fields_from(client)
    payload = {}
    for key, value in inputs.items():
        if key == "title":
            if not isinstance(value, str) or not value.strip() or any(ord(c) < 32 for c in value):
                raise ValueError("nonempty task title required")
            payload[key] = value
        elif key == "status":
            payload[key] = task_status.resolve(value, fields)[0]
        elif key in {"owner", "group"}:
            payload[key + "_id"] = None if value == "none" else (resolve_owner(value, client) if key == "owner" else resolve_group(value, client))
        elif key == "note":
            _note(value)
            payload["description"] = "<div>" + html.escape(value).replace("\n", "<br>") + "</div>"
        elif key in {"planned_start", "planned_end"}:
            payload[key + "_date"] = _date(value)
        elif key == "planned_effort":
            payload[key] = _effort(value)
        elif key == "notify_before":
            payload[key] = reminder(value, fields)
        elif key == "values":
            payload["custom_fields"] = custom_values(fields, value)
    start = payload.get("planned_start_date", before.get("planned_start_date"))
    end = payload.get("planned_end_date", before.get("planned_end_date"))
    if start and end and datetime.fromisoformat(end.replace("Z", "+00:00")) < datetime.fromisoformat(start.replace("Z", "+00:00")):
        raise ValueError("planned end must not precede start")
    if "status" in payload:
        custom = {**before.get("custom_fields", {}), **payload.get("custom_fields", {})}
        label = task_status.resolve(str(payload["status"]), fields)[1]
        if label.casefold() == "completed":
            missing = []
            for f in fields:
                opts = f.get("field_options") or {}
                if f.get("required_for_closure") is True or opts.get("required_for_closure") in (True, "true"):
                    name = f.get("name", "")
                    value = payload.get(name, before.get(name, custom.get(name.removeprefix("cf_"))))
                    if value is None or value == "":
                        missing.append(str(f.get("label") or name))
            if missing:
                raise ReviewedWriteError("required closure task fields missing: " + ", ".join(missing))
    return TaskUpdatePlan("change_task_update", target, config.DOMAIN, "PUT", f"changes/{target}/tasks/{task}",
                          json.dumps(payload, sort_keys=True), journal.snapshot_hash({"task": before, "fields": fields}) or "", task, json.dumps(inputs, sort_keys=True))


def send(plan: TaskUpdatePlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    def verify(_response):
        exact = next((r for r in inventory(client, plan.target) if positive_id(r["id"]) == plan.task_id), {})
        results = {}
        for key, expected in plan.payload.items():
            actual_key = "agent_id" if key == "owner_id" and key not in exact else key
            actual = exact.get(actual_key)
            if actual_key not in exact:
                results[key] = "unavailable"
                continue
            if key.endswith("_date"):
                matched = same_instant(actual, expected)
            elif key == "description":
                matched = isinstance(actual, str) and strip_html(actual).strip() == strip_html(expected).strip()
            else:
                matched = submitted_value_matches(actual, expected)
            results[key] = "matched" if matched else "mismatched"
        return {"verified": bool(exact) and all(v == "matched" for v in results.values()), "fields": results, "readback": exact}
    return execute(plan, client, reviewed_hash=reviewed_hash,
                   rebuild=lambda: build_plan(client, plan.target, plan.task_id, json.loads(plan.inputs_json)), verify=verify)
