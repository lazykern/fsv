"""Single-ticket spam / not spam (captured from the ticket page and Spam view).

* spam: ``PUT tickets/{id}/spam`` with ``{}`` (204, no confirm in the UI);
* not spam: ``PUT tickets/{id}/unspam`` without a body (204).
Readback: the exact ticket's ``spam`` flag. Bulk spam lives in ``tickets bulk``.
"""
from __future__ import annotations

from typing import Any

from fsv import config, journal
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute
from fsv.resources import TICKETS, parse_id
from fsv.ticket_properties import _record
from fsv.ticket_trash import _exact
from fsv.watchers import positive_id


def build_plan(client: Any, ticket: str | int, *, spam: bool) -> WritePlan:
    tid = positive_id(parse_id(str(ticket), TICKETS))
    row = _exact(client, tid)
    flag = row.get("spam")
    if type(flag) is not bool:
        raise ReviewedWriteError(f"ticket {tid}: spam flag unavailable; nothing was sent")
    if flag is spam:
        raise ReviewedWriteError(f"ticket {tid} is {'already' if spam else 'not'} marked as spam; nothing was sent")
    if row.get("deleted") is True:
        raise ReviewedWriteError(f"ticket {tid} is in the trash; restore it first; nothing was sent")
    action, body = ("ticket_spam", "{}") if spam else ("ticket_unspam", "null")
    return WritePlan(action, tid, config.DOMAIN, "PUT", f"tickets/{tid}/{'spam' if spam else 'unspam'}", body,
                     journal.snapshot_hash({"ticket": row}) or "")


def send(plan: WritePlan, client: Any, *, reviewed_hash: str | None,
         confirm_destructive: bool = False) -> dict[str, Any]:
    spam = plan.action == "ticket_spam"
    if spam and confirm_destructive is not True and reviewed_hash is not None:
        raise ReviewedWriteError("spam hides the ticket from every view; pass --confirm-destructive after review")

    def verify(_response: Any) -> dict[str, Any]:
        flag = _record(client, plan.target).get("spam")
        return {"ticket_id": plan.target, "spam": flag, "verified": flag is spam}

    return execute(plan, client, reviewed_hash=reviewed_hash, verify=verify,
                   rebuild=lambda: build_plan(client, plan.target, spam=spam))
