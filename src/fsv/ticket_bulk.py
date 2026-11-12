"""Captured bulk wires, frozen explicit selection and per-ticket readback."""
from __future__ import annotations

import json
from dataclasses import dataclass, replace
from typing import Any
from urllib.parse import urlencode

from fsv import config, journal, ticket_assignment, ticket_merge, ticket_properties
from fsv.query import build_query_hash
from fsv.resources import TICKETS, parse_id
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute
from fsv.reviewed_inventory import complete_inventory
from fsv.watchers import positive_id


def selection(client: Any, ids: list[str], where: list[str], select_all: bool) -> list[int]:
    if where:
        if ids or not select_all:
            raise ValueError("filters require --select-all and no explicit IDs")
        query, _, _ = build_query_hash(client, TICKETS, where)
        rows = complete_inventory(client, "tickets", "tickets",
                                  {"query_hash": query, "page": 1, "per_page": 100})
        values = [positive_id(r.get("id")) for r in rows]
    else:
        if select_all:
            raise ValueError("--select-all requires an explicit filter")
        values = [positive_id(parse_id(v, TICKETS)) for v in ids]
    if not values or len(values) > 100 or len(values) != len(set(values)):
        raise ValueError("select 1 to 100 unique tickets; never silently truncate or chunk")
    return sorted(values)


@dataclass(frozen=True)
class BulkPlan:
    write: WritePlan
    intent_json: str
    ids: tuple[int, ...]
    expected_json: str
    delegate: Any = None

    @property
    def hash(self) -> str:
        return journal.plan_hash({"write": self.write.hash, "selection": json.loads(self.intent_json)}) or ""

    def review(self) -> dict[str, Any]:
        return {**self.write.review(), "plan_hash": self.hash, "ticket_ids": list(self.ids),
                "selection": json.loads(self.intent_json),
                "warning": "Frozen explicit IDs, not server select-all. Partial outcomes are possible; never replay an uncertain bulk action."}


def build_plan(client: Any, ids: list[str], *, action: str, where: list[str] | None = None,
               select_all: bool = False, status: str | None = None, agent: str | None = None,
               group: str | None = None, primary: str | None = None,
               note_primary: str | None = None, note_secondary: str | None = None) -> BulkPlan:
    where = where or []
    cids = selection(client, ids, where, select_all)
    intent = dict(ids=ids, action=action, where=where, select_all=select_all, status=status,
                  agent=agent, group=group, primary=primary, note_primary=note_primary, note_secondary=note_secondary)
    if action not in {"status", "assign", "pickup", "merge", "spam", "unspam", "delete"}:
        raise ValueError("unsupported bulk action")
    if (status is not None and action != "status") or ((agent or group) and action != "assign") or (
            (primary or note_primary or note_secondary) and action != "merge"):
        raise ValueError("options do not match bulk action")
    delegate = None
    if action == "pickup":
        delegate = ticket_assignment.build_plan(client, cids, pickup=True)
        write = delegate.write
        expected = json.loads(delegate.intent_json)["expected"]
    elif action == "merge":
        pid = positive_id(parse_id(primary or "", TICKETS))
        if pid not in cids:
            raise ValueError("merge primary must be in the printed selected ID list")
        delegate = ticket_merge.build_plan(client, pid, [i for i in cids if i != pid],
                                            note_primary=note_primary, note_secondary=note_secondary)
        write, expected = delegate, []
    else:
        baselines = [ticket_properties._record(client, i) for i in cids]
        if action == "status":
            if not status or status.casefold() in {"resolved", "closed"}:
                raise ValueError("bulk closure needs separate prerequisites; select an ordinary runtime status")
            props = [ticket_properties.build_plan(client, i, answers={"status": status}) for i in cids]
            statuses = {p.payload.get("status") for p in props}
            if len(statuses) != 1 or None in statuses:
                raise ReviewedWriteError("status metadata disagrees across selected tickets")
            fields = {"status": next(iter(statuses))}
        elif action == "assign":
            if not agent or not group:
                raise ValueError("captured bulk assignment requires both --agent and --group; null/unassignment is unproven")
            props = [ticket_assignment.build_plan(client, [i], agent=agent, group=group) for i in cids]
            fields = {k: props[0].write.payload[k] for k in ("group_id", "responder_id")}
            if any(any(p.write.payload.get(k) != v for k, v in fields.items()) for p in props):
                raise ReviewedWriteError("assignment metadata disagrees")
        else:
            props = []
            fields = {"deleted": True} if action == "delete" else {"spam": action == "spam"}
            if any(type(r.get(next(iter(fields)))) is not bool for r in baselines):
                raise ReviewedWriteError("spam/deleted baseline flag unavailable")
        expected = [fields for _ in cids]
        if action in {"status", "assign"}:
            method, path, body = "PUT", "bulk_ticket_actions/update_multiple", {**fields, "ids": cids, "reply": {}}
        elif action == "delete":
            method, path, body = "DELETE", "tickets/delete_multiple?" + urlencode({"ids": ",".join(map(str, cids))}), None
        else:
            method, path, body = "POST", f"tickets/{action}_multiple", {"ids": cids}
        write = WritePlan("ticket_bulk_" + action, cids[0], config.DOMAIN, method, path, json.dumps(body, sort_keys=True),
                          journal.snapshot_hash({"records": baselines, "plans": [p.hash for p in props]}) or "")
    # Bind filter intent and expanded IDs to every write, including delegates.
    return BulkPlan(write, json.dumps(intent, sort_keys=True), tuple(cids), json.dumps(expected), delegate)


def send(plan: BulkPlan, client: Any, *, reviewed_hash: str | None, confirm_destructive: bool = False) -> dict[str, Any]:
    if reviewed_hash is None:
        return {"status": "cancelled", "verified": False}
    if reviewed_hash != plan.hash:
        raise ReviewedWriteError("full bulk review hash changed")
    intent = json.loads(plan.intent_json)
    if intent["action"] in {"spam", "delete"} and confirm_destructive is not True:
        raise ReviewedWriteError("spam/delete require --confirm-destructive after inspecting the printed ID list")
    events = [e for e in journal.read() if e.get("kind") == "plan:" + plan.write.action and e.get("plan") == plan.write.hash]
    if events and events[-1].get("event") in {"started", "done"}:
        raise ReviewedWriteError("bulk write was sent or outcome unknown; do not replay")
    def rebuild():
        fresh = build_plan(client, **intent)
        if fresh.hash != plan.hash:
            raise ReviewedWriteError("bulk selection, metadata or records changed; review again")
        return fresh.write
    rebuild()
    if intent["action"] == "merge":
        return ticket_merge.send(plan.delegate, client, reviewed_hash=plan.delegate.hash)
    if intent["action"] == "pickup":
        return ticket_assignment.send(plan.delegate, client, reviewed_hash=plan.delegate.hash)
    def verify(_response):
        result = {}
        for cid, expected in zip(plan.ids, json.loads(plan.expected_json)):
            try:
                row = ticket_properties._record(client, cid)
                result[str(cid)] = {k: "unavailable" if k not in row else
                                    "matched" if type(row[k]) is type(v) and row[k] == v else "mismatched"
                                    for k, v in expected.items()}
            except Exception:
                result[str(cid)] = {k: "unavailable" for k in expected}
        return {"ticket_ids": list(plan.ids), "tickets": result,
                "verified": all(all(v == "matched" for v in fields.values()) for fields in result.values())}
    return execute(plan.write, client, reviewed_hash=plan.write.hash, rebuild=rebuild, verify=verify)
