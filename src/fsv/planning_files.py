"""Reviewed planning-field update (changes and problems): description text
plus file uploads.

``changes|problems update ID --planning FIELD [--description TEXT] [--file F ...]``
is one reviewed transaction of two kinds of step: one attachment upload
(``POST /api/_/attachments``) per new file, then the planning-field save
(``PUT .../planning-fields/<name>`` for an existing row, ``POST
.../planning-fields?id=<name>`` for a new one) carrying the kept and the
uploaded attachment IDs. The plan hash binds the tenant, change, section,
description, local file names + sizes + sha256, the duplicate decisions,
the size limit and a record-state token. The journal gets one entry per
upload and one for the save; an unknown step is never replayed (no
duplicate attachments). Readback re-reads the planning inventory and
compares the row's attachment IDs and names and its description.

Problems use the same captured wire on ``problems/<id>/planning-fields``
for their three native analysis sections (``problem_cause``,
``problem_impact``, ``problem_symptom``); the size limit then comes from the
problem form metadata.
"""
from __future__ import annotations

import base64
import hashlib
import json
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable
from urllib.parse import quote

from fsv import config, journal
from fsv.attachments import (NOT_PUBLISHED, LocalFile, local_files, owner_row, resolve_limit)
from fsv.create import match_planning_row, planning_server_key, resolve_planning_definition
from fsv.planning_text import definitions, inventory, section_content
from fsv.readback import same_text, submitted_value_matches
from fsv.reviewed_write import ReviewedWriteError
from fsv.watchers import positive_id

ACTION = "change_planning_update"
PROBLEM_ACTION = "problem_planning_update"
ACTIONS = {"changes": ACTION, "problems": PROBLEM_ACTION}
FORM_FIELDS = {"changes": "change_form_fields", "problems": "problem_form_fields"}


def _resolve_section(module: str, section: str, defs: list[dict[str, Any]]) -> dict[str, Any]:
    if module == "changes":
        return resolve_planning_definition(section, defs)
    raw = str(section or "").strip()
    hits = [d for d in defs if raw == d["name"] or raw.casefold() == str(d["label"]).casefold()]
    if len(hits) != 1:
        known = ", ".join(f"{d['label']} ({d['name']})" for d in defs)
        raise ValueError(f"unknown problem planning section {section!r}; choices: {known}")
    return hits[0]
DUPLICATE_MODES = ("prompt", "skip", "replace", "append")


def _att_name(att: dict[str, Any]) -> str:
    return str(att.get("name") or att.get("content_file_name") or "")


def _att_size(att: dict[str, Any]) -> Any:
    return att.get("size", att.get("content_file_size"))


def _existing_attachments(row: dict[str, Any] | None) -> list[dict[str, Any]]:
    if row is None:
        return []
    value = row.get("attachments", [])
    if value is None:
        value = []
    if not isinstance(value, list) or any(not isinstance(a, dict) for a in value):
        raise ReviewedWriteError("planning attachment inventory unavailable; no write attempted")
    ids = [positive_id(a.get("id")) for a in value]
    if len(set(ids)) != len(ids):
        raise ReviewedWriteError("planning attachment inventory has duplicate IDs; no write attempted")
    return value


def _ask_duplicate(name: str, count: int) -> str:
    if not sys.stdin.isatty():
        raise ReviewedWriteError(f"attachment already exists: {name}; use --duplicate skip|replace|append")
    sys.stderr.write(f"  duplicate {name} ({count} existing). Choose [s]kip, [r]eplace, [a]ppend: ")
    sys.stderr.flush()
    answer = input().strip().casefold()
    return "replace" if answer in ("r", "replace") else "append" if answer in ("a", "append") else "skip"


