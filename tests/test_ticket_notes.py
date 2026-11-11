"""Synthetic note contract tests; no accepted live wire is claimed."""
from __future__ import annotations

import json
from dataclasses import replace
from typing import Any

import httpx
import pytest
import typer
from typer.testing import CliRunner

from fsv import config, journal, ticket_notes as notes
from fsv.client import Client


@pytest.fixture(params=["acme.example", "globex.example"])
def tenant(request, monkeypatch):
    monkeypatch.setattr(config, "DOMAIN", request.param)
    monkeypatch.setattr(config, "API_INT", f"https://{request.param}/api/_")
    return request.param


def mock_client(monkeypatch, handler):
    c = Client.__new__(Client)
    c._client = httpx.Client(transport=httpx.MockTransport(handler), follow_redirects=False)
    c._csrf = "synthetic-token"
    c._fw_domain = c._fw_session_id = None
    c._persist = False
    c._rl_rem = c._rl_tot = None
    c._probe_ok_until = 0
    monkeypatch.setattr(c, "_probe_session", lambda: True)
    return c


def app():
    result = typer.Typer()

    @result.callback()
    def root():
        pass

    notes.register(result)
    return result


@pytest.mark.parametrize("public", [False, True])
@pytest.mark.parametrize("recipients", [None, ["agent@example.com", "reviewer@example.com"]])
@pytest.mark.parametrize("envelope", ["note", "conversation", None])
def test_exact_wire_and_returned_identity(monkeypatch, tenant, public, envelope, recipients):
    seen = []
    exact = {"id": 9000001001, "body": "<div>Acme review</div>", "private": not public, "source": 2}

    def handler(req):
        seen.append(req)
        if req.method == "POST":
            return httpx.Response(200, json={envelope: exact} if envelope else exact)
        return httpx.Response(200, json={"conversations": [
            {"id": 9000001002, "body": exact["body"]}, exact], "meta": {"count": 2}})

    c = mock_client(monkeypatch, handler)
    try:
        plan = notes.build_plan("SR-123", "Acme review", public=public, notify_emails=recipients)
        result = notes.send(plan, c, reviewed_hash=plan.hash, public_confirmed=public)
        posts = [req for req in seen if req.method == "POST"]
        assert len(posts) == 1
        assert str(posts[0].url) == f"https://{tenant}/api/_/tickets/123/notes"
        assert posts[0].headers["content-type"] == "application/json"
        assert posts[0].headers["x-csrf-token"] == "synthetic-token"
        assert json.loads(posts[0].content) == {"body": "<div>Acme review</div>",
                "private": not public, "conversation_from_summary": False, "notify_emails": recipients or []}
        assert result["note_id"] == exact["id"]
        assert result["readback"] == result["note"] == exact
        assert result["verified"] is True
        rows = [r for r in journal.read() if r.get("kind") == "plan:ticket_note"]
        assert [r["event"] for r in rows] == ["started", "done"]
        assert rows[-1]["created_id"] == exact["id"]
        assert "Acme review" not in journal.journal_path().read_text()
        with pytest.raises(notes.NoteError, match="sent or its outcome is unknown"):
            notes.send(plan, c, reviewed_hash=plan.hash, public_confirmed=public)
        assert len([req for req in seen if req.method == "POST"]) == 1
    finally:
        c.close()


def test_notification_list_and_immutable_review():
    recipients = ["agent@example.com", "reviewer@example.com"]
    plan = notes.build_plan(123, "Globex review", notify_emails=recipients)
    recipients.append("other@example.com")
    payload = plan.payload
    payload["notify_emails"].append("another@example.com")
    payload["private"] = False
    assert plan.payload["notify_emails"] == ["agent@example.com", "reviewer@example.com"]
    assert plan.payload["private"] is True
    assert notes.build_plan(123, "Globex review").payload["notify_emails"] == []
    assert plan.hash != notes.build_plan(124, "Globex review", notify_emails=recipients[:2]).hash


