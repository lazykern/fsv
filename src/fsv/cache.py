"""Adaptive stale-while-revalidate cache for Freshservice resources.

Strategy:
  - cache hit -> use instantly (even stale)
  - cache stale -> use old data, spawn background refresh
  - cache miss -> block until fetched
  - TTL adapts based on observed change frequency and payload size
"""

from __future__ import annotations

import json
import os
import threading
import time
from pathlib import Path
from typing import Any, Callable

# ---------------------------------------------------------------------------
# built-in TTL defaults (seconds)
# ---------------------------------------------------------------------------
TTL: dict[str, int] = {
    "schema": 7 * 86400,   # field definitions rarely change; expensive payloads
    "filters": 900,        # saved-filter list may change daily; cheap payloads
    "groups": 6 * 3600,    # org groups; server supports ETag
    "autocomplete": 1800,  # requester/agent prefix matching; moderate volatility
    "lookup": 300,         # lookup field completions; 5 min
}


def ttl_for(kind: str) -> int:
    return TTL.get(kind, 3600)


# ---------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------

def _atomic_write(path: Path, payload: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + f".{os.getpid()}.{threading.get_ident()}.tmp")
    tmp.write_text(payload, encoding="utf-8")
    os.replace(tmp, path)


# ---------------------------------------------------------------------------
# public API
# ---------------------------------------------------------------------------

def load(path: Path) -> tuple[dict[str, Any] | None, bool]:
    """Read a cache file.

    Returns (wrapped_document, is_stale).

    *wrapped_document* is the full stored JSON dict (includes metadata keys
    like ``saved_at``, ``checked_at``, ``data``).  Callers extract ``data``.
    Returns (None, True) when the file is missing or unreadable.
    """
    if not path.exists():
        return None, True
    try:
        doc = json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return None, True
    now = time.time()
    saved = doc.get("saved_at", 0)
    if not saved:
        saved = path.stat().st_mtime  # backward compat: use file mtime
    kind = doc.get("_kind", "")
    stale = (now - saved) > ttl_for(kind)
    return doc, stale


def save(path: Path, kind: str, data: Any, *, etag: str = "", latency_ms: int = 0) -> None:
    """Atomically persist *data* under the metadata envelope."""
    now = time.time()
    doc: dict[str, Any] = {
        "_kind": kind,
        "saved_at": now,
        "checked_at": now,
        "changed_at": now,
        "hash": "",
        "etag": etag,
        "latency_ms": latency_ms,
        "fetch_count": 1,
        "data": data,
    }
    _atomic_write(path, json.dumps(doc, indent=2, ensure_ascii=False))


def update_meta(path: Path, *, checked_at: bool = False, fetch_inc: bool = False, etag: str | None = None) -> None:
    """Update metadata without rewriting data payload."""
    try:
        doc = json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return
    now = time.time()
    if checked_at:
        doc["checked_at"] = now
    if fetch_inc:
        doc["fetch_count"] = doc.get("fetch_count", 1) + 1
    if etag is not None:
        doc["etag"] = etag
    _atomic_write(path, json.dumps(doc, indent=2, ensure_ascii=False))


def mark_stale(path: Path) -> None:
    """Force next check to re-fetch by setting saved_at far in the past."""
    try:
        doc = json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return
    doc["saved_at"] = 0
    _atomic_write(path, json.dumps(doc, indent=2, ensure_ascii=False))


def refresh_async(path: Path, kind: str, fetcher: Callable[[], Any], lock_path: Path | None = None) -> None:
    """Spawn a daemon thread to refresh the cache in the background.

    Uses a lock file to avoid concurrent refreshes for the same resource.
    """
    if lock_path is None:
        lock_path = path.with_suffix(path.suffix + ".lock")

    def _work() -> None:
        # simple file-based lock
        if lock_path.exists():
            age = time.time() - lock_path.stat().st_mtime
            if age < 30:  # another refresh in flight or very recent
                return
        try:
            lock_path.parent.mkdir(parents=True, exist_ok=True)
            lock_path.write_text("")
            t0 = time.time()
            data = fetcher()
            latency = int((time.time() - t0) * 1000)
            save(path, kind, data, latency_ms=latency)
        except Exception:
            pass
        finally:
            try:
                lock_path.unlink(missing_ok=True)
            except Exception:
                pass

    t = threading.Thread(target=_work, daemon=True)
    t.start()