@dataclass(frozen=True)
class PlanningUpdatePlan:
    tenant: str
    target: int
    section: str
    label: str
    method: str
    path: str
    description: str | None
    before: str
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
    module: str = "changes"

    @property
    def action(self) -> str:
        return ACTIONS[self.module]

    @property
    def noop(self) -> bool:
        return (not self.files and not self.replaced and
                (self.description is None or (self.method == "PUT" and same_text(self.before, self.description))))

    @property
    def hash(self) -> str:
        return journal.plan_hash({
            "action": self.action, "tenant": self.tenant, "target": self.target, "section": self.section,
            "method": self.method, "path": self.path, "description": self.description,
            "kept_attachment_ids": list(self.kept_ids), "replaced": [list(r) for r in self.replaced],
            "skipped": list(self.skipped), "files": [f.review() for f in self.files], "duplicate": self.duplicate,
            "max_bytes": self.limit, "limit_source": self.limit_source, "published_bytes": self.published_limit,
            "context": self.context_hash, "record_state": self.state}) or ""

    def payload(self, uploaded: list[int]) -> dict[str, Any]:
        body: dict[str, Any] = {}
        if self.description is not None:
            body["description"] = self.description
        if self.method == "POST" or self.files or self.replaced:
            ids = list(self.kept_ids) + list(uploaded)
            if ids or self.method == "PUT":
                body["attachments"] = ids
        return body

    def review(self) -> dict[str, Any]:
        out: dict[str, Any] = {
            "action": self.action, "target": self.target, "section": self.section, "label": self.label,
            "steps": ([{"step": "upload", "method": "POST", "path": "/api/_/attachments", "file": f.name}
                       for f in self.files] +
                      [{"step": "planning", "method": self.method, "path": "/api/_/" + self.path}]),
            "description": {"before": self.before,
                            "after": self.before if self.description is None else self.description,
                            "sent": self.description is not None},
            "files": [f.review() for f in self.files], "duplicate": self.duplicate,
            "kept_attachment_ids": list(self.kept_ids),
            "replaced_attachments": [{"id": i, "name": n} for i, n in self.replaced],
            "skipped_files": list(self.skipped), "record_state": self.state, "plan_hash": self.hash,
            "warning": ("Uploads and the planning save are not atomic. A failed or unknown step may leave "
                        "orphan uploads; inspect the change and journal, never replay the transaction."),
        }
        if self.files:
            published = NOT_PUBLISHED if self.published_limit is None else self.published_limit
            out["attachment_size_limit"] = {
                "max_bytes": self.limit, "source": self.limit_source, "published_bytes": published,
                "total_bytes": sum(f.size for f in self.files), "applies_to": "total of all files in this submit"}
            if self.limit is None:
                out["attachment_size_limit"]["note"] = ("the tenant publishes no attachment size limit; "
                                                        "pass --max-bytes N to set one before sending")
        return out


