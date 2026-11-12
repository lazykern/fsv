"""Exact active-agent identities from the native autocomplete results."""
from __future__ import annotations

import re
from typing import Any

_EMAIL = re.compile(r"[^\s@<>,;]+@[^\s@<>,;]+\.[^\s@<>,;]+\Z")


def is_email(value: str) -> bool:
    return bool(_EMAIL.fullmatch(value))


def exact_agent_id(client: Any, value: str, *, name: bool = False) -> int:
    """Return a user ID only for one exact, active autocomplete result.

    Native autocomplete uses an email-valued ``id``; other observed clients
    expose ``email`` or ``details``. Never treat its numeric id as a user ID.
    """
    try:
        rows = client.autocomplete("agents", value)
    except Exception:
        raise ValueError("could not resolve agent; lookup unavailable") from None
    if not isinstance(rows, list) or not all(isinstance(row, dict) for row in rows):
        raise ValueError("agent lookup returned an unreadable result")
    folded = value.casefold()
    matches = []
    for row in rows:
        if (any(row.get(k) for k in ("deleted", "deactivated", "is_deleted", "deleted_at"))
                or row.get("active") is False
                or "(deactivated)" in str(row.get("value") or "").casefold()):
            continue
        keys = ("value", "name") if name else ("id", "email", "details")
        if not any(isinstance(row.get(k), str) and row[k].casefold() == folded for k in keys):
            continue
        if not name:
            emails = [row[k].casefold() for k in ("id", "email", "details")
                      if isinstance(row.get(k), str) and is_email(row[k])]
            if any(email != folded for email in emails):
                continue
        matches.append(row)
    ident = matches[0].get("user_id") if len(matches) == 1 else None
    if (len(matches) != 1 or isinstance(ident, bool) or not isinstance(ident, (str, int))
            or not str(ident).isascii() or not str(ident).isdigit() or int(ident) <= 0):
        raise ValueError(f"agent must match exactly one active entry with a user ID; {len(matches)} exact active candidates")
    return int(ident)
