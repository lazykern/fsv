"""Reviewed reply or note-with-status from blocked UI submissions."""
from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any, Literal

from fsv import config, journal
from fsv.errors import APIError
from fsv.formmeta import RULES_UNAVAILABLE_KEY, existing_from_record, is_status, load_meta, resolve_requester
from fsv.forms import FormState, build_br_validation_excludes
from fsv.forms.payload import format_requester
from fsv.ticket_notes import build_plan as note_plan, _note_id, _returned_note, _valid_recipient
from fsv.ticket_properties import _record
from fsv.views import load_conversations


class ConversationError(RuntimeError):
    pass


def recipients(client: Any, emails: list[str]) -> tuple[str, ...]:
    """Resolve each bare email to exactly one non-deleted user via the UI lookup."""
    from fsv.ticket_notes import _EMAIL
    out = []
    for email in emails:
        if not isinstance(email, str) or not _EMAIL.fullmatch(email):
            raise ValueError("CC must be a plain email address")
        try:
            person = resolve_requester(client, email)
            if not person.email or person.email.casefold() != email.casefold():
                raise ValueError("not an exact email match")
            value = format_requester(person.email, person.display)
            if not _valid_recipient(value):
                raise ValueError("invalid formatted address")
            out.append(value)
        except Exception:
            raise ConversationError("each CC email must resolve to exactly one non-deleted user with a valid name and email") from None
    return tuple(out)


def status_context(client: Any, cid: int, label: str) -> tuple[int, str, str]:
    # Only Pending was captured; other transitions may require resolution text.
    if not isinstance(label, str) or label.casefold() != "pending":
        raise ValueError("only the Pending status transition was captured; use the web form for other statuses")
    try:
        row = _record(client, cid)
        meta = load_meta(client, "ticket", "edit", fresh=True, module_id=cid)
        if meta.rules.get(RULES_UNAVAILABLE_KEY):
            raise ValueError("rules unavailable")
        state = FormState(meta.spec, meta.engine(), mode="edit", existing=existing_from_record(meta.spec, row))
        fields = [f for f in meta.spec.fields if is_status(f)]
        if len(fields) != 1:
            raise ValueError("status field not unique")
        field = fields[0]
        choices = [choice for choice in field.choices if choice.visible and
                   (choice.label.casefold() == label.casefold() or choice.value.casefold() == label.casefold())]
        if len(choices) != 1 or not isinstance(choices[0].id, int) or isinstance(choices[0].id, bool) or choices[0].id <= 0:
            raise ValueError("status choice not unique")
        state.set(field.api_name, choices[0].value)
        if (field.api_name not in state.evaluated.visible or state.errors() or
                state.rules.unavailable_dependencies("edit") or
                any("unknown field" in w or "none was given" in w for w in state.rules.warnings)):
            raise ValueError("status form rules or context unavailable")
        token = build_br_validation_excludes(state.evaluated, "ticket")
        return choices[0].id, token, journal.plan_hash(row) or ""
    except Exception:
        raise ConversationError("could not resolve Pending and evaluate ticket rules from fresh metadata; no send attempted") from None


@dataclass(frozen=True)
class ConversationPlan:
    kind: str
    ticket_id: int
    tenant: str
    payload_json: str
    status_label: str | None
    record_hash: str | None

    @property
    def payload(self) -> dict[str, Any]:
        return json.loads(self.payload_json)

    @property
    def path(self) -> str:
        return f"tickets/{self.ticket_id}/" + {"reply": "reply", "forward": "forward", "note": "notes"}[self.kind]

    @property
    def hash(self) -> str:
        return journal.plan_hash({"tenant": self.tenant, "path": self.path, "payload": self.payload,
                                  "record_hash": self.record_hash}) or ""

    def review(self) -> dict[str, Any]:
        return {"action": f"ticket_{self.kind}", "method": "POST", "path": f"/api/_/{self.path}",
                "payload": self.payload, "status_label": self.status_label, "plan_hash": self.hash,
                "warning": "Replies and forwards send mail; notes may trigger tenant workflows. Explicit sender addresses are user-asserted, not verified mailbox configuration."}


def build_plan(client: Any, ticket_id: str | int, body: str, *, kind: str = "reply",
               cc: tuple[str, ...] = (), public: bool = False, notify: tuple[str, ...] = (),
               body_format: Literal["plain", "html"] = "plain", set_status: str | None = None,
               bcc: tuple[str, ...] = (), sender: str | None = None) -> ConversationPlan:
    from fsv.ticket_notes import _EMAIL
    if kind not in ("reply", "note", "forward"):
        raise ValueError("invalid conversation kind")
    note = note_plan(ticket_id, body, public=public, notify_emails=notify, body_format=body_format)
    if sender is not None and not _EMAIL.fullmatch(sender):
        raise ValueError("sender must be a plain configured support email address")
    if kind == "forward" and (not sender or not notify or set_status is not None or cc or bcc):
        raise ValueError("whole-ticket forward requires explicit sender and recipients; no combined status/CC/BCC wire is proven")
    if kind == "note" and (bcc or sender):
        raise ValueError("note sender/BCC wire is not proven")
    if bcc and (cc or set_status is not None):
        raise ValueError("combined BCC/CC/status wire is not proven")
    if bcc and (not sender or any(not _EMAIL.fullmatch(v) for v in bcc)):
        raise ValueError("BCC requires a configured --sender and bare recipient emails")
    if kind == "reply":
        if any(not _valid_recipient(v) for v in cc):
            raise ValueError("invalid CC recipient")
        payload = {"body": note.body_html, "conversation_from_summary": False,
                   "cc_emails": list(cc), "full_text": note.body_html,
                   "translate_reply": False, "freddy_message_id": None}
        if bcc:
            payload["bcc_emails"] = list(bcc)
            if not cc:
                payload.pop("cc_emails")
        if sender:
            payload["from_email"] = sender
    elif kind == "forward":
        if any(not _EMAIL.fullmatch(v) for v in notify):
            raise ValueError("forward requires bare recipient emails")
        payload = {"body": note.body_html, "from_email": sender, "notify_emails": list(notify),
                   "conversation_from_summary": False, "private": True}
    else:
        payload = note.payload
    record_hash = None
    if set_status is not None:
        status_id, token, record_hash = status_context(client, note.ticket_id, set_status)
        payload["helpdesk_ticket"] = {"status": status_id, "br_validation_excludes": token}
    return ConversationPlan(kind, note.ticket_id, config.DOMAIN,
                            json.dumps(payload, sort_keys=True, ensure_ascii=False), set_status, record_hash)


