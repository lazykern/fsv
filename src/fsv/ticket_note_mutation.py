"""Reviewed edits and deletes of exact ticket conversation notes."""
from __future__ import annotations

import json
from typing import Any

from fsv import config, journal
from fsv.readback import same_text
from fsv.resources import TICKETS, parse_id
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute
from fsv.ticket_notes import build_plan as note_plan
from fsv.views import load_conversations


def _note(client: Any, ticket_id: int, note_id: int) -> dict[str, Any]:
    rows, meta, missing = load_conversations(client, ticket_id, all_pages=True)
    if missing or meta.get("partial"):
        raise ReviewedWriteError("complete conversation history unavailable; no write attempted")
    matches = [r for r in rows if str(r.get("id")) == str(note_id)]
    if len(matches) != 1:
        raise ReviewedWriteError("exact note not found or ambiguous")
    row = matches[0]
    if type(row.get("source")) is not int or row["source"] != 2 or not isinstance(row.get("private"), bool):
        raise ReviewedWriteError("conversation is not a verified note")
    return row


def build_plan(client: Any, ticket_id: str | int, note_id: str | int, *, body: str | None = None) -> WritePlan:
    cid = parse_id(str(ticket_id), TICKETS)
    if cid <= 0 or isinstance(note_id, bool) or not str(note_id).isascii() or not str(note_id).isdigit() or int(note_id) <= 0:
        raise ValueError("ticket and conversation IDs must be positive")
    nid = int(note_id)
    before = _note(client, cid, nid)
    action = "ticket_note_delete" if body is None else "ticket_note_edit"
    payload = None if body is None else {"body": note_plan(cid, body).body_html, "conversation_from_summary": False}
    return WritePlan(action, cid, config.DOMAIN, "DELETE" if body is None else "PUT",
                     f"conversations/{nid}", json.dumps(payload, sort_keys=True, ensure_ascii=False),
                     journal.snapshot_hash(before) or "")


def review(plan: WritePlan, client: Any) -> dict[str, Any]:
    before = _note(client, plan.target, int(plan.path.rsplit("/", 1)[-1]))
    if journal.snapshot_hash(before) != plan.context_hash:
        raise ReviewedWriteError("note changed since review; review again")
    return {**plan.review(), "ticket_id": plan.target, "before": before}


def send(plan: WritePlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    nid = int(plan.path.rsplit("/", 1)[-1])
    body_html = plan.payload.get("body") if isinstance(plan.payload, dict) else None
    def rebuild() -> WritePlan:
        before = _note(client, plan.target, nid)
        return WritePlan(plan.action, plan.target, config.DOMAIN, plan.method, plan.path, plan.payload_json,
                         journal.snapshot_hash(before) or "")
    def verify(_response: Any) -> dict[str, Any]:
        rows, meta, missing = load_conversations(client, plan.target, all_pages=True)
        complete = not missing and not meta.get("partial")
        found = [r for r in rows if str(r.get("id")) == str(nid)]
        if plan.method == "DELETE":
            return {"conversation_id": nid, "history_complete": complete, "verified": bool(complete and not found)}
        status = "unavailable" if len(found) != 1 or "body" not in found[0] else (
            "matched" if same_text(found[0]["body"], body_html) else "mismatched")
        return {"conversation_id": nid, "history_complete": complete, "body": status,
                "verified": bool(complete and status == "matched")}
    return execute(plan, client, reviewed_hash=reviewed_hash, rebuild=rebuild, verify=verify)
