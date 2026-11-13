"""Which approval model a tenant runs: parallel approval chains/groups, or the
legacy flat per-member approvals list.

Tenants without the parallel-approvals feature answer
``GET <module>/<id>/approval-chains`` with 400 ("feature needs to be
enabled") or 404. They still expose the flat ``<module>/<id>/approvals``
list (rows with ``member_id``, ``level_id``, ``status{id,name}``,
``type{id,name}``), so read-only listing works on both models, while the
group-based writes (named group request, group cancel, decisions verified
through chains) do not apply there and must refuse clearly instead of failing
on an unexpected 400.
"""
from __future__ import annotations

from typing import Any

from fsv.errors import APIError
from fsv.reviewed_write import ReviewedWriteError

CHAINS = "chains"
LEGACY = "legacy"
UNKNOWN = "unknown"
LEGACY_LABEL = "legacy flat approvals"
NOT_SUPPORTED = f"{LEGACY_LABEL}: not yet supported"
_ABSENT = (400, 404)


class LegacyApprovalsError(ReviewedWriteError):
    """The tenant has no parallel approval chains (legacy flat approvals)."""

    def __init__(self, message: str, status: int | None = None) -> None:
        super().__init__(message)
        self.status = status


def is_absent(exc: BaseException) -> bool:
    """An approval-chains error that means "no parallel approvals here"."""
    return isinstance(exc, APIError) and exc.status in _ABSENT


def describe(status: int | None) -> str:
    return f"this tenant uses {LEGACY_LABEL} (approval-chains answered HTTP {status if status is not None else '?'})"


def detect(client: Any, module: str, target: int) -> tuple[str, int | None]:
    """``(CHAINS, None)``, ``(LEGACY, status)`` or ``(UNKNOWN, status)`` from
    one read-only GET.

    Only 400/404 prove the legacy model. Any other API error (403, 5xx) is
    ``UNKNOWN``: callers keep their existing chains-driver path, which has
    its own fail-closed inventory checks. Session errors propagate."""
    if module not in ("changes", "tickets"):
        raise ValueError("unsupported approval module")
    try:
        client.int_get(f"{module}/{target}/approval-chains", params={"page": 1, "per_page": 100})
    except APIError as exc:
        if exc.status in _ABSENT:
            return LEGACY, exc.status
        return UNKNOWN, exc.status
    return CHAINS, None


def require_chains(client: Any, module: str, target: int, action: str) -> None:
    """Refuse a chains/groups-only action on a proven legacy tenant, naming
    the model."""
    model, status = detect(client, module, target)
    if model == LEGACY:
        raise LegacyApprovalsError(f"{action} refused: {describe(status)}; {NOT_SUPPORTED}", status)


# Keys every flat approval row carries in `tickets approvals --json` and
# `sr approvals list --json` (added next to the native record's own keys).
FLAT_KEYS = ("level", "approver", "approver_id", "status_name", "status_id", "type_name", "decided", "comment")


def _named(value: Any) -> tuple[Any, str]:
    if isinstance(value, dict):
        return value.get("id"), str(value.get("name") or value.get("label") or "")
    return value, ""


def flat_row(record: Any) -> dict[str, Any]:
    """Display fields of one flat (legacy) approval record. Status and type
    names come from the record's own status/type objects; a record without
    a name shows its id rather than a guessed label."""
    a = record if isinstance(record, dict) else {}
    remarks = a.get("remark") if isinstance(a.get("remark"), list) else []
    remark = remarks[0] if remarks and isinstance(remarks[0], dict) else {}
    member = a.get("member") if isinstance(a.get("member"), dict) else {}
    status_id, status_name = _named(a.get("status"))
    type_id, type_name = _named(a.get("type"))
    decided = remark.get("updated_at") or a.get("updated_at")
    return {
        "level": a.get("level_id"),
        "approver": member.get("name") or (str(a.get("member_id")) if a.get("member_id") is not None else ""),
        "approver_id": a.get("member_id", member.get("id")),
        "status_name": status_name or ("" if status_id is None else str(status_id)),
        "status_id": status_id,
        "type_name": type_name or ("" if type_id is None else str(type_id)),
        "decided": str(decided)[:19] if decided else "",
        "comment": str(remark.get("data") or ""),
    }


def with_flat(records: Any) -> list[dict[str, Any]]:
    """Native records with the :data:`FLAT_KEYS` added (native keys kept)."""
    return [{**r, **flat_row(r)} for r in records or [] if isinstance(r, dict)]


def flat_table(records: Any, title: str | None = None) -> Any:
    from rich.markup import escape
    from rich.table import Table
    t = Table(title=title)
    for col in ("Level", "Approver", "Status", "Decided", "Type", "Comment"):
        t.add_column(col, overflow="fold")
    for r in records or []:
        row = flat_row(r)
        t.add_row(*(escape(str(x if x not in (None, "") else "-")) for x in
                    (row["level"], row["approver"], row["status_name"], row["decided"], row["type_name"],
                     row["comment"] or "")))
    return t
