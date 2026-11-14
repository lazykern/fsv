"""Reviewed inline images in ticket replies and notes.

Captured wire (agent rich-text editor):

1. ``POST /api/_/solutions/inline_images`` as ``multipart/form-data`` with one
   field ``file`` (filename + image content type) -> 201 ``{"link", "fileid"}``.
2. The editor embeds ``<img src="<link>" ... fileid="<id>" data-id="<id>">`` in
   the body HTML; the reply/note POST carries that HTML in ``body`` (and the
   reply's ``full_text``). There is no attachment-id array for inline images.
3. Readback: the exact conversation's ``body`` keeps the ``<img>`` with the
   link and ``data-id`` (the server drops ``fileid``).

No delete path exists: removing an image in the editor sends no request and
the probed delete routes answer 404/500, so an uploaded inline image whose
reply/note fails stays orphaned. Uploads and the owning write are not atomic;
every step is single-attempt and journaled; never replay.
"""
from __future__ import annotations

from fsv.tenant import tenant_json

import hashlib
import json
import mimetypes
import secrets
from dataclasses import dataclass, replace
from html import escape, unescape
from pathlib import Path
from typing import Any, Callable

from fsv import attachments, config, journal
from fsv.reviewed_write import ReviewedWriteError
from fsv.watchers import positive_id

UPLOAD_PATH = "solutions/inline_images"
# The editor's file input accepts exactly these image types.
IMAGE_TYPES = frozenset({"image/jpeg", "image/png", "image/gif", "image/webp"})


@dataclass(frozen=True)
class InlinePlan:
    attached: attachments.AttachedPlan

    @property
    def base(self):
        return self.attached.base

    @property
    def files(self):
        return self.attached.files

    @property
    def limit(self) -> int | None:
        return self.attached.limit

    @property
    def hash(self) -> str:
        return journal.plan_hash({"inline_images": self.attached.hash}) or ""

    def review(self) -> dict[str, Any]:
        review = self.attached.review()
        review.update({
            "plan_hash": self.hash,
            "inline_images": [{"name": f.name, "size": f.size, "content_type": f.mime} for f in self.files],
            "upload": "POST /api/_/solutions/inline_images, multipart field `file`, once per image; each returned "
                      "link is embedded as <img src=link data-id=fileid> after the body (no attachment IDs)",
            "warning": "Uploads and the reply/note are not atomic. A failed or unknown step leaves orphan inline "
                       "images that cannot be deleted (no delete route exists). Inspect the journal and ticket; "
                       "never replay.",
        })
        return review


def _check_images(files: tuple[attachments.LocalFile, ...]) -> None:
    for f in files:
        mime = mimetypes.guess_type(f.name)[0]
        if mime not in IMAGE_TYPES or f.mime not in IMAGE_TYPES:
            raise ReviewedWriteError(f"inline image must be JPEG, PNG, GIF or WebP: {f.name}")


def plan(client: Any, target: str | int, body: str, paths: list[str], *, kind: str, public: bool = False,
         sender: str | None = None, html_body: bool = False, notify: tuple[str, ...] = (),
         max_bytes: int | None = None) -> InlinePlan:
    if kind not in {"reply", "note"}:
        raise ReviewedWriteError("inline images are captured only for ticket replies and notes")
    attached = attachments.conversation_plan(client, target, body, paths, kind=kind, public=public, sender=sender,
                                             html_body=html_body, notify=notify, max_bytes=max_bytes)
    attached = replace(attached, base=replace(attached.base, action=attached.base.action.replace(
        "_attachments", "_inline_images")))
    _check_images(attached.files)
    return InlinePlan(attached)


def image_html(link: str, fileid: int) -> str:
    """The editor's exact inline-image block."""
    return (f'<div><img src="{escape(link, quote=True)}" style="width: auto;" class="fr-fil fr-dib" '
            f'fileid="{fileid}" data-id="{fileid}"></div>')


def _multipart(name: str, mime: str, raw: bytes) -> tuple[bytes, str]:
    boundary = "----fsv" + secrets.token_hex(16)
    safe = name.replace('"', "%22").replace("\r", "").replace("\n", "")
    head = (f"--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{safe}\"\r\n"
            f"Content-Type: {mime}\r\n\r\n").encode()
    return head + raw + f"\r\n--{boundary}--\r\n".encode(), f"multipart/form-data; boundary={boundary}"


def _upload_result(response: Any) -> tuple[str, int]:
    if not isinstance(response, dict):
        raise ReviewedWriteError("inline image upload returned no JSON object")
    link, fileid = response.get("link"), positive_id(response.get("fileid"))
    if (not isinstance(link, str) or not link.startswith("https://") or any(c in link for c in "\"'<> \r\n\t")):
        raise ReviewedWriteError("inline image upload returned no usable https link")
    return link, fileid


def final_payload(payload: dict[str, Any], images: list[tuple[str, int]]) -> dict[str, Any]:
    blocks = "".join(image_html(link, fid) for link, fid in images)
    out = dict(payload)
    for key in ("body", "full_text"):
        if isinstance(out.get(key), str):
            out[key] = out[key] + blocks
    return out


