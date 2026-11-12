"""Reviewed ticket task lifecycle using fresh ticket-task metadata."""
from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any

from fsv import config, journal, task_status
from fsv.problem_workflows import _choice
from fsv.readback import submitted_value_matches
from fsv.resources import TICKETS, parse_id
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute
from fsv.views import collect_pages
from fsv.watchers import positive_id


@dataclass(frozen=True)
class TaskPlan(WritePlan):
    inputs_json: str


def inventory(client: Any, target: int) -> list[dict[str, Any]]:
    rows, meta, _ = collect_pages(client, f"tickets/{target}/tasks", "tasks",
                                  {"page": 1, "per_page": 30}, what="ticket tasks")
    if not meta.get("complete"):
        raise ReviewedWriteError("complete ticket task inventory unavailable")
    ids = [positive_id(r.get("id")) for r in rows]
    if len(set(ids)) != len(ids):
        raise ReviewedWriteError("ambiguous task identities")
    return sorted(rows, key=lambda r: positive_id(r["id"]))


def fields_from(client: Any) -> list[dict[str, Any]]:
    raw = client.int_get("ticket_task_fields")
    for key in ("ticket_task_fields", "task_fields", "fields"):
        if isinstance(raw, dict) and isinstance(raw.get(key), list) and all(isinstance(f, dict) for f in raw[key]):
            return raw[key]
    raise ReviewedWriteError("ticket task metadata unavailable")


def custom_values(fields: list[dict[str, Any]], values: dict[str, Any]) -> dict[str, Any]:
    result = {}
    for key, value in values.items():
        matches = [f for f in fields if key.casefold() in (str(f.get("name", "")).casefold(), str(f.get("label", "")).casefold())]
        if len(matches) != 1:
            raise ValueError("custom field must match unique runtime name or label: " + key)
        field = matches[0]
        name = field.get("name")
        if not (field.get("custom") is True or field.get("is_custom") is True or str(field.get("field_type", "")).startswith("custom_")):
            raise ValueError("--set accepts only proven custom task fields")
        if any(field.get(k) for k in ("deleted", "is_deleted", "deleted_at")):
            raise ValueError("inactive task field")
        if field.get("choices"):
            value = task_status.resolve(str(value), [{**field, "name": "status"}])[0]
        elif str(field.get("field_type", "")) in ("custom_text", "custom_paragraph"):
            if not isinstance(value, str):
                raise ValueError("task text field requires text")
        else:
            raise ValueError("unproven task custom field type: " + str(name))
        if name in result:
            raise ValueError("duplicate custom field assignment")
        result[name] = value
    return result


def build_plan(client: Any, target: str | int, *, action: str, task: str | int | None = None,
               title: str | None = None, status: str = "Open", values: dict[str, Any] | None = None) -> TaskPlan:
    target = positive_id(parse_id(str(target), TICKETS))
    if action not in {"add", "status", "delete"}:
        raise ValueError("unknown task action")
    before = inventory(client, target)
    fields = fields_from(client)
    tid = positive_id(task) if action != "add" else None
    if tid is not None and len([r for r in before if positive_id(r["id"]) == tid]) != 1:
        raise ReviewedWriteError("exact task unavailable")
    path = f"tickets/{target}/tasks"
    if action == "delete":
        payload = {"id": str(tid), "action": "delete"}
        path += "/update"
    else:
        payload: dict[str, Any] = {"status": task_status.resolve(status, fields)[0]}
        if action == "status":
            path += f"/{tid}"
        else:
            if not isinstance(title, str) or not title.strip() or any(ord(c) < 32 for c in title):
                raise ValueError("nonempty task title without controls required")
            payload.update(title=title, notify_before=_choice(fields, "notify_before", "Never"))
            custom = custom_values(fields, values or {})
            for field in fields:
                if any(field.get(k) is True for k in ("required", "required_for_create", "required_on_create", "required_for_agents")):
                    name = field.get("name")
                    if name not in payload and (name not in custom or custom[name] in (None, "")):
                        raise ReviewedWriteError("required task field missing: " + str(field.get("label") or name))
            payload["custom_fields"] = custom
    inputs = {"action": action, "task": task, "title": title, "status": status, "values": values}
    return TaskPlan("ticket_task_" + action, target, config.DOMAIN, "POST" if action == "add" else "PUT",
                    path, json.dumps(payload, sort_keys=True), journal.plan_hash({"before": before, "fields": fields}) or "",
                    json.dumps(inputs, sort_keys=True))


def send(plan: TaskPlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    inputs = json.loads(plan.inputs_json)
    before_ids = {positive_id(r["id"]) for r in inventory(client, plan.target)}
    def verify(response: Any) -> dict[str, Any]:
        rows = inventory(client, plan.target)
        if inputs["action"] == "delete":
            tid = positive_id(inputs["task"])
            return {"task_id": tid, "verified": not any(positive_id(r["id"]) == tid for r in rows)}
        if inputs["action"] == "add":
            raw = response.get("task", response) if isinstance(response, dict) else {}
            tid = positive_id(raw.get("id"))
            if tid in before_ids:
                return {"task_id": tid, "verified": False}
        else:
            tid = positive_id(inputs["task"])
        matches = [r for r in rows if positive_id(r["id"]) == tid]
        row = matches[0] if len(matches) == 1 else {}
        fields = {k: "unavailable" if k not in row else "matched" if submitted_value_matches(row[k], v) else "mismatched"
                  for k, v in plan.payload.items()}
        return {"task_id": tid, "fields": fields, "verified": bool(row) and all(v == "matched" for v in fields.values())}
    return execute(plan, client, reviewed_hash=reviewed_hash,
                   rebuild=lambda: build_plan(client, plan.target, **inputs), verify=verify)
