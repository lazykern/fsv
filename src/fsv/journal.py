"""Local write journal: an append-only JSONL record of every write request.

Location: ``$XDG_STATE_HOME/fsv/journal.jsonl`` (default ``~/.local/state/fsv``).
Entries never contain cookies, headers or request bodies: only a SHA-256 of
the normalized JSON body (the "plan hash"), so what was sent can be compared
with a reviewed draft without storing its content.
"""
from __future__ import annotations

import hashlib
import json
import os
import re
import time
from pathlib import Path
from typing import Any

_ID_SEGMENT = re.compile(r"^\d+$")
_VERBS = {"POST": "create", "PUT": "update", "PATCH": "update", "DELETE": "delete"}


def journal_path() -> Path:
    base = os.environ.get("XDG_STATE_HOME") or str(Path.home() / ".local" / "state")
    return Path(base) / "fsv" / "journal.jsonl"


def plan_hash(body: Any) -> str | None:
    """sha256 of the body as canonical JSON (sorted keys, no whitespace)."""
    if body is None:
        return None
    raw = json.dumps(body, sort_keys=True, separators=(",", ":"), ensure_ascii=False, default=str)
    return hashlib.sha256(raw.encode("utf-8")).hexdigest()


def _split_path(url: str) -> list[str]:
    path = re.sub(r"^https?://[^/]+", "", url).split("?", 1)[0].split("#", 1)[0]
    return [p for p in path.split("/") if p]


def _classify(method: str, segments: list[str]) -> tuple[str, str | None, str]:
    """-> (kind, target id, normalized path without query)."""
    segs = list(segments)
    while segs and segs[0] in ("api", "_", "v2", "itil"):
        segs.pop(0)
    last = segs[-1] if segs else ""
    resource = next((s for s in segs if not _ID_SEGMENT.match(s)), "")
    target = None
    for i, s in enumerate(segs):
        if _ID_SEGMENT.match(s):
            target = f"{segs[i - 1]}/{s}" if i else s
            break
    verb = _VERBS.get(method.upper(), method.lower())
    if method.upper() == "POST" and last in ("notes", "reply", "attachments"):
        kind = {"notes": "note", "reply": "reply", "attachments": "upload"}[last]
    else:
        kind = f"{verb}:{resource}" if resource else verb
    return kind, target, "/" + "/".join(segments)


def _created_id(data: Any) -> Any:
    if not isinstance(data, dict):
        return None
    if data.get("id") is not None and not isinstance(data["id"], (dict, list)):
        return data["id"]
    for v in data.values():
        if isinstance(v, dict) and v.get("id") is not None and not isinstance(v["id"], (dict, list)):
            return v["id"]
    return None


def _command() -> str | None:
    try:
        try:  # typer >= 0.26 vendors click
            from typer._click.globals import get_current_context
        except ImportError:
            from click import get_current_context
        ctx = get_current_context(silent=True)
        return ctx.command_path if ctx else None
    except Exception:
        return None


def record(method: str, url: str, body: Any, *, status: int | None, ok: bool,
           response: Any = None, error: str | None = None) -> None:
    """Append one entry. Never raises: the journal must not break a command."""
    try:
        kind, target, path = _classify(method, _split_path(url))
        entry = {
            "ts": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
            "command": _command(),
            "kind": kind,
            "target": target,
            "method": method.upper(),
            "path": path,
            "plan_hash": plan_hash(body),
            "status": status,
            "ok": ok,
            "created_id": _created_id(response) if ok and method.upper() == "POST" else None,
        }
        if error:
            entry["error"] = error[:120]
        p = journal_path()
        p.parent.mkdir(parents=True, exist_ok=True)
        fd = os.open(p, os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o600)
        try:
            os.write(fd, (json.dumps(entry, ensure_ascii=False) + "\n").encode("utf-8"))
        finally:
            os.close(fd)
    except Exception:
        pass


def read(limit: int | None = None) -> list[dict[str, Any]]:
    p = journal_path()
    if not p.exists():
        return []
    rows: list[dict[str, Any]] = []
    for line in p.read_text(encoding="utf-8").splitlines():
        try:
            rows.append(json.loads(line))
        except ValueError:
            continue
    return rows[-limit:] if limit else rows
