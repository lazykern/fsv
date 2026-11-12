"""Reviewed problem-to-incident association mutations."""
from __future__ import annotations

import json
from typing import Any

from fsv import config, journal, views
from fsv.change_associations import AssociationReader
from fsv.resources import PROBLEMS, TICKETS, parse_id
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute


def _inventory(client: Any, problem_id: int) -> list[dict[str, Any]]:
    rows, meta, missing = views.collect_pages(AssociationReader(client), f"problems/{problem_id}/tickets", "tickets",
                                              {"page": 1, "per_page": 50}, what="tickets")
    if missing or meta.get("complete") is not True:
        raise ReviewedWriteError("complete problem ticket inventory unavailable; no write attempted")
    ids = [str(row["id"]) for row in rows]
    if len(ids) != len(set(ids)):
        raise ReviewedWriteError("problem ticket inventory has duplicate IDs")
    return rows


def build_plan(client: Any, problem: str | int, incident: str | int, *, unlink: bool = False) -> WritePlan:
    if isinstance(incident, str) and incident.upper().startswith("SR-"):
        raise ValueError("incident ID must use the INC prefix, not SR")
    pid, tid = parse_id(str(problem), PROBLEMS), parse_id(str(incident), TICKETS)
    if pid <= 0 or tid <= 0 or type(unlink) is not bool:
        raise ValueError("problem and incident IDs must be positive")
    raw_problem = client.int_get(f"problems/{pid}")
    raw_ticket = client.int_get(f"tickets/{tid}")
    pr = raw_problem.get("problem", raw_problem) if isinstance(raw_problem, dict) else None
    ticket = raw_ticket.get("ticket", raw_ticket) if isinstance(raw_ticket, dict) else None
    if not isinstance(pr, dict) or str(pr.get("id")) != str(pid):
        raise ReviewedWriteError("problem read did not identify exact target")
    if not isinstance(ticket, dict) or str(ticket.get("id")) != str(tid):
        raise ReviewedWriteError("incident read did not identify exact target")
    kind = ticket.get("type")
    display = ticket.get("human_display_id")
    if kind != "Incident" or (display is not None and display != f"INC-{tid}"):
        raise ReviewedWriteError("ticket is not a verified incident")
    rows = _inventory(client, pid)
    present = any(str(row["id"]) == str(tid) for row in rows)
    if present != unlink:
        raise ReviewedWriteError("incident is already in the requested association state")
    path = f"problems/{pid}/tickets/{tid if unlink else 'associate'}"
    payload = {"type": "unlink"} if unlink else {"item_ids": [tid]}
    context = journal.snapshot_hash({"problem": pr, "ticket": ticket, "inventory": rows}) or ""
    return WritePlan("problem_unlink_incident" if unlink else "problem_link_incident", pid, config.DOMAIN,
                     "DELETE" if unlink else "PUT", path, json.dumps(payload, sort_keys=True), context)


def send(plan: WritePlan, client: Any, incident: str | int, *, reviewed_hash: str | None) -> dict[str, Any]:
    tid = parse_id(str(incident), TICKETS)
    unlink = plan.action == "problem_unlink_incident"
    def rebuild() -> WritePlan:
        return build_plan(client, plan.target, tid, unlink=unlink)
    def verify(_response: Any) -> dict[str, Any]:
        rows = _inventory(client, plan.target)
        present = any(str(row["id"]) == str(tid) for row in rows)
        return {"problem_id": plan.target, "incident_id": tid, "linked": present, "verified": present != unlink}
    return execute(plan, client, reviewed_hash=reviewed_hash, rebuild=rebuild, verify=verify)
