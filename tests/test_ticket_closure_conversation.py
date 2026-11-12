"""Accepted closure conversation shapes exercised offline on synthetic tenants."""
import json

import pytest
from typer.testing import CliRunner

import fsv.cli as cli
from fsv import journal, ticket_conversation as conv
from test_ticket_properties_http import wire  # noqa: F401


def prepare(w, label):
    w.closed = w.pending + 100
    w.fields[1]["choices"].append({"id": w.closed, "value": label})
    w.row.update(group_id=None, responder_id=9000001234,
                 resolution_notes={"notes_html": "<p>Acme recovery verified</p>", "user": {"name": "Acme Operator"}},
                 custom_fields={"summary": "Existing fix"})
    w.fields.append({"id": 9000008910, "name": "summary", "label": "Resolution Summary",
                     "field_type": "custom_text", "position": 3, "required_for_closure": True})


@pytest.mark.parametrize("kind", ["note", "reply"])
@pytest.mark.parametrize("label", ["Resolved", "Closed"])
def test_closure_exact_nested_routing_and_readback(wire, kind, label):
    w = wire
    prepare(w, label)
    p = conv.build_plan(w.client, w.cid, "Acme self-only update", kind=kind, set_status=label)
    nested = p.payload["helpdesk_ticket"]
    assert nested["status"] == w.closed and nested["group_id"] is None and nested["responder_id"] == 9000001234
    assert "resolution_notes" not in nested
    assert not [r for r in w.requests if r.method == "POST"]
    result = conv.send(p, w.client, reviewed_hash=p.hash, public_confirmed=kind == "reply")
    assert result["verified"], result
    posts = [r for r in w.requests if r.method == "POST"]
    assert len(posts) == 1 and json.loads(posts[0].content) == p.payload
    assert journal.read()[-1]["event"] == "done"
    with pytest.raises(conv.ConversationError, match="was sent"):
        conv.send(p, w.client, reviewed_hash=p.hash, public_confirmed=True)
    assert len([r for r in w.requests if r.method == "POST"]) == 1


@pytest.mark.parametrize("missing", ["body", "to_emails", "wrong-id", "contradict"])
def test_closure_ambiguous_readback_stays_started(wire, missing):
    w = wire
    prepare(w, "Resolved")
    w.conv_readback = missing
    p = conv.build_plan(w.client, w.cid, "Acme update", kind="note", set_status="Resolved")
    result = conv.send(p, w.client, reviewed_hash=p.hash)
    assert not result["verified"]
    assert [e["event"] for e in journal.read() if e.get("plan") == p.hash] == ["started"]
    with pytest.raises(conv.ConversationError, match="unknown"):
        conv.send(p, w.client, reviewed_hash=p.hash)
    assert len([r for r in w.requests if r.method == "POST"]) == 1


@pytest.mark.parametrize("failure", [403, 429, 500, "disconnect"])
def test_closure_transport_is_single_attempt(wire, failure):
    w = wire
    prepare(w, "Closed")
    p = conv.build_plan(w.client, w.cid, "Acme update", kind="note", set_status="Closed")
    w.failure = failure
    for _ in range(2):
        with pytest.raises(conv.ConversationError, match="unknown"):
            conv.send(p, w.client, reviewed_hash=p.hash)
    assert len([r for r in w.requests if r.method == "POST"]) == 1


def test_closure_missing_custom_requirements_and_stale_metadata(wire):
    w = wire
    prepare(w, "Resolved")
    w.row["custom_fields"]["summary"] = ""
    with pytest.raises(conv.ConversationError, match="required fields.*summary"):
        conv.build_plan(w.client, w.cid, "Acme update", kind="note", set_status="Resolved")
    w.row["custom_fields"]["summary"] = "Fixed"
    p = conv.build_plan(w.client, w.cid, "Acme update", kind="note", set_status="Resolved")
    w.fields[-1]["label"] = "Changed requirement"
    with pytest.raises(conv.ConversationError, match="changed"):
        conv.send(p, w.client, reviewed_hash=p.hash)
    assert not [r for r in w.requests if r.method == "POST"]


@pytest.mark.parametrize("kind", ["reply", "note"])
def test_cli_closure_review_and_full_hash(wire, monkeypatch, kind):
    w = wire
    prepare(w, "Closed")
    monkeypatch.setattr(cli, "_client", lambda: w.client)
    monkeypatch.setattr("fsv.ticket_notes.get_client", lambda: w.client)
    args = (["tickets", "reply", str(w.cid), "Acme self-only reply"] if kind == "reply" else
            ["tickets", "add-note", str(w.cid), "--body", "Acme private note"])
    args += ["--set-status", "Closed"]
    runner = CliRunner()
    review = runner.invoke(cli.app, args + ["--dry-run"])
    assert review.exit_code == 0, review.output
    digest = json.loads(review.stdout)["plan_hash"]
    assert runner.invoke(cli.app, args + ["--yes"]).exit_code == 2
    assert not [r for r in w.requests if r.method == "POST"]
    sent = runner.invoke(cli.app, args + ["--yes", "--expect-plan", digest])
    assert sent.exit_code == 0, sent.output
    assert json.loads(sent.stdout)["verified"]
