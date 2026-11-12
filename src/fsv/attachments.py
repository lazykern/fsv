"""Hash-bound local files, single-attempt uploads, and owning-write readback.

No multipart uploads, guessed upload limits, attachment GET, or automatic cleanup.

Size limit: the tenant publishes it in the agent app page as
``window.MAX_ATTACHMENT_LIMIT = <n>;``. The web UI divides file sizes by
1048576 and compares the total of all files in one form with it, so ``n`` is
MiB per submit. Explicit byte keys in form metadata, when present, take
precedence. When the tenant publishes neither, the review says so and a send
needs an explicit ``--max-bytes`` (never a built-in default).
"""
from __future__ import annotations

import base64
import hashlib
import json
import mimetypes
import re
from dataclasses import dataclass
from decimal import Decimal
from pathlib import Path
from typing import Any, Callable

from fsv import config, journal
from fsv.readback import same_text, submitted_value_matches
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute
from fsv.ticket_notes import _returned_note
from fsv.views import load_conversations
from fsv.watchers import positive_id


MIB = 1024 * 1024
NOT_PUBLISHED = "not published by the tenant"
PAGE_SOURCE = "tenant agent page MAX_ATTACHMENT_LIMIT (MiB, total per submit)"
METADATA_SOURCE = "tenant form metadata (bytes)"
OVERRIDE_SOURCE = "--max-bytes override"
PAGE_PATH = "/a/dashboard"
_PAGE_LIMIT = re.compile(r"\bMAX_ATTACHMENT_LIMIT\s*=\s*([^;\s]+)\s*;")


def size_limit(metadata: Any) -> int | None:
    """Explicitly byte-denominated limits in form metadata; None when absent.
    Invalid or conflicting values block."""
    found = []
    def visit(value):
        if isinstance(value, dict):
            for key, item in value.items():
                if key in {"max_attachment_size_bytes", "max_file_size_bytes"}:
                    if type(item) is not int or item <= 0:
                        raise ReviewedWriteError("invalid runtime attachment byte limit")
                    found.append(item)
                elif key in {"attachment_settings", "attachments", "settings", "options", "field_options", "ticket_fields", "ticket_form_fields", "problem_fields", "problem_form_fields", "fields"}:
                    visit(item)
        elif isinstance(value, list):
            for item in value:
                visit(item)
    visit(metadata)
    if len(set(found)) > 1:
        raise ReviewedWriteError("conflicting runtime attachment byte limits; no upload attempted")
    return found[0] if found else None


def page_limit(client: Any) -> int | None:
    """Bytes from the agent page's MAX_ATTACHMENT_LIMIT (MiB); None if absent."""
    from fsv.client import _looks_like_login_html
    from fsv.session import SessionError
    try:
        r = client._send("GET", f"https://{config.DOMAIN}{PAGE_PATH}",
                         headers={"Accept": "text/html,application/xhtml+xml"}, follow_redirects=False)
    except Exception:
        raise ReviewedWriteError("could not read the tenant's published attachment size limit; no upload attempted") from None
    if 300 <= r.status_code < 400 or r.status_code in (401, 403) or (r.status_code == 200 and _looks_like_login_html(r.text)):
        raise SessionError("session expired or invalid; run `fsv auth login`")
    if r.status_code != 200:
        raise ReviewedWriteError(f"could not read the tenant's published attachment size limit (HTTP {r.status_code}); no upload attempted")
    values = set(_PAGE_LIMIT.findall(r.text))
    if not values:
        return None
    if len(values) != 1 or not re.fullmatch(r"[0-9]+(\.[0-9]+)?", next(iter(values))):
        raise ReviewedWriteError("ambiguous or invalid published attachment size limit; no upload attempted")
    limit = int(Decimal(next(iter(values))) * MIB)
    if limit <= 0:
        raise ReviewedWriteError("invalid published attachment size limit; no upload attempted")
    return limit


@dataclass(frozen=True)
class Limit:
    max_bytes: int | None
    source: str
    published: int | None = None


def resolve_limit(client: Any, metadata: Any, max_bytes: int | None = None) -> Limit:
    if max_bytes is not None and (type(max_bytes) is not int or max_bytes <= 0):
        raise ReviewedWriteError("--max-bytes must be a positive whole number of bytes")
    published = size_limit(metadata)
    source = METADATA_SOURCE
    if published is None:
        published, source = page_limit(client), PAGE_SOURCE
    if max_bytes is not None:
        return Limit(max_bytes, OVERRIDE_SOURCE, published)
    if published is None:
        return Limit(None, NOT_PUBLISHED)
    return Limit(published, source, published)


def metadata(client: Any, module: str) -> Any:
    return client.int_get("ticket_form_fields" if module == "tickets" else "problem_form_fields")


