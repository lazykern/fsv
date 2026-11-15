"""Plain ticket reply on the reviewed-write contract.

`tickets reply ID BODY` without CC/BCC/sender/status keeps its wire (a POST
of ``{"body": BODY}`` to ``tickets/ID/reply``), now with a plan hash bound to
the tenant, target, exact body and (empty) recipients, an optional
``--expect-plan``, a single-attempt journaled send and an independent
readback of the returned conversation by listing the ticket's conversations.
"""
from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any

from fsv import config, journal
from fsv.errors import APIError, server_message
from fsv.readback import same_text

ACTION = "ticket_reply_plain"
_UNKNOWN = "reply outcome unknown; inspect the ticket conversations and journal before any retry; do not resend"


class PlainReplyError(RuntimeError):
    pass


@dataclass(frozen=True)
class PlainReplyPlan:
    ticket_id: int
    tenant: str
    body: str
    # Verified sends of a plain reply on this ticket (journal.repeat_token):
    # the same reply sent again later plans a fresh hash; zero keeps it bare.
    sends: int = 0

    @property
    def path(self) -> str:
        return f"tickets/{self.ticket_id}/reply"

    @property
    def payload(self) -> dict[str, Any]:
        return {"body": self.body}

    @property
    def hash(self) -> str:
        identity: dict[str, Any] = {"action": ACTION, "tenant": self.tenant, "target": self.ticket_id,
                                    "method": "POST", "path": self.path, "payload": self.payload,
                                    "recipients": {"cc": [], "bcc": []}}
        if self.sends:
            identity["state"] = {"sends": self.sends}
        return journal.plan_hash(identity) or ""


def build_plan(ticket_id: int, body: str) -> PlainReplyPlan:
    if not isinstance(body, str) or not body.strip() or "\x00" in body:
        raise PlainReplyError("reply body must be nonempty text without NUL characters")
    if not config.DOMAIN:
        raise PlainReplyError("configure a tenant before replying")
    return PlainReplyPlan(ticket_id, config.DOMAIN, body, journal.verified_sends((ACTION,), ticket_id))


def _conversation_id(response: Any) -> Any:
    # Same supported envelopes as reviewed notes/replies: note, conversation or bare row.
    from fsv.ticket_notes import _note_id, _returned_note
    return _note_id(_returned_note(response))


def send(plan: PlainReplyPlan, client: Any) -> dict[str, Any]:
    """One POST, journaled; readback by listing the ticket's conversations."""
    from fsv.views import load_conversations
    if plan.tenant != config.DOMAIN:
        raise PlainReplyError("tenant changed since the review; review again")
    events = [e for e in journal.read() if e.get("kind") == f"plan:{ACTION}" and e.get("plan") == plan.hash]
    refusal = journal.replay_refusal(events, _UNKNOWN)
    if refusal:
        raise PlainReplyError(refusal)
    try:
        csrf = client._get_csrf()
    except Exception:
        raise PlainReplyError("could not prepare the session; no reply was sent") from None
    journal.record_plan("started", plan.hash, action=ACTION, target=plan.ticket_id)
    events = [e for e in journal.read() if e.get("kind") == f"plan:{ACTION}" and e.get("plan") == plan.hash]
    if not events or events[-1].get("event") != "started":
        raise PlainReplyError("could not persist the reply journal; no reply was sent")
    try:
        response = client._request("POST", f"{config.API_INT}/{plan.path}",
                                   content=json.dumps(plan.payload, ensure_ascii=False).encode("utf-8"),
                                   headers={"Content-Type": "application/json", "X-CSRF-Token": csrf})
    except APIError as exc:
        reason = server_message(exc.body)
        if 400 <= exc.status < 500 and exc.status not in (403, 408, 429):
            journal.record_plan("failed", plan.hash, action=ACTION, target=plan.ticket_id)
            raise PlainReplyError(f"reply rejected (HTTP {exc.status})" + (f": {reason}" if reason else "")
                                  + "; no retry") from None
        raise PlainReplyError(f"{_UNKNOWN} (HTTP {exc.status})") from None
    except Exception as exc:
        raise PlainReplyError(f"{_UNKNOWN} ({type(exc).__name__})") from None
    conv_id = _conversation_id(response)
    if conv_id is None:
        raise PlainReplyError("reply response has no exact conversation ID; " + _UNKNOWN)
    journal.record_plan("done", plan.hash, action=ACTION, target=plan.ticket_id, created_id=conv_id)
    result: dict[str, Any] = {"status": "submitted", "ticket_id": plan.ticket_id, "conversation_id": conv_id,
                              "plan_hash": plan.hash, "verified": False, "readback": None}
    try:
        rows, meta, missing = load_conversations(client, plan.ticket_id, all_pages=True)
        exact = next((r for r in rows if isinstance(r, dict) and str(r.get("id")) == str(conv_id)), None)
        result["readback"] = exact
        if exact is not None:
            actual = exact.get("body") if "body" in exact else exact.get("body_text")
            result["fields"] = {"body": "unavailable" if actual is None else
                                "matched" if same_text(actual, plan.body) else "mismatched"}
        result["verified"] = bool(exact is not None and not missing and not meta.get("partial")
                                  and result["fields"]["body"] == "matched")
    except Exception:
        pass
    if result["verified"] is True:
        journal.record_verified(plan.hash, action=ACTION, target=plan.ticket_id)
    else:
        result["warning"] = "Reply sent but conversation read-back did not verify it; do not resend."
    return result
