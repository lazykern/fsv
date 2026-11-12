"""Offline native task envelopes and single-attempt lifecycle."""
import copy
import json

import httpx
import pytest
from typer.testing import CliRunner

import fsv.cli as cli
from fsv import config, journal, ticket_tasks as flow
from fsv.client import Client
from fsv.reviewed_write import ReviewedWriteError


@pytest.fixture(params=[21, 61])
def api(request, monkeypatch, tmp_path):
    n = request.param
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(config, "API_INT", "https://acme.example.com/api/_")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    state = {"rows": [], "writes": [], "fail": None, "partial": False, "mismatch": False}
    fields = [{"name": "title", "required_for_create": True},
              {"name": "status", "choices": [{"id": n, "value": "Open"}, {"id": n+1, "value": "Completed"}, {"id": n+2, "value": "Canceled"}]},
              {"name": "notify_before", "choices": [{"id": 0, "value": "Never"}]},
              {"name": "tier", "label": "Tier", "field_type": "custom_dropdown", "required_for_create": True,
               "choices": [{"id": n+3, "value": "Standard"}]}]
    def handler(req):
        path = req.url.path
        if req.method == "GET":
            if path.endswith("ticket_task_fields"):
                return httpx.Response(200, json={"ticket_task_fields": copy.deepcopy(fields)})
            assert path == "/api/_/tickets/123/tasks"
            if state["partial"]:
                return httpx.Response(403, json={"error": "denied"})
            return httpx.Response(200, json={"tasks": copy.deepcopy(state["rows"]), "meta": {"count": len(state["rows"])}})
        body = json.loads(req.content)
        state["writes"].append((req.method, path, body))
        if state["fail"] == "disconnect":
            raise httpx.ReadError("disconnect", request=req)
        if state["fail"]:
            return httpx.Response(state["fail"], json={"error": "failure"})
        if path.endswith("/update"):
            state["rows"] = []
            result = {"tasks": []}
        elif req.method == "POST":
            row = {"id": 701, "human_display_id": "TSK-701", **body}
            state["rows"] = [copy.deepcopy(row)]
            result = {"task": row}
        else:
            state["rows"][0].update(body)
            result = {"task": copy.deepcopy(state["rows"][0])}
        if state["mismatch"] and state["rows"]:
            state["rows"][0]["status"] = 999
        if state.get("partial_after"):
            state["partial"] = True
        return httpx.Response(201 if req.method == "POST" else 200, json=result)
    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handler))
    c._csrf = "synthetic"
    monkeypatch.setattr(cli, "_client", lambda: c)
    yield c, state, fields, n
    c.close()


def add(c):
    return flow.build_plan(c, "INC-123", action="add", title="Acme task", values={"Tier": "Standard"})


def test_missing_reminder_metadata_is_optional(api):
    c, s, fields, _ = api
    fields[:] = [f for f in fields if f["name"] != "notify_before"]
    runner = CliRunner()
    r = runner.invoke(cli.app, ["tickets", "tasks", "add", "123", "--title", "Acme task",
                               "--set", "Tier=Standard", "--dry-run", "--json"])
    assert r.exit_code == 0, r.output
    assert "notify_before" not in json.loads(r.output)["payload"]
    assert not s["writes"]


def test_native_string_choices_and_nested_required_flags(api):
    c, s, fields, n = api
    fields[2]["choices"][0]["id"] = "0"
    fields[-1].pop("field_type")
    fields[-1].pop("required_for_create")
    fields[-1].update(name="cf_tier", default=False, type=1003, dom_type="dropdown",
                      field_options={"required_for_create": "true", "required_for_closure": "false"})
    fields[-1]["choices"][0]["id"] = str(n+3)
    fields.append({"name": "workspace_id", "default": True, "field_options": {"required_for_create": "true"}})
    with pytest.raises(ReviewedWriteError, match="Tier"):
        flow.build_plan(c, 123, action="add", title="Acme task")
    plan = add(c)
    # Same body as the accepted web create: no workspace_id, unprefixed custom key.
    assert plan.payload == {"title": "Acme task", "status": n, "notify_before": 0, "custom_fields": {"tier": n+3}}
    assert flow.send(plan, c, reviewed_hash=plan.hash)["verified"]
    assert s["writes"][-1][2] == plan.payload


