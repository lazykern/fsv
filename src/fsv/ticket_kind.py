"""Exact ticket-type proof from native ticket reads."""
from __future__ import annotations

from typing import Any

from fsv.reviewed_write import ReviewedWriteError


def exact_ticket_type(record: Any) -> str:
    """Return the case-folded ticket type from an exact native ticket read.

    Native exact reads expose ``type``; some echoes and form adapters expose
    ``ticket_type``. Accept either, but require every present representation to
    be a nonempty string and to agree, so a conflicting record cannot pass a
    type guard.
    """
    if not isinstance(record, dict):
        raise ReviewedWriteError("exact ticket type unavailable")
    values = [record[key] for key in ("type", "ticket_type") if key in record and record[key] is not None]
    if not values or any(not isinstance(value, str) or not value.strip() for value in values):
        raise ReviewedWriteError("exact ticket type unavailable")
    kinds = {" ".join(value.replace("_", " ").split()).casefold() for value in values}
    if len(kinds) != 1:
        raise ReviewedWriteError("conflicting exact ticket type representations")
    return kinds.pop()


INCIDENT, SERVICE_REQUEST = "incident", "service request"
_TYPE_KINDS = {INCIDENT: INCIDENT, SERVICE_REQUEST: SERVICE_REQUEST}


def _display_prefix(record: dict) -> str | None:
    """The prefix of the record's own human display id, checked against its
    own display number (``IT-77`` with display_id 77 -> ``IT``)."""
    hid = record.get("human_display_id")
    did = record.get("display_id", record.get("id"))
    if not isinstance(hid, str) or isinstance(did, bool) or did is None:
        return None
    head, sep, tail = hid.strip().rpartition("-")
    if not sep or not head or tail != str(did):
        return None
    return head.upper()


def kind_evidence(record: Any) -> dict[str, str]:
    """Each piece of evidence the record itself carries about its module:
    ``type`` (only the platform's built-in type names), the display-id prefix
    (built-in or configured, see :func:`fsv.resources.kind_prefixes`) and
    requested items (only service requests carry them)."""
    from fsv.resources import kind_prefixes
    if not isinstance(record, dict):
        return {}
    out: dict[str, str] = {}
    for key in ("type", "ticket_type"):
        value = record.get(key)
        if isinstance(value, str):
            folded = " ".join(value.replace("_", " ").split()).casefold()
            if folded in _TYPE_KINDS:
                out[key] = _TYPE_KINDS[folded]
    prefix = _display_prefix(record)
    if prefix is not None:
        mapped = kind_prefixes().get(prefix)
        if mapped in (INCIDENT, SERVICE_REQUEST):
            out["human_display_id"] = mapped
    if record.get("has_requested_items") is True:
        out["has_requested_items"] = SERVICE_REQUEST
    return out


def ticket_kind(record: Any) -> str | None:
    """``incident`` / ``service request`` when the record's own fields agree;
    None when they carry no evidence or conflict."""
    kinds = set(kind_evidence(record).values())
    return kinds.pop() if len(kinds) == 1 else None


def require_kind(record: Any, kind: str, what: str) -> None:
    """Refuse unless the record proves ``kind`` from its own fields."""
    if ticket_kind(record) == kind:
        return
    row = record if isinstance(record, dict) else {}
    seen = {k: row.get(k) for k in ("type", "human_display_id") if row.get(k) is not None}
    setting = "ids.incident_prefix" if kind == INCIDENT else "ids.service_request_prefix"
    raise ReviewedWriteError(f"{what} requires a proven {kind}; the record's own fields do not prove it "
                             f"({', '.join(f'{k}={v!r}' for k, v in seen.items()) or 'no type or display id'}). "
                             f"A tenant with a custom display-id prefix can declare it: fsv config set {setting} PREFIX")
