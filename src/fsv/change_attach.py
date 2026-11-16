"""Reviewed attach-only change update (``changes update ID --attach FILE``).

One transaction of two kinds of step: one upload (``POST /api/_/attachments``)
per new file, then ``PUT /api/_/changes/<id>`` with the full attachment ID
list (kept + uploaded), the same wire the legacy path sent. The plan hash
binds the tenant, change, file names + sizes + sha256, the duplicate
decisions, the size limit (``--max-bytes`` or the tenant's published limit)
and a record-state token. ``--yes`` needs the full ``--expect-plan``; the
journal gets one entry per upload and one for the save; an unknown step is
never replayed. Readback re-reads the change's attachment list and compares
IDs and names (``verified``). ``--duplicate`` keeps its rules: prompt (only
when a same-name attachment exists), skip, replace, append.
"""
from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable

from fsv import config, journal
from fsv.attachments import NOT_PUBLISHED, LocalFile, local_files, owner_row, resolve_limit
from fsv.planning_files import DUPLICATE_MODES, _ask_duplicate, _att_name, _att_size, upload_files
from fsv.readback import submitted_value_matches
from fsv.reviewed_write import ReviewedWriteError
from fsv.watchers import positive_id

ACTION = "change_attachments_update"


def _attachments(row: dict[str, Any]) -> list[dict[str, Any]]:
    value = row.get("attachments", [])
    if value is None:
        value = []
    if not isinstance(value, list) or any(not isinstance(a, dict) for a in value):
        raise ReviewedWriteError("change attachment inventory unavailable; no write attempted")
    ids = [positive_id(a.get("id")) for a in value]
    if len(set(ids)) != len(ids):
        raise ReviewedWriteError("change attachment inventory has duplicate IDs; no write attempted")
    return value


@dataclass(frozen=True)
class ChangeAttachPlan:
    tenant: str
    target: int
    kept_ids: tuple[int, ...]
    replaced: tuple[tuple[int, str], ...]
    skipped: tuple[str, ...]
    files: tuple[LocalFile, ...]
    duplicate: str
    limit: int | None
    limit_source: str
    published_limit: int | None
    context_hash: str
    state: dict[str, Any] = field(default_factory=dict)
    action: str = ACTION
    label: str = "Attachments"

    @property
    def path(self) -> str:
        return f"changes/{self.target}"

    @property
    def noop(self) -> bool:
        return not self.files

    @property
    def hash(self) -> str:
        return journal.plan_hash({
            "action": ACTION, "tenant": self.tenant, "target": self.target, "method": "PUT", "path": self.path,
            "kept_attachment_ids": list(self.kept_ids), "replaced": [list(r) for r in self.replaced],
            "skipped": list(self.skipped), "files": [f.review() for f in self.files], "duplicate": self.duplicate,
            "max_bytes": self.limit, "limit_source": self.limit_source, "published_bytes": self.published_limit,
            "context": self.context_hash, "record_state": self.state}) or ""

    def payload(self, uploaded: list[int]) -> dict[str, Any]:
        return {"attachments": list(self.kept_ids) + list(uploaded)}

    def review(self) -> dict[str, Any]:
        published = NOT_PUBLISHED if self.published_limit is None else self.published_limit
        out: dict[str, Any] = {
            "action": ACTION, "target": self.target,
            "steps": ([{"step": "upload", "method": "POST", "path": "/api/_/attachments", "file": f.name}
                       for f in self.files] + [{"step": "attach", "method": "PUT", "path": "/api/_/" + self.path}]),
            "files": [f.review() for f in self.files], "duplicate": self.duplicate,
            "kept_attachment_ids": list(self.kept_ids),
            "replaced_attachments": [{"id": i, "name": n} for i, n in self.replaced],
            "skipped_files": list(self.skipped), "record_state": self.state, "plan_hash": self.hash,
            "attachment_size_limit": {"max_bytes": self.limit, "source": self.limit_source,
                                      "published_bytes": published, "total_bytes": sum(f.size for f in self.files),
                                      "applies_to": "total of all files in this submit"},
            "warning": ("Uploads and the attachment save are not atomic. A failed or unknown step may leave "
                        "orphan uploads; inspect the change and journal, never replay the transaction."),
        }
        if self.limit is None:
            out["attachment_size_limit"]["note"] = ("the tenant publishes no attachment size limit; "
                                                    "pass --max-bytes N to set one before sending")
        return out


