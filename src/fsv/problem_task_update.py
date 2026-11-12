"""Captured problem-task updates and completion; exact task GET verifies all fields."""
from __future__ import annotations
import html
import json
from dataclasses import dataclass
from datetime import datetime
from typing import Any
from fsv import config, journal, task_status
from fsv.change_tasks import TaskError, _date, _effort, _note, resolve_group, resolve_owner, same_effort
from fsv.change_task_update import reminder
from fsv.readback import same_instant, same_text, submitted_value_matches
from fsv.resources import PROBLEMS, parse_id
from fsv.attachments import owner_row
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute
from fsv.reviewed_snapshot import stable_snapshot
from fsv.reviewed_inventory import complete_inventory
from fsv.watchers import positive_id


@dataclass(frozen=True)
class TaskUpdatePlan(WritePlan):
    task_id: int
    inputs_json: str


def inventory(client: Any, target: int) -> list[dict[str, Any]]:
    rows = complete_inventory(client, f"problems/{target}/tasks", "tasks", {"page": 1, "per_page": 100})
    ids = [positive_id(r.get("id")) for r in rows]
    if len(set(ids)) != len(ids):
        raise ReviewedWriteError("duplicate task identities")
    return rows


def fields_from(client: Any) -> list[dict]:
    raw = client.int_get("problem_task_fields", {"include_deleted_choices": "true"})
    for key in ("problem_task_fields", "task_fields", "fields"):
        if isinstance(raw, dict) and isinstance(raw.get(key), list):
            return raw[key]
    raise ReviewedWriteError("runtime problem-task fields unavailable")


def exact_task(client: Any, target: int, task: int) -> dict:
    raw = client.int_get(f"problems/{target}/tasks/{task}")
    row = raw.get("task", raw) if isinstance(raw, dict) else {}
    if positive_id(row.get("id")) != task or str(row.get("taskable_display_id")) != str(target) or row.get("taskable_type") != "Itil::Problem":
        raise ReviewedWriteError("exact problem task identity/parent unavailable")
    return row


def build_plan(client: Any, target: str | int, task: int, inputs: dict[str, Any]) -> TaskUpdatePlan:
    target = positive_id(parse_id(str(target), PROBLEMS))
    task = positive_id(task)
    allowed = {"title", "status", "owner", "group", "note", "planned_start", "planned_end", "planned_effort", "notify_before", "due_date"}
    if not inputs or set(inputs) - allowed:
        raise ValueError("provide captured task fields; unsupported fields cannot be sent")
    before = next((r for r in inventory(client, target) if positive_id(r["id"]) == task), None)
    if before is None:
        raise ReviewedWriteError("exact task unavailable")
    before = exact_task(client, target, task)
    parent = owner_row(client, "problems", target)
    fields = fields_from(client)
    payload = {}
    for key, value in inputs.items():
        if key == "title":
            if not isinstance(value, str) or not value.strip() or any(ord(c) < 32 for c in value):
                raise ValueError("nonempty task title required")
            payload[key] = value
        elif key == "status":
            payload[key] = task_status.resolve(value, fields)[0]
        elif key in {"owner", "group"}:
            if value == "none":
                raise ReviewedWriteError("null owner/group update is not captured")
            try:
                payload[key + "_id"] = None if value == "none" else (resolve_owner(value, client) if key == "owner" else resolve_group(value, client))
            except TaskError as exc:
                raise ValueError(str(exc)) from None
        elif key == "note":
            _note(value)
            payload["description"] = "<div>" + html.escape(value).replace("\n", "<br>") + "</div>"
        elif key == "due_date":
            payload[key] = _date(value)
        elif key in {"planned_start", "planned_end"}:
            payload[key + "_date"] = _date(value)
        elif key == "planned_effort":
            payload[key] = _effort(value)
        elif key == "notify_before":
            payload[key] = reminder(value, fields)
    if any(v is None for v in payload.values()):
        raise ReviewedWriteError("clearing task fields is not a captured update")
    start = payload.get("planned_start_date", before.get("planned_start_date"))
    end = payload.get("planned_end_date", before.get("planned_end_date"))
    if start and end and datetime.fromisoformat(end.replace("Z", "+00:00")) < datetime.fromisoformat(start.replace("Z", "+00:00")):
        raise ValueError("planned end must not precede start")
    if "status" in payload:
        custom = {**before.get("custom_fields", {}), **payload.get("custom_fields", {})}
        label = task_status.resolve(str(payload["status"]), fields)[1]
        if label.casefold() == "completed":
            if set(payload) != {"status"}:
                raise ReviewedWriteError("captured completion is a separate status-only write")
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
    return TaskUpdatePlan("problem_task_update", target, config.DOMAIN, "PUT", f"problems/{target}/tasks/{task}",
                          json.dumps(payload, sort_keys=True), journal.snapshot_hash({"task": before, "parent": parent, "fields": fields}) or "", task, json.dumps(inputs, sort_keys=True))


def send(plan: TaskUpdatePlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    def verify(_response):
        exact = exact_task(client, plan.target, plan.task_id)
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
                matched = same_text(actual, expected)
            elif key == "planned_effort":
                matched = same_effort(actual, expected)
            else:
                matched = submitted_value_matches(actual, expected)
            results[key] = "matched" if matched else "mismatched"
        # Exact native problem-task GET exposes reminder and owner_id; missing is not proof.
        verified = bool(exact) and all(v == "matched" for v in results.values())
        result = {"verified": verified, "fields_verified": verified, "fields": results, "readback": exact,
                  "warning": "Task completion may rewrite planned dates and stop OLA timers; inspect the exact readback. Never resend."}
        return result
    return execute(plan, client, reviewed_hash=reviewed_hash,
                   rebuild=lambda: build_plan(client, plan.target, plan.task_id, json.loads(plan.inputs_json)), verify=verify)


def delete_plan(client: Any, target: str | int, task: int) -> WritePlan:
    target = positive_id(parse_id(str(target), PROBLEMS))
    task = positive_id(task)
    parent = owner_row(client, "problems", target)
    rows = inventory(client, target)
    if not any(positive_id(r["id"]) == task for r in rows):
        raise ReviewedWriteError("exact problem task unavailable")
    before = exact_task(client, target, task)
    return WritePlan("problem_task_delete", target, config.DOMAIN, "PUT", f"problems/{target}/tasks/update",
                     json.dumps({"id": str(task), "action": "delete"}, sort_keys=True),
                     json.dumps(stable_snapshot({"parent": parent, "rows": rows, "task": before}), sort_keys=True))


def delete(plan: WritePlan, client: Any, *, reviewed_hash: str | None) -> dict:
    task = positive_id(plan.payload["id"])
    def verify(_response):
        rows = inventory(client, plan.target)
        exact = [r for r in rows if positive_id(r["id"]) == task]
        before = {positive_id(r["id"]) for r in json.loads(plan.context_hash)["rows"]}
        after = {positive_id(r["id"]) for r in rows}
        removed = (not exact and after == before - {task}) or (
            len(exact) == 1 and exact[0].get("deleted") is True and after == before)
        return {"task_id": task, "verified": bool(removed)}
    return execute(plan, client, reviewed_hash=reviewed_hash,
                   rebuild=lambda: delete_plan(client, plan.target, task), verify=verify)
