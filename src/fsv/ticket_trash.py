"""Ticket trash: list, restore, delete forever (captured from the Trash view).

* list: ``GET tickets?filter=deleted&...`` (the Trash filter id is ``deleted``).
* restore: ``PUT tickets/{id}/restore`` (no body, 204), one request per ticket;
  readback ``GET tickets/{id}`` shows ``deleted: false``.
* delete forever: ``DELETE tickets/delete_forever?ids={id}`` (204, no confirm
  in the UI); readback ``GET tickets/{id}`` answers 404. Only the single-id
  form is captured, so several IDs are sent as one request each, in order,
  stopping at the first failed or unverified ticket.
"""
from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any
from urllib.parse import urlencode

from fsv import config, journal
from fsv.errors import APIError
from fsv.resources import TICKETS, parse_id
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute
from fsv.ticket_properties import _record
from fsv.watchers import positive_id

MAX_FOREVER = 50
LIST_INCLUDE = "stats,responder,requester,ticket_states,ticket_status,group"


def _exact(client: Any, tid: int) -> dict[str, Any]:
    try:
        return _record(client, tid)
    except APIError as exc:
        if exc.status == 404:
            raise ReviewedWriteError(f"ticket {tid} not found (HTTP 404); nothing was sent") from None
        raise
    except ReviewedWriteError:
        raise
    except Exception as exc:  # PropertyError: exact identity
        raise ReviewedWriteError(str(exc)) from None


def _deleted(row: dict[str, Any], tid: int) -> bool:
    flag = row.get("deleted")
    if type(flag) is not bool:
        raise ReviewedWriteError(f"ticket {tid}: deleted flag unavailable; nothing was sent")
    return flag


def trash(client: Any, *, page: int = 1, per_page: int = 30) -> dict[str, Any]:
    if type(page) is not int or page < 1 or type(per_page) is not int or not 1 <= per_page <= 100:
        raise ValueError("page must be positive and per-page 1 to 100")
    raw = client.int_get("tickets", {"advanced_query_hash": "", "filter": "deleted", "include": LIST_INCLUDE,
                                     "order_by": "created_at", "order_type": "desc", "page": page,
                                     "per_page": per_page, "query_hash": ""})
    rows = raw.get("tickets") if isinstance(raw, dict) else None
    if not isinstance(rows, list) or any(not isinstance(r, dict) for r in rows):
        raise ReviewedWriteError("trash list is unreadable")
    meta = raw["meta"] if isinstance(raw, dict) and isinstance(raw.get("meta"), dict) else {}
    out = [{k: r.get(k) for k in ("id", "human_display_id", "subject", "deleted", "updated_at")} for r in rows]
    return {"tickets": out, "page": page, "has_next": meta.get("has_next"), "count": meta.get("count"),
            "note": "the trash holds every agent's deleted tickets; act on explicit IDs only"}


def build_restore(client: Any, ticket: str | int) -> WritePlan:
    tid = positive_id(parse_id(str(ticket), TICKETS))
    row = _exact(client, tid)
    if not _deleted(row, tid):
        raise ReviewedWriteError(f"ticket {tid} is not in the trash; nothing was sent")
    return WritePlan("ticket_restore", tid, config.DOMAIN, "PUT", f"tickets/{tid}/restore", "null",
                     journal.snapshot_hash({"ticket": row}) or "")


def send_restore(plan: WritePlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    def verify(_response: Any) -> dict[str, Any]:
        row = _record(client, plan.target)
        flag = row.get("deleted")
        return {"ticket_id": plan.target, "deleted": flag, "verified": flag is False}
    return execute(plan, client, reviewed_hash=reviewed_hash, verify=verify,
                   rebuild=lambda: build_restore(client, plan.target))


@dataclass(frozen=True)
class ForeverPlan:
    plans: tuple[WritePlan, ...]
    subjects_json: str
    max_records: int

    @property
    def ids(self) -> list[int]:
        return [p.target for p in self.plans]

    @property
    def hash(self) -> str:
        return journal.plan_hash({"action": "ticket_delete_forever", "plans": [p.hash for p in self.plans],
                                  "ids": self.ids, "max_records": self.max_records}) or ""

    def review(self) -> dict[str, Any]:
        return {"action": "ticket_delete_forever", "method": "DELETE",
                "requests": ["/api/_/" + p.path for p in self.plans], "ticket_ids": self.ids,
                "selected_tickets": json.loads(self.subjects_json), "plan_hash": self.hash,
                "warning": "PERMANENT: the tickets cannot be restored. One request per ticket, in order; "
                           "stops at the first failed or unverified ticket. Never replay an uncertain outcome."}


def build_forever(client: Any, tickets: list[str], *, max_records: int | None) -> ForeverPlan:
    if type(max_records) is not int or not 1 <= max_records <= MAX_FOREVER:
        raise ValueError(f"delete-forever requires --max-records N (1 to {MAX_FOREVER})")
    ids = [positive_id(parse_id(str(t), TICKETS)) for t in tickets]
    if not ids or len(ids) != len(set(ids)):
        raise ValueError("give one or more distinct explicit ticket IDs")
    if len(ids) > max_records:
        raise ValueError("more IDs than --max-records; refusing the whole plan")
    plans, subjects = [], []
    for tid in ids:
        row = _exact(client, tid)
        if not _deleted(row, tid):
            raise ReviewedWriteError(f"ticket {tid} is not in the trash; delete it first; nothing was sent")
        subjects.append({"id": tid, "subject": row.get("subject")})
        plans.append(WritePlan("ticket_delete_forever", tid, config.DOMAIN, "DELETE",
                               "tickets/delete_forever?" + urlencode({"ids": tid}), "null",
                               journal.snapshot_hash({"ticket": row}) or ""))
    return ForeverPlan(tuple(plans), json.dumps(subjects, sort_keys=True), max_records)


def _gone(client: Any, tid: int) -> bool:
    try:
        _record(client, tid)
    except APIError as exc:
        return exc.status == 404
    return False


def send_forever(plan: ForeverPlan, client: Any, *, reviewed_hash: str | None,
                 confirm_destructive: bool = False) -> dict[str, Any]:
    if reviewed_hash is None:
        return {"status": "cancelled", "verified": False}
    if reviewed_hash != plan.hash:
        raise ReviewedWriteError("full review hash changed; review again")
    if confirm_destructive is not True:
        raise ReviewedWriteError("delete-forever requires --confirm-destructive after inspecting the printed ID list")
    tickets: dict[str, Any] = {}
    for index, sub in enumerate(plan.plans):
        def verify(_response: Any, tid: int = sub.target) -> dict[str, Any]:
            gone = _gone(client, tid)
            return {"ticket_id": tid, "not_found_after": gone, "verified": gone}
        try:
            result = execute(sub, client, reviewed_hash=sub.hash, verify=verify,
                             rebuild=lambda tid=sub.target: build_forever(client, [str(tid)], max_records=1).plans[0])
        except ReviewedWriteError as exc:
            result = {"verified": False, "error": str(exc)}
        tickets[str(sub.target)] = {k: result.get(k) for k in ("verified", "error", "warning") if k in result}
        if not result.get("verified"):
            return {"status": "partial" if index else "failed", "verified": False, "plan_hash": plan.hash,
                    "ticket_ids": plan.ids, "tickets": tickets, "stopped_at": sub.target,
                    "not_attempted": plan.ids[index + 1:],
                    "warning": "Stopped at the first failed or unverified ticket; inspect it and the journal; never replay."}
    return {"status": "submitted", "verified": True, "plan_hash": plan.hash, "ticket_ids": plan.ids, "tickets": tickets}
