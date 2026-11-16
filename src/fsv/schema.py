from __future__ import annotations

import re

import json
import math
import sys
import time
from typing import Any

from pathlib import Path

from contextlib import contextmanager
from contextvars import ContextVar
from typing import Callable, TypeVar

from fsv import cache as _cache
from fsv.cache import load as _cache_load, refresh_async, save, update_meta
from fsv.client import Client
from fsv.errors import APIError, SessionError
from fsv.config import ensure_dirs, schema_cache_candidates, schema_cache_path
from fsv.resources import Resource

CACHE_TTL = _cache.TTL["schema"]  # 15 minutes: admins edit labels/choices without notice


def _path(res: Resource) -> Path:
    return schema_cache_path(res.name)


def _doc_fields(doc: Any) -> list[dict[str, Any]] | None:
    """Return the non-empty field list stored in a cache doc, else None.

    Empty or malformed entries (e.g. written when an expired session made
    both field APIs fail) count as a cache miss.
    """
    if not isinstance(doc, dict):
        return None
    for key in ("data", "fields"):
        inner = doc.get(key)
        if isinstance(inner, list) and inner:
            return inner
    return None


def _cached_path(res: Resource) -> tuple[Path, dict[str, Any] | None, bool]:
    for path in schema_cache_candidates(res.name):
        doc, stale = _cache_load(path)
        if doc is not None and _doc_fields(doc):
            saved = doc.get("saved_at", path.stat().st_mtime)
            valid = isinstance(saved, (int, float)) and not isinstance(saved, bool) and math.isfinite(saved)
            stale = (not valid or not 0 <= time.time() - saved <= _cache.TTL["schema"]
                     or _cache.forced(path))
            return path, doc, stale
    primary = _path(res)
    return primary, None, True


def _fetch_schema(c: Client, res: Resource) -> list[dict[str, Any]]:
    """Try internal API first (richer data: choices for reference fields),
    fall back to v2 API. Always merges v2 choices for fields where v2 has more.

    Never returns an empty list: if neither API yields fields, raises the
    underlying SessionError/APIError (or an APIError if both answered empty).
    """
    int_fields: list[dict[str, Any]] | None = None
    errors: list[Exception] = []
    try:
        raw = c.int_get(res.form_fields_path)
        if raw.get(res.form_fields_key):
            int_fields = raw[res.form_fields_key]
    except Exception as e:
        errors.append(e)

    try:
        v2_raw = c.v2_get(res.form_fields_path)
        v2_fields = v2_raw.get(res.form_fields_key) or []
    except Exception as e:
        errors.append(e)
        v2_fields = []

    if not int_fields:
        if v2_fields:
            return v2_fields
        # Prefer a session error (the actionable one), then any API error.
        for kind in (SessionError, APIError):
            for e in errors:
                if isinstance(e, kind):
                    raise e
        if errors:
            raise errors[0]
        raise APIError(200, f"no form fields returned for {res.name}", "GET", res.form_fields_path)

    if v2_fields:
        v2_by_name = {f["name"]: f for f in v2_fields if "name" in f}
        for f in int_fields:
            name = f.get("name")
            v2f = v2_by_name.get(name)
            if v2f and len(v2f.get("choices") or []) > len(f.get("choices") or []):
                f["choices"] = v2f["choices"]

    return int_fields


def _choice_labels(choices: Any) -> set[str]:
    """Every choice value/label in a field's choices (nested included)."""
    out: set[str] = set()
    if isinstance(choices, dict):
        for k, v in choices.items():
            out.add(str(k))
            if isinstance(v, (dict, list)):
                out |= _choice_labels(v)
    elif isinstance(choices, list):
        for item in choices:
            if isinstance(item, dict):
                for key in ("value", "label", "name"):
                    if item.get(key) not in (None, ""):
                        out.add(str(item[key]))
                for key in ("nested_options", "choices"):
                    if isinstance(item.get(key), (dict, list)):
                        out |= _choice_labels(item[key])
            elif isinstance(item, (list, tuple)):
                out |= {str(x) for x in item if isinstance(x, (str, int)) and not isinstance(x, bool)}
            elif item is not None:
                out.add(str(item))
    return out


