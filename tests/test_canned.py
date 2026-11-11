"""Offline captured template reads and source-bound ordinary reply transport."""
import copy
import json

import httpx
import pytest
from typer.testing import CliRunner

from fsv import canned, cli, config, journal, scenarios, ticket_conversation as conv
from fsv.client import Client


@pytest.fixture
def wire(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    state = {"source": {"canned_response": {"id": 701, "content": "<div>Acme response</div>"}},
             "writes": [], "requests": [], "failure": None, "readback": "ok"}

    def handle(request):
        path = request.url.path
        state["requests"].append(path)
        if path.endswith("/canned_responses/701"):
            return httpx.Response(200, json=state["source"])
        if path.endswith("/canned_response_folders"):
            return httpx.Response(200, json={"canned_response_folders": [{"id": 702}]})
        if path.endswith("/canned_response_folders/702/canned_responses"):
            return httpx.Response(200, json={"canned_responses": [{"id": 701}]})
        if path.endswith("/scenario_automations"):
            return httpx.Response(200, json={"scenario_automations": [{"id": 703, "name": "Globex"}]})
        if request.method == "GET" and path.endswith("/tickets/123"):
            return httpx.Response(200, json={"ticket": {"id": 123, "subject": "Acme", "status": 42}})
        if request.method == "PUT":
            assert path.endswith("/tickets/execute_scenario")
            state["writes"].append(json.loads(request.content))
            if state["failure"] == "disconnect":
                raise httpx.ReadError("synthetic disconnect", request=request)
            return httpx.Response(state["failure"] or 204)
        if request.method == "POST":
            assert path.endswith("/tickets/123/reply")
            body = json.loads(request.content)
            state["writes"].append(body)
            if state["failure"] == "disconnect":
                raise httpx.ReadError("synthetic disconnect", request=request)
            if state["failure"]:
                return httpx.Response(state["failure"], json={"error": "synthetic"})
            return httpx.Response(201, json={"conversation": {"id": 704}})
        assert path.endswith("/tickets/123/conversations")
        row = {"id": 704, **state["writes"][0]}
        if state["readback"] == "missing":
            row.pop("body")
        if state["readback"] == "wrong":
            row["id"] = 999
        return httpx.Response(200, json={"conversations": [row], "has_more": False})

    clients = []
    def make_client():
        c = Client(cookies={})
        c._client.close()
        c._client = httpx.Client(transport=httpx.MockTransport(handle))
        monkeypatch.setattr(c, "_get_csrf", lambda: "synthetic")
        clients.append(c)
        return c
    state["client"] = make_client()
    state["make_client"] = make_client
    yield state
    for c in clients:
        c.close()


def plan(w):
    return canned.reply_plan(w["client"], "INC-123", 701, "/canned_response/content", sender="support@example.com")


def test_exact_read_routes_and_cli(wire, monkeypatch):
    monkeypatch.setattr(cli, "_client", lambda: wire["client"])
    for args, expected in [(["canned", "folders"], "canned_response_folders"),
                           (["canned", "list", "--folder", "702"], "canned_responses"),
                           (["canned", "show", "701"], "canned_response"),
                           (["scenarios", "list"], "scenario_automations")]:
        result = CliRunner().invoke(cli.app, ["tickets", *args, "--json"])
        assert result.exit_code == 0, result.output
        assert expected in json.loads(result.output)
    result = CliRunner().invoke(cli.app, ["tickets", "scenarios", "show", "703", "--output", "json"])
    assert result.exit_code == 0 and json.loads(result.output)["id"] == 703
    assert not wire["writes"]


def test_review_cli_and_exact_reply_transport(wire, monkeypatch):
    monkeypatch.setattr(cli, "_client", lambda: wire["client"])
    args = ["tickets", "canned", "reply", "INC-123", "--response", "701", "--content-pointer", "/canned_response/content", "--sender", "support@example.com", "--json"]
    runner = CliRunner()
    review = runner.invoke(cli.app, args + ["--dry-run"])
    assert review.exit_code == 0, review.output
    p = json.loads(review.output)
    assert runner.invoke(cli.app, args + ["--yes"]).exit_code == 2
    assert not wire["writes"]
    result = runner.invoke(cli.app, args + ["--yes", "--expect-plan", p["plan_hash"]])
    assert result.exit_code == 0, result.output
    assert json.loads(result.output)["verified"]
    body = wire["writes"][0]
    assert body == {"body": "<div>Acme response</div>", "full_text": "<div>Acme response</div>",
                    "conversation_from_summary": False, "cc_emails": [], "translate_reply": False,
                    "freddy_message_id": None, "from_email": "support@example.com"}
    assert not any("canned" in key for key in body)
    with pytest.raises(conv.ConversationError, match="was sent"):
        conv.send(plan(wire), wire["make_client"](), reviewed_hash=p["plan_hash"], public_confirmed=True)
    assert len(wire["writes"]) == 1


@pytest.mark.parametrize("failure", [403, 429, 500, "disconnect"])
def test_ambiguous_single_attempt_across_clients(wire, failure):
    p = plan(wire)
    wire["failure"] = failure
    with pytest.raises(conv.ConversationError, match="unknown"):
        conv.send(p, wire["client"], reviewed_hash=p.hash, public_confirmed=True)
    with pytest.raises(conv.ConversationError, match="was sent"):
        conv.send(p, wire["make_client"](), reviewed_hash=p.hash, public_confirmed=True)
    assert len(wire["writes"]) == 1


@pytest.mark.parametrize("readback", ["missing", "wrong"])
def test_absent_or_wrong_exact_readback_never_guesses(wire, readback):
    p = plan(wire)
    wire["readback"] = readback
    assert not conv.send(p, wire["client"], reviewed_hash=p.hash, public_confirmed=True)["verified"]
    assert len(wire["writes"]) == 1


def test_stale_source_and_confirmation_block_before_send(wire):
    p = plan(wire)
    with pytest.raises(conv.ConversationError, match="confirmation"):
        conv.send(p, wire["client"], reviewed_hash=p.hash)
    wire["source"]["canned_response"]["name"] = "Changed metadata"
    with pytest.raises(conv.ConversationError, match="source changed"):
        conv.send(p, wire["client"], reviewed_hash=p.hash, public_confirmed=True)
    assert not wire["writes"]


@pytest.mark.parametrize("value", ["{{name}}", "{% if user %}", "${name}", "[[name]]", "", None, 123])
def test_no_guessed_placeholder_expansion(value):
    with pytest.raises(ValueError):
        canned.content({"body": value}, "/body")


@pytest.mark.parametrize("pointer", ["body", "/missing", "/body/~2"])
def test_invalid_content_pointer_refused(pointer):
    with pytest.raises(ValueError):
        canned.content({"body": "Acme"}, pointer)


@pytest.mark.parametrize("failure", [None, 403, 429, 500, "disconnect"])
def test_scenario_native_wire_and_durable_guard(wire, failure):
    from fsv.reviewed_write import ReviewedWriteError
    p = scenarios.build_plan(wire["client"], "INC-123", 703)
    wire["failure"] = failure
    if failure:
        with pytest.raises(ReviewedWriteError, match="unknown"):
            scenarios.send(p, wire["client"], reviewed_hash=p.hash)
    else:
        result = scenarios.send(p, wire["client"], reviewed_hash=p.hash)
        assert result["identity_verified"] and not result["verified"]
        assert "side effects" in result["warning"]
    assert wire["writes"] == [{"scenario_id": 703, "ids": [123]}]
    with pytest.raises(ReviewedWriteError, match="was sent"):
        scenarios.send(p, wire["make_client"](), reviewed_hash=p.hash)
    assert len(wire["writes"]) == 1


def test_legacy_conversation_hash_is_unchanged(wire):
    p = conv.build_plan(wire["client"], 123, "Acme")
    expected = journal.plan_hash({"tenant": p.tenant, "path": p.path, "payload": p.payload, "record_hash": p.record_hash})
    assert p.hash == expected
