import copy
import json
from types import SimpleNamespace
import pytest
from fsv import config, change_task_update as a
from fsv.reviewed_write import ReviewedWriteError


@pytest.fixture
def wire(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setenv("FSV_CACHE_DIR", str(tmp_path))
    task = {"id": 17, "title": "Acme task", "owner_id": 81, "group_id": 91, "status": 1,
            "planned_start_date": "2026-12-01T10:00:00Z", "planned_end_date": "2026-12-01T11:00:00Z"}
    fields = [{"name": "status", "choices": [{"id": "1", "value": "Open"}, {"id": "7", "value": "In Progress"}]},
              {"name": "notify_before", "choices": [{"id": "0", "value": "Never"}, {"id": "900", "value": "15 Minutes"}]}]
    state = {"task": task, "fields": fields, "writes": [], "persist": True}
    def get(path, params=None):
        if path == "change_task_fields":
            return {"change_task_fields": copy.deepcopy(fields)}
        return {"tasks": [copy.deepcopy(task)], "meta": {"has_next": False}}
    def request(method, path, **kw):
        state["writes"].append((method, path, kw))
        body = json.loads(kw["content"])
        if state["persist"]:
            task.update(body)
            if "owner_id" in body:
                task["agent_id"] = task.pop("owner_id")
            if "description" in body:
                task["description"] = "<div style='font:inherit'>" + body["description"] + "</div>"
            if "planned_start_date" in body:
                task["planned_start_date"] = "2026-12-01T10:15:00Z"
        return {"task": body}
    return SimpleNamespace(int_get=get, _request=request, _get_csrf=lambda: "synthetic", state=state)


def test_full_put_and_independent_normalization(wire):
    inputs = dict(title="Acme revised", status="In Progress", owner="82", group="92", note="Review evidence",
                  planned_start="2026-12-01T05:15:00-05:00", planned_end="2026-12-01T06:15:00-05:00",
                  planned_effort="01:30", notify_before="15 Minutes")
    plan = a.build_plan(wire, "CHN-23", 17, inputs)
    assert plan.payload["status"] == 7
    assert plan.payload["notify_before"] == 900
    assert plan.payload["planned_effort"] == "1h 30m"
    result = a.send(plan, wire, reviewed_hash=plan.hash)
    assert result["verified"], result
    assert len(wire.state["writes"]) == 1
    assert wire.state["writes"][0][1].endswith("changes/23/tasks/17")


@pytest.mark.parametrize("mode", ["normalized", "effort-mismatch", "reminder-mismatch", "title-missing"])
def test_collection_normalizes_effort_and_omits_reminder(wire, mode):
    original = wire._request
    def request(*args, **kwargs):
        result = original(*args, **kwargs)
        wire.state["task"]["planned_effort"] = "2h" if mode == "effort-mismatch" else "1h"
        wire.state["task"].pop("notify_before", None)
        if mode == "reminder-mismatch":
            wire.state["task"]["notify_before"] = 0
        if mode == "title-missing":
            wire.state["task"].pop("title")
        return result
    wire._request = request
    plan = a.build_plan(wire, 23, 17, dict(title="Acme revised", planned_effort="1h", notify_before="15 Minutes"))
    result = a.send(plan, wire, reviewed_hash=plan.hash)
    assert result["verified"] is (mode == "normalized")
    assert not result["fields_verified"]
    assert result["fields"]["notify_before"] == ("mismatched" if mode == "reminder-mismatch" else "unavailable")
    assert result["fields"]["planned_effort"] == ("mismatched" if mode == "effort-mismatch" else "matched")
    assert len(wire.state["writes"]) == 1


def test_echo_not_readback_and_no_replay(wire):
    plan = a.build_plan(wire, 23, 17, {"title": "Acme revised"})
    wire.state["persist"] = False
    assert not a.send(plan, wire, reviewed_hash=plan.hash)["verified"]
    with pytest.raises(ReviewedWriteError, match="already sent at .* answered; nothing was sent now"):
        a.send(plan, wire, reviewed_hash=plan.hash)
    assert len(wire.state["writes"]) == 1


def test_validation_and_stale_task(wire):
    with pytest.raises(ValueError, match="runtime reminder"):
        a.build_plan(wire, 23, 17, {"notify_before": "15"})
    with pytest.raises(ValueError, match="precede"):
        a.build_plan(wire, 23, 17, {"planned_end": "2026-12-01T09:00:00Z"})
    plan = a.build_plan(wire, 23, 17, {"title": "Acme revised"})
    wire.state["task"]["title"] = "Concurrent update"
    with pytest.raises(ReviewedWriteError, match="changed since review"):
        a.send(plan, wire, reviewed_hash=plan.hash)
    assert not wire.state["writes"]
