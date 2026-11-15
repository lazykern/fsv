"""Local write journal: an append-only JSONL record of every write request.

Location: ``$XDG_STATE_HOME/fsv/journals/<profile>/<tenant>/journal.jsonl``.
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

from fsv import config

_ID_SEGMENT = re.compile(r"^\d+$")
_VERBS = {"POST": "create", "PUT": "update", "PATCH": "update", "DELETE": "delete"}
_PATH_SEGMENTS = frozenset({
    "api", "_", "v2", "itil", "changes", "tickets", "problems", "releases",
    "notes", "reply", "attachments", "tasks", "assets", "associate", "detach",
    "planning-fields", "catalog", "service-items", "place-request",
})


def journal_path() -> Path:
    base = os.environ.get("XDG_STATE_HOME") or str(Path.home() / ".local" / "state")
    profile = hashlib.sha256(os.fsencode(config.CONFIG_DIR.resolve())).hexdigest()
    domain = config.DOMAIN
    tenant = hashlib.sha256(domain.encode("utf-8")).hexdigest() if domain else "unknown"
    return Path(base) / "fsv" / "journals" / profile / tenant / "journal.jsonl"


def _raw_hash(body: Any) -> str:
    raw = json.dumps(body, sort_keys=True, separators=(",", ":"), ensure_ascii=False, default=str)
    return hashlib.sha256(raw.encode("utf-8")).hexdigest()


def plan_hash(body: Any) -> str | None:
    """sha256 of the body as canonical JSON (sorted keys, no whitespace),
    bound to the configured tenant: a plan reviewed on one tenant never
    matches on another (``fsv.tenant.bind_plan``)."""
    if body is None:
        return None
    from fsv import tenant

    return tenant.bind_plan(_raw_hash(tenant.unbind(body)))


def snapshot_hash(body: Any) -> str | None:
    """Hash stable read-side state; keep plan_hash exact for wire payloads."""
    from fsv.reviewed_snapshot import stable_snapshot

    return _raw_hash(stable_snapshot(body))


def _split_path(url: str) -> list[str]:
    path = re.sub(r"^https?://[^/]+", "", url).split("?", 1)[0].split("#", 1)[0]
    return [p if p in _PATH_SEGMENTS or _ID_SEGMENT.fullmatch(p) else "[redacted]"
            for p in path.split("/") if p]


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
        return _safe_id(data["id"])
    for v in data.values():
        if isinstance(v, dict) and v.get("id") is not None and not isinstance(v["id"], (dict, list)):
            return _safe_id(v["id"])
    return None


def _safe_id(value: Any) -> Any:
    """Keep numeric service IDs, never arbitrary server-provided text."""
    if isinstance(value, int) and not isinstance(value, bool):
        return value
    if isinstance(value, str) and _ID_SEGMENT.fullmatch(value):
        return value
    return None


def _safe_target(value: Any) -> Any:
    if isinstance(value, str) and re.fullmatch(r"catalog-item/\d+", value):
        return value
    return _safe_id(value)


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
            "tenant_bound": True,
            "status": status,
            "ok": ok,
            "created_id": _created_id(response) if ok and method.upper() == "POST" else None,
        }
        if error:
            entry["error"] = "details omitted"
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
    from fsv.tenant import bind_legacy

    rows: list[dict[str, Any]] = []
    for line in p.read_text(encoding="utf-8").splitlines():
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if isinstance(row, dict) and not row.get("tenant_bound"):
            # Written before plan hashes were tenant-bound: this journal is
            # per tenant, so bind its hashes to compare with current plans
            # (an unresolved older submission must still refuse a replay).
            for key in ("plan", "plan_hash"):
                if key in row:
                    row[key] = bind_legacy(row[key])
        rows.append(row)
    return rows[-limit:] if limit else rows


# ---------------------------------------------------------------------------
# reviewed plans (form-engine creates/updates)
#
# A plan entry ties the reviewed plan hash (``fsv.forms.plan_hash``: body +
# files) to what happened: ``started`` before the first request, then
# ``done`` (with the created id and per-step results) or ``failed``. A
# ``started`` without an outcome means the outcome is unknown (crash,
# timeout) and counts as a possible duplicate.

PLAN_EVENTS = ("started", "done", "failed")


def _append(entry: dict[str, Any]) -> None:
    p = journal_path()
    p.parent.mkdir(parents=True, exist_ok=True)
    fd = os.open(p, os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o600)
    try:
        os.write(fd, (json.dumps(entry, ensure_ascii=False, default=str) + "\n").encode("utf-8"))
    finally:
        os.close(fd)



def delete_state_tokens(rows: list[dict[str, Any]], targets: list[Any],
                        restore_actions: tuple[str, ...]) -> list[dict[str, Any]]:
    """Record-state tokens that move when a record is restored, for delete plans.

    Stable snapshots drop ``updated_at``, so a record restored to the same
    business state would plan the same delete hash and a legitimate delete
    after a restore would be refused as a replay. Each token binds the raw
    server ``updated_at`` read at review time (the server bumps it on restore)
    and how many restores of this exact target fsv has journaled as answered
    (monotonic, even when the server keeps ``updated_at``). A true replay
    against an unchanged record keeps the token, so the guard still refuses it;
    a ``started`` without an answer is unchanged and stays non-replayable.
    The key is not ``updated_at``: snapshot hashing drops that name.
    """
    kinds = {f"plan:{action}" for action in restore_actions}
    counts: dict[Any, int] = {}
    for event in read():
        if event.get("event") == "done" and event.get("kind") in kinds:
            counts[event.get("target")] = counts.get(event.get("target"), 0) + 1
    return [{"server_version": row.get("updated_at"), "restores": counts.get(_safe_target(target), 0)}
            for row, target in zip(rows, targets)]


def record_verified(plan: str, *, action: str, target: Any = None) -> None:
    """Note that an answered plan was also verified by independent readback.

    Kept in ``verified.jsonl`` next to the journal (same profile and tenant),
    so replay guards and ``fsv journal`` that read the last plan event are
    unaffected. Hashes, kinds and ids only. Never raises."""
    try:
        entry = {"ts": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "kind": f"verified:{action}", "plan": plan,
                 "target": _safe_target(target)}
        p = _verified_path()
        p.parent.mkdir(parents=True, exist_ok=True)
        fd = os.open(p, os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o600)
        try:
            os.write(fd, (json.dumps(entry, ensure_ascii=False) + "\n").encode("utf-8"))
        finally:
            os.close(fd)
    except Exception:
        pass


def _verified_path() -> Path:
    return journal_path().with_name("verified.jsonl")


def _verified_rows() -> list[dict[str, Any]]:
    try:
        lines = _verified_path().read_text(encoding="utf-8").splitlines()
    except OSError:
        return []
    rows = []
    for line in lines:
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if isinstance(row, dict):
            rows.append(row)
    return rows


def verified_sends(actions: tuple[str, ...], target: Any) -> int:
    """How many plans of ``actions`` on ``target`` this journal has recorded as
    sent, answered and verified by readback. Monotonic. A ``started`` without
    an answer, or an answered but unverified send (the server may not have
    applied it; "do not resend"), does not count, so the next review plans
    the same hash and the replay guard still refuses it."""
    kinds = {f"verified:{action}" for action in actions}
    want = str(_safe_target(target))
    return sum(1 for event in _verified_rows() if event.get("kind") in kinds and str(event.get("target")) == want)


_NO_ROW = object()


def repeat_token(actions: tuple[str, ...], target: Any, row: Any = _NO_ROW) -> dict[str, Any]:
    """Record-state token for writes that may legitimately repeat a payload.

    A plan hash of only target + payload (+ stable snapshot, which drops
    ``updated_at``) is the same for update #2 and #4 of an A/B/A/B
    alternation, so a legitimate later send was refused as a replay. The
    token binds the raw server ``updated_at`` read at review time (``row``;
    omitted for writes that read no record, e.g. notes) and the number of
    verified sends of the same action on the same target (``verified_sends``).
    A legitimate repeat after the record changed plans a fresh hash; a replay
    of the same reviewed hash is refused by the journal guard, and a fresh
    review after an ambiguous or unverified send against an unchanged record
    rebuilds the same hash, so it is refused too.
    The key is not ``updated_at``: snapshot hashing drops that name.
    """
    token: dict[str, Any] = {"sends": verified_sends(actions, target)}
    if row is not _NO_ROW:
        token["server_version"] = row.get("updated_at") if isinstance(row, dict) else None
    return token


# Set when this process refused a journaled replay (already sent, or an
# unresolved send); the shared CLI command maps such a stop to EXIT_REPLAY.
_refused = False


def note_refusal(message: str | None = None) -> str | None:
    """Mark that a journal replay guard refused this command (nothing was
    sent); returns ``message`` unchanged so callers can raise it."""
    global _refused
    _refused = True
    return message


def refusal_noted() -> bool:
    return _refused


def clear_refusal() -> None:
    global _refused
    _refused = False


def stale_plan_refusal(expected: str | None, actions: tuple[str, ...]) -> str | None:
    """Why a reviewed hash that no longer matches the rebuilt plan must not be
    sent, when the journal shows that exact plan was already sent (or may
    have been); None when the journal has no such plan."""
    if not expected:
        return None
    kinds = {f"plan:{action}" for action in actions}
    want = expected.strip().lower().removeprefix("sha256:")
    if len(want) < 12:  # the shortest accepted --expect-plan prefix
        return None
    events = [e for e in read() if e.get("kind") in kinds and isinstance(e.get("plan"), str)
              and e["plan"].startswith(want)]
    last = events[-1] if events else {}
    when = last.get("ts") or "an earlier run"
    if last.get("event") == "started":
        return note_refusal(f"plan {expected} was started at {when} with no recorded outcome; nothing was sent now. "
                "Inspect the record and `fsv journal tail` before any retry; do not replay it")
    if last.get("event") == "done":
        return note_refusal(f"plan {expected} was already sent at {when} and the server answered; nothing was sent now. "
                "Plans are single-use; to send the same values again, review again with --dry-run (the new "
                "plan hash differs because the record or the journal moved on)")
    return None


def replay_refusal(events: list[dict[str, Any]], unknown: str) -> str | None:
    """Why a journaled plan must not be sent again; None when it may be sent.

    ``started`` without a later event is an ambiguous outcome and keeps the
    caller's ``unknown`` text. ``done`` means this exact plan was already sent
    and answered: say so with its time, because an identical repeat (e.g. a
    delete again after a restore) hashes the same when nothing changed.
    """
    last = events[-1] if events else {}
    event = last.get("event")
    if event == "started":
        return note_refusal(unknown)
    if event == "done":
        when = last.get("ts") or "an earlier run"
        return note_refusal(f"this exact plan was already sent at {when} and the server answered; nothing was sent now. "
                "Plans are single-use and this one hashes the same because the target, payload and record "
                "state are unchanged; there is no override. Make a real change (e.g. edit a field of the "
                "record, or change the requested values), then review again for a new plan hash; do not replay it")
    return None

def record_plan(event: str, plan: str, *, action: str, target: Any = None, created_id: Any = None,
                steps: list[dict[str, Any]] | None = None, error: str | None = None) -> None:
    """Append a plan event. Never raises."""
    try:
        entry: dict[str, Any] = {
            "ts": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
            "epoch": int(time.time()),
            "command": _command(),
            "kind": f"plan:{action}",
            "event": event,
            "plan": plan,
            "tenant_bound": True,
            "target": _safe_target(target),
            "created_id": _safe_id(created_id),
        }
        if steps is not None:
            # Filenames and exception messages can contain private data.
            entry["steps"] = [
                {"step": s.get("step") if s.get("step") in
                 ("create", "update", "attachments", "upload", "place-request", "after")
                 else "other", "ok": bool(s.get("ok"))}
                for s in steps
            ]
        if error:
            entry["error"] = "details omitted"
        _append(entry)
    except Exception:
        pass


def recent_plan(plan: str, *, action: str, hours: float = 24.0) -> dict[str, Any] | None:
    """The latest entry showing this plan was (or may have been) sent within
    ``hours``: a ``done`` entry, or a ``started`` one with no outcome.
    ``failed`` plans (rejected by the server) can be retried."""
    cutoff = time.time() - hours * 3600
    outcome: dict[str, dict[str, Any]] = {}
    started: list[dict[str, Any]] = []
    for row in read():
        if row.get("plan") != plan or row.get("kind") != f"plan:{action}":
            continue
        if (row.get("epoch") or 0) < cutoff:
            continue
        if row.get("event") == "started":
            started.append(row)
        else:
            outcome[str(row.get("epoch"))] = row
    done = [r for r in outcome.values() if r.get("event") == "done"]
    if done:
        return max(done, key=lambda r: r.get("epoch") or 0)
    for s in reversed(started):
        later = [r for r in outcome.values() if (r.get("epoch") or 0) >= (s.get("epoch") or 0)]
        if not later:
            return s
    return None
