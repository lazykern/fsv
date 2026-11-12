"""Runtime choice identities from native metadata.

Freshservice metadata serializes some choice IDs as JSON integers and others as
digit strings (for example problem_task_fields). Accepted write wires send the
integer form. Booleans, signs, whitespace, non-ASCII digits and other shapes are
never identities.
"""
from __future__ import annotations

from typing import Any


def runtime_choice_id(value: Any, *, minimum: int = 1) -> int | None:
    if type(value) is int:
        ident = value
    elif type(value) is str and value.isascii() and value.isdigit():
        ident = int(value)
    else:
        return None
    return ident if ident >= minimum else None
