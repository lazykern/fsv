"""Reviewed ticket notes using the blocked UI contract, not the reply endpoint.

No accepted live write is implied. Keep this separate from generic notes and
from TICKETS.has_notes, which also controls conversation-reading behavior.
"""
from __future__ import annotations

import html
import json
import re
from dataclasses import dataclass
from typing import Any, Literal, Sequence

import typer

from fsv import config, journal
from fsv.client import get_client
from fsv.errors import APIError
from fsv.readback import same_text
from fsv.render import strip_html
from fsv.resources import TICKETS, parse_id
from fsv.safety import note_risks
from fsv.views import load_conversations

_ACTION = "ticket_note"
_EMAIL = re.compile(r"[^\s@<>,;]+@[^\s@<>,;]+\.[^\s@<>,;]+\Z")
_FORMATTED_EMAIL = re.compile(r"[^<>\r\n]+ <([^\s@<>,;]+@[^\s@<>,;]+\.[^\s@<>,;]+)>\Z")


def _valid_recipient(value: Any) -> bool:
    if not isinstance(value, str) or any(ord(ch) < 32 or ord(ch) == 127 for ch in value):
        return False
    return bool(_EMAIL.fullmatch(value) or _FORMATTED_EMAIL.fullmatch(value))


class NoteError(RuntimeError):
    """Safe user-facing failure; an unknown outcome must not be resubmitted."""


@dataclass(frozen=True)
class NotePlan:
    ticket_id: int
    body_html: str
    public: bool
    notify_emails: tuple[str, ...]
    tenant: str

    @property
    def path(self) -> str:
        return f"tickets/{self.ticket_id}/notes"

    @property
    def payload(self) -> dict[str, Any]:
        # A new object each time prevents a caller changing the reviewed body.
        return {"body": self.body_html, "conversation_from_summary": False,
                "private": not self.public, "notify_emails": list(self.notify_emails)}

    @property
    def hash(self) -> str:
        return journal.plan_hash({"tenant": self.tenant, "path": self.path, "payload": self.payload}) or ""

    def review(self) -> dict[str, Any]:
        return {"action": "ticket_note", "tenant": self.tenant, "ticket_id": self.ticket_id,
                "visibility": "public note (visible to customer)" if self.public else "private note",
                "path": f"/api/_/{self.path}", "method": "POST", "payload": self.payload,
                "body_text": strip_html(self.body_html), "plan_hash": self.hash,
                "risks": note_risks(self.public),
                "warning": "Not a reply. Empty notify_emails does not disable tenant workflows or guarantee no notifications."}


def build_plan(ticket_id: int | str, body: str, *, public: bool = False,
               notify_emails: Sequence[str] | None = None,
               body_format: Literal["plain", "html"] = "plain") -> NotePlan:
    """Pure local preparation. Plain text is escaped; explicit HTML is retained.

    Uses the form payload convention of div paragraphs and br line breaks,
    without guessing that a leading '<' means the user intended HTML.
    """
    if isinstance(ticket_id, bool) or not isinstance(ticket_id, (int, str)):
        raise ValueError("ticket ID must be a positive display ID")
    cid = parse_id(str(ticket_id), TICKETS)
    if cid <= 0:
        raise ValueError("ticket ID must be positive")
    if not isinstance(public, bool):
        raise ValueError("public must be a boolean")
    if not isinstance(body, str) or not body.strip() or "\x00" in body:
        raise ValueError("note body must be nonempty text without NUL characters")
    if body_format not in ("plain", "html"):
        raise ValueError("body format must be plain or html")
    if body_format == "html":
        body_html = body
        if not strip_html(body).strip():
            raise ValueError("HTML note body must contain visible text")
    else:
        paragraphs = body.replace("\r\n", "\n").split("\n\n")
        body_html = "".join(f"<div>{html.escape(p).replace(chr(10), '<br>')}</div>"
                            for p in paragraphs if p.strip())
    if isinstance(notify_emails, (str, bytes)) or (notify_emails is not None
                                                 and not isinstance(notify_emails, (list, tuple))):
        raise ValueError("notify_emails must be a list or tuple of email addresses")
    recipients = tuple(notify_emails or ())
    if any(not _valid_recipient(email) for email in recipients):
        raise ValueError("notify_emails contains an invalid email address")
    return NotePlan(cid, body_html, public, recipients, config.DOMAIN)


def _check_plan(plan: NotePlan) -> None:
    # Validate even directly constructed plans, not just build_plan results.
    new = build_plan(plan.ticket_id, plan.body_html, public=plan.public,
                     notify_emails=plan.notify_emails, body_format="html")
    if new != plan:
        raise NoteError("note plan or tenant changed; review a new plan before sending")


def _duplicate(plan: NotePlan) -> None:
    rows = [r for r in journal.read() if r.get("kind") == f"plan:{_ACTION}"
            and r.get("plan") == plan.hash]
    if rows and rows[-1].get("event") in ("started", "done"):
        raise NoteError("this note was sent or its outcome is unknown; check the exact ticket and journal before any retry")


