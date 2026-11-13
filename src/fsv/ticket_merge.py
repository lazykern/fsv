"""Reviewed, irreversible ticket merges using the accepted complete-merge contract."""
from __future__ import annotations

import json
from typing import Any, Sequence

from fsv import config, journal
from fsv.reviewed_snapshot import stable_snapshot
from fsv.formmeta import is_status, load_meta
from fsv.readback import same_text, submitted_value_matches
from fsv.resources import TICKETS, parse_id
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute
from fsv.ticket_properties import _record
from fsv.views import load_conversations


def _id(value: str | int) -> int:
    if isinstance(value, bool) or not isinstance(value, (str, int)):
        raise ValueError("ticket ID must be a positive display ID")
    result = parse_id(str(value), TICKETS)
    if result <= 0:
        raise ValueError("ticket ID must be positive")
    return result


def _history(client: Any, ticket: int) -> list[dict[str, Any]]:
    rows, meta, missing = load_conversations(client, ticket, all_pages=True)
    if missing or not meta.get("complete") or meta.get("partial"):
        raise ReviewedWriteError("complete conversation history is required; no merge attempted")
    ids = [row.get("id") for row in rows]
    if any(isinstance(i, bool) or not str(i).isdigit() or int(str(i)) <= 0 for i in ids) or len(set(map(str, ids))) != len(ids):
        raise ReviewedWriteError("conversation history lacks unique IDs; no merge attempted")
    return rows


def _closed(client: Any, ticket: int) -> tuple[int, str]:
    meta = load_meta(client, "ticket", "edit", fresh=True, module_id=ticket)
    fields = [f for f in meta.spec.fields if is_status(f)]
    from fsv import status_semantics
    from fsv.choice_ids import runtime_choice_id
    try:
        # the built-in Closed status by internal id, whatever its label
        choice = status_semantics.find([c for c in fields[0].choices if c.visible], "ticket",
                                       status_semantics.CLOSED) if len(fields) == 1 else None
    except status_semantics.StatusSemanticsError:
        choice = None
    ident = runtime_choice_id(choice.id) if choice is not None else None
    if ident is None:
        raise ReviewedWriteError("a unique runtime Closed status ID is required")
    return ident, meta.fingerprint.hash


def build_plan(client: Any, primary: str | int, secondaries: Sequence[str | int], *,
               note_primary: str, note_secondary: str, private_primary: bool = True,
               private_secondary: bool = True) -> WritePlan:
    target = _id(primary)
    if isinstance(secondaries, (str, bytes)) or not secondaries:
        raise ValueError("at least one secondary ticket is required")
    ids = [_id(value) for value in secondaries]
    if target in ids or len(ids) != len(set(ids)):
        raise ValueError("primary and secondary tickets must be distinct")
    if any(not isinstance(n, str) or not n.strip() or "\x00" in n for n in (note_primary, note_secondary)):
        raise ValueError("both merge notes must be nonempty text")
    if type(private_primary) is not bool or type(private_secondary) is not bool:
        raise ValueError("note visibility must be boolean")
    records = {str(i): _record(client, i) for i in [target, *ids]}
    histories = {str(i): _history(client, i) for i in [target, *ids]}
    closed = {str(i): _closed(client, i) for i in ids}
    context = {"records": records, "histories": histories, "closed": closed}
    payload = {"target": {"ticket_id": target, "note": note_primary, "is_private": private_primary},
               "source": {"ticket_ids": ids, "note": note_secondary, "is_private": private_secondary}}
    return WritePlan("ticket_merge", target, config.DOMAIN, "POST", "merge_tickets/complete_merge",
                     json.dumps(payload, sort_keys=True, ensure_ascii=False), json.dumps(stable_snapshot(context), sort_keys=True, ensure_ascii=False))


def review(plan: WritePlan) -> dict[str, Any]:
    return {**plan.review(), "surviving_ticket": plan.target,
            "merged_tickets": plan.payload["source"]["ticket_ids"],
            "warning": "Irreversible: secondary tickets are closed and their conversations move to the surviving primary. Tenant workflows may notify people. Unknown outcomes must not be resent."}


def send(plan: WritePlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    payload = plan.payload
    target, source = payload["target"], payload["source"]
    before = json.loads(plan.context_hash)

    def rebuild() -> WritePlan:
        return build_plan(client, target["ticket_id"], source["ticket_ids"], note_primary=target["note"],
                          note_secondary=source["note"], private_primary=target["is_private"],
                          private_secondary=source["is_private"])

    def verify(_response: Any) -> dict[str, Any]:
        primary_record = _record(client, plan.target)
        primary_history = _history(client, plan.target)
        primary_ids = {str(row["id"]) for row in primary_history}
        prior_primary_ids = {str(row["id"]) for row in before["histories"][str(plan.target)]}
        new_primary = [row for row in primary_history if str(row["id"]) not in prior_primary_ids]
        primary_note = any(same_text(row.get("body"), target["note"]) and row.get("private") is target["is_private"]
                           for row in new_primary)
        unchanged = ("status" in primary_record and "status" in before["records"][str(plan.target)] and
                     submitted_value_matches(primary_record["status"], before["records"][str(plan.target)]["status"]))
        checks = {}
        for sid in source["ticket_ids"]:
            record = _record(client, sid)
            history = _history(client, sid)
            original = {str(row["id"]) for row in before["histories"][str(sid)]}
            new = [row for row in history if str(row["id"]) not in original]
            checks[str(sid)] = {"closed": "status" in record and submitted_value_matches(record["status"], before["closed"][str(sid)][0]),
                                "conversations_moved": original.issubset(primary_ids),
                                "source_note": any(same_text(row.get("body"), source["note"]) and row.get("private") is source["is_private"] for row in new)}
        return {"surviving_ticket": plan.target, "primary_status_unchanged": unchanged,
                "primary_note": primary_note, "secondaries": checks,
                "verified": bool(unchanged and primary_note and all(all(v.values()) for v in checks.values()))}

    from fsv.readback import bounded_readback
    return execute(plan, client, reviewed_hash=reviewed_hash, rebuild=rebuild, verify=lambda response: bounded_readback(lambda: verify(response)))
