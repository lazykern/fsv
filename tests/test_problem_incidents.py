"""Offline problem-incident association contracts with real HTTP client."""
import copy
import json

import httpx
import pytest
from typer.testing import CliRunner

import fsv.cli as cli
from fsv import config, journal, problem_incidents as flow
from fsv.client import Client
from fsv.reviewed_write import ReviewedWriteError


@pytest.fixture(params=["acme.example.com", "globex.example.com"])
def api(request, monkeypatch, tmp_path):
    domain = request.param
    monkeypatch.setattr(config, "DOMAIN", domain)
    monkeypatch.setattr(config, "API_INT", f"https://{domain}/api/_")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    state = {"linked": False, "writes": [], "fail": None, "partial": False, "mismatch": False, "type": "Incident"}
    def handler(req):
        path = req.url.path
        if req.method == "GET":
            if path == "/api/_/problems/7":
                return httpx.Response(200, json={"problem": {"id": 7, "subject": "Acme problem"}})
            if path == "/api/_/tickets/123":
                return httpx.Response(200, json={"ticket": {"id": 123, "type": state["type"], "human_display_id": "INC-123"}})
            assert path == "/api/_/problems/7/tickets"
            if state["partial"]:
                return httpx.Response(403, json={"error": "denied"})
            rows = [{"id": 123, "human_display_id": "INC-123", "subject": "Acme incident"}] if state["linked"] else []
            return httpx.Response(200, json={"tickets": copy.deepcopy(rows), "meta": {"count": len(rows)}})
        state["writes"].append((req.method, path, json.loads(req.content)))
        if state["fail"] == "disconnect":
            raise httpx.ReadError("disconnect", request=req)
        if state["fail"]:
            return httpx.Response(state["fail"], json={"error": "failure"})
        if state.get("partial_after"):
            state["partial"] = True
        if not state["mismatch"]:
            state["linked"] = req.method == "PUT"
        return httpx.Response(200, json={"tickets": []}) if req.method == "PUT" else httpx.Response(204)
    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handler))
    c._csrf = "synthetic"
    monkeypatch.setattr(cli, "_client", lambda: c)
    yield c, state
    c.close()


@pytest.mark.parametrize("unlink", [False, True])
def test_exact_wire_and_readback(api, unlink):
    c, state = api
    state["linked"] = unlink
    plan = flow.build_plan(c, "PRB-7", "INC-123", unlink=unlink)
    result = flow.send(plan, c, "INC-123", reviewed_hash=plan.hash)
    assert result["verified"] and result["linked"] is not unlink
    assert state["writes"] == ([('DELETE', '/api/_/problems/7/tickets/123', {'type': 'unlink'})] if unlink else
                                [('PUT', '/api/_/problems/7/tickets/associate', {'item_ids': [123]})])
    with pytest.raises(ReviewedWriteError, match="was sent"):
        flow.send(plan, c, 123, reviewed_hash=plan.hash)


@pytest.mark.parametrize("failure", [403, 429, 500, "disconnect"])
def test_ambiguous_never_replayed(api, failure):
    c, state = api
    plan = flow.build_plan(c, 7, 123)
    state["fail"] = failure
    with pytest.raises(ReviewedWriteError, match="unknown"):
        flow.send(plan, c, 123, reviewed_hash=plan.hash)
    with pytest.raises(ReviewedWriteError, match="was sent"):
        flow.send(plan, c, 123, reviewed_hash=plan.hash)
    assert len(state["writes"]) == 1
    assert [r["event"] for r in journal.read() if r.get("kind") == "plan:problem_link_incident"] == ["started"]


def test_unavailable_readback_does_not_replay(api):
    c, state = api
    plan = flow.build_plan(c, 7, 123)
    state["partial_after"] = True
    result = flow.send(plan, c, 123, reviewed_hash=plan.hash)
    assert not result["verified"] and "do not resend" in result["warning"].lower()
    with pytest.raises(ReviewedWriteError, match="was sent"):
        flow.send(plan, c, 123, reviewed_hash=plan.hash)
    assert len(state["writes"]) == 1


def test_preflight_and_freshness(api):
    c, state = api
    plan = flow.build_plan(c, 7, 123)
    state["type"] = "Service Request"
    with pytest.raises(ReviewedWriteError, match="not a verified incident"):
        flow.send(plan, c, 123, reviewed_hash=plan.hash)
    with pytest.raises(ValueError, match="INC prefix"):
        flow.build_plan(c, 7, "SR-123")
    state["type"] = "Incident"
    state["partial"] = True
    with pytest.raises(ReviewedWriteError, match="inventory unavailable"):
        flow.build_plan(c, 7, 123)
    assert not state["writes"]


def test_mismatch_and_cli_review(api):
    c, state = api
    runner = CliRunner()
    args = ["problems", "link-incident", "PRB-7", "INC-123"]
    review = runner.invoke(cli.app, args + ["--dry-run", "--json"])
    assert review.exit_code == 0, review.output
    hash_ = json.loads(review.output)["plan_hash"]
    assert runner.invoke(cli.app, args + ["--yes", "--expect-plan", hash_[:16]]).exit_code == 2
    state["mismatch"] = True
    result = runner.invoke(cli.app, args + ["--yes", "--expect-plan", hash_, "--output", "json"])
    assert result.exit_code == 0 and not json.loads(result.output)["verified"]
    assert len(state["writes"]) == 1
