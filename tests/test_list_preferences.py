"""Offline list preference and export subset, never invented download jobs."""
import copy
import json

import httpx
import pytest

from fsv.errors import APIError
from typer.testing import CliRunner

from fsv import cli, config, journal, list_preferences as flow
from fsv.client import Client
from fsv.reviewed_write import ReviewedWriteError


@pytest.fixture(params=["tickets", "changes", "problems"])
def wire(request, monkeypatch, tmp_path):
    module = request.param
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    rows = [{"name": key, "label": label, "position": i + 1, "selected": True, "frozen": key == "display_id"}
            for i, (key, label) in enumerate([("ticket_subject", "Acme subject"), ("requester", "Acme requester"),
                                            ("status", "Acme state"), ("priority", "Acme priority"), ("display_id", "Acme ID")])]
    row = {"id": 701, "name": "Acme queue", "query_hash": [{"condition": "status", "operator": "is_in", "value": ["47"], "type": "default"}], "advanced_query_hash": None}
    state = {"module": module, "headers": rows, "row": row, "writes": [], "failure": None, "readback": "ok"}
    filters = {"tickets": "ticket_filters", "changes": "change-filters", "problems": "problem-filters"}[module]
    item_key = module[:-1] + "_filter"

    def handle(req):
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "GET":
            if path == f"{module}/headers":
                rows = copy.deepcopy(state["headers"])
                if state["writes"] and state["readback"] == "missing":
                    rows[0].pop("frozen")
                return httpx.Response(200, json={"headers": rows})
            if path == filters:
                return httpx.Response(200, json={"filters": [row]})
            assert path == filters + "/701"
            return httpx.Response(200, json={item_key: row})
        body = json.loads(req.content)
        state["writes"].append((path, body))
        if state["failure"] == "disconnect":
            raise httpx.ReadError("synthetic disconnect", request=req)
        if state["failure"]:
            return httpx.Response(state["failure"], json={"error": "synthetic"})
        if path.endswith("/headers"):
            for header in state["headers"]:
                header.update(selected=header["name"] in body["columns"],
                              position=body["columns"].get(header["name"]),
                              frozen=header["name"] in body["frozen_columns"])
        else:
            assert path == f"{module}/export"
        return httpx.Response(204)

    clients = []
    def make():
        c = Client(cookies={})
        c._client.close()
        c._client = httpx.Client(transport=httpx.MockTransport(handle))
        monkeypatch.setattr(c, "_get_csrf", lambda: "synthetic")
        clients.append(c)
        return c
    state.update(client=make(), make=make)
    yield state
    for c in clients:
        c.close()


def plan(wire, kind):
    if kind == "columns":
        return flow.columns_plan(wire["client"], wire["module"], ["ticket_subject", "requester", "status"], [])
    return flow.export_plan(wire["client"], wire["module"], "701", ["subject", "status_name", "display_id"])


def test_exact_columns_payload_and_independent_readback(wire):
    p = plan(wire, "columns")
    assert p.payload == {"columns": {"ticket_subject": 1, "requester": 2, "status": 3, "display_id": 1}, "frozen_columns": ["display_id"]}
    assert flow.send(p, wire["client"], reviewed_hash=p.hash)["verified"]
    assert wire["writes"] == [(wire["module"] + "/headers", p.payload)]


def test_exact_export_request_uses_runtime_view_and_labels_not_download_claim(wire):
    p = plan(wire, "export")
    expected = {"format": "csv", "date_filter": 30, "export_fields": {"subject": "Acme subject", "status_name": "Acme state", "display_id": "Acme ID"},
                "custom_fields": {}, "data_hash": wire["row"]["query_hash"]}
    if wire["module"] == "tickets":
        expected.update(ticket_state_filter="created_at", advanced_data_hash=None,
                        selected_filter_view={"id": "701", "name": "Acme queue", "is_additional_filters_applied": False})
    else:
        expected["state_filter"] = "created_at"
        if wire["module"] == "changes":
            expected["advanced_data_hash"] = None
    assert p.payload == expected
    result = flow.send(p, wire["client"], reviewed_hash=p.hash)
    assert not result["verified"] and result["source_readback"]
    assert result["job"] == result["download"] == "unavailable"
    assert wire["writes"] == [(wire["module"] + "/export", expected)]


@pytest.mark.parametrize("kind", ["columns", "export"])
@pytest.mark.parametrize("failure", [403, 429, 500, "disconnect"])
def test_durable_single_attempt_failure_guards(wire, kind, failure):
    p = plan(wire, kind)
    wire["failure"] = failure
    with pytest.raises(ReviewedWriteError, match="unknown"):
        flow.send(p, wire["client"], reviewed_hash=p.hash)
    with pytest.raises(ReviewedWriteError, match="was sent"):
        flow.send(p, wire["make"](), reviewed_hash=p.hash)
    assert len(wire["writes"]) == 1


def test_header_drift_and_unavailable_preference_readback(wire):
    p = plan(wire, "columns")
    wire["headers"][0]["label"] = "Changed"
    with pytest.raises(ReviewedWriteError, match="changed"):
        flow.send(p, wire["client"], reviewed_hash=p.hash)
    assert not wire["writes"]
    p = plan(wire, "columns")
    wire["readback"] = "missing"
    result = flow.send(p, wire["client"], reviewed_hash=p.hash)
    assert not result["verified"] and result["fields"]["ticket_subject"] == "unavailable"


def test_cli_hash_gate_and_json_options(wire, monkeypatch):
    monkeypatch.setattr(cli, "_client", lambda: wire["client"])
    args = [wire["module"], "columns", "set", "--column", "ticket_subject", "--column", "status", "--json"]
    runner = CliRunner()
    review = runner.invoke(cli.app, args + ["--dry-run"])
    assert review.exit_code == 0, review.output
    digest = json.loads(review.output)["plan_hash"]
    assert runner.invoke(cli.app, args + ["--yes"]).exit_code == 2 and not wire["writes"]
    sent = runner.invoke(cli.app, args + ["--yes", "--expect-plan", digest])
    assert sent.exit_code == 0, sent.output
    assert json.loads(sent.output)["verified"]



@pytest.mark.parametrize("me,mailbox", [({"id": 9000001, "email": "agent@example.com"}, "agent@example.com"),
                                        (APIError(403, "hidden"), "unavailable"), ({"id": 9000001}, "unavailable")])
def test_export_review_shows_the_recipient_mailbox(wire, monkeypatch, me, mailbox):
    def fake_me():
        if isinstance(me, Exception):
            raise me
        return me
    monkeypatch.setattr(wire["client"], "me", fake_me)
    p = plan(wire, "export")
    review = p.review()
    assert review["recipient"]["mailbox"] == mailbox
    assert "no recipient field" in review["recipient"]["note"]
    # Display only: the reviewed request and hash do not change with the mailbox.
    monkeypatch.setattr(wire["client"], "me", lambda: {"email": "other@example.com"})
    assert plan(wire, "export").hash == p.hash
    assert "recipient" not in plan(wire, "columns").review() and not wire["writes"]
