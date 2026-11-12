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
