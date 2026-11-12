"""Reviewed reply and note status options; all HTTP is mocked."""
import json

import pytest
from typer.testing import CliRunner

import acme_change_form as a
import fsv.cli as cli
from fsv import config, journal, ticket_conversation as conv
from fsv.errors import APIError
from fsv.formmeta import FormMeta


class Client:
    def __init__(self, status_id=3):
        self.status_id = status_id
        self.row = {"id": 123, "subject": "Test", "status": 2}
        self.writes = []
        self.failure = None
    def int_get(self, path):
        return {"ticket": dict(self.row)}
    def autocomplete(self, kind, query, params):
        return [{"id": 11, "value": "Acme User", "email": query}]
    def _get_csrf(self):
        return "token"
    def _request(self, method, path, **kw):
        body = json.loads(kw["content"])
        self.writes.append((method, path, body))
        if self.failure:
            raise self.failure
        if "helpdesk_ticket" in body:
            self.row["status"] = body["helpdesk_ticket"]["status"]
        return {"conversation": {"id": 456}}


def metadata(status_id=3):
    fields = [a._f("subject", "Subject", "default_subject", 1),
              a._f("status", "Status", "default_status", 2,
                   choices=[{"id": 2, "value": "Open"}, {"id": status_id, "value": "Pending"}])]
    return FormMeta("ticket", "edit_form", {"ticket_fields": fields}, {"business_rules": []}, 1, 0, "fixture", 123)


@pytest.fixture(autouse=True)
def setup(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "j")
    monkeypatch.setattr(conv, "load_meta", lambda c, *a, **kw: metadata(c.status_id))


@pytest.mark.parametrize("tenant,status_id", [("Acme", 3), ("Globex", 17)])
def test_reply_cc_pending_exact_wire_and_readback(monkeypatch, tenant, status_id):
    c = Client(status_id)
    cc = conv.recipients(c, ["person@example.com"])
    p = conv.build_plan(c, "123", "Hello <team>", cc=cc, set_status="Pending")
    assert p.payload["cc_emails"] == ["Acme User <person@example.com>"]
    assert p.payload["body"] == "<div>Hello &lt;team&gt;</div>"
    assert p.payload["full_text"] == p.payload["body"]
    assert p.payload["helpdesk_ticket"]["status"] == status_id
    monkeypatch.setattr(conv, "load_conversations", lambda *a, **kw: ([{"id": 456, "body": p.payload["body"], "cc_emails": list(cc)}], {}, []))
    result = conv.send(p, c, reviewed_hash=p.hash, public_confirmed=True)
    assert result["verified"] and len(c.writes) == 1
    assert c.writes[0][2] == p.payload
    with pytest.raises(conv.ConversationError, match="was sent"):
        conv.send(p, c, reviewed_hash=p.hash, public_confirmed=True)


def test_note_status_and_ambiguous_outcome():
    c = Client()
    p = conv.build_plan(c, 123, "Internal", kind="note", notify=("person@example.com",), set_status="Pending")
    assert p.payload["private"] and p.payload["notify_emails"] == ["person@example.com"]
    assert p.payload["helpdesk_ticket"]["status"] == 3
    c.failure = APIError(403, "hidden")
    with pytest.raises(conv.ConversationError, match="outcome unknown"):
        conv.send(p, c, reviewed_hash=p.hash)
    with pytest.raises(conv.ConversationError, match="outcome unknown"):
        conv.send(p, c, reviewed_hash=p.hash)
    assert len(c.writes) == 1 and journal.read()[-1]["event"] == "started"


def test_resolved_closed_ambiguous_and_stale_fail(monkeypatch):
    c = Client()
    with pytest.raises(ValueError, match="Pending"):
        conv.build_plan(c, 123, "hi", set_status="3")
    monkeypatch.setattr(c, "autocomplete", lambda *a: [{"id": 1, "value": "A", "email": "a@example.com"}, {"id": 2, "value": "B", "email": "a@example.com"}])
    with pytest.raises(conv.ConversationError, match="exactly one"):
        conv.recipients(c, ["a@example.com"])
    p = conv.build_plan(c, 123, "hi", set_status="Pending")
    c.status_id = 99
    with pytest.raises(conv.ConversationError, match="changed"):
        conv.send(p, c, reviewed_hash=p.hash, public_confirmed=True)
    assert not c.writes


