"""Ticket templates, maintenance windows and agent notify entries (offline)."""
import json

import httpx
import pytest
from typer.testing import CliRunner

import fsv.change_flow as change_flow
import fsv.cli as cli
from fsv import change_windows as W, config, journal, schema, templates as T
from fsv.client import Client
from fsv.reviewed_write import ReviewedWriteError
from test_ticket_create import Client as CreateClient, meta

TEMPLATES = {"ticket_templates": [{"id": 501, "name": "Acme outage", "type": "shared", "workspace_id": 3},
                                  {"id": 502, "name": "Globex access", "type": "shared", "workspace_id": 3}],
             "meta": {"max_template_count": 2}}
TEMPLATE = {"ticket_template": {"id": 501, "name": "Acme outage", "template_data": {
    "subject": "Acme outage", "description": "<p>Please help</p>", "status": None, "tags": ["x"],
    "custom_fields": {"stage": "Live", "nested": {"a": 1}}}}}


class TemplateClient(CreateClient):
    def int_get(self, path, params=None):
        return {"ticket-templates": TEMPLATES, "ticket-templates/501": TEMPLATE}[path]


@pytest.fixture(autouse=True)
def isolated_journal(monkeypatch, tmp_path):
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal.jsonl")


def test_template_list_show_and_prefill_mapping():
    c = TemplateClient()
    assert [r["name"] for r in T.list_templates(c)] == ["Acme outage", "Globex access"]
    row = T.show(c, "acme OUTAGE")
    draft, applied, ignored = T.to_draft(row)
    assert draft.answers == {"subject": "Acme outage", "stage": "Live"}
    assert draft.description_html == "<p>Please help</p>"
    assert applied == ["subject", "stage", "description"] and ignored == ["nested", "tags"]
    with pytest.raises(T.TemplateError, match="no unique ticket template"):
        T.find(c, "Initech")


def test_create_with_template_goes_through_the_form_review(monkeypatch):
    c = TemplateClient()
    m = meta()
    monkeypatch.setattr(cli, "_client", lambda: c)
    monkeypatch.setattr(change_flow, "interactive", lambda: False)
    monkeypatch.setattr("fsv.ticket_cli.load_meta", lambda *a, **kw: m)
    args = ["tickets", "create", "--requester", "jane.doe@example.com", "--template", "Acme outage",
            "--set", "Cloud Only=Yes"]
    res = CliRunner().invoke(cli.app, args + ["--dry-run", "--json"])
    assert res.exit_code == 0, res.output
    doc = json.loads(res.output[res.output.index("{"):])
    assert doc["payload"]["subject"] == "Acme outage" and "Please help" in doc["payload"]["description"]
    assert not c.writes
    res = CliRunner().invoke(cli.app, ["tickets", "create", "--requester", "jane.doe@example.com",
                                       "--template", "Initech", "--dry-run"])
    assert res.exit_code == 2 and not c.writes


class AgentClient:
    def __init__(self, rows):
        self.rows, self.calls = rows, []

    def autocomplete(self, kind, query, params=None):
        self.calls.append((kind, query, params))
        return self.rows


def test_notify_agent_entry_is_name_and_email_from_agent_lookup():
    c = AgentClient([{"id": "robin@example.com", "value": "Robin Agent", "user_id": 77},
                     {"id": 8, "value": "Robin Agentson", "email": "ra@example.com"}])
    assert T.notify_entry(c, "Robin Agent") == "Robin Agent <robin@example.com>"
    assert c.calls[0] == ("agents", "Robin Agent", {"qf": "name"})
    assert T.notify_entry(c, "ra@example.com") == "Robin Agentson <ra@example.com>"
    for bad in ("Robin", "", "nobody@example.com"):
        with pytest.raises(T.TemplateError):
            T.notify_entry(c, bad)
    with pytest.raises(T.TemplateError):
        T.notify_entry(AgentClient([{"id": "x@example.com", "value": "Robin Agent", "active": False}]), "Robin Agent")