def test_html_plain_conventions():
    assert notes.build_plan(123, '<literal> & "quote"\r\nline\n\nnext').body_html == (
        '<div>&lt;literal&gt; &amp; &quot;quote&quot;<br>line</div><div>next</div>')
    body = '<div class="note"><strong>Globex</strong><br>review</div>'
    assert notes.build_plan(123, body, body_format="html").body_html == body
    assert notes.build_plan(123, body).body_html != body


@pytest.mark.parametrize("ticket_id,body,kwargs", [
    (0, "review", {}), (-1, "review", {}), (True, "review", {}),
    (1.5, "review", {}), ("CHN-123", "review", {}), ("123/x", "review", {}),
    (123, "", {}), (123, " \n", {}), (123, None, {}), (123, "x\x00", {}),
    (123, "<div><br></div>", {"body_format": "html"}),
    (123, "review", {"body_format": "markdown"}), (123, "review", {"public": "false"}),
    (123, "review", {"notify_emails": "agent@example.com"}),
    (123, "review", {"notify_emails": ["not-an-email"]}),
    (123, "review", {"notify_emails": [None]}),
    (123, "review", {"notify_emails": ["agent@example.com\nother@example.com"]}),
    (123, "review", {"notify_emails": ["Agent <agent@example.com>"]}),
])
def test_invalid_inputs(ticket_id, body, kwargs):
    with pytest.raises(ValueError):
        notes.build_plan(ticket_id, body, **kwargs)
    assert journal.read() == []


@pytest.mark.parametrize("mode", ["read_only", "dry_run", "cancel", "hash", "public"])
def test_pre_send_guards_zero_io(monkeypatch, mode):
    def forbidden(*a, **k):
        pytest.fail("pre-send guard performed I/O")

    monkeypatch.setattr(notes, "get_client", forbidden)
    monkeypatch.setattr(journal, "record_plan", forbidden)
    plan = notes.build_plan(123, "Acme review", public=mode == "public")
    kwargs: dict[str, Any] = {"reviewed_hash": plan.hash}
    if mode in ("read_only", "dry_run"):
        kwargs[mode] = True
    elif mode == "cancel":
        kwargs["reviewed_hash"] = None
    elif mode == "hash":
        kwargs["reviewed_hash"] = "not-the-reviewed-hash"
    if mode in ("dry_run", "cancel"):
        assert notes.send(plan, **kwargs)["status"] in ("dry_run", "cancelled")
    else:
        with pytest.raises(notes.NoteError):
            notes.send(plan, **kwargs)
    assert journal.read() == []


def test_tenant_change_and_edited_plan_require_new_review(monkeypatch):
    plan = notes.build_plan(123, "Acme review")
    with pytest.raises(notes.NoteError, match="hash"):
        notes.send(replace(plan, body_html="<div>Edited</div>"), reviewed_hash=plan.hash)
    monkeypatch.setattr(config, "DOMAIN", "globex.example")
    with pytest.raises(notes.NoteError, match="tenant changed"):
        notes.send(plan, reviewed_hash=plan.hash)
    assert journal.read() == []


@pytest.mark.parametrize("failure", [400, 401, 403, 404, 408, 429, 500, 503, "timeout", "malformed"])
def test_one_post_no_retry_or_fallback(monkeypatch, tenant, failure):
    seen = []

    def handler(req):
        seen.append(req)
        if failure == "timeout":
            raise httpx.ReadTimeout("synthetic uncertainty", request=req)
        if failure == "malformed":
            return httpx.Response(200, json={"unexpected": True})
        return httpx.Response(failure, json={"error": "synthetic rejection"})

    c = mock_client(monkeypatch, handler)
    try:
        plan = notes.build_plan(123, "Acme review")
        with pytest.raises(notes.NoteError, match="rejected|unknown"):
            notes.send(plan, c, reviewed_hash=plan.hash)
        assert len(seen) == 1
        assert seen[0].method == "POST"
        assert seen[0].url.path == "/api/_/tickets/123/notes"
        events = [r["event"] for r in journal.read() if r.get("kind") == "plan:ticket_note"]
        ambiguous = failure in (401, 408, 429, 500, 503, "timeout", "malformed")
        assert events == (["started"] if ambiguous else ["started", "failed"])
        if ambiguous:
            with pytest.raises(notes.NoteError, match="sent or its outcome is unknown"):
                notes.send(plan, c, reviewed_hash=plan.hash)
            assert len(seen) == 1
    finally:
        c.close()


