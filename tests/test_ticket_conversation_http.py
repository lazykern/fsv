"""Transport-level mail/status safety; synthetic schemas are not live proof."""
import json

import pytest

from fsv import config, ticket_conversation as conv
from test_ticket_properties_http import wire  # noqa: F401


@pytest.mark.parametrize("kind", ["reply", "note"])
def test_exact_pending_http_contract_and_public_consent(wire, kind):
    w = wire
    cc = conv.recipients(w.client, ["person@example.com"]) if kind == "reply" else ()
    p = conv.build_plan(w.client, w.cid, "Acme update", kind=kind, cc=cc, set_status="Pending")
    assert p.payload["helpdesk_ticket"]["status"] == w.pending
    assert not [r for r in w.requests if r.method == "POST"]
    if kind == "reply":
        with pytest.raises(conv.ConversationError, match="confirmation"):
            conv.send(p, w.client, reviewed_hash=p.hash)
    result = conv.send(p, w.client, reviewed_hash=p.hash, public_confirmed=kind == "reply")
    assert result["verified"], result
    posts = [r for r in w.requests if r.method == "POST"]
    assert len(posts) == 1 and str(posts[0].url) == f"{config.API_INT}/tickets/{w.cid}/" + ("reply" if kind == "reply" else "notes")
    assert posts[0].headers["content-type"] == "application/json"
    assert posts[0].headers["x-csrf-token"] == "synthetic-token"
    assert json.loads(posts[0].content) == p.payload
    assert "from_email" not in p.payload  # no fabricated mailbox identity
    if kind == "reply":
        assert p.payload["cc_emails"] == [w.lookup_name + " <person@example.com>"]
    else:
        assert p.payload["private"] is True


@pytest.mark.parametrize("missing", ["body", "cc_emails", "wrong-id", "contradict"])
def test_missing_or_wrong_conversation_readback_never_resends(wire, missing):
    w = wire
    w.conv_readback = missing
    p = conv.build_plan(w.client, w.cid, "Acme update", cc=conv.recipients(w.client, ["person@example.com"]))
    result = conv.send(p, w.client, reviewed_hash=p.hash, public_confirmed=True)
    assert result["verified"] is False and "do not resend" in result["warning"]
    with pytest.raises(conv.ConversationError, match="was sent"):
        conv.send(p, w.client, reviewed_hash=p.hash, public_confirmed=True)
    assert len([r for r in w.requests if r.method == "POST"]) == 1


@pytest.mark.parametrize("failure", [403, 429, 500, "disconnect"])
def test_each_conversation_write_caller_leaves_ambiguity_guard(wire, failure):
    w = wire
    w.failure = failure
    p = conv.build_plan(w.client, w.cid, "Acme update", kind="note", set_status="Pending")
    for _ in range(2):
        with pytest.raises(conv.ConversationError, match="unknown"):
            conv.send(p, w.client, reviewed_hash=p.hash)
    assert len([r for r in w.requests if r.method == "POST"]) == 1


def test_changed_cc_identity_refuses_instead_of_changing_reviewed_payload(wire):
    w = wire
    cc = conv.recipients(w.client, ["person@example.com"])
    p = conv.build_plan(w.client, w.cid, "Acme update", cc=cc)
    w.lookup_name = "Different Operator"
    with pytest.raises(conv.ConversationError, match="identity changed"):
        conv.send(p, w.client, reviewed_hash=p.hash, public_confirmed=True)
    assert not [r for r in w.requests if r.method == "POST"]


@pytest.mark.parametrize("label", ["Resolved", "Closed"])
def test_resolution_workflow_cannot_be_guessed_from_http_metadata(wire, label):
    with pytest.raises(ValueError, match="Pending"):
        conv.build_plan(wire.client, wire.cid, "Acme update", set_status=label)
    assert not [r for r in wire.requests if r.method == "POST"]
