"""Offline server-shaped recipient readbacks, independent of POST echoes."""
import json

import pytest
from typer.testing import CliRunner

import fsv.cli as cli
from fsv import ticket_conversation as conv
from test_ticket_properties_http import wire  # noqa: F401


@pytest.mark.parametrize("kind,key,readback,status", [
    ("forward", "notify_emails", {"to_emails": ["SECOND@example.com", "Person <PERSON@example.com>"]}, "matched"),
    ("forward", "notify_emails", {"notify_emails": ["PERSON@example.com", "second@example.com"]}, "matched"),
    ("forward", "notify_emails", {"to_emails": ["person@example.com", "second@example.com"],
                                    "notify_emails": ["second@example.com", "person@example.com"]}, "matched"),
    ("forward", "notify_emails", {"to_emails": ["person@example.com", "second@example.com"],
                                    "notify_emails": ["wrong@example.com"]}, "mismatched"),
    ("forward", "notify_emails", {"to_emails": ["person@example.com"]}, "mismatched"),
    ("forward", "notify_emails", {"to_emails": ["person@example.com", "second@example.com", "extra@example.com"]}, "mismatched"),
    ("forward", "notify_emails", {"to_emails": None}, "mismatched"),
    ("forward", "notify_emails", {"to_emails": "person@example.com"}, "mismatched"),
    ("forward", "notify_emails", {"to_emails": ["person@example.com", 42]}, "mismatched"),
    ("forward", "notify_emails", {"cc_emails": ["person@example.com", "second@example.com"]}, "unavailable"),
    ("bcc", "bcc_emails", {"to_emails": ["person@example.com"], "bcc_emails": ["PERSON@example.com"]}, "matched"),
    ("bcc", "bcc_emails", {"to_emails": ["person@example.com"]}, "unavailable"),
    ("bcc", "bcc_emails", {"bcc_emails": ["wrong@example.com"]}, "mismatched"),
    ("bcc", "bcc_emails", {"bcc_emails": []}, "mismatched"),
    ("cc", "cc_emails", {"cc_emails": ["Person <PERSON@example.com>"]}, "matched"),
    ("cc", "cc_emails", {"to_emails": ["person@example.com"]}, "unavailable"),
    ("cc", "cc_emails", {"cc_emails": ["wrong@example.com"]}, "mismatched"),
])
def test_recipient_readback_http(wire, kind, key, readback, status):
    w = wire
    if kind == "forward":
        p = conv.build_plan(w.client, w.cid, "Acme update", kind="forward",
                            sender="support@example.com", notify=("person@example.com", "second@example.com"))
    elif kind == "bcc":
        p = conv.build_plan(w.client, w.cid, "Acme update", sender="support@example.com", bcc=("person@example.com",))
    else:
        p = conv.build_plan(w.client, w.cid, "Acme update", cc=conv.recipients(w.client, ["person@example.com"]))

    def server_readback(row):
        for name in ("to_emails", "notify_emails", "cc_emails", "bcc_emails"):
            row.pop(name, None)
        row.update(readback)

    w.conv_transform = server_readback
    result = conv.send(p, w.client, reviewed_hash=p.hash, public_confirmed=True)
    assert result["fields"][key] == status
    assert result["verified"] is (status == "matched")
    posts = [r for r in w.requests if r.method == "POST"]
    assert len(posts) == 1 and json.loads(posts[0].content) == p.payload
    if status != "matched":
        assert "do not resend" in result["warning"]
        with pytest.raises(conv.ConversationError, match="was sent"):
            conv.send(p, w.client, reviewed_hash=p.hash, public_confirmed=True)
        assert len([r for r in w.requests if r.method == "POST"]) == 1


def test_forward_cli_with_native_to_emails_readback(wire, monkeypatch):
    monkeypatch.setattr(cli, "_client", lambda: wire.client)
    args = ["tickets", "forward", str(wire.cid), "Acme update", "--to", "person@example.com",
            "--sender", "support@example.com", "--json"]
    runner = CliRunner()
    review = runner.invoke(cli.app, args + ["--dry-run"])
    assert review.exit_code == 0, review.output
    sent = runner.invoke(cli.app, args + ["--yes", "--expect-plan", json.loads(review.stdout)["plan_hash"]])
    assert sent.exit_code == 0, sent.output
    result = json.loads(sent.stdout)
    assert result["verified"] and result["fields"]["notify_emails"] == "matched"
    assert wire.conversation["to_emails"] == ["person@example.com"]
    assert "notify_emails" not in wire.conversation
    assert len([r for r in wire.requests if r.method == "POST"]) == 1


def test_email_sets_allow_order_case_and_duplicates_but_not_invalid_data():
    row = {"bcc_emails": ["SECOND@example.com", "Person <person@example.com>", "PERSON@example.com"]}
    assert conv._recipient_readback(row, "bcc_emails", ["person@example.com", "second@example.com"]) == "matched"
    assert conv._recipient_readback({"bcc_emails": ["not-an-email"]}, "bcc_emails", []) == "mismatched"
    assert conv._recipient_readback({"bcc_emails": []}, "bcc_emails", []) == "matched"
    assert conv._recipient_readback({}, "bcc_emails", []) == "unavailable"
