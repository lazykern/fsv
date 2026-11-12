"""A 'shifted' synthetic tenant: every tenant-configurable id is moved away
from the platform defaults (statuses 21-23, task statuses 7-9, reminders
600/0 with renamed labels) and the tenant has two workspaces (5 and 9) whose
form fields differ. Touched modules must take every value from metadata."""
from __future__ import annotations

import copy
import json

import httpx
import pytest

from fsv import change_tasks, config, formmeta, journal, problem_status
from fsv.client import Client
from fsv.forms import FormSpec, RulesEngine

PROBLEM_STATUSES = [{"id": 21, "value": "New"}, {"id": 22, "value": "Wip"}, {"id": 23, "value": "Done"}]
TASK_FIELDS = [{"name": "status", "choices": [{"id": 7, "value": "Todo"}, {"id": 8, "value": "Doing"},
                                              {"id": 9, "value": "Done"}]},
               {"name": "notify_before", "choices": [{"id": 0, "value": "None"}, {"id": 600, "value": "10 Min"}]}]
DEFAULT_LITERALS = {1, 2, 3, 4, 5, 900, 1800, 2700, 3600, 7200}


def problem_fields(workspace: int) -> list[dict]:
    extra = "ws5_only" if workspace == 5 else "ws9_only"
    return [{"id": 601, "name": "subject", "label": "Subject", "field_type": "default_subject", "position": 1,
             "workspace_id": workspace},
            {"id": 602, "name": "status", "label": "Status", "field_type": "default_status", "position": 2,
             "choices": copy.deepcopy(PROBLEM_STATUSES), "workspace_id": workspace},
            {"id": 603, "name": extra, "label": extra, "field_type": "custom_text", "position": 3,
             "workspace_id": workspace, "default_field": False}]


@pytest.fixture
def shifted(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "globex.example.com")
    monkeypatch.setattr(config, "API_INT", "https://globex.example.com/api/_")
    monkeypatch.setattr(config, "CACHE_DIR", tmp_path / "cache")
    monkeypatch.setattr(config, "CONFIG_FILE", tmp_path / "config.json")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    s = {"workspace": 5, "field_reads": 0, "writes": [],
         "row": {"id": 77, "subject": "Globex problem", "status": 21, "planned_start_date": None, "planned_end_date": None}}

    def handler(req):
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "GET":
            if path == "problem_form_fields":
                s["field_reads"] += 1
                return httpx.Response(200, json={"problem_fields": problem_fields(s["workspace"])})
            if path.endswith("business_rules"):
                return httpx.Response(200, json={"business_rules": []})
            if path == "problems/77":
                return httpx.Response(200, json={"problem": copy.deepcopy(s["row"])})
            raise AssertionError(path)
        s["writes"].append(json.loads(req.content))
        s["row"].update(s["writes"][-1])
        return httpx.Response(200, json={})
    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handler))
    c._csrf = "synthetic"
    yield c, s
    c.close()


def use_workspace(ws: int) -> None:
    config.save_config({"domain": "globex.example.com", "forms": {"workspace_id": ws}})


def test_problem_status_sends_the_shifted_runtime_id(shifted):
    c, s = shifted
    use_workspace(5)
    plan = problem_status.build_plan(c, "PRB-77", "Wip")
    assert plan.payload == {"status": 22}
    result = problem_status.send(plan, c, reviewed_hash=plan.hash)
    assert result["verified"] and s["writes"] == [{"status": 22}]


def test_two_workspaces_never_share_cached_form_fields(shifted):
    c, s = shifted
    use_workspace(5)
    assert formmeta.load_meta(c, "problem", "new").spec.find("ws5_only") is not None
    s["workspace"] = 9
    use_workspace(9)
    meta9 = formmeta.load_meta(c, "problem", "new")
    assert meta9.spec.find("ws9_only") is not None and meta9.spec.find("ws5_only") is None
    assert s["field_reads"] == 2
    use_workspace(5)
    formmeta.load_meta(c, "problem", "new")  # ws5 cache still valid: no third read
    assert s["field_reads"] == 2


def test_change_task_payload_has_no_default_tenant_literals(monkeypatch):
    monkeypatch.setattr(config, "DOMAIN", "globex.example.com")
    label = change_tasks.runtime_reminder_label("10 Min", TASK_FIELDS)
    status = change_tasks.default_status(TASK_FIELDS)
    status_id = change_tasks.task_status.resolve(status, TASK_FIELDS)[0]
    plan = change_tasks.build_plan(77, "Globex task", status=status, status_id=status_id,
                                   notify_before=label, fields=TASK_FIELDS)
    assert plan.payload["status"] == 7 and plan.payload["notify_before"] == 600
    assert not {plan.payload["status"], plan.payload["notify_before"]} & DEFAULT_LITERALS


def test_current_entity_rule_reads_a_shifted_status_id():
    spec = FormSpec.from_form_fields({"problem_fields": problem_fields(5)}, module="problem")
    rule = {"id": 9001, "name": "Done locks subject", "module_type": "problem", "state": "active",
            "rule_type": "client_side", "form_type": "edit_form", "user_type": "both", "is_reversible": False,
            "condition_data": {"all": [{"name": "status", "operator": "is", "value": ["23"],
                                        "evaluate_on": "current_entity"}]},
            "action_data": {"action": [{"name": ["subject"], "property": "disable"}]}}
    eng = RulesEngine(spec, {"business_rules": [rule]}, module="problem")
    assert "subject" in eng.evaluate({"status": 21}, "edit", base={"status": 23, "subject": "x"}).disabled
    assert "subject" not in eng.evaluate({"status": 23}, "edit", base={"status": 3, "subject": "x"}).disabled