@pytest.mark.parametrize("case", ["missing", "forbidden", "timeout"])
def test_readback_failure_retains_exact_returned_id_no_resend(monkeypatch, tenant, case):
    posts = []

    def handler(req):
        if req.method == "POST":
            posts.append(req)
            return httpx.Response(200, json={"note": {"id": "9000001001"}})
        if case == "forbidden":
            return httpx.Response(403, json={"error": "denied"})
        if case == "timeout":
            raise RuntimeError("synthetic read failure")
        return httpx.Response(200, json={"conversations": [{"id": 9000001002, "body": "Acme review"}],
                                             "meta": {"count": 1}})

    c = mock_client(monkeypatch, handler)
    try:
        plan = notes.build_plan(123, "Acme review")
        result = notes.send(plan, c, reviewed_hash=plan.hash)
        assert result["note_id"] == "9000001001"
        assert result["verified"] is False
        assert result["readback"] is None
        assert "do not resend" in result["warning"]
        with pytest.raises(notes.NoteError):
            notes.send(plan, c, reviewed_hash=plan.hash)
        assert len(posts) == 1
    finally:
        c.close()


def test_failed_journal_and_csrf_prevent_post(monkeypatch):
    class NoPost:
        def _get_csrf(self):
            return "synthetic-token"

        def _request(self, *a, **k):
            pytest.fail("POST should not run")

    plan = notes.build_plan(123, "Acme review")
    monkeypatch.setattr(journal, "record_plan", lambda *a, **k: None)
    with pytest.raises(notes.NoteError, match="persist"):
        notes.send(plan, NoPost(), reviewed_hash=plan.hash)
    monkeypatch.setattr(NoPost, "_get_csrf", lambda self: (_ for _ in ()).throw(RuntimeError("secret")))
    with pytest.raises(notes.NoteError, match="no note POST was attempted"):
        notes.send(plan, NoPost(), reviewed_hash=plan.hash)
    assert journal.read() == []


@pytest.mark.parametrize("args,input_", [
    ([], "n\n"), (["--public"], "y\nn\n"), (["--no-input"], None),
    (["--yes"], None), (["--read-only"], None),
])
def test_cli_cancel_and_guards_zero_writes(monkeypatch, args, input_):
    monkeypatch.setattr(notes, "get_client", lambda: pytest.fail("should not construct a client"))
    result = CliRunner().invoke(app(), ["add-note", "SR-123", "--body", "Acme review", *args], input=input_)
    assert result.exit_code in (0, 2), result.output
    assert "Acme review" in result.output
    assert journal.read() == []


def test_cli_dryrun_payload_and_explicit_public_send(monkeypatch):
    runner = CliRunner()
    cli = app()
    draft = runner.invoke(cli, ["add-note", "123", "--body", "Globex review", "--public", "--dry-run"])
    assert draft.exit_code == 0, draft.output
    review = json.loads(draft.stdout)
    assert review["payload"] == {"body": "<div>Globex review</div>", "private": False,
                                 "conversation_from_summary": False, "notify_emails": []}
    assert journal.read() == []
    calls = []

    def fake_send(plan, **kwargs):
        calls.append((plan, kwargs))
        return {"note_id": 9000001001, "verified": True}

    monkeypatch.setattr(notes, "send", fake_send)
    base = ["add-note", "123", "--body", "Globex review", "--public", "--yes",
            "--expect-plan", review["plan_hash"], "--no-input"]
    denied = runner.invoke(cli, base)
    assert denied.exit_code == 2
    assert not calls
    accepted = runner.invoke(cli, [*base, "--confirm-public", "--json"])
    assert accepted.exit_code == 0, accepted.output
    assert len(calls) == 1
    assert calls[0][1] == {"reviewed_hash": review["plan_hash"], "public_confirmed": True}
    assert json.loads(accepted.stdout)["note_id"] == 9000001001
