"""Plan hashes and metadata fingerprints for the write path.

``plan_hash`` identifies exactly what a create would send: the request body
plus the content of every file that will be uploaded. ``--dry-run`` prints
it; ``--yes --expect-plan <hash>`` refuses to send anything else (a metadata
refresh that changed a populate, an edited file, a different answer).

``br_validation_excludes`` is *included* in the hash. It is not volatile: it
is computed deterministically from the engine state (no timestamp, no
signature), and it changes exactly when the rules' effect on the form
changes, which is a change an approver should re-review.

``metadata_fingerprint`` summarises the form metadata (fields, rules, item)
so the caller can re-fetch just before submit and re-run the engine when
anything moved; ``updated_at`` gives "form metadata as of ..." for reviews.
"""
from __future__ import annotations

import datetime as dt
import hashlib
import json
import os
from dataclasses import dataclass
from typing import Any, Iterable, Mapping, Sequence

PLAN_VERSION = 1
MIN_PREFIX = 12


def canonical_json(data: Any) -> bytes:
    """Stable JSON bytes: sorted keys, no whitespace, UTF-8."""
    return json.dumps(data, sort_keys=True, separators=(",", ":"), ensure_ascii=False, default=_default).encode(
        "utf-8"
    )


def _default(o: Any) -> Any:
    if isinstance(o, (dt.datetime, dt.date)):
        return o.isoformat()
    if isinstance(o, (set, frozenset)):
        return sorted(o, key=str)
    if isinstance(o, tuple):
        return list(o)
    raise TypeError(f"not JSON-serialisable: {type(o).__name__}")


def file_digest(path: str | os.PathLike[str]) -> dict[str, Any]:
    h = hashlib.sha256()
    size = 0
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 16), b""):
            h.update(chunk)
            size += len(chunk)
    return {"name": os.path.basename(os.fspath(path)), "size": size, "sha256": h.hexdigest()}


def _file_entries(files: Any) -> list[dict[str, Any]]:
    if not files:
        return []
    groups: Mapping[str, Any] = files if isinstance(files, Mapping) else {"attachments": files}
    out = []
    for key, paths in groups.items():
        items = [paths] if isinstance(paths, (str, os.PathLike)) else list(paths or [])
        for i, p in enumerate(items):
            out.append({"field": str(key), "index": i, **file_digest(p)})
    return sorted(out, key=lambda e: (e["field"], e["index"]))


def plan_hash(payload: Mapping[str, Any], files: Mapping[str, Any] | Sequence[Any] | None = None) -> str:
    """sha256 hex over the canonical payload and each file's name, size and
    sha256. ``files``: ``{field: path | [paths]}`` or a list of general
    attachment paths."""
    from fsv import tenant

    doc = {"v": PLAN_VERSION, "payload": tenant.unbind(payload), "files": _file_entries(files)}
    return tenant.bind_plan(hashlib.sha256(canonical_json(doc)).hexdigest())


def plan_matches(expected: str, actual: str) -> bool:
    """``expected`` may be the full hash or a prefix of at least 12 hex chars."""
    e = (expected or "").strip().lower()
    if e.startswith("sha256:"):
        e = e[7:]
    return len(e) >= MIN_PREFIX and actual.lower().startswith(e)


@dataclass(frozen=True)
class MetadataFingerprint:
    hash: str  # sha256 over the canonical metadata
    updated_at: str | None  # newest updated_at found (ISO), for "metadata as of"
    counts: tuple[tuple[str, int], ...] = ()

    def changed(self, other: "MetadataFingerprint") -> bool:
        return self.hash != other.hash


def _walk_updated(data: Any) -> Iterable[dt.datetime]:
    if isinstance(data, Mapping):
        for k, v in data.items():
            if k == "updated_at" and isinstance(v, str):
                t = _parse_ts(v)
                if t is not None:
                    yield t
            else:
                yield from _walk_updated(v)
    elif isinstance(data, list):
        for v in data:
            yield from _walk_updated(v)


def _parse_ts(v: str) -> dt.datetime | None:
    s = v.strip().replace("Z", "+00:00")
    try:
        t = dt.datetime.fromisoformat(s)
    except ValueError:
        return None
    return t if t.tzinfo else t.replace(tzinfo=dt.timezone.utc)


def _count(data: Any) -> int:
    if isinstance(data, list):
        return len(data)
    if isinstance(data, Mapping):
        for v in data.values():
            if isinstance(v, list):
                return len(v)
        return 1
    return 0


def metadata_fingerprint(
    form_fields: Any = None, rules: Any = None, item: Any = None
) -> MetadataFingerprint:
    """Fingerprint of the metadata a plan was computed from. The rules
    response's ``user`` object is included: rules can condition on it."""
    doc = {"form_fields": form_fields, "rules": rules, "item": item}
    stamps = list(_walk_updated(doc))
    newest = max(stamps).isoformat() if stamps else None
    counts = tuple((k, _count(v)) for k, v in doc.items() if v is not None)
    return MetadataFingerprint(hashlib.sha256(canonical_json(doc)).hexdigest(), newest, counts)