def _returned_note(response: Any) -> dict[str, Any] | None:
    # These are supported response envelopes, not a claim about a live response.
    if not isinstance(response, dict):
        return None
    for key in ("note", "conversation"):
        if isinstance(response.get(key), dict):
            return response[key]
    return response if "id" in response else None


def _note_id(note: dict[str, Any] | None) -> int | str | None:
    value = note.get("id") if note else None
    if isinstance(value, int) and not isinstance(value, bool) and value > 0:
        return value
    if isinstance(value, str) and re.fullmatch(r"[0-9]+", value) and int(value) > 0:
        return value
    return None


def send(plan: NotePlan, client: Any = None, *, reviewed_hash: str | None,
         public_confirmed: bool = False, read_only: bool = False,
         dry_run: bool = False) -> dict[str, Any]:
    """Send at most one POST after exact-body review and separate public consent.

    TUI: display plan.review(), then pass plan.hash only after user confirmation.
    Cancellation is reviewed_hash=None. Pass the effective TUI read_only flag.
    Dry runs/cancellation/read-only never construct a client or write a journal.
    Read-back matches only the returned ID, never body text or 'latest note'.
    """
    _check_plan(plan)
    if dry_run:
        return {"status": "dry_run", **plan.review()}
    if read_only:
        raise NoteError("read-only mode: no note was sent")
    if reviewed_hash is None:
        return {"status": "cancelled", "ticket_id": plan.ticket_id}
    if reviewed_hash != plan.hash:
        raise NoteError("note review hash does not match; review the exact body again")
    if plan.public and public_confirmed is not True:
        raise NoteError("public note requires separate explicit confirmation; no note was sent")
    if not plan.tenant:
        raise NoteError("configure a tenant before sending a note")
    _duplicate(plan)
    # Prepare CSRF first and submit the reviewed payload once.
    try:
        c = client if client is not None else get_client()
        csrf = c._get_csrf()
    except Exception:
        raise NoteError("could not prepare the session; no note POST was attempted") from None
    journal.record_plan("started", plan.hash, action=_ACTION, target=plan.ticket_id)
    rows = [r for r in journal.read() if r.get("kind") == f"plan:{_ACTION}" and r.get("plan") == plan.hash]
    if not rows or rows[-1].get("event") != "started":
        raise NoteError("could not persist the note journal; no note was sent")
    try:
        response = c._request("POST", f"{config.API_INT}/{plan.path}",
                              content=json.dumps(plan.payload, ensure_ascii=False).encode("utf-8"),
                              headers={"Content-Type": "application/json", "X-CSRF-Token": csrf})
    except APIError as exc:
        if 400 <= exc.status < 500 and exc.status not in (403, 408, 429):
            journal.record_plan("failed", plan.hash, action=_ACTION, target=plan.ticket_id)
            raise NoteError(f"note rejected (HTTP {exc.status}); no automatic retry") from None
        raise NoteError("note outcome unknown; check the exact ticket and journal before any retry") from None
    except Exception:
        raise NoteError("note outcome unknown; check the exact ticket and journal before any retry") from None
    note = _returned_note(response)
    nid = _note_id(note)
    if nid is None:
        raise NoteError("note response has no exact note ID; outcome unknown, check the ticket before any retry")
    journal.record_plan("done", plan.hash, action=_ACTION, target=plan.ticket_id, created_id=nid)
    result: dict[str, Any] = {"status": "submitted", "ticket_id": plan.ticket_id, "note_id": nid,
                              "note": note, "plan_hash": plan.hash, "verified": False, "readback": None,
                              "identity_verified": False, "fields_verified": False,
                              "readback_complete": False}
    try:
        rows, meta, missing = load_conversations(c, plan.ticket_id, all_pages=True)
        exact = next((row for row in rows if _note_id(row) is not None and str(row["id"]) == str(nid)), None)
        result["readback"] = exact
        result["readback_complete"] = not missing and not meta.get("partial")
        result["identity_verified"] = exact is not None
        if exact is not None:
            expected = {"body": plan.body_html, "private": not plan.public,
                        "notify_emails": list(plan.notify_emails)}
            result["fields"] = {}
            for key, value in expected.items():
                actual_key = "to_emails" if key == "notify_emails" and key not in exact and "to_emails" in exact else key
                actual = exact.get(actual_key)
                match = same_text(actual, value) if key == "body" else type(actual) is type(value) and actual == value
                result["fields"][key] = ("unavailable" if actual_key not in exact else
                                         "matched" if match else "mismatched")
            result["fields_verified"] = all(value == "matched" for value in result["fields"].values())
            result["verified"] = result["fields_verified"] and result["readback_complete"]
        if not result["verified"]:
            result["warning"] = "Read-back did not fully verify the note fields and history; do not resend."
    except Exception:
        result["warning"] = "Note returned an ID but conversation read-back failed; do not resend."
    return result


