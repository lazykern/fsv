"""Offline exact-target note mutation contracts."""
import copy
import json

import httpx
import pytest
from typer.testing import CliRunner

import fsv.cli as cli
from fsv import config, journal, ticket_note_mutation as flow
from fsv.client import Client
from fsv.reviewed_write import ReviewedWriteError


@pytest.fixture(params=["acme.example.com", "globex.example.com"])
def api(request, monkeypatch, tmp_path):
    domain = request.param
    monkeypatch.setattr(config, "DOMAIN", domain)
    monkeypatch.setattr(config, "API_INT", f"https://{domain}/api/_")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    state = {"rows": [{"id": 99001, "source": 2, "private": True, "body": "<div>Old</div>"}], "writes": [], "fail": None, "partial": False}
    def handler(req):
        if req.method == "GET":
            assert req.url.path == "/api/_/tickets/123/conversations"
            if state["partial"]:
                return httpx.Response(403, json={"message": "denied"})
            return httpx.Response(200, json={"conversations": copy.deepcopy(state["rows"]), "meta": {"count": len(state["rows"])}})
        state["writes"].append((req.method, req.url.path, json.loads(req.content) if req.content else None))
        if state["fail"] == "disconnect":
            raise httpx.ReadError("disconnect", request=req)
        if state["fail"]:
            return httpx.Response(state["fail"], json={"message": "failure"})
        assert req.url.path == "/api/_/conversations/99001"
        if state.get("partial_after"):
            state["partial"] = True
        if req.method == "PUT":
            state["rows"][0]["body"] = ('<div>Unexpected</div>' if state.get("mismatch") else
                                         '<div class="server"><div>New &amp; text</div></div>')
            return httpx.Response(200, json={"conversation": copy.deepcopy(state["rows"][0])})
        assert req.method == "DELETE"
        state["rows"].clear()
        return httpx.Response(204)
    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handler))
    c._csrf = "synthetic"
    monkeypatch.setattr(cli, "_client", lambda: c)
    yield c, state
    c.close()


@pytest.mark.parametrize("body,method", [("New & text", "PUT"), (None, "DELETE")])
def test_exact_write_and_readback(api, body, method):
    c, state = api
    plan = flow.build_plan(c, "INC-123", "99001", body=body)
    assert flow.review(plan, c)["before"]["body"] == "<div>Old</div>"
    result = flow.send(plan, c, reviewed_hash=plan.hash)
    assert result["verified"] and result["conversation_id"] == 99001
    assert len(state["writes"]) == 1 and state["writes"][0][0] == method
    assert state["writes"][0][2] == (None if body is None else {"body": "<div>New &amp; text</div>", "conversation_from_summary": False})
    with pytest.raises(ReviewedWriteError, match="was sent"):
        flow.send(plan, c, reviewed_hash=plan.hash)


@pytest.mark.parametrize("failure", [403, 429, 500, "disconnect"])
def test_unknown_outcome_never_replayed(api, failure):
    c, state = api
    plan = flow.build_plan(c, 123, 99001, body="New & text")
    state["fail"] = failure
    with pytest.raises(ReviewedWriteError, match="unknown"):
        flow.send(plan, c, reviewed_hash=plan.hash)
    with pytest.raises(ReviewedWriteError, match="was sent"):
        flow.send(plan, c, reviewed_hash=plan.hash)
    assert len(state["writes"]) == 1
    assert [r["event"] for r in journal.read() if r.get("kind") == "plan:ticket_note_edit"] == ["started"]


@pytest.mark.parametrize("mode", ["partial_after", "mismatch"])
def test_unverified_readback_never_replayed(api, mode):
    c, state = api
    plan = flow.build_plan(c, 123, 99001, body="New & text")
    state[mode] = True
    result = flow.send(plan, c, reviewed_hash=plan.hash)
    assert not result["verified"] and "do not resend" in result["warning"].lower()
    with pytest.raises(ReviewedWriteError, match="was sent"):
        flow.send(plan, c, reviewed_hash=plan.hash)
    assert len(state["writes"]) == 1


def test_preflight_and_freshness_fail_closed(api):
    c, state = api
    plan = flow.build_plan(c, 123, 99001, body="New")
    state["rows"][0]["body"] = "changed"
    with pytest.raises(ReviewedWriteError, match="changed"):
        flow.send(plan, c, reviewed_hash=plan.hash)
    state["rows"][0]["source"] = 0
    with pytest.raises(ReviewedWriteError, match="not a verified note"):
        flow.build_plan(c, 123, 99001)
    state["partial"] = True
    with pytest.raises(ReviewedWriteError, match="history unavailable"):
        flow.build_plan(c, 123, 99001)
    assert not state["writes"]


def test_cli_review_and_full_hash(api):
    c, state = api
    runner = CliRunner()
    args = ["tickets", "note-edit", "INC-123", "99001", "--body", "New & text"]
    reviewed = runner.invoke(cli.app, args + ["--dry-run", "--json"])
    assert reviewed.exit_code == 0, reviewed.output
    hash_ = json.loads(reviewed.output)["plan_hash"]
    assert runner.invoke(cli.app, args + ["--yes", "--expect-plan", hash_[:16]]).exit_code == 2
    sent = runner.invoke(cli.app, args + ["--yes", "--expect-plan", hash_, "--output", "json"])
    assert sent.exit_code == 0 and json.loads(sent.output)["verified"]
    assert len(state["writes"]) == 1
