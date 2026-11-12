"""Synthetic, real-client ticket routing contracts and membership checks."""
import copy
import json

import httpx
import pytest
from typer.testing import CliRunner

import acme_change_form as a
import fsv.cli as cli
from fsv import config, journal, ticket_assignment as flow, ticket_properties
from fsv.client import Client
from fsv.formmeta import FormMeta
from fsv.reviewed_write import ReviewedWriteError


def metadata():
    fields = [a._f("group", "Group", "default_group", 1), a._f("agent", "Agent", "default_agent", 2)]
    for i, f in enumerate(fields, 9000007100):
        f["id"] = i
    return FormMeta("ticket", "edit_form", {"ticket_fields": fields}, {"business_rules": []}, 1, 0, "fixture", 123)


@pytest.fixture
def api(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(config, "API_INT", "https://acme.example.com/api/_")
    monkeypatch.setattr(config, "API_V2", "https://acme.example.com/api/v2")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    monkeypatch.setattr(ticket_properties, "load_meta", lambda *a, **k: metadata())
    state = {"boot": {"agents": [{"id": 71, "name": "Acme Agent", "email": "agent@example.com", "group_ids": [81], "deleted": False},
                                  {"id": 72, "name": "Globex Agent", "group_ids": [], "deleted": False}],
                      "groups": [{"id": 81, "name": "Acme Desk", "agent_ids": [71]},
                                 {"id": 82, "name": "Globex Desk", "agent_ids": []}]},
             "tickets": {123: {"id": 123, "responder_id": 71, "group_id": 81},
                         124: {"id": 124, "responder_id": None, "group_id": 81}},
             "writes": [], "fail": None, "mismatch": False}
    def handle(req):
        path = req.url.path
        if path == "/api/v2/agents/me":
            return httpx.Response(200, json={"agent": {"id": 71}})
        suffix = path.removeprefix("/api/_/")
        if suffix == "bootstrap/agents_groups":
            return httpx.Response(200, json=copy.deepcopy(state["boot"]))
        if req.method == "GET" and suffix.startswith("tickets/"):
            return httpx.Response(200, json={"ticket": copy.deepcopy(state["tickets"][int(suffix.split("/")[1])])})
        if req.method == "PUT":
            body = json.loads(req.content)
            state["writes"].append((suffix, body))
            if state["fail"] == "disconnect":
                raise httpx.ReadError("connection lost", request=req)
            if state["fail"]:
                return httpx.Response(state["fail"], json={"message": "synthetic failure"})
            if suffix == "tickets/pick_up":
                for cid in body["ids"]:
                    if not state["mismatch"]:
                        state["tickets"][cid]["responder_id"] = 71
                return httpx.Response(204)
            cid = int(suffix.split("/")[1])
            if not state["mismatch"]:
                state["tickets"][cid].update({k: v for k, v in body.items() if k != "br_validation_excludes"})
            return httpx.Response(200, json={"ticket": copy.deepcopy(state["tickets"][cid])})
        raise AssertionError(path)
    client = Client(cookies={})
    client._client.close()
    client._client = httpx.Client(transport=httpx.MockTransport(handle))
    client._csrf = "synthetic"
    return client, state


def test_single_unassign_never_bulk(api):
    c, state = api
    p = flow.build_plan(c, [123], unassign=True)
    assert p.write.path == "tickets/123" and p.write.payload["responder_id"] is None
    result = flow.send(p, c, reviewed_hash=p.hash)
    assert result["verified"] and len(state["writes"]) == 1
    assert state["writes"][0][1]["responder_id"] is None
    with pytest.raises(ReviewedWriteError, match="was sent"):
        flow.send(p, c, reviewed_hash=p.hash)


def test_group_agent_and_bulk_pickup(api):
    c, state = api
    p = flow.build_plan(c, [123], group="Acme Desk", agent="agent@example.com")
    assert p.write.payload["group_id"] == 81 and p.write.payload["responder_id"] == 71
    assert flow.send(p, c, reviewed_hash=p.hash)["verified"]
    p = flow.build_plan(c, [123, 124], pickup=True)
    assert p.write.payload == {"ids": [123, 124]}
    assert flow.send(p, c, reviewed_hash=p.hash)["verified"]
    assert state["writes"][-1] == ("tickets/pick_up", {"ids": [123, 124]})


def test_group_only_clears_previous_responder(api):
    c, state = api
    p = flow.build_plan(c, [123], group="Globex Desk")
    assert p.write.payload["group_id"] == 82 and p.write.payload["responder_id"] is None
    assert flow.send(p, c, reviewed_hash=p.hash)["verified"]


def test_membership_missing_or_mismatch_fails_closed(api):
    c, state = api
    with pytest.raises(ReviewedWriteError, match="not a verified member"):
        flow.build_plan(c, [123], agent="Globex Agent")
    state["boot"]["groups"][0].pop("agent_ids")
    with pytest.raises(ReviewedWriteError, match="membership metadata unavailable"):
        flow.build_plan(c, [123], pickup=True)
    assert not state["writes"]


@pytest.mark.parametrize("failure", [403, 429, 500, "disconnect"])
def test_ambiguous_never_replayed(api, failure):
    c, state = api
    p = flow.build_plan(c, [123], unassign=True)
    state["fail"] = failure
    with pytest.raises(ReviewedWriteError, match="unknown"):
        flow.send(p, c, reviewed_hash=p.hash)
    with pytest.raises(ReviewedWriteError, match="was sent"):
        flow.send(p, c, reviewed_hash=p.hash)
    assert len(state["writes"]) == 1
    assert [r["event"] for r in journal.read() if r.get("kind") == "plan:ticket_assign"] == ["started"]


def test_changed_metadata_and_mismatched_readback(api):
    c, state = api
    p = flow.build_plan(c, [123], unassign=True)
    state["boot"]["groups"][0]["name"] = "Changed"
    with pytest.raises(ReviewedWriteError, match="changed"):
        flow.send(p, c, reviewed_hash=p.hash)
    p = flow.build_plan(c, [123], unassign=True)
    state["mismatch"] = True
    result = flow.send(p, c, reviewed_hash=p.hash)
    assert not result["verified"] and result["tickets"]["123"]["responder_id"] == "mismatched"
    assert len(state["writes"]) == 1


def test_cli_review(api, monkeypatch):
    c, state = api
    monkeypatch.setattr(cli, "_client", lambda: c)
    runner = CliRunner()
    args = ["tickets", "assign", "INC-123", "--unassign"]
    review = runner.invoke(cli.app, args + ["--dry-run", "--json"])
    assert review.exit_code == 0, review.output
    payload = json.loads(review.output)
    assert payload["payload"]["responder_id"] is None
    bad = runner.invoke(cli.app, args + ["--yes", "--expect-plan", payload["plan_hash"][:16]])
    assert bad.exit_code == 2 and not state["writes"]