def images_in_body(row: Any, images: list[tuple[str, int]]) -> dict[str, str]:
    body = row.get("body") if isinstance(row, dict) else None
    if not isinstance(body, str):
        return {str(fid): "unavailable" for _l, fid in images}
    text = unescape(body)
    return {str(fid): "matched" if (link in text and f'data-id="{fid}"' in text) else "mismatched"
            for link, fid in images}


def send(p: InlinePlan, client: Any, *, reviewed_hash: str | None,
         rebuild: Callable[[], InlinePlan]) -> dict[str, Any]:
    if reviewed_hash is None:
        return {"status": "cancelled", "verified": False}
    base = p.base
    if not base.tenant or base.tenant != config.DOMAIN or reviewed_hash != p.hash:
        raise ReviewedWriteError("tenant or full inline-image review hash changed")
    if p.limit is None:
        raise ReviewedWriteError("the tenant publishes no attachment size limit; review again with --max-bytes N; "
                                 "nothing was sent")
    action = base.action
    events = [e for e in journal.read() if e.get("kind") == "plan:" + action and e.get("plan") == p.hash]
    refusal = journal.replay_refusal(events, "inline-image transaction was sent or outcome unknown; do not replay")
    if refusal:
        raise ReviewedWriteError(refusal)
    if rebuild().hash != p.hash or attachments.local_files([f.path for f in p.files], p.limit) != p.files:
        raise ReviewedWriteError("ticket, metadata or image bytes changed; review again (nothing was sent)")
    csrf = client._get_csrf()
    journal.record_plan("started", p.hash, action=action, target=base.target)
    events = [e for e in journal.read() if e.get("kind") == "plan:" + action and e.get("plan") == p.hash]
    if not events or events[-1].get("event") != "started":
        raise ReviewedWriteError("could not persist inline-image journal; nothing was sent")
    images: list[tuple[str, int]] = []
    try:
        for f in p.files:
            raw = Path(f.path).read_bytes()
            if len(raw) != f.size or hashlib.sha256(raw).hexdigest() != f.sha256:
                raise ReviewedWriteError("image changed after review")
            content, ctype = _multipart(f.name, f.mime, raw)
            response = client._request("POST", f"{config.API_INT}/{UPLOAD_PATH}", content=content,
                                       headers={"Content-Type": ctype, "Accept": "application/json",
                                                "X-CSRF-Token": csrf})
            link, fid = _upload_result(response)
            if any(fid == known for _l, known in images):
                raise ReviewedWriteError("upload returned a duplicate inline image identity")
            images.append((link, fid))
            journal.record_plan("uploaded", p.hash, action=action + "_upload", target=base.target, created_id=fid)
        payload = final_payload(base.payload, images)
        response = client._request(base.method, f"{config.API_INT}/{base.path}",
                                   content=json.dumps(payload).encode(),
                                   headers={"Content-Type": "application/json", "X-CSRF-Token": csrf})
    except Exception:
        raise ReviewedWriteError("inline-image transaction failed or outcome unknown; uploaded image IDs: "
                                 + json.dumps([fid for _l, fid in images])
                                 + " (no delete route exists); inspect the ticket and journal, do not replay") from None
    raw_owner = attachments._returned_note(response)
    try:
        owner = positive_id(raw_owner.get("id") if isinstance(raw_owner, dict) else None)
    except ReviewedWriteError:
        owner = None
    journal.record_plan("done", p.hash, action=action, target=base.target, created_id=owner)
    result: dict[str, Any] = {"status": "submitted", "plan_hash": p.hash, "inline_image_ids": [f for _l, f in images],
                              "owner_id": owner, "verified": False}
    try:
        final = replace(p.attached, base=replace(base, payload_json=json.dumps(payload)))
        checked = attachments.readback(client, final, response, [])
        from fsv.views import load_conversations
        rows, _meta, _missing = load_conversations(client, base.target, all_pages=True)
        match = [r for r in rows if str(r.get("id")) == str(owner)]
        inline = images_in_body(match[0] if len(match) == 1 else None, images)
        checked["fields"].pop("full_text", None)
        result.update({"owner_id": checked["owner_id"], "fields": checked["fields"], "inline_images": inline,
                       "verified": checked["verified"] and all(v == "matched" for v in inline.values())})
    except Exception:
        pass
    if not result["verified"]:
        result["warning"] = "Independent conversation readback did not confirm every image link; do not replay."
    return result


def finish(p: InlinePlan, client: Any, *, dry_run: bool, yes: bool, expected: str | None,
           rebuild: Callable[[], InlinePlan]) -> None:
    import typer
    if dry_run:
        typer.echo(tenant_json({"status": "dry_run", **p.review()}, indent=2))
        return
    if not yes or expected != p.hash:
        raise ReviewedWriteError("review with --dry-run, then --yes --expect-plan FULL_HASH")
    result = send(p, client, reviewed_hash=expected, rebuild=rebuild)
    typer.echo(tenant_json(result, indent=2))
    if not result.get("verified"):
        raise typer.Exit(4)