def freshness_warning(res: Resource, client: Any, schema: dict[str, Any]) -> str | None:
    """One GET of the form fields compared with a cached schema: a warning
    naming the fields whose choices the server has but the cache lacks (a
    tenant added a choice after caching), else None. Errors are ignored:
    this check is advisory and never blocks."""
    if not isinstance(schema, dict) or schema.get("source") != "cache":
        return None
    try:
        raw = client.int_get(res.form_fields_path)
        fresh = raw.get(res.form_fields_key) if isinstance(raw, dict) else None
    except Exception:
        return None
    if not isinstance(fresh, list):
        return None
    cached = {f.get("name"): _choice_labels(f.get("choices")) for f in schema.get("fields") or []
              if isinstance(f, dict)}
    changed = [str(f.get("name")) for f in fresh if isinstance(f, dict) and f.get("name") in cached
               and _choice_labels(f.get("choices")) - cached[f.get("name")]]
    if not changed:
        return None
    shown = ", ".join(changed[:5]) + (" ..." if len(changed) > 5 else "")
    return (f"cached {res.name} schema is out of date: the server has new choices for {shown}; "
            f"run `fsv {res.name} fields --refresh` (or `fsv cache clear schema`) and review again")


def _unwrap(d: dict[str, Any]) -> dict[str, Any]:
    """Extract {saved_at, fields} from an envelope or old-format cache.

    Uses key presence (not truthiness) so an empty list never drops 'fields'.
    """
    fields = _doc_fields(d) or []
    return {"saved_at": d.get("saved_at", 0), "fields": fields}


def load(res: Resource, client: Client | None = None, force: bool = False, *,
         for_write: bool = False) -> dict[str, Any]:
    """Return the cached schema for *res*, fetching on miss or when *force*.

    Reads use stale-while-revalidate: if the cache exists but is stale, the
    old data is returned immediately and a background refresh is spawned.
    *for_write* (every write, dry-run included): a schema older than
    ``cache.WRITE_MAX_AGE`` is refetched before it is returned, so a write is
    never validated against stale field labels or choices.  Empty schemas
    are never cached or returned; a failed fetch raises.
    """
    p = _path(res)
    cache_path, doc, stale = _cached_path(res)
    if for_write and doc is not None and not stale:
        saved = doc.get("saved_at")
        if not isinstance(saved, (int, float)) or time.time() - saved > _cache.WRITE_MAX_AGE:
            stale = True
    if for_write and stale:
        force = True
    if doc is not None and _cache.forced(cache_path):
        force = True  # global `fsv --refresh`: one blocking refetch per process

    if doc is not None and not force and not stale:
        update_meta(cache_path, checked_at=True, fetch_inc=True)
        return {**_unwrap(doc), "stale": False, "source": "cache"}

    if doc is not None and not force:
        # stale — return old data, refresh in background
        print(f"Warning: stale {res.name} schema; refreshing in background. "
              f"Run `fsv {res.name} fields --refresh` for current definitions.", file=sys.stderr)
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
        return {**_unwrap(doc), "stale": True, "source": "cache"}

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
    if not fields:
        raise APIError(200, f"empty schema for {res.name}", "GET", res.form_fields_path)
    ensure_dirs()
    save(p, "schema", fields, latency_ms=latency)
    return {"saved_at": time.time(), "fields": fields, "stale": False, "source": "server"}


class SchemaRefusal(Exception):
    """A local validation refusal raised inside validate_with_refetch (the
    CLI's error helper raises it instead of exiting) so the cached schema can
    be refetched once and the validation retried."""


_RETRY_SCOPE: ContextVar[bool] = ContextVar("fsv_schema_retry_scope", default=False)
_REFUSAL_MARKERS = ("not offered", "field not found", "required field", "no choice ", "unknown ",
                    "did not match a")
T = TypeVar("T")


def is_schema_refusal(message: str) -> bool:
    """True inside validate_with_refetch for a refusal a newer schema could
    lift: an unknown choice value, an unknown field, a missing required field."""
    if not _RETRY_SCOPE.get():
        return False
    low = str(message).lower()
    return any(m in low for m in _REFUSAL_MARKERS)


