"""Reviewed soft deletion of one exact change through the captured native wire.

Accepted evidence: ``DELETE /api/_/changes/{id}`` with no body returned 204, and
an independent exact GET then returned HTTP 200 with ``change.deleted: true``.
A 200 detail after soft deletion is not a failed delete. Restoration and bulk
change deletion are not captured.
"""
from __future__ import annotations

from typing import Any

from fsv import config, journal
from fsv.attachments import owner_row
from fsv.resources import CHANGES, parse_id
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute
from fsv.watchers import positive_id


def _exact(client: Any, cid: int) -> dict[str, Any]:
    try:
        return owner_row(client, "changes", cid)
    except ReviewedWriteError:
        raise ReviewedWriteError("exact change unavailable") from None


def build_plan(client: Any, target: str | int) -> WritePlan:
    cid = positive_id(parse_id(str(target), CHANGES))
    row = _exact(client, cid)
    if row.get("deleted") is not False:
        raise ReviewedWriteError("change must be readable with deleted:false before deletion")
    if not isinstance(row.get("subject"), str):
        raise ReviewedWriteError("exact change subject unavailable")
    return WritePlan("change_delete", cid, config.DOMAIN, "DELETE", f"changes/{cid}", "null",
                     journal.snapshot_hash({"change": row}) or "")


def review(plan: WritePlan, client: Any) -> dict[str, Any]:
    row = _exact(client, plan.target)
    return {**plan.review(), "change_id": plan.target, "subject": row.get("subject"),
            "warning": "Destructive soft delete of one change. Restoration is not a captured fsv contract; "
                       "unknown outcomes must be inspected, never automatically resent."}


def send(plan: WritePlan, client: Any, *, reviewed_hash: str | None, confirm_destructive: bool = False) -> dict[str, Any]:
    if reviewed_hash is not None and confirm_destructive is not True:
        raise ReviewedWriteError("change delete requires --confirm-destructive after inspecting the review")

    def verify(_response: Any) -> dict[str, Any]:
        row = _exact(client, plan.target)
        state = "unavailable" if "deleted" not in row else "matched" if row["deleted"] is True else "mismatched"
        return {"change_id": plan.target, "fields": {"deleted": state}, "verified": state == "matched"}

    from fsv.readback import bounded_readback
    return execute(plan, client, reviewed_hash=reviewed_hash, rebuild=lambda: build_plan(client, plan.target),
                   verify=lambda response: bounded_readback(lambda: verify(response)))
