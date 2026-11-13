"""Captured module time and problem-task wires with independent native/v2 GETs."""
import copy
import json

import httpx
import pytest
from typer.testing import CliRunner

from fsv import cli, config, journal, module_time, problem_task_update as tasks
from fsv.client import Client
from fsv.reviewed_write import ReviewedWriteError


@pytest.fixture
def api(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(config, "API_INT", "https://acme.example.com/api/_")
    monkeypatch.setattr(config, "API_V2", "https://acme.example.com/api/v2")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    fields = [{"name": "status", "choices": [{"id": 1, "value": "Open"}, {"id": 2, "value": "In Progress"}, {"id": 3, "value": "Completed"}]},
              {"name": "notify_before", "choices": [{"id": 0, "value": "Never"}, {"id": 900, "value": "15 Minutes"}]}]
    s = {"entries": [], "task": {"id": 71, "title": "Acme task", "status": 1, "taskable_display_id": 123,
         "taskable_type": "Itil::Problem", "owner_id": 901, "group_id": 81, "notify_before": 0,
         "planned_start_date": "2026-01-02T09:00:00+00:00", "planned_end_date": "2026-01-02T10:00:00+00:00", "custom_fields": {}},
         "writes": [], "failure": False, "omit": None, "fields": fields, "deleted": False}
    def handler(req):
        path = req.url.path
        suffix = path.split("/", 3)[-1]
        if req.method == "GET":
            if suffix == "agents/me":
                return httpx.Response(200, json={"agent": {"id": 901, "email": "agent@example.com"}})
            if suffix in {"changes/123", "problems/123"}:
                return httpx.Response(200, json={suffix.split("/")[0][:-1]: {"id": 123, "subject": "Acme work"}})
            if suffix.endswith("/time_entries"):
                assert path.startswith("/api/v2/")
                rows = copy.deepcopy(s["entries"])
                for r in rows:
                    r["time_spent"] += ":00" if len(r["time_spent"]) == 5 else ""
                    if s["omit"] and s["writes"]:
                        r.pop(s["omit"], None)
                return httpx.Response(200, json={"time_entries": rows, "has_more": False})
            if suffix == "problem_task_fields":
                return httpx.Response(200, json={"problem_task_fields": fields})
            if suffix == "problems/123/tasks":
                row = {**copy.deepcopy(s["task"]), "agent_id": s["task"]["owner_id"]}
                row.pop("owner_id")
                row.pop("notify_before", None)
                return httpx.Response(200, json={"tasks": [] if s["deleted"] else [row], "has_more": False})
            if suffix == "problems/123/tasks/71":
                row = copy.deepcopy(s["task"])
                if s["omit"] and s["writes"]:
                    row.pop(s["omit"], None)
                return httpx.Response(200, json={"task": row})
            raise AssertionError(path)
        body = json.loads(req.content) if req.content else None
        s["writes"].append((req.method, suffix, body))
        if s["failure"]:
            raise httpx.ReadError("disconnect", request=req)
        if "/time_entries" in suffix:
            if req.method == "POST":
                s["entries"].append({"id": 456, **body})
            elif req.method == "PUT":
                s["entries"][0].update(body)
            else:
                s["entries"] = []
            return httpx.Response(204) if req.method == "DELETE" else httpx.Response(200, json={"time_entry": {"id": 456}})
        if suffix.endswith("/tasks/update"):
            assert body == {"id": "71", "action": "delete"}
            s["deleted"] = True
            return httpx.Response(200, content=b"", headers={"content-type": "application/json"})
        s["task"].update(body)
        if body == {"status": 3}:
            s["task"].update(planned_start_date="2026-01-01T09:00:00Z", planned_end_date="2026-01-01T09:15:00Z", closed_at="2026-01-01T09:15:00Z")
        return httpx.Response(200, json={"task": {"id": 71}})
    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handler))
    c._csrf = "synthetic"
    monkeypatch.setattr(cli, "_client", lambda: c)
    yield c, s
    c.close()