@pytest.fixture(params=["acme.example.com", "globex.example.com"])
def wapi(request, monkeypatch):
    domain = request.param
    monkeypatch.setattr(config, "DOMAIN", domain)
    monkeypatch.setattr(config, "API_INT", f"https://{domain}/api/_")
    monkeypatch.setattr("fsv.formmeta.configured_workspace", lambda: None)
    sch = {"fields": [{"name": "change_type", "label": "Change Type", "workspace_id": 3,
                       "choices": [{"id": 1, "value": "Normal"}, {"id": 4, "value": "Emergency"}]}]}
    monkeypatch.setattr(schema, "load", lambda res, client=None, force=False: sch)
    s = {"windows": [{"id": 61, "name": "Globex freeze", "start_time": "a", "end_time": "b"}], "writes": [],
         "overlaps": []}

    def handler(req):
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "GET":
            assert path == "maintenance_windows"
            return httpx.Response(200, json={"maintenance_windows": s["windows"]})
        body = json.loads(req.content) if req.content else None
        s["writes"].append((req.method, path, body))
        if path == W.OVERLAP_PATH:
            return httpx.Response(200, json={"overlapping_windows": s["overlaps"]})
        if req.method == "POST":
            row = {"id": 62, "name": body["change_window"]["name"]}
            s["windows"].append(row)
            return httpx.Response(201, json={"maintenance_window": row})
        wid = int(path.rsplit("/", 1)[1])
        s["windows"] = [w for w in s["windows"] if w["id"] != wid]
        return httpx.Response(200, json={"success": True, "message": "delete_success"})

    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handler))
    c._csrf = "synthetic"
    monkeypatch.setattr(cli, "_client", lambda: c)
    yield c, s
    c.close()


def _plan(c):
    return W.create_plan(c, "Acme release", "Quarterly", "2030-11-21 10:00", "2030-11-21 13:30", ["change_type=Normal"])


def test_window_create_sends_overlap_check_then_captured_body(wapi):
    c, s = wapi
    p = _plan(c)
    assert p.write.payload == {"change_window": {
        "name": "Acme release", "description": "Quarterly", "start_date": "21-11-2030", "start_time": "10:00am",
        "end_date": "21-11-2030", "end_time": "1:30pm",
        "change_event": {"additional_info": {}, "criteria": [{"name": "change_type", "operator": "is", "value": "1"}]},
        "workspace_id": 3, "alert_suppression": True}}
    assert not s["writes"]
    r = W.send_create(p, c, reviewed_hash=p.hash, rebuild=lambda: _plan(c))
    assert r["verified"] and r["window_id"] == 62
    assert [w[1] for w in s["writes"]] == [W.OVERLAP_PATH, "maintenance_windows"]
    assert s["writes"][0][2]["type"] == "blackout_events"
    with pytest.raises(ReviewedWriteError, match="do not replay"):
        W.send_create(p, c, reviewed_hash=p.hash)
    assert len(s["writes"]) == 2


def test_window_overlap_and_invalid_input_refuse(wapi):
    c, s = wapi
    s["overlaps"] = [{"id": 61}]
    p = _plan(c)
    with pytest.raises(ReviewedWriteError, match="overlapping"):
        W.send_create(p, c, reviewed_hash=p.hash)
    assert [w[1] for w in s["writes"]] == [W.OVERLAP_PATH]
    for kw in ({"criteria": []}, {"criteria": ["priority=High"]}, {"criteria": ["change_type=Minor"]},
               {"end": "2030-11-21 09:00"}, {"start": "21-11-2030 10:00"}):
        args = {"name": "x", "description": "", "start": "2030-11-21 10:00", "end": "2030-11-21 11:00",
                "criteria": ["change_type=Normal"], **kw}
        with pytest.raises(ReviewedWriteError):
            W.create_plan(c, args["name"], args["description"], args["start"], args["end"], args["criteria"])


def test_window_delete_and_cli(wapi):
    c, s = wapi
    runner = CliRunner()
    r = runner.invoke(cli.app, ["changes", "windows", "list", "--json"])
    assert r.exit_code == 0 and json.loads(r.output)[0]["id"] == 61
    r = runner.invoke(cli.app, ["changes", "windows", "delete", "61", "--dry-run"])
    assert r.exit_code == 0, r.output
    h = json.loads(r.output)["plan_hash"]
    r = runner.invoke(cli.app, ["changes", "windows", "delete", "61", "--yes", "--expect-plan", h])
    assert r.exit_code == 0, r.output
    assert json.loads(r.output)["fields"] == {"absent": "matched"}
    assert s["writes"] == [("DELETE", "maintenance_windows/61", None)]
    r = runner.invoke(cli.app, ["changes", "windows", "create", "--name", "Acme", "--start", "2030-11-21 10:00",
                                "--end", "2030-11-21 11:00", "--criteria", "change_type=Emergency", "--dry-run"])
    assert r.exit_code == 0, r.output
    assert json.loads(r.output)["overlap_check"]["payload"]["criteria"][0]["value"] == "4"
    assert len(s["writes"]) == 1
