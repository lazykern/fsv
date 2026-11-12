"""Reviewed ticket task lifecycle using fresh ticket-task metadata."""
from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any

from fsv import config, journal, task_status
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


# Required on the task form but supplied by the parent ticket: the accepted
# web create body omits it, so it is neither asked for nor sent.
CONTEXT_FIELDS = frozenset({"workspace_id"})


def required_for_create(field: dict[str, Any]) -> bool:
    options = field.get("field_options") or {}
    return any(value is True or value == "true" for value in (
        *(field.get(k) for k in ("required", "required_for_create", "required_on_create", "required_for_agents")),
        options.get("required_for_create")))


def reminder(fields: list[dict[str, Any]]) -> int:
    matches = [f for f in fields if f.get("name") == "notify_before"]
    if len(matches) != 1 or any(matches[0].get(k) for k in ("deleted", "is_deleted", "deleted_at")):
        raise ReviewedWriteError("unique active runtime notify_before metadata unavailable")
    choices = [c for c in matches[0].get("choices", []) if isinstance(c, dict)
               and not any(c.get(k) for k in ("deleted", "is_deleted", "deleted_at", "deactivated"))
               and str(c.get("value") or c.get("label") or "").casefold() == "never"]
    ident = choices[0].get("id") if len(choices) == 1 else None
    if isinstance(ident, bool) or not str(ident).isascii() or not str(ident).isdigit():
        raise ReviewedWriteError("active runtime notify_before choice is unavailable or ambiguous")
    return int(str(ident))


def custom_values(fields: list[dict[str, Any]], values: dict[str, Any]) -> dict[str, Any]:
    result = {}
    for key, value in values.items():
        matches = [f for f in fields if key.casefold() in (str(f.get("name", "")).casefold(), str(f.get("label", "")).casefold())]
        if len(matches) != 1:
            raise ValueError("custom field must match unique runtime name or label: " + key)
        field = matches[0]
        name = field.get("name")
        if not (field.get("custom") is True or field.get("is_custom") is True or field.get("default") is False or str(field.get("field_type", "")).startswith("custom_")):
            raise ValueError("--set accepts only proven custom task fields")
        if any(field.get(k) for k in ("deleted", "is_deleted", "deleted_at")):
            raise ValueError("inactive task field")
        if field.get("choices"):
            try:
                value = task_status.resolve(str(value), [{**field, "name": "status"}])[0]
            except ValueError:
                offered = [label for _, label in task_status.active_choices([{**field, "name": "status"}])]
                raise ValueError(f"{field.get('label') or name}: {value!r} is not an active choice; offered: "
                                 + ", ".join(offered)) from None
        elif str(field.get("field_type", "")) in ("custom_text", "custom_paragraph") or field.get("dom_type") in ("text", "textarea"):
            if not isinstance(value, str):
                raise ValueError("task text field requires text")
        else:
            raise ValueError("unproven task custom field type: " + str(name))
        # The accepted web create sends native custom task fields without the
        # metadata's "cf_" prefix (custom_fields.<key>).
        if field.get("default") is False and isinstance(name, str) and name.startswith("cf_"):
            name = name[3:]
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
            payload["title"] = title
            if any(f.get("name") == "notify_before" for f in fields):
                payload["notify_before"] = reminder(fields)
            custom = custom_values(fields, values or {})
            def answered(field: dict[str, Any]) -> bool:
                name = str(field.get("name"))
                if name in payload or name in CONTEXT_FIELDS:
                    return True
                key = name[3:] if field.get("default") is False and name.startswith("cf_") else name
                return custom.get(key) not in (None, "")
            missing = [str(f.get("label") or f.get("name")) for f in fields if required_for_create(f) and not answered(f)]
            if missing:
                raise ReviewedWriteError("required task fields missing: " + ", ".join(missing))
            payload["custom_fields"] = custom
    inputs = {"action": action, "task": task, "title": title, "status": status, "values": values}
    return TaskPlan("ticket_task_" + action, target, config.DOMAIN, "POST" if action == "add" else "PUT",
                    path, json.dumps(payload, sort_keys=True), journal.snapshot_hash({"before": before, "fields": fields}) or "",
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
