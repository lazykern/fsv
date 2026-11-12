"""Offline merge wire, readback and duplicate-guard regressions."""
import copy
import json

import httpx
import pytest
from typer.testing import CliRunner

import acme_change_form as a
import fsv.cli as cli
from fsv import config, journal, ticket_merge as flow
from fsv.client import Client
from fsv.formmeta import FormMeta
from fsv.reviewed_write import ReviewedWriteError


def metadata():
    fields = [a._f("status", "Status", "default_status", 1, choices=[
        {"id": 31, "value": "Open"}, {"id": 89, "value": "Closed"}])]
    fields[0]["id"] = 9000007101
    return FormMeta("ticket", "edit_form", {"ticket_fields": fields}, {"business_rules": []}, 1, 0, "fixture", 123)


@pytest.fixture
def api(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(config, "API_INT", "https://acme.example.com/api/_")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    monkeypatch.setattr(flow, "load_meta", lambda *a, **k: metadata())
    state = {"records": {i: {"id": i, "status": 31, "subject": f"Acme {i}"} for i in (123, 124, 125)},
             "histories": {123: [{"id": 10, "body": "earlier"}],
                           124: [{"id": 11, "body": "source A"}],
                           125: [{"id": 12, "body": "source B"}]}, "writes": [], "fail": None}

    def handle(request):
        path = request.url.path.removeprefix("/api/_/")
        if request.method == "POST":
            state["writes"].append((path, json.loads(request.content)))
            if state["fail"] == "disconnect":
                raise httpx.ReadError("connection lost", request=request)
            if state["fail"]:
                return httpx.Response(state["fail"], json={"message": "synthetic failure"})
            body = json.loads(request.content)
            assert path == "merge_tickets/complete_merge"
            for i in body["source"]["ticket_ids"]:
                state["records"][i]["status"] = 89
                state["histories"][123].extend(state["histories"][i])
                state["histories"][i] = [{"id": 100 + i, "body": "<div>Secondary note</div>", "private": True}]
            state["histories"][123].append({"id": 20, "body": "<p>Primary note</p>", "private": True})
            if state.get("bad_readback") == "status":
                state["records"][124].pop("status")
            elif state.get("bad_readback") == "note":
                state["histories"][123][-1].pop("body")
            elif state.get("bad_readback") == "moved":
                state["histories"][123] = [row for row in state["histories"][123] if row["id"] != 11]
            return httpx.Response(200, json={"count": 2, "target": body["target"], "source": body["source"]})
        parts = path.split("/")
        if len(parts) == 2 and parts[0] == "tickets":
            return httpx.Response(200, json={"ticket": copy.deepcopy(state["records"][int(parts[1])])})
        if len(parts) == 3 and parts[2] == "conversations":
            rows = state["histories"][int(parts[1])]
            return httpx.Response(200, json={"conversations": copy.deepcopy(rows), "meta": {"has_next": False}})
        raise AssertionError(path)

    client = Client(cookies={})
    client._client.close()
    client._client = httpx.Client(transport=httpx.MockTransport(handle))
    client._csrf = "synthetic"
    return client, state


def plan(c):
    return flow.build_plan(c, "INC-123", ["INC-124", "INC-125"],
                           note_primary="Primary note", note_secondary="Secondary note")


def test_exact_wire_and_independent_readback(api):
    c, state = api
    p = plan(c)
    assert p.review()["path"].endswith("complete_merge")
    assert "Irreversible" in flow.review(p)["warning"]
    assert not state["writes"]
    result = flow.send(p, c, reviewed_hash=p.hash)
    assert result["verified"] and result["surviving_ticket"] == 123
    assert all(all(v.values()) for v in result["secondaries"].values())
    assert state["writes"] == [("merge_tickets/complete_merge", {
        "target": {"ticket_id": 123, "note": "Primary note", "is_private": True},
        "source": {"ticket_ids": [124, 125], "note": "Secondary note", "is_private": True}})]
    with pytest.raises(ReviewedWriteError, match="was sent"):
        flow.send(p, c, reviewed_hash=p.hash)
    assert len(state["writes"]) == 1


@pytest.mark.parametrize("failure", [403, 429, 500, "disconnect"])
def test_ambiguous_never_replayed(api, failure):
    c, state = api
    p = plan(c)
    state["fail"] = failure
    with pytest.raises(ReviewedWriteError, match="unknown"):
        flow.send(p, c, reviewed_hash=p.hash)
    replacement = Client(cookies={})
    replacement._client.close()
    replacement._client = c._client
    replacement._csrf = "synthetic"
    with pytest.raises(ReviewedWriteError, match="was sent"):
        flow.send(p, replacement, reviewed_hash=p.hash)
    assert len(state["writes"]) == 1
    assert [r["event"] for r in journal.read() if r.get("kind") == "plan:ticket_merge"] == ["started"]


@pytest.mark.parametrize("bad", ["status", "note", "moved"])
def test_missing_or_mismatched_readback_is_not_verified(api, bad):
    c, state = api
    p = plan(c)
    state["bad_readback"] = bad
    result = flow.send(p, c, reviewed_hash=p.hash)
    assert result["verified"] is False and "do not resend" in result["warning"]
    with pytest.raises(ReviewedWriteError, match="was sent"):
        flow.send(p, c, reviewed_hash=p.hash)
    assert len(state["writes"]) == 1


def test_stale_and_invalid_are_refused(api):
    c, state = api
    p = plan(c)
    with pytest.raises(ReviewedWriteError, match="full review"):
        flow.send(p, c, reviewed_hash=p.hash[:16])
    state["records"][124]["subject"] = "changed"
    with pytest.raises(ReviewedWriteError, match="changed"):
        flow.send(p, c, reviewed_hash=p.hash)
    with pytest.raises(ValueError, match="distinct"):
        flow.build_plan(c, 123, [123], note_primary="A", note_secondary="B")
    with pytest.raises(ValueError, match="nonempty"):
        flow.build_plan(c, 123, [124], note_primary="", note_secondary="B")
    assert not state["writes"]


def test_cli_json_review_and_full_hash_gate(api, monkeypatch):
    c, state = api
    monkeypatch.setattr(cli, "_client", lambda: c)
    args = ["tickets", "merge", "INC-123", "INC-124", "--note-primary", "Primary note", "--note-secondary", "Secondary note"]
    runner = CliRunner()
    result = runner.invoke(cli.app, args + ["--dry-run", "--json"])
    assert result.exit_code == 0, result.output
    review = json.loads(result.output)
    assert review["surviving_ticket"] == 123 and review["merged_tickets"] == [124]
    bad = runner.invoke(cli.app, args + ["--yes", "--expect-plan", review["plan_hash"][:16]])
    assert bad.exit_code == 2 and not state["writes"]


@pytest.mark.parametrize("ident,expected", [("89", 89), (89, 89), ("0", None), ("8a", None), (True, None), ("-89", None)])
def test_closed_status_accepts_only_positive_digit_identities(monkeypatch, ident, expected):
    fields = [a._f("status", "Status", "default_status", 1, choices=[
        {"id": "31", "value": "Open"}, {"id": ident, "value": "Closed"}])]
    fields[0]["id"] = 9000007101
    monkeypatch.setattr(flow, "load_meta", lambda *a, **k: FormMeta(
        "ticket", "edit_form", {"ticket_fields": fields}, {"business_rules": []}, 1, 0, "fixture", 123))
    if expected is None:
        with pytest.raises(ReviewedWriteError, match="Closed status"):
            flow._closed(None, 123)
    else:
        status, _ = flow._closed(None, 123)
        assert status == expected and type(status) is int