def build_plan(client: Any, target: int, section: str, description: str | None, paths: list[str], *,
               duplicate: str = "prompt", backup_replaced: bool | None = None,
               max_bytes: int | None = None, module: str = "changes") -> PlanningUpdatePlan:
    if module not in ACTIONS:
        raise ValueError("planning files are supported for changes and problems")
    mode = (duplicate or "").casefold()
    if mode not in DUPLICATE_MODES:
        raise ValueError("--duplicate must be one of: prompt, skip, replace, append")
    if backup_replaced:
        raise ValueError("--backup-replaced is not on the reviewed planning path; download the attachment "
                         f"first (fsv {module} download ID --planning SECTION), then --duplicate replace")
    if description is not None and not description.strip():
        raise ValueError("--description must be nonempty text/HTML")
    if description is None and not paths:
        raise ValueError("--planning needs --description and/or --file")
    cid = positive_id(target)
    defs = definitions(client, module)
    definition = _resolve_section(module, section, defs)
    rows = inventory(client, module, cid)
    existing = match_planning_row(definition, rows, defs)
    key = str(existing["name"]) if existing is not None and existing.get("name") else planning_server_key(definition)
    attachments = _existing_attachments(existing)
    before = (section_content(existing) or "") if existing is not None else ""
    if existing is not None and section_content(existing) is None and any(
            k in existing for k in ("description", "content", "description_html", "description_text", "body")):
        raise ReviewedWriteError("planning text state is unavailable; no write attempted")
    kept = [positive_id(a.get("id")) for a in attachments]
    replaced: list[tuple[int, str]] = []
    skipped: list[str] = []
    upload: list[str] = []
    for value in paths:
        name = Path(value).name
        matches = [a for a in attachments if _att_name(a) == name]
        action = mode
        if matches and action == "prompt":
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
    if description is None and upload and not before.strip():
        description = ", ".join(Path(p).name for p in paths)  # the server needs text with files
    limit_value, source, published = None, NOT_PUBLISHED, None
    files: tuple[LocalFile, ...] = ()
    if upload:
        limit = resolve_limit(client, client.int_get(FORM_FIELDS[module]), max_bytes)
        limit_value, source, published = limit.max_bytes, limit.source, limit.published
        files = local_files(upload, limit_value)
    elif max_bytes is not None and (type(max_bytes) is not int or max_bytes <= 0):
        raise ReviewedWriteError("--max-bytes must be a positive whole number of bytes")
    if existing is None and description is None:
        raise ReviewedWriteError("a new planning section needs --description text; no write attempted")
    method = "PUT" if existing is not None else "POST"
    base = f"{module}/{cid}/planning-fields"
    path = f"{base}/{quote(key, safe='')}" if method == "PUT" else f"{base}?id={quote(key, safe='')}"
    owner = owner_row(client, module, cid)
    state = journal.repeat_token((ACTIONS[module],), cid, owner)
    return PlanningUpdatePlan(config.DOMAIN, cid, key, str(definition.get("label") or section), method, path,
                              description, before, tuple(kept), tuple(replaced), tuple(skipped), files, mode,
                              limit_value, source, published,
                              journal.snapshot_hash({"definitions": defs, "rows": rows}) or "", state, module)


def readback(client: Any, plan: PlanningUpdatePlan, uploaded: list[int]) -> dict[str, Any]:
    defs = definitions(client, plan.module)
    definition = next((d for d in defs if planning_server_key(d) == plan.section), {"name": plan.section})
    row = match_planning_row(definition, inventory(client, plan.module, plan.target), defs)
    if row is None:
        return {"section": plan.section, "attachments": "unavailable", "description": "unavailable", "verified": False}
    observed = {positive_id(a.get("id")): a for a in _existing_attachments(row)}
    expected = set(plan.kept_ids) | set(uploaded)
    files_ok = set(observed) == expected and all(
        _att_name(observed[i]) == f.name and (_att_size(observed[i]) is None or submitted_value_matches(_att_size(observed[i]), f.size))
        for i, f in zip(uploaded, plan.files))
    content = section_content(row)
    if plan.description is None:
        desc = "not sent"
    else:
        desc = "unavailable" if content is None else "matched" if same_text(content, plan.description) else "mismatched"
    return {"section": plan.section,
            "attachments": [{"id": i, "name": _att_name(a)} for i, a in sorted(observed.items())],
            "attachments_verified": files_ok, "description": desc,
            "verified": files_ok and desc in ("matched", "not sent")}