def test_cli_reply_and_note_review_then_send(monkeypatch):
    c = Client()
    monkeypatch.setattr(cli, "_client", lambda: c)
    monkeypatch.setattr("fsv.ticket_notes.get_client", lambda: c)
    runner = CliRunner()
    args = ["tickets", "reply", "123", "Hello", "--cc", "a@example.com", "--set-status", "Pending"]
    r = runner.invoke(cli.app, args + ["--dry-run"])
    assert r.exit_code == 0, r.output
    doc = json.loads(r.output)
    assert runner.invoke(cli.app, args + ["--yes"]).exit_code == 2
    r = runner.invoke(cli.app, args + ["--yes", "--expect-plan", doc["plan_hash"]])
    assert r.exit_code == 0, r.output
    assert len(c.writes) == 1
    args = ["tickets", "add-note", "123", "--body", "Private", "--set-status", "Pending"]
    r = runner.invoke(cli.app, args + ["--dry-run"])
    assert r.exit_code == 0, r.output
    doc = json.loads(r.output)
    r = runner.invoke(cli.app, args + ["--yes", "--expect-plan", doc["plan_hash"]])
    assert r.exit_code == 0, r.output
    assert len(c.writes) == 2 and c.writes[-1][1].endswith("/notes")


@pytest.mark.parametrize("kind", ["reply", "forward"])
def test_bcc_and_whole_ticket_forward_exact_wire_hash_and_readback(monkeypatch, kind):
    c = Client()
    options = {"bcc": ("person@example.com",)} if kind == "reply" else {"notify": ("person@example.com",)}
    p = conv.build_plan(c, 123, "Acme update", kind=kind, sender="support@example.com", **options)
    assert p.payload["from_email"] == "support@example.com"
    key = "bcc_emails" if kind == "reply" else "notify_emails"
    assert p.payload[key] == ["person@example.com"]
    if kind == "forward":
        assert p.path.endswith("/forward")
        assert p.payload == {"body": "<div>Acme update</div>", "from_email": "support@example.com",
                             "notify_emails": ["person@example.com"], "conversation_from_summary": False, "private": True}
    changed = conv.build_plan(c, 123, "Acme update", kind=kind, sender="other@example.com", **options)
    assert p.hash != changed.hash
    monkeypatch.setattr(conv, "load_conversations", lambda *a, **kw: ([{"id": 456, **p.payload}], {}, []))
    result = conv.send(p, c, reviewed_hash=p.hash, public_confirmed=True)
    assert result["verified"] and len(c.writes) == 1


def test_bcc_missing_readback_is_unavailable_not_verified(monkeypatch):
    c = Client()
    p = conv.build_plan(c, 123, "Hello", bcc=("person@example.com",), sender="support@example.com")
    monkeypatch.setattr(conv, "load_conversations", lambda *a, **kw: ([{"id": 456, "body": p.payload["body"], "cc_emails": []}], {}, []))
    result = conv.send(p, c, reviewed_hash=p.hash, public_confirmed=True)
    assert not result["verified"] and result["fields"]["bcc_emails"] == "unavailable"
    with pytest.raises(conv.ConversationError, match="was sent"):
        conv.send(p, c, reviewed_hash=p.hash, public_confirmed=True)
    assert len(c.writes) == 1


def test_bcc_and_forward_cli_require_exact_review(monkeypatch):
    c = Client()
    monkeypatch.setattr(cli, "_client", lambda: c)
    runner = CliRunner()
    for args in (["tickets", "reply", "123", "Hello", "--bcc", "person@example.com", "--sender", "support@example.com"],
                 ["tickets", "forward", "123", "Hello", "--to", "person@example.com", "--sender", "support@example.com"]):
        review = runner.invoke(cli.app, args + ["--dry-run"])
        assert review.exit_code == 0, review.output
        digest = json.loads(review.output)["plan_hash"]
        assert runner.invoke(cli.app, args + ["--yes"]).exit_code == 2
        sent = runner.invoke(cli.app, args + ["--yes", "--expect-plan", digest])
        assert sent.exit_code == 0, sent.output
    assert len(c.writes) == 2


def test_forward_uncaptured_variants_refused():
    c = Client()
    with pytest.raises(ValueError, match="sender"):
        conv.build_plan(c, 123, "Hello", bcc=("person@example.com",))
    with pytest.raises(ValueError, match="forward requires"):
        conv.build_plan(c, 123, "Hello", kind="forward", sender="support@example.com", notify=("person@example.com",), set_status="Pending")

