"""Reviewed ticket routing with fresh, explicit agent/group membership."""
from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any, Sequence

from fsv import config, journal, ticket_properties
from fsv.readback import submitted_value_matches
from fsv.resources import TICKETS, parse_id
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute


def _positive(value: Any) -> int | None:
    if isinstance(value, bool) or not isinstance(value, (str, int)) or not str(value).isdigit() or int(value) <= 0:
        return None
    return int(value)


def _resolve(rows: Any, value: str, kind: str) -> dict[str, Any]:
    if not isinstance(rows, list) or not value.strip():
        raise ReviewedWriteError(f"runtime {kind} metadata unavailable")
    query = value.strip().casefold()
    matches = [r for r in rows if isinstance(r, dict) and not r.get("deleted") and not r.get("deactivated") and
               any(str(r.get(k, "")).casefold() == query for k in ("id", "name", "email"))]
    if len(matches) != 1 or _positive(matches[0].get("id")) is None:
        raise ReviewedWriteError(f"{kind} must identify exactly one active runtime entry")
    return matches[0]


def _member(agent: dict[str, Any], group: dict[str, Any]) -> None:
    aid, gid = _positive(agent.get("id")), _positive(group.get("id"))
    agent_groups, group_agents = agent.get("group_ids"), group.get("agent_ids")
    # Both sides must be explicit, consistent lists. Missing is not empty.
    if not isinstance(agent_groups, list) or not isinstance(group_agents, list):
        raise ReviewedWriteError("agent/group membership metadata unavailable")
    if any(_positive(v) is None for v in agent_groups + group_agents):
        raise ReviewedWriteError("agent/group membership metadata is invalid")
    if gid not in map(_positive, agent_groups) or aid not in map(_positive, group_agents):
        raise ReviewedWriteError("agent is not a verified member of the group")


@dataclass(frozen=True)
class AssignmentPlan:
    write: WritePlan
    intent_json: str
    before_json: str

    @property
    def hash(self) -> str:
        return self.write.hash

    def review(self) -> dict[str, Any]:
        return {**self.write.review(), "before": json.loads(self.before_json),
                "warning": "Assignment can notify people. Inspect each ticket after an unknown or partial outcome; never resend automatically."}


def build_plan(client: Any, ids: Sequence[str | int], *, agent: str | None = None,
               group: str | None = None, unassign: bool = False, pickup: bool = False) -> AssignmentPlan:
    if isinstance(ids, (str, bytes)) or not ids or len(ids) > 100:
        raise ValueError("specify between 1 and 100 ticket IDs")
    cids = [parse_id(str(value), TICKETS) for value in ids]
    if any(i <= 0 for i in cids) or len(set(cids)) != len(cids):
        raise ValueError("ticket IDs must be positive and unique")
    if pickup and (agent is not None or group is not None or unassign):
        raise ValueError("pick-up does not accept assignment options")
    if not pickup and (len(cids) != 1 or (unassign and (agent is not None or group is not None)) or
                       (not unassign and agent is None and group is None)):
        raise ValueError("assign one ticket with --agent and/or --group, or --unassign")
    boot = client.int_get("bootstrap/agents_groups")
    if not isinstance(boot, dict) or not isinstance(boot.get("agents"), list) or not isinstance(boot.get("groups"), list):
        raise ReviewedWriteError("runtime agent/group metadata unavailable")
    agent_row = None
    if pickup:
        me = client.me()
        mid = _positive(me.get("id")) if isinstance(me, dict) else None
        if mid is None:
            raise ReviewedWriteError("current agent identity unavailable")
        agent_row = _resolve(boot["agents"], str(mid), "agent")
    elif agent is not None:
        agent_row = _resolve(boot["agents"], agent, "agent")
    group_row = _resolve(boot["groups"], group, "group") if group is not None else None
    before = []
    plans = []
    for cid in cids:
        row = ticket_properties._record(client, cid)
        if "group_id" not in row or "responder_id" not in row:
            raise ReviewedWriteError("ticket routing fields unavailable")
        selected_group = group_row
        if group is None and row["group_id"] is not None and agent_row is not None:
            selected_group = _resolve(boot["groups"], str(row["group_id"]), "group")
        if selected_group is not None and agent_row is not None:
            _member(agent_row, selected_group)
        answers: dict[str, Any] = {}
        if group_row is not None:
            answers["group"] = _positive(group_row["id"])
            # Native group assignment clears the previous agent unless supplied.
            answers["agent"] = _positive(agent_row["id"]) if agent_row is not None else None
        elif unassign:
            answers["agent"] = None
        elif agent_row is not None:
            answers["agent"] = _positive(agent_row["id"])
        prop = ticket_properties.build_plan(client, cid, answers=answers)
        plans.append(prop)
        before.append({"ticket_id": cid, "group_id": row["group_id"], "responder_id": row["responder_id"]})
    if pickup:
        path, body, action = "tickets/pick_up", {"ids": cids}, "ticket_pick_up"
    else:
        path, body, action = f"tickets/{cids[0]}", plans[0].payload, "ticket_assign"
    context = journal.snapshot_hash({"properties": [p.hash for p in plans], "bootstrap": boot,
                                 "pickup_agent": _positive(agent_row["id"]) if pickup and agent_row else None}) or ""
    write = WritePlan(action, cids[0], config.DOMAIN, "PUT", path, json.dumps(body, sort_keys=True), context)
    intent = {"ids": cids, "agent": agent, "group": group, "unassign": unassign, "pickup": pickup,
              "expected": [{k: v for k, v in p.payload.items() if k != "br_validation_excludes"} for p in plans]}
    return AssignmentPlan(write, json.dumps(intent, sort_keys=True), json.dumps(before, sort_keys=True))


def send(plan: AssignmentPlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    intent = json.loads(plan.intent_json)
    def rebuild() -> WritePlan:
        fresh = build_plan(client, intent["ids"], agent=intent["agent"], group=intent["group"],
                           unassign=intent["unassign"], pickup=intent["pickup"])
        if fresh.intent_json != plan.intent_json:
            raise ReviewedWriteError("assignment intent changed; review again")
        return fresh.write
    def verify(_response: Any) -> dict[str, Any]:
        results = {}
        for cid, expected in zip(intent["ids"], intent["expected"]):
            try:
                row = ticket_properties._record(client, cid)
                fields = {k: ("unavailable" if k not in row else "matched" if submitted_value_matches(row[k], v) else "mismatched")
                          for k, v in expected.items()}
            except Exception:
                fields = {k: "unavailable" for k in expected}
            results[str(cid)] = fields
        return {"tickets": results, "verified": all(fields and all(v == "matched" for v in fields.values()) for fields in results.values())}
    return execute(plan.write, client, reviewed_hash=reviewed_hash, rebuild=rebuild, verify=verify)