@dataclass(frozen=True)
class LocalFile:
    path: str
    name: str
    size: int
    mime: str
    sha256: str

    def review(self) -> dict[str, Any]:
        return {"name": self.name, "size": self.size, "content_type": self.mime, "sha256": self.sha256}


def _over(total: int, limit: int | None) -> bool:
    return limit is not None and total > limit


def local_files(paths: list[str], limit: int | None) -> tuple[LocalFile, ...]:
    """Unique non-empty regular files; the total must fit an explicit limit."""
    files = []
    seen = set()
    for value in paths:
        path = Path(value).expanduser().resolve()
        if path in seen or not path.is_file():
            raise ReviewedWriteError("attachments must be unique regular local files")
        seen.add(path)
        if any(ord(c) < 32 for c in path.name):
            raise ReviewedWriteError("unsafe attachment filename")
        if path.stat().st_size <= 0:
            raise ReviewedWriteError("attachment is empty: " + path.name)
        if _over(path.stat().st_size + sum(f.size for f in files), limit):
            raise ReviewedWriteError(f"attachments exceed the {limit}-byte size limit; no upload attempted")
        raw = path.read_bytes()
        if not raw or _over(len(raw) + sum(f.size for f in files), limit):
            raise ReviewedWriteError("attachment changed, is empty or exceeds the size limit; no upload attempted")
        files.append(LocalFile(str(path), path.name, len(raw), mimetypes.guess_type(path.name)[0] or "application/octet-stream", hashlib.sha256(raw).hexdigest()))
    if not files:
        raise ReviewedWriteError("at least one attachment required")
    return tuple(files)


@dataclass(frozen=True)
class AttachedPlan:
    base: WritePlan
    files: tuple[LocalFile, ...]
    limit: int | None
    limit_source: str = METADATA_SOURCE
    published_limit: int | None = None

    @property
    def hash(self) -> str:
        return journal.plan_hash({"owner": self.base.hash, "files": [f.review() for f in self.files], "max_bytes": self.limit,
                                  "limit_source": self.limit_source, "published_bytes": self.published_limit}) or ""

    def review(self) -> dict[str, Any]:
        published = NOT_PUBLISHED if self.published_limit is None else self.published_limit
        limit = {"max_bytes": self.limit, "source": self.limit_source, "published_bytes": published,
                 "total_bytes": sum(f.size for f in self.files), "applies_to": "total of all files in this submit"}
        if self.limit is None:
            limit["note"] = "the tenant publishes no attachment size limit; pass --max-bytes N to set one before sending"
        return {**self.base.review(), "plan_hash": self.hash, "files": [f.review() for f in self.files],
                "max_attachment_size_bytes": self.limit, "attachment_size_limit": limit, "upload": "POST /api/_/attachments, JSON base64 data URI, once per file; returned IDs become owning payload attachments",
                "warning": "Uploads and owning write are not atomic. A failed/unknown step may leave orphan uploads. Inspect journal and owner; never replay the transaction."}


def wrap(base: WritePlan, paths: list[str], raw_metadata: Any, *, client: Any,
         max_bytes: int | None = None) -> AttachedPlan:
    limit = resolve_limit(client, raw_metadata, max_bytes)
    return AttachedPlan(base, local_files(paths, limit.max_bytes), limit.max_bytes, limit.source, limit.published)


def owner_row(client: Any, module: str, target: int) -> dict[str, Any]:
    key = {"tickets": "ticket", "problems": "problem", "changes": "change"}[module]
    raw = client.int_get(f"{module}/{target}")
    row = raw.get(key, raw) if isinstance(raw, dict) else None
    if not isinstance(row, dict) or positive_id(row.get("id")) != target:
        raise ReviewedWriteError("exact attachment owner unavailable")
    return row


def conversation_plan(client: Any, target: str | int, body: str, paths: list[str], *, kind: str,
                      public: bool = False, sender: str | None = None, html_body: bool = False,
                      notify: tuple[str, ...] = (), max_bytes: int | None = None) -> AttachedPlan:
    from fsv.ticket_conversation import build_plan
    if kind == "reply" and not sender:
        raise ReviewedWriteError("attached reply requires explicit configured --sender")
    plan = build_plan(client, target, body, kind=kind, public=public, sender=sender,
                      body_format="html" if html_body else "plain", notify=notify)
    # Bind the target baseline as well as payload and metadata; no reply-draft writes.
    parent = owner_row(client, "tickets", plan.ticket_id)
    raw = metadata(client, "tickets")
    base = WritePlan("ticket_" + kind + "_attachments", plan.ticket_id, config.DOMAIN, "POST", plan.path,
                     plan.payload_json, journal.plan_hash({"parent": parent, "metadata": raw}) or "")
    return wrap(base, paths, raw, client=client, max_bytes=max_bytes)


