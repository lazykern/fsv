"""Captured explicit-ticket bulk priority request; no other bulk operations."""
from __future__ import annotations

import json
from collections.abc import Sequence
from dataclasses import dataclass
from typing import Any

from fsv import config, journal, ticket_properties
from fsv.resources import TICKETS, parse_id
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute


@dataclass(frozen=True)
class BulkPriorityPlan:
    write: WritePlan
    label: str
    before_json: str

    @property
    def hash(self) -> str:
        return self.write.hash

    def review(self) -> dict[str, Any]:
        return {**self.write.review(), "before": json.loads(self.before_json),
                "priority_label": self.label,
                "warning": "Explicit tickets only. Bulk actions may notify people; partial per-ticket outcomes are possible."}


def build_plan(client: Any, ids: Sequence[str | int], priority: str) -> BulkPriorityPlan:
    if not ids or len(ids) > 100:
        raise ValueError("specify between 1 and 100 explicit ticket IDs")
    cids = [parse_id(str(value), TICKETS) for value in ids]
    if any(cid < 1 for cid in cids) or len(set(cids)) != len(cids):
        raise ValueError("ticket IDs must be positive and unique")
    plans = [ticket_properties.build_plan(client, cid, answers={"priority": priority}) for cid in cids]
    enum_ids = {p.payload.get("priority") for p in plans}
    if len(enum_ids) != 1 or None in enum_ids:
        raise ReviewedWriteError("ticket metadata resolves different priorities; separate reviews are required")
    body = {"priority": next(iter(enum_ids)), "ids": cids, "reply": {}}
    context = journal.plan_hash([p.hash for p in plans]) or ""
    write = WritePlan("ticket_bulk_priority", cids[0], config.DOMAIN, "PUT",
                      "bulk_ticket_actions/update_multiple", json.dumps(body, sort_keys=True), context)
    before = [{"ticket_id": p.ticket_id, "priority": p.before.get("priority")} for p in plans]
    return BulkPriorityPlan(write, priority, json.dumps(before))


def send(plan: BulkPriorityPlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    def rebuild() -> WritePlan:
        return build_plan(client, plan.write.payload["ids"], plan.label).write
    def verify(response: Any) -> dict[str, Any]:
        fields = {}
        expected = plan.write.payload["priority"]
        for cid in plan.write.payload["ids"]:
            try:
                row = ticket_properties._record(client, cid)
                fields[str(cid)] = ("unavailable" if "priority" not in row else
                                    "matched" if row["priority"] == expected else "mismatched")
            except Exception:
                fields[str(cid)] = "unavailable"
        return {"tickets": fields, "verified": all(value == "matched" for value in fields.values())}
    return execute(plan.write, client, reviewed_hash=reviewed_hash, rebuild=rebuild, verify=verify)