def send(plan: ConversationPlan, client: Any, *, reviewed_hash: str | None,
         public_confirmed: bool = False) -> dict[str, Any]:
    if reviewed_hash is None:
        return {"status": "cancelled"}
    if plan.hash != reviewed_hash or not plan.tenant or plan.tenant != config.DOMAIN:
        raise ConversationError("tenant or review hash changed; review again")
    payload = plan.payload
    if (plan.kind in ("reply", "forward") or not payload.get("private", True)) and not public_confirmed:
        raise ConversationError("public conversation requires explicit confirmation")
    action = f"ticket_{plan.kind}_reviewed"
    events = [r for r in journal.read() if r.get("kind") == f"plan:{action}" and r.get("plan") == plan.hash]
    if events and events[-1].get("event") in ("started", "done"):
        raise ConversationError("conversation was sent or outcome unknown; inspect ticket and journal before retry")
    if plan.status_label is not None:
        status_id, token, record_hash = status_context(client, plan.ticket_id, plan.status_label)
        if record_hash != plan.record_hash or payload.get("helpdesk_ticket") != {"status": status_id, "br_validation_excludes": token}:
            raise ConversationError("ticket or status metadata changed; review again")
    # Reconfirm formatted CC recipients without silently changing reviewed bytes.
    if payload.get("cc_emails"):
        from email.utils import parseaddr
        emails = [parseaddr(value)[1] for value in payload["cc_emails"]]
        if recipients(client, emails) != tuple(payload["cc_emails"]):
            raise ConversationError("CC recipient identity changed; review again")
    # New recipient variants are looked up again before any send. No opaque
    # user IDs or guessed sender identities are substituted into the payload.
    for address in payload.get("bcc_emails", []) + (payload.get("notify_emails", []) if plan.kind == "forward" else []):
        recipients(client, [address])
    try:
        csrf = client._get_csrf()
    except Exception:
        raise ConversationError("could not prepare session; no send attempted") from None
    journal.record_plan("started", plan.hash, action=action, target=plan.ticket_id)
    events = [r for r in journal.read() if r.get("kind") == f"plan:{action}" and r.get("plan") == plan.hash]
    if not events or events[-1].get("event") != "started":
        raise ConversationError("could not persist journal; no send attempted")
    try:
        response = client._request("POST", f"{config.API_INT}/{plan.path}",
                                   content=plan.payload_json.encode("utf-8"),
                                   headers={"Content-Type": "application/json", "X-CSRF-Token": csrf})
    except APIError as exc:
        if 400 <= exc.status < 500 and exc.status not in (403, 408, 429):
            journal.record_plan("failed", plan.hash, action=action, target=plan.ticket_id)
            raise ConversationError(f"conversation rejected (HTTP {exc.status}); no retry") from None
        raise ConversationError("conversation outcome unknown; inspect ticket and journal before retry") from None
    except Exception:
        raise ConversationError("conversation outcome unknown; inspect ticket and journal before retry") from None
    conversation = _returned_note(response)
    conv_id = _note_id(conversation)
    if conv_id is None:
        raise ConversationError("response has no exact conversation ID; outcome unknown, do not resend")
    journal.record_plan("done", plan.hash, action=action, target=plan.ticket_id, created_id=conv_id)
    result: dict[str, Any] = {"status": "submitted", "conversation_id": conv_id, "ticket_id": plan.ticket_id,
                              "plan_hash": plan.hash, "verified": False}
    try:
        rows, meta, missing = load_conversations(client, plan.ticket_id, all_pages=True)
        exact = next((row for row in rows if str(row.get("id")) == str(conv_id)), None)
        expected = {"body": payload["body"]}
        if plan.kind == "reply":
            if "cc_emails" in payload:
                expected["cc_emails"] = payload["cc_emails"]
            if "bcc_emails" in payload:
                expected["bcc_emails"] = payload["bcc_emails"]
        else:
            expected.update(private=payload["private"], notify_emails=payload["notify_emails"])
        if "from_email" in payload:
            expected["from_email"] = payload["from_email"]
        fields = {k: ("unavailable" if exact is None or k not in exact else
                      "matched" if exact[k] == v else "mismatched") for k, v in expected.items()}
        if plan.status_label is not None:
            row = _record(client, plan.ticket_id)
            fields["ticket_status"] = ("unavailable" if "status" not in row else
                "matched" if row["status"] == payload["helpdesk_ticket"]["status"] else "mismatched")
        result["fields"] = fields
        result["verified"] = bool(exact is not None and not missing and not meta.get("partial") and all(v == "matched" for v in fields.values()))
    except Exception:
        pass
    if not result["verified"]:
        result["warning"] = "Conversation or ticket fields were not fully verified on read-back; do not resend."
    return result
