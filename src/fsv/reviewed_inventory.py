"""Strict complete read inventories for mutation authorization/readback.

The general view paginator deliberately discards malformed rows and deduplicates
for display. Writes must validate the raw pages before that information is lost.
"""
from __future__ import annotations

from typing import Any

from fsv.reviewed_write import ReviewedWriteError
from fsv.views import collect_pages


def complete_inventory(client: Any, path: str, key: str, params: dict | None = None, *, api: str = "internal") -> list[dict]:
    seen: set[int] = set()
    totals: set[int] = set()
    page_totals: set[int] = set()
    requested_pages = 0

    class Reader:
        def int_get(self, route, query=None):
            nonlocal requested_pages
            requested_pages += 1
            raw = client.v2_get(route, query) if api == "v2" else client.int_get(route, query)
            if not isinstance(raw, dict) or not isinstance(raw.get(key), list):
                raise ReviewedWriteError("unreadable mutation inventory page")
            for row in raw[key]:
                value = row.get("id") if isinstance(row, dict) else None
                if isinstance(value, bool) or not isinstance(value, (str, int)) or not str(value).isascii() or not str(value).isdigit() or int(value) <= 0 or int(value) in seen:
                    raise ReviewedWriteError("invalid or duplicate mutation inventory identity")
                seen.add(int(value))
            meta = raw.get("meta", {})
            if not isinstance(meta, dict):
                raise ReviewedWriteError("invalid mutation inventory metadata")
            flags: set[bool] = set()
            for source in (raw, meta):
                for name in ("has_more", "has_next"):
                    if name in source and source[name] is not None:
                        if type(source[name]) is not bool:
                            raise ReviewedWriteError("ambiguous inventory continuation flag")
                        flags.add(source[name])
                if source.get("next_page") is not None:
                    raise ReviewedWriteError("unsupported next_page mutation continuation")
                for name in ("page", "total_pages"):
                    if name in source and (type(source[name]) is not int or source[name] < (1 if name == "page" else 0)):
                        raise ReviewedWriteError("invalid inventory page metadata")
                if "page" in source and source["page"] != int((query or {}).get("page", requested_pages)):
                    raise ReviewedWriteError("inventory page identity disagrees")
                if "total_pages" in source:
                    if source["total_pages"] == 0 and raw[key]:
                        raise ReviewedWriteError("nonempty inventory with zero pages")
                    page_totals.add(max(1, source["total_pages"]))
                for name in ("total", "total_count", "count"):
                    if name in source:
                        value = source[name]
                        if type(value) is not int or value < 0:
                            raise ReviewedWriteError("invalid mutation inventory total")
                        totals.add(value)
            if len(flags) > 1:
                raise ReviewedWriteError("conflicting inventory continuation flags")
            continuing = []
            for name in ("next_page_url", "next_start_token", "next_cursor"):
                values = [source[name] for source in (raw, meta) if name in source]
                if any(value is not None and (not isinstance(value, str) or not value) for value in values):
                    raise ReviewedWriteError("invalid inventory continuation token")
                if len(values) == 2 and values[0] != values[1]:
                    raise ReviewedWriteError("conflicting inventory continuation tokens")
                if values and values[0]:
                    continuing.append(name)
            if flags == {False} and continuing:
                raise ReviewedWriteError("terminal inventory flag contradicts continuation")
            if flags == {True} and any("next_page_url" in source and not source["next_page_url"] for source in (raw, meta)) and not continuing:
                raise ReviewedWriteError("missing claimed inventory continuation")
            return raw

    rows, meta, missing = collect_pages(Reader(), path, key, params, what=key)
    if missing or not meta.get("complete") or meta.get("partial") or len(totals) > 1 or (totals and totals != {len(rows)}) or (page_totals and page_totals != {meta.get("pages")}):
        raise ReviewedWriteError("complete, consistent mutation inventory unavailable")
    return sorted(rows, key=lambda row: int(row["id"]))


def single_page_inventory(raw: Any, key: str) -> list[dict]:
    """Validate a captured unpaginated inventory; never make extra wire requests."""
    class Reader:
        read = False
        def int_get(self, _path, _params=None):
            if self.read:
                raise ReviewedWriteError("single-page inventory claims more rows")
            self.read = True
            return raw
    count = len(raw.get(key, [])) if isinstance(raw, dict) and isinstance(raw.get(key), list) else 0
    return complete_inventory(Reader(), "captured-inventory", key, {"page": 1, "per_page": count + 1})
