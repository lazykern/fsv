"""Read-only change CI/asset candidate selection; never associates a record."""
from __future__ import annotations

from typing import Any

from fsv.create import search_assets_for_change
from fsv.resources import CHANGES, parse_id
from fsv.reviewed_write import ReviewedWriteError
from fsv.watchers import positive_id


def candidates(client: Any, target: str | int, query: str, *, page: int = 1) -> dict[str, Any]:
    target = positive_id(parse_id(str(target), CHANGES))
    if type(page) is not int or page < 1:
        raise ValueError("page must be positive")
    raw = search_assets_for_change(target, query, page=page, per_page=30, c=client)
    rows = raw.get("assets") if isinstance(raw, dict) else None
    if not isinstance(rows, list) or any(not isinstance(row, dict) for row in rows):
        raise ReviewedWriteError("asset candidate response is unavailable")
    ids = [positive_id(row.get("display_id")) for row in rows]
    if len(set(ids)) != len(ids):
        raise ReviewedWriteError("duplicate asset display identities; selection is ambiguous")
    return {"change_id": target, "page": page, "scope": "single search page; not a complete inventory",
            "assets": rows, "mutation": "none", "warning": "Selection only. Asset/CI add-remove variants remain unsupported by this picker."}


def select(result: dict[str, Any], display_id: int) -> dict[str, Any]:
    display_id = positive_id(display_id)
    rows = [row for row in result["assets"] if positive_id(row.get("display_id")) == display_id]
    if len(rows) != 1:
        raise ReviewedWriteError("exact asset display ID is not unique on this search page")
    return rows[0]