def problem_note_plan(client: Any, target: str | int, body: str, paths: list[str], *,
                      max_bytes: int | None = None) -> AttachedPlan:
    from fsv.problem_workflows import child_plan
    plan = child_plan(client, target, kind="note", text=body)
    raw = metadata(client, "problems")
    base = WritePlan("problem_note_attachments", plan.target, config.DOMAIN, plan.method, plan.path,
                     plan.payload_json, journal.plan_hash({"base": plan.hash, "metadata": raw}) or "")
    return wrap(base, paths, raw, client=client, max_bytes=max_bytes)


def _rows_attachments(row: Any) -> list[dict[str, Any]]:
    if not isinstance(row, dict) or not isinstance(row.get("attachments"), list):
        raise ReviewedWriteError("owner attachment inventory unavailable")
    rows = row["attachments"]
    ids = [positive_id(r.get("id")) for r in rows if isinstance(r, dict)]
    if len(ids) != len(rows) or len(set(ids)) != len(ids):
        raise ReviewedWriteError("invalid attachment inventory")
    return rows


def readback(client: Any, plan: AttachedPlan, response: Any, ids: list[int]) -> dict[str, Any]:
    base = plan.base
    if base.path == "tickets":
        raw = response.get("ticket", response) if isinstance(response, dict) else {}
        returned_id = positive_id(raw.get("id"))
        row = owner_row(client, "tickets", returned_id)
    elif base.path.startswith("problems/"):
        from fsv.problem_workflows import inventory
        raw = response.get("note", response) if isinstance(response, dict) else {}
        returned_id = positive_id(raw.get("id"))
        matches = [r for r in inventory(client, base.target, "notes") if str(r.get("id")) == str(returned_id)]
        row = matches[0] if len(matches) == 1 else None
    else:
        raw = _returned_note(response)
        returned_id = positive_id(raw.get("id") if raw else None)
        rows, meta, missing = load_conversations(client, base.target, all_pages=True)
        if missing or meta.get("partial"):
            raise ReviewedWriteError("complete conversation readback unavailable")
        matches = [r for r in rows if str(r.get("id")) == str(returned_id)]
        row = matches[0] if len(matches) == 1 else None
    attachments = _rows_attachments(row)
    observed = {positive_id(r["id"]): r for r in attachments}
    files_match = set(observed) == set(ids) and all(
        observed[i].get("name") == f.name and submitted_value_matches(observed[i].get("size"), f.size)
        for i, f in zip(ids, plan.files))
    # Ignore transport-only keys, not submitted business values.
    ignored = {"conversation_from_summary", "full_text", "translate_reply", "freddy_message_id", "br_validation_excludes"}
    fields = {}
    for key, expected in base.payload.items():
        if key in ignored:
            continue
        actual_key = "to_emails" if key == "notify_emails" and row is not None and key not in row else key
        actual = row.get(actual_key) if row is not None else None
        match = same_text(actual, expected) if key in {"body", "description"} else submitted_value_matches(actual, expected)
        fields[key] = "unavailable" if row is None or actual_key not in row else "matched" if match else "mismatched"
    return {"owner_id": returned_id, "attachments": ids, "attachments_verified": files_match,
            "fields": fields, "verified": files_match and all(v == "matched" for v in fields.values())}