def build_plan(client: Any, target: int, paths: list[str], *, duplicate: str = "prompt",
               backup_replaced: bool | None = None, max_bytes: int | None = None,
               no_input: bool = False) -> ChangeAttachPlan:
    mode = (duplicate or "").casefold()
    if mode not in DUPLICATE_MODES:
        raise ValueError("--duplicate must be one of: prompt, skip, replace, append")
    if backup_replaced:
        raise ValueError("--backup-replaced is not on the reviewed attachment path; download the attachment "
                         "first (fsv changes download ID), then --duplicate replace")
    if not paths:
        raise ValueError("--attach needs at least one file")
    cid = positive_id(target)
    row = owner_row(client, "changes", cid)
    attachments = _attachments(row)
    kept = [positive_id(a.get("id")) for a in attachments]
    replaced: list[tuple[int, str]] = []
    skipped: list[str] = []
    upload: list[str] = []
    for value in paths:
        name = Path(value).name
        matches = [a for a in attachments if _att_name(a) == name]
        action = mode
        if matches and action == "prompt":
            if no_input:
                raise ReviewedWriteError(f"attachment already exists: {name}; --no-input cannot prompt, "
                                         "use --duplicate skip|replace|append")
            action = _ask_duplicate(name, len(matches))
        if matches and action == "skip":
            skipped.append(name)
            continue
        if matches and action == "replace":
            for a in matches:
                ident = positive_id(a.get("id"))
                if ident in kept:
                    kept.remove(ident)
                    replaced.append((ident, name))
        upload.append(value)
    limit_value, source, published = None, NOT_PUBLISHED, None
    files: tuple[LocalFile, ...] = ()
    if upload:
        limit = resolve_limit(client, None, max_bytes)
        limit_value, source, published = limit.max_bytes, limit.source, limit.published
        total = sum(Path(p).expanduser().stat().st_size for p in upload if Path(p).expanduser().is_file())
        if limit_value is not None and total > limit_value:
            raise ReviewedWriteError(f"--attach files total {total} bytes and exceed the {limit_value}-byte limit "
                                     f"({source}); nothing was uploaded")
        files = local_files(upload, limit_value)
    elif max_bytes is not None and (type(max_bytes) is not int or max_bytes <= 0):
        raise ReviewedWriteError("--max-bytes must be a positive whole number of bytes")
    state = journal.repeat_token((ACTION,), cid, row)
    return ChangeAttachPlan(config.DOMAIN, cid, tuple(kept), tuple(replaced), tuple(skipped), files, mode,
                            limit_value, source, published,
                            journal.snapshot_hash({"attachments": attachments}) or "", state)


def readback(client: Any, plan: ChangeAttachPlan, uploaded: list[int]) -> dict[str, Any]:
    observed = {positive_id(a.get("id")): a for a in _attachments(owner_row(client, "changes", plan.target))}
    expected = set(plan.kept_ids) | set(uploaded)
    ok = set(observed) == expected and all(
        _att_name(observed[i]) == f.name
        and (_att_size(observed[i]) is None or submitted_value_matches(_att_size(observed[i]), f.size))
        for i, f in zip(uploaded, plan.files))
    return {"attachments": [{"id": i, "name": _att_name(a)} for i, a in sorted(observed.items())],
            "attachments_verified": ok, "verified": ok}


def send(plan: ChangeAttachPlan, client: Any, *, reviewed_hash: str | None,
         rebuild: Callable[[], ChangeAttachPlan]) -> dict[str, Any]:
    if reviewed_hash is None:
        return {"status": "cancelled", "verified": False}
    if not plan.tenant or plan.tenant != config.DOMAIN or reviewed_hash != plan.hash:
        raise ReviewedWriteError("tenant or full attachment review hash changed; review again")
    if plan.files and plan.limit is None:
        raise ReviewedWriteError("the tenant publishes no attachment size limit; review again with --max-bytes N; "
                                 "no upload attempted")
    events = [e for e in journal.read() if e.get("kind") == "plan:" + ACTION and e.get("plan") == plan.hash]
    refusal = journal.replay_refusal(events, "attachment transaction was sent or outcome unknown; do not replay")
    if refusal:
        raise ReviewedWriteError(refusal)
    fresh = rebuild()
    if fresh.hash != plan.hash or (plan.files and local_files([f.path for f in plan.files], plan.limit) != plan.files):
        raise ReviewedWriteError("change attachments or file bytes changed since review; review again")
    try:
        csrf = client._get_csrf()
    except Exception:
        raise ReviewedWriteError("could not prepare session; no write attempted") from None
    journal.record_plan("started", plan.hash, action=ACTION, target=plan.target)
    events = [e for e in journal.read() if e.get("kind") == "plan:" + ACTION and e.get("plan") == plan.hash]
    if not events or events[-1].get("event") != "started":
        raise ReviewedWriteError("could not persist attachment journal; no write attempted")
    headers = {"Content-Type": "application/json", "X-CSRF-Token": csrf}
    uploaded: list[int] = []
    step = "upload"
    try:
        upload_files(client, plan.files, headers, uploaded, plan_hash=plan.hash, action=ACTION, target=plan.target)
        step = "attach"
        client._request("PUT", f"{config.API_INT}/{plan.path}",
                        content=json.dumps(plan.payload(uploaded)).encode(), headers=headers)
    except Exception as exc:
        status = getattr(exc, "status", None)
        detail = f" (HTTP {status})" if isinstance(status, int) and status else ""
        raise ReviewedWriteError(f"attachment transaction failed at the {step} step{detail} or outcome unknown; "
                                 "uploaded IDs: " + json.dumps(uploaded) + "; inspect the change and journal, "
                                 "do not replay", http_status=status if isinstance(status, int) and status else None) from None
    journal.record_plan("done", plan.hash, action=ACTION, target=plan.target)
    result: dict[str, Any] = {"status": "submitted", "plan_hash": plan.hash, "uploaded_ids": uploaded,
                              "skipped": list(plan.skipped), "verified": False}
    try:
        result.update(readback(client, plan, uploaded))
    except Exception:
        result["warning"] = "Independent attachment readback failed; do not resend."
    if result.get("verified") is True:
        journal.record_verified(plan.hash, action=ACTION, target=plan.target)
    else:
        result.setdefault("warning", "Attachment readback was not fully verified; do not resend.")
    return result


__all__ = ["ACTION", "ChangeAttachPlan", "build_plan", "readback", "send"]
