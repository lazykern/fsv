"""Reviewed native change-task deletion with complete exact-parent readback."""
from __future__ import annotations
import json
from typing import Any
from fsv import config, journal
from fsv.change_task_update import inventory
from fsv.resources import CHANGES, parse_id
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute
from fsv.watchers import positive_id


def build_plan(client: Any, target: str | int, task: int) -> WritePlan:
    target = positive_id(parse_id(str(target), CHANGES))
    task = positive_id(task)
    rows = inventory(client, target)
    if not any(positive_id(row["id"]) == task for row in rows):
        raise ReviewedWriteError("exact task unavailable")
    return WritePlan("change_task_delete", target, config.DOMAIN, "PUT", f"changes/{target}/tasks/update",
                     json.dumps({"id": str(task), "action": "delete"}, sort_keys=True),
                     journal.snapshot_hash(rows) or "")


def send(plan: WritePlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    task = positive_id(plan.payload["id"])
    before = {positive_id(row["id"]) for row in inventory(client, plan.target)}
    def verify(_response):
        rows = inventory(client, plan.target)
        actual = {positive_id(row["id"]) for row in rows}
        return {"task_id": task, "verified": actual == before - {task}, "readback": rows}
    return execute(plan, client, reviewed_hash=reviewed_hash,
                   rebuild=lambda: build_plan(client, plan.target, task), verify=verify)
