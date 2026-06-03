from __future__ import annotations

import json
import time
from typing import Any

from pathlib import Path

from fsv.cache import load as _cache_load, refresh_async, save, update_meta
from fsv.client import Client
from fsv.config import ensure_dirs, schema_cache_candidates, schema_cache_path
from fsv.resources import Resource

CACHE_TTL = 7 * 86400  # 7 days — field definitions rarely change, payloads expensive


def _path(res: Resource) -> Path:
    return schema_cache_path(res.name)


def _cached_path(res: Resource) -> tuple[Path, dict[str, Any] | None, bool]:
    for path in schema_cache_candidates(res.name):
        doc, stale = _cache_load(path)
        if doc is not None:
            return path, doc, stale
    primary = _path(res)
    return primary, None, True


def _fetch_schema(c: Client, res: Resource) -> list[dict[str, Any]]:
    """Try internal API first (richer data: choices for reference fields),
    fall back to v2 API. Always merges v2 choices for fields where v2 has more."""
    int_fields: list[dict[str, Any]] | None = None
    try:
        raw = c.int_get(res.form_fields_path)
        if raw.get(res.form_fields_key):
            int_fields = raw[res.form_fields_key]
    except Exception:
        pass

    try:
        v2_raw = c.v2_get(res.form_fields_path)
        v2_fields = v2_raw.get(res.form_fields_key) or []
    except Exception:
        v2_fields = []

    if not int_fields:
        return v2_fields or []

    if v2_fields:
        v2_by_name = {f["name"]: f for f in v2_fields if "name" in f}
        for f in int_fields:
            name = f.get("name")
            v2f = v2_by_name.get(name)
            if v2f and len(v2f.get("choices") or []) > len(f.get("choices") or []):
                f["choices"] = v2f["choices"]

    return int_fields


def load(res: Resource, client: Client | None = None, force: bool = False) -> dict[str, Any]:
    """Return the cached schema for *res*, fetching on miss or when *force*.

    Uses stale-while-revalidate: if cache exists but stale, returns old data
    immediately and spawns a background refresh.
    """
    p = _path(res)
    cache_path, doc, stale = _cached_path(res)

    def _unwrap(d: dict[str, Any]) -> dict[str, Any]:
        """Extract schema dict from envelope or old-format cache."""
        inner = d.get("data") or d.get("fields")
        if isinstance(inner, list):
            return {"saved_at": d.get("saved_at", 0), "fields": inner}
        return d  # already in old format {saved_at, fields}

    if doc is not None and not force and not stale:
        update_meta(cache_path, checked_at=True, fetch_inc=True)
        return _unwrap(doc)

    if doc is not None and not force:
        # stale — return old data, refresh in background
        own = client is None
        c = client or Client()

        def _fetch() -> list[dict[str, Any]]:
            nonlocal c, own
            try:
                return _fetch_schema(c, res)
            finally:
                if own:
                    c.close()

        refresh_async(p, "schema", _fetch)
        return _unwrap(doc)

    # miss or force — block and fetch
    own = client is None
    c = client or Client()
    try:
        t0 = time.time()
        fields = _fetch_schema(c, res)
        latency = int((time.time() - t0) * 1000)
    finally:
        if own:
            c.close()
    ensure_dirs()
    save(p, "schema", fields, latency_ms=latency)
    return {"saved_at": time.time(), "fields": fields}


def field(name: str, schema: dict[str, Any]) -> dict[str, Any] | None:
    for f in schema["fields"]:
        if f["name"] == name:
            return f
    return None


def choice_label(field_name: str, val: Any, schema: dict[str, Any]) -> str:
    f = field(field_name, schema)
    if not f or val in (None, ""):
        return "" if val is None else str(val)
    for c in f.get("choices") or []:
        if c.get("id") == val or c.get("value") == val:
            return c.get("value") or c.get("name") or str(val)
    return str(val)


# Hardcoded defaults used as fallback (also unchanged across tenants for OOTB fields)
PRIORITY = {1: "Low", 2: "Medium", 3: "High", 4: "Urgent"}
IMPACT = {1: "Low", 2: "Medium", 3: "High"}
RISK = {1: "Low", 2: "Medium", 3: "High", 4: "Very High"}