def send(plan: AttachedPlan, client: Any, *, reviewed_hash: str | None,
         rebuild: Callable[[], AttachedPlan]) -> dict[str, Any]:
    if reviewed_hash is None:
        return {"status": "cancelled", "verified": False}
    if not plan.base.tenant or plan.base.tenant != config.DOMAIN or reviewed_hash != plan.hash:
        raise ReviewedWriteError("tenant or full attachment review hash changed")
    if plan.limit is None:
        raise ReviewedWriteError("the tenant publishes no attachment size limit; review again with --max-bytes N; no upload attempted")
    action = plan.base.action
    events = [e for e in journal.read() if e.get("kind") == "plan:" + action and e.get("plan") == plan.hash]
    if events and events[-1].get("event") in {"started", "done"}:
        raise ReviewedWriteError("attachment transaction was sent or outcome unknown; do not replay")
    if rebuild().hash != plan.hash or local_files([f.path for f in plan.files], plan.limit) != plan.files:
        raise ReviewedWriteError("owner, metadata or attachment bytes changed; review again")
    csrf = client._get_csrf()
    journal.record_plan("started", plan.hash, action=action, target=plan.base.target)
    events = [e for e in journal.read() if e.get("kind") == "plan:" + action and e.get("plan") == plan.hash]
    if not events or events[-1].get("event") != "started":
        raise ReviewedWriteError("could not persist attachment journal; no upload attempted")
    headers = {"Content-Type": "application/json", "X-CSRF-Token": csrf}
    uploaded = []
    try:
        for file in plan.files:
            raw = Path(file.path).read_bytes()
            if len(raw) != file.size or hashlib.sha256(raw).hexdigest() != file.sha256:
                raise ReviewedWriteError("attachment changed after review")
            body = {"content": f"data:{file.mime};base64," + base64.b64encode(raw).decode("ascii"),
                    "content_file_name": file.name, "content_file_size": file.size, "content_content_type": file.mime}
            response = client._request("POST", f"{config.API_INT}/attachments", content=json.dumps(body).encode(), headers=headers)
            ident = positive_id(response.get("id") if isinstance(response, dict) else None)
            if ident in uploaded:
                raise ReviewedWriteError("upload returned duplicate attachment identity")
            uploaded.append(ident)
            journal.record_plan("uploaded", plan.hash, action=action + "_upload", target=plan.base.target, created_id=ident)
            persisted = [e for e in journal.read() if e.get("kind") == "plan:" + action + "_upload" and e.get("plan") == plan.hash]
            if not persisted or persisted[-1].get("created_id") != ident:
                raise ReviewedWriteError("could not persist uploaded attachment identity")
        body = {**plan.base.payload, "attachments": uploaded}
        response = client._request(plan.base.method, f"{config.API_INT}/{plan.base.path}", content=json.dumps(body).encode(), headers=headers)
    except Exception:
        raise ReviewedWriteError("attachment transaction failed or outcome unknown; uploaded IDs: " + json.dumps(uploaded) + "; inspect owner and journal, do not replay") from None
    journal.record_plan("done", plan.hash, action=action, target=plan.base.target)
    result = {"status": "submitted", "plan_hash": plan.hash, "uploaded_ids": uploaded, "verified": False}
    try:
        result.update(readback(client, plan, response, uploaded))
    except Exception:
        pass
    if not result["verified"]:
        result["warning"] = "Independent owning-record readback not fully verified; do not replay."
    return result


def deletion_owner(client: Any, owner: str, child: int | None = None, section: str | None = None) -> dict[str, Any]:
    match = re.fullmatch(r"(tickets|problems|changes)/([1-9][0-9]*)", owner)
    if not match or (child is not None and section is not None):
        raise ValueError("owner must be tickets/ID, problems/ID or changes/ID; choose at most one child or section")
    module, target = match[1], int(match[2])
    parent = owner_row(client, module, target)
    if section is not None:
        from fsv.planning_text import inventory
        matches = [r for r in inventory(client, module, target) if r.get("name") == section]
        if len(matches) != 1:
            raise ReviewedWriteError("exact planning owner unavailable")
        return matches[0]
    if child is None:
        return parent
    child = positive_id(child)
    if module == "tickets":
        rows, meta, missing = load_conversations(client, target, all_pages=True)
        if missing or meta.get("partial"):
            raise ReviewedWriteError("complete conversation inventory unavailable")
    elif module == "problems":
        from fsv.problem_workflows import inventory
        rows = inventory(client, target, "notes")
    else:
        raise ReviewedWriteError("change note attachment deletion not proven")
    matches = [r for r in rows if str(r.get("id")) == str(child)]
    if len(matches) != 1:
        raise ReviewedWriteError("exact child attachment owner unavailable")
    return matches[0]


def delete_plan(client: Any, ident: int | str, owner: str, *, child: int | None = None, section: str | None = None) -> WritePlan:
    ident = positive_id(ident)
    row = deletion_owner(client, owner, child, section)
    if ident not in {positive_id(r["id"]) for r in _rows_attachments(row)}:
        raise ReviewedWriteError("attachment not present on exact owner; orphan deletion cannot be verified")
    return WritePlan("attachment_delete", ident, config.DOMAIN, "DELETE", f"attachments/{ident}", "null",
                     journal.plan_hash({"owner": owner, "child": child, "section": section, "row": row}) or "")


def delete(plan: WritePlan, client: Any, owner: str, *, reviewed_hash: str | None,
           child: int | None = None, section: str | None = None) -> dict[str, Any]:
    before_ids = {positive_id(r["id"]) for r in _rows_attachments(deletion_owner(client, owner, child, section))}
    def verify(_response):
        rows = _rows_attachments(deletion_owner(client, owner, child, section))
        return {"attachment_id": plan.target, "verified": {positive_id(r["id"]) for r in rows} == before_ids - {plan.target}}
    return execute(plan, client, reviewed_hash=reviewed_hash,
                   rebuild=lambda: delete_plan(client, plan.target, owner, child=child, section=section), verify=verify)