@contextmanager
def _retry_scope() -> Any:
    token = _RETRY_SCOPE.set(True)
    try:
        yield
    finally:
        _RETRY_SCOPE.reset(token)


def validate_with_refetch(res: Resource, client: Any, sch: dict[str, Any],
                          build: Callable[[dict[str, Any]], T]) -> tuple[dict[str, Any], T]:
    """Run ``build(sch)`` (the local validation of a write). When it refuses
    with a schema refusal and *sch* came from the cache, refetch that module's
    schema ONCE and run the validation again; a second refusal is final (no
    loop). A schema just fetched from the server is never refetched.
    Returns the schema actually used and the build result."""
    if not isinstance(sch, dict) or sch.get("source") != "cache":
        return sch, build(sch)
    try:
        with _retry_scope():
            return sch, build(sch)
    except SchemaRefusal as exc:
        print(f"note: {exc}; refetching the {res.name} form schema once and validating again",
              file=sys.stderr)
    fresh = load(res, client, force=True)
    return fresh, build(fresh)


def revalidate_for_write(res: Resource, client: Any) -> None:
    """Called before every write (dry-run included): a cached schema older
    than ``cache.WRITE_MAX_AGE`` is refetched now (blocking), so the following
    ``load`` validates against current labels and choices. No cache: no-op
    (``load`` fetches on a miss anyway)."""
    _path_, doc, stale = _cached_path(res)
    if doc is None:
        return
    saved = doc.get("saved_at")
    if stale or not isinstance(saved, (int, float)) or time.time() - saved > _cache.WRITE_MAX_AGE:
        load(res, client, force=True)


def field(name: str, schema: dict[str, Any]) -> dict[str, Any] | None:
    for f in schema["fields"]:
        if f["name"] == name:
            return f
    return None


_I18N_KEY = re.compile(r"[a-z][a-z0-9]*(?:_[a-z0-9]+)*_key")


def humanize_key(text: Any) -> str:
    """Some tenants' choice metadata carries untranslated i18n keys as labels
    (``portal_key``, ``feedback_widget_key``). Show them as words (``Portal``,
    ``Feedback Widget``); any other label is returned unchanged."""
    s = "" if text is None else str(text)
    if not _I18N_KEY.fullmatch(s):
        return s
    return " ".join(w.capitalize() for w in s[: -len("_key")].split("_"))


def choice_label(field_name: str, val: Any, schema: dict[str, Any]) -> str:
    f = field(field_name, schema)
    if not f or val in (None, ""):
        return "" if val is None else str(val)
    for c in f.get("choices") or []:
        if c.get("id") == val or c.get("value") == val:
            return humanize_key(c.get("value") or c.get("name") or str(val))
    return str(val)


# Static labels for the platform's default priority/impact/risk ids. These are
# ONLY a documented fallback when no runtime schema is loaded (or the schema
# has no choices for the field): tenants rename and re-label these choices,
# so `display_label` always prefers the field's own runtime choices.
PRIORITY = {1: "Low", 2: "Medium", 3: "High", 4: "Urgent"}
IMPACT = {1: "Low", 2: "Medium", 3: "High"}
RISK = {1: "Low", 2: "Medium", 3: "High", 4: "Very High"}
STATIC_LABELS = {"priority": PRIORITY, "impact": IMPACT, "risk": RISK}


def display_label(field_name: str, val: Any, schema: dict[str, Any] | None, default: str = "-") -> str:
    """Label of a choice field value: the runtime schema's choices first, the
    static platform map only when the schema has no choices for the field."""
    if val in (None, ""):
        return default
    f = field(field_name, schema) if isinstance(schema, dict) and isinstance(schema.get("fields"), list) else None
    choices = (f or {}).get("choices") or []
    if choices:
        for c in choices:
            if isinstance(c, dict) and (str(c.get("id")) == str(val) or c.get("value") == val):
                return humanize_key(c.get("value") or c.get("name") or c.get("label") or val)
        return str(val)  # a runtime value the static map must not relabel
    return STATIC_LABELS.get(field_name, {}).get(val, str(val))