def upload_files(client: Any, files: tuple[LocalFile, ...], headers: dict[str, str], uploaded: list[int], *,
                 plan_hash: str, action: str, target: int) -> None:
    """Upload each reviewed file once (``POST /api/_/attachments``), checking
    its bytes against the review and journaling every returned ID before the
    next step. ``uploaded`` collects the IDs (also on failure)."""
    for file in files:
        raw = Path(file.path).read_bytes()
        if len(raw) != file.size or hashlib.sha256(raw).hexdigest() != file.sha256:
            raise ReviewedWriteError("attachment changed after review")
        body = {"content": f"data:{file.mime};base64," + base64.b64encode(raw).decode("ascii"),
                "content_file_name": file.name, "content_file_size": file.size, "content_content_type": file.mime}
        response = client._request("POST", f"{config.API_INT}/attachments",
                                   content=json.dumps(body).encode(), headers=headers)
        ident = positive_id(response.get("id") if isinstance(response, dict) else None)
        if ident in uploaded:
            raise ReviewedWriteError("upload returned duplicate attachment identity")
        uploaded.append(ident)
        journal.record_plan("uploaded", plan_hash, action=action + "_upload", target=target, created_id=ident)
        persisted = [e for e in journal.read()
                     if e.get("kind") == "plan:" + action + "_upload" and e.get("plan") == plan_hash]
        if not persisted or persisted[-1].get("created_id") != ident:
            raise ReviewedWriteError("could not persist uploaded attachment identity")


def send(plan: PlanningUpdatePlan, client: Any, *, reviewed_hash: str | None,
         rebuild: Callable[[], PlanningUpdatePlan]) -> dict[str, Any]:
    if reviewed_hash is None:
        return {"status": "cancelled", "verified": False}
    if not plan.tenant or plan.tenant != config.DOMAIN or reviewed_hash != plan.hash:
        raise ReviewedWriteError("tenant or full planning review hash changed; review again")
    if plan.files and plan.limit is None:
        raise ReviewedWriteError("the tenant publishes no attachment size limit; review again with --max-bytes N; "
                                 "no upload attempted")
    action = plan.action
    events = [e for e in journal.read() if e.get("kind") == "plan:" + action and e.get("plan") == plan.hash]
    refusal = journal.replay_refusal(events, "planning transaction was sent or outcome unknown; do not replay")
    if refusal:
        raise ReviewedWriteError(refusal)
    fresh = rebuild()
    if fresh.hash != plan.hash or (plan.files and local_files([f.path for f in plan.files], plan.limit) != plan.files):
        raise ReviewedWriteError(f"{plan.module[:-1]}, planning section or file bytes changed since review; "
                                 "review again")
    try:
        csrf = client._get_csrf()
    except Exception:
        raise ReviewedWriteError("could not prepare session; no write attempted") from None
    journal.record_plan("started", plan.hash, action=action, target=plan.target)
    events = [e for e in journal.read() if e.get("kind") == "plan:" + action and e.get("plan") == plan.hash]
    if not events or events[-1].get("event") != "started":
        raise ReviewedWriteError("could not persist planning journal; no write attempted")
    headers = {"Content-Type": "application/json", "X-CSRF-Token": csrf}
    uploaded: list[int] = []
    step = "upload"
    try:
        upload_files(client, plan.files, headers, uploaded, plan_hash=plan.hash, action=action, target=plan.target)
        step = "planning"
        client._request(plan.method, f"{config.API_INT}/{plan.path}",
                        content=json.dumps(plan.payload(uploaded)).encode(), headers=headers)
    except Exception as exc:
        status = getattr(exc, "status", None)
        detail = f" (HTTP {status})" if isinstance(status, int) and status else ""
        raise ReviewedWriteError(f"planning transaction failed at the {step} step{detail} or outcome unknown; "
                                 "uploaded IDs: " + json.dumps(uploaded) + f"; inspect the {plan.module[:-1]} and journal, "
                                 "do not replay", http_status=status if isinstance(status, int) and status else None) from None
    journal.record_plan("done", plan.hash, action=action, target=plan.target)
    result: dict[str, Any] = {"status": "submitted", "plan_hash": plan.hash, "uploaded_ids": uploaded, "verified": False}
    try:
        result.update(readback(client, plan, uploaded))
    except Exception:
        result["warning"] = "Independent planning readback failed; do not resend."
    if result.get("verified") is True:
        journal.record_verified(plan.hash, action=action, target=plan.target)
    else:
        result.setdefault("warning", "Planning readback was not fully verified; do not resend.")
    return result