def register(app: typer.Typer) -> None:
    """Register only on the tickets Typer app; does not change resource flags."""
    @app.command("add-note", help="Review and add a ticket note (private by default; not a reply).")
    def add_note(
        id_: str = typer.Argument(..., metavar="ID"),
        body: str = typer.Option(..., "--body", "-b", help="note body to review"),
        public: bool = typer.Option(False, "--public", help="select a public note visible to the customer"),
        confirm_public: bool = typer.Option(False, "--confirm-public", help="separately confirm public visibility"),
        html_body: bool = typer.Option(False, "--html", help="body is already HTML; otherwise escape plain text"),
        notify: list[str] | None = typer.Option(None, "--notify-email", help="explicit notification recipient (repeatable; default none)"),
        set_status: str | None = typer.Option(None, "--set-status", help="add and set status (Pending, Resolved or Closed; runtime requirements apply)"),
        dry_run: bool = typer.Option(False, "--dry-run", help="review only; no POST or journal writes"),
        read_only: bool = typer.Option(False, "--read-only", help="refuse sends"),
        yes: bool = typer.Option(False, "--yes", "-y", help="confirm the exact previously reviewed --expect-plan"),
        expected: str | None = typer.Option(None, "--expect-plan", help="full hash from a prior body review"),
        no_input: bool = typer.Option(False, "--no-input", help="never prompt"),
        json_out: bool = typer.Option(False, "--json", help="emit structured result"),
    ) -> None:
        try:
            if set_status is not None:
                from fsv.ticket_conversation import ConversationError, build_plan as build_conversation, send as send_conversation
                try:
                    if read_only and not dry_run:
                        raise ConversationError("read-only mode: no note was sent")
                    c = get_client()
                    plan = build_conversation(c, id_, body, kind="note", public=public, notify=tuple(notify or ()),
                                              body_format="html" if html_body else "plain", set_status=set_status)
                    if dry_run:
                        typer.echo(json.dumps({"status": "dry_run", **plan.review()}, ensure_ascii=False, indent=2))
                        return
                    typer.echo(json.dumps(plan.review(), ensure_ascii=False, indent=2), err=True)
                    if expected is not None and expected != plan.hash:
                        raise ConversationError("note review hash does not match; review again")
                    if yes:
                        if expected != plan.hash:
                            raise ConversationError("--yes requires the full --expect-plan from a prior review")
                    elif no_input:
                        raise ConversationError("sending requires --yes and --expect-plan in no-input mode")
                    elif not typer.confirm("Add this exact reviewed note?", default=False):
                        typer.echo("Cancelled; no note was sent.")
                        return
                    if public and not confirm_public:
                        if yes or no_input:
                            raise ConversationError("--public also requires --confirm-public")
                        if not typer.confirm("Confirm PUBLIC note visible to the customer?", default=False):
                            typer.echo("Cancelled; no note was sent.")
                            return
                        confirm_public = True
                    result = send_conversation(plan, c, reviewed_hash=plan.hash, public_confirmed=confirm_public)
                    typer.echo(json.dumps(result, ensure_ascii=False, indent=2))
                    return
                except ConversationError as exc:
                    raise NoteError(str(exc)) from None
            plan = build_plan(id_, body, public=public, notify_emails=notify,
                              body_format="html" if html_body else "plain")
            if dry_run:
                typer.echo(json.dumps(send(plan, reviewed_hash=None, dry_run=True), ensure_ascii=False, indent=2))
                return
            typer.echo(json.dumps(plan.review(), ensure_ascii=False, indent=2), err=True)
            if read_only:
                raise NoteError("read-only mode: no note was sent")
            if expected is not None and expected != plan.hash:
                raise NoteError("note review hash does not match; review the exact body again")
            if yes:
                if expected != plan.hash:
                    raise NoteError("--yes requires the full --expect-plan from a prior body review")
            elif no_input:
                raise NoteError("sending requires --yes and --expect-plan in no-input mode")
            elif not typer.confirm("Add this exact reviewed note?", default=False):
                typer.echo("Cancelled; no note was sent.")
                return
            if public and not confirm_public:
                if yes or no_input:
                    raise NoteError("--public also requires --confirm-public")
                if not typer.confirm("Confirm PUBLIC note visible to the customer (not a reply)?", default=False):
                    typer.echo("Cancelled; no note was sent.")
                    return
                confirm_public = True
            result = send(plan, reviewed_hash=plan.hash, public_confirmed=confirm_public)
            if json_out:
                typer.echo(json.dumps(result, ensure_ascii=False, indent=2))
            else:
                typer.echo(f"Note {result['note_id']} submitted on ticket {plan.ticket_id}; exact-ID read-back: {result.get('identity_verified', result['verified'])}; all fields verified: {result['verified']}")
                if result.get("warning"):
                    typer.echo(result["warning"], err=True)
        except (ValueError, NoteError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None