def time_create(c, module):
    return module_time.build_plan(c, module, 123, action="create", spent="00:01", executed_at="2026-01-01T09:00:00Z", note="Acme work", billable=True)


@pytest.mark.parametrize("module", ["changes", "problems"])
def test_self_time_crud_exact_wires(api, module):
    c, s = api
    p = time_create(c, module)
    assert p.payload["agent_id"] == 901 and p.payload["billable"] is True
    assert module_time.send(p, c, reviewed_hash=p.hash)["verified"]
    if module == "changes":
        q = module_time.build_plan(c, module, 123, action="update", entry_id=456, spent="00:02", note="Updated", billable=False)
        assert q.payload == {"billable": False, "time_spent": "00:02", "note": "Updated"}
        assert module_time.send(q, c, reviewed_hash=q.hash)["verified"]
    else:
        with pytest.raises(ValueError, match="billable"):
            module_time.build_plan(c, module, 123, action="update", entry_id=456, note="x", billable=False)
        # Captured problem update: only time_spent and note; HH:MM:SS readback.
        q = module_time.build_plan(c, module, 123, action="update", entry_id=456, spent="00:30", note="Acme edited")
        assert (q.method, q.path, q.payload) == ("PUT", "problems/123/time_entries/456",
                                                 {"time_spent": "00:30", "note": "Acme edited"})
        assert module_time.send(q, c, reviewed_hash=q.hash)["verified"]
        assert s["entries"][0]["time_spent"] == "00:30"
    d = module_time.build_plan(c, module, 123, action="delete", entry_id=456)
    assert module_time.send(d, c, reviewed_hash=d.hash)["verified"]
    assert s["writes"][0][1] == f"{module}/123/time_entries"
    assert s["writes"][-1] == ("DELETE", f"{module}/123/time_entries/456", None)


@pytest.mark.parametrize("change", [{"agent_id": 902}, {"timer_running": True}])
def test_other_owner_or_running_time_refused(api, change):
    c, s = api
    s["entries"] = [{"id": 456, "agent_id": 901, "timer_running": False, "billable": True, "time_spent": "00:01", **change}]
    with pytest.raises(ReviewedWriteError, match="self-owned"):
        module_time.build_plan(c, "changes", 123, action="delete", entry_id=456)
    assert not s["writes"]


def test_problem_task_full_update_exact_readback(api):
    c, s = api
    inputs = dict(title="Acme updated", note="Acme task note", planned_effort="1h30m", due_date="2026-01-03T09:00:00Z",
                  planned_start="2026-01-02T09:00:00Z", planned_end="2026-01-02T10:30:00Z",
                  owner="901", group="81", status="In Progress", notify_before="15 Minutes")
    p = tasks.build_plan(c, "PRB-123", 71, inputs)
    assert p.payload["owner_id"] == 901 and p.payload["notify_before"] == 900
    assert p.payload["due_date"] == "2026-01-03T09:00:00+00:00"
    result = tasks.send(p, c, reviewed_hash=p.hash)
    assert result["verified"] and result["fields"]["notify_before"] == "matched"
    assert len(s["writes"]) == 1 and s["writes"][0][2] == p.payload


@pytest.mark.parametrize("action", ["complete", "delete"])
def test_problem_task_completion_side_effect_and_native_delete(api, action):
    c, s = api
    p = tasks.delete_plan(c, 123, 71) if action == "delete" else tasks.build_plan(c, 123, 71, {"status": "Completed"})
    sender = tasks.delete if action == "delete" else tasks.send
    result = sender(p, c, reviewed_hash=p.hash)
    assert result["verified"] and len(s["writes"]) == 1
    if action == "complete":
        assert s["writes"][0][2] == {"status": 3}
        assert result["readback"]["planned_start_date"] == "2026-01-01T09:00:00Z"
        assert "rewrite planned dates" in result["warning"]
    else:
        assert s["writes"][0] == ("PUT", "problems/123/tasks/update", {"id": "71", "action": "delete"})
    with pytest.raises(ReviewedWriteError, match="was sent"):
        sender(p, c, reviewed_hash=p.hash)
    assert len(s["writes"]) == 1