def test_all_unknown_required_fields_are_listed(api):
    c, s, fields, _ = api
    fields.extend([{"name": "unproven_a", "label": "Acme dependency", "required": True},
                   {"name": "unproven_b", "label": "Globex dependency", "field_options": {"required_for_create": "true"}},
                   {"name": "closure_only", "field_options": {"required_for_closure": "true"}}])
    with pytest.raises(ReviewedWriteError, match="Acme dependency, Globex dependency") as exc:
        add(c)
    assert "closure_only" not in str(exc.value)
    assert not s["writes"]


@pytest.mark.parametrize("choice", [True, -1, "-1", "bad", None])
def test_present_reminder_must_be_proven(api, choice):
    c, s, fields, _ = api
    fields[2]["choices"][0]["id"] = choice
    with pytest.raises(ReviewedWriteError, match="notify_before"):
        add(c)
    assert not s["writes"]


def test_lifecycle_and_exact_wires(api):
    c, s, _, n = api
    plan = add(c)
    assert flow.send(plan, c, reviewed_hash=plan.hash)["verified"]
    assert s["writes"][-1] == ("POST", "/api/_/tickets/123/tasks", {"title": "Acme task", "status": n, "notify_before": 0, "custom_fields": {"tier": n+3}})
    plan = flow.build_plan(c, 123, action="status", task=701, status="Canceled")
    assert flow.send(plan, c, reviewed_hash=plan.hash)["verified"]
    assert s["writes"][-1] == ("PUT", "/api/_/tickets/123/tasks/701", {"status": n+2})
    plan = flow.build_plan(c, 123, action="delete", task=701)
    assert flow.send(plan, c, reviewed_hash=plan.hash)["verified"]
    assert s["writes"][-1] == ("PUT", "/api/_/tickets/123/tasks/update", {"id": "701", "action": "delete"})
    with pytest.raises(ReviewedWriteError):
        flow.send(plan, c, reviewed_hash=plan.hash)
    assert len(s["writes"]) == 3


@pytest.mark.parametrize("failure", [403, 429, 500, "disconnect"])
def test_unknown_blocks_replay(api, failure):
    c, s, _, _ = api
    plan = add(c)
    s["fail"] = failure
    with pytest.raises(ReviewedWriteError, match="unknown"):
        flow.send(plan, c, reviewed_hash=plan.hash)
    with pytest.raises(ReviewedWriteError, match="was sent"):
        flow.send(plan, c, reviewed_hash=plan.hash)
    assert len(s["writes"]) == 1


def test_validation_and_drift(api):
    c, s, fields, _ = api
    with pytest.raises(ReviewedWriteError, match="Tier"):
        flow.build_plan(c, 123, action="add", title="Acme")
    with pytest.raises(ReviewedWriteError):
        flow.build_plan(c, 123, action="status", task="bad", status="Open")
    plan = add(c)
    fields[1]["choices"][0]["id"] += 100
    with pytest.raises(ReviewedWriteError, match="changed"):
        flow.send(plan, c, reviewed_hash=plan.hash)
    assert not s["writes"]


@pytest.mark.parametrize("mode", ["mismatch", "partial_after"])
def test_readback_is_independent(api, mode):
    c, s, _, _ = api
    plan = add(c)
    s[mode] = True
    assert not flow.send(plan, c, reviewed_hash=plan.hash)["verified"]
    assert len(s["writes"]) == 1


def test_cli_full_hash(api):
    _, s, _, _ = api
    runner = CliRunner()
    args = ["tickets", "tasks", "add", "INC-123", "--title", "Acme task", "--set", "Tier=Standard"]
    r = runner.invoke(cli.app, args + ["--dry-run", "--json"])
    assert r.exit_code == 0, r.output
    hash_ = json.loads(r.output)["plan_hash"]
    assert not s["writes"]
    assert runner.invoke(cli.app, args + ["--yes", "--expect-plan", hash_[:16]]).exit_code == 2
    r = runner.invoke(cli.app, args + ["--yes", "--expect-plan", hash_, "--json"])
    assert r.exit_code == 0, r.output
    assert json.loads(r.output)["verified"]
    assert runner.invoke(cli.app, ["tickets", "tasks", "list", "INC-123", "--json"]).exit_code == 0