@pytest.mark.parametrize("kind", ["time", "task"])
@pytest.mark.parametrize("mode", ["disconnect", "missing-field"])
def test_unknown_or_unverified_module_write_never_replays(api, kind, mode):
    c, s = api
    p = time_create(c, "changes") if kind == "time" else tasks.build_plan(c, 123, 71, {"notify_before": "15 Minutes"})
    sender = module_time.send if kind == "time" else tasks.send
    if mode == "disconnect":
        s["failure"] = True
        with pytest.raises(ReviewedWriteError, match="unknown"):
            sender(p, c, reviewed_hash=p.hash)
    else:
        s["omit"] = "time_spent" if kind == "time" else "notify_before"
        assert not sender(p, c, reviewed_hash=p.hash)["verified"]
    with pytest.raises(ReviewedWriteError, match="was sent"):
        sender(p, c, reviewed_hash=p.hash)
    assert len(s["writes"]) == 1


def test_task_closure_requirements_and_combined_completion_refused(api):
    c, s = api
    with pytest.raises(ReviewedWriteError, match="separate status-only"):
        tasks.build_plan(c, 123, 71, {"title": "Acme", "status": "Completed"})
    s["fields"].append({"name": "cf_answer", "label": "Closure answer", "field_options": {"required_for_closure": "true"}})
    with pytest.raises(ReviewedWriteError, match="Closure answer"):
        tasks.build_plan(c, 123, 71, {"status": "Completed"})
    assert not s["writes"]


@pytest.mark.parametrize("module", ["changes", "problems"])
@pytest.mark.parametrize("flags,expected", [([], False), (["--billable"], True), (["--non-billable"], False), (["--no-billable"], False)])
def test_time_create_billable_only_when_chosen(api, module, flags, expected):
    c, s = api
    prefix = "CHN" if module == "changes" else "PRB"
    r = CliRunner().invoke(cli.app, [module, "time", "write", f"{prefix}-123", "create", "--spent", "00:01",
                                     "--executed-at", "2026-01-01T09:00:00Z", *flags, "--dry-run", "--json"])
    assert r.exit_code == 0, r.output
    payload = json.loads(r.stdout)["payload"]
    payload = json.loads(payload) if isinstance(payload, str) else payload
    assert payload["billable"] is expected
    p = module_time.build_plan(c, module, 123, action="create", spent="00:01", executed_at="2026-01-01T09:00:00Z")
    assert p.payload["billable"] is False and not s["writes"]


@pytest.mark.parametrize("args", [["changes", "time", "write", "CHN-123", "create", "--spent", "00:01", "--executed-at", "2026-01-01T09:00:00Z", "--billable"],
                                 ["problems", "task-write", "PRB-123", "71", "complete"]])
def test_time_task_cli_review_and_send(api, args):
    c, s = api
    runner = CliRunner()
    r = runner.invoke(cli.app, args + ["--dry-run", "--json"])
    assert r.exit_code == 0, r.output
    p = json.loads(r.stdout)
    sent = runner.invoke(cli.app, args + ["--yes", "--expect-plan", p["plan_hash"], "--json"])
    assert sent.exit_code == 0, sent.output
    assert json.loads(sent.stdout)["verified"] and len(s["writes"]) == 1


def test_problem_time_update_cli(api):
    c, s = api
    p = time_create(c, "problems")
    module_time.send(p, c, reviewed_hash=p.hash)
    from typer.testing import CliRunner
    args = ["problems", "time", "update", "PRB-123", "456", "--spent", "00:45"]
    review = CliRunner().invoke(cli.app, args + ["--dry-run"])
    assert review.exit_code == 0, review.output
    body = json.loads(review.stdout)
    assert body["payload"] == {"time_spent": "00:45", "note": "Acme work"}
    done = CliRunner().invoke(cli.app, args + ["--yes", "--expect-plan", body["plan_hash"]])
    assert done.exit_code == 0 and json.loads(done.stdout)["verified"]
    assert s["writes"][-1] == ("PUT", "problems/123/time_entries/456", {"time_spent": "00:45", "note": "Acme work"})
