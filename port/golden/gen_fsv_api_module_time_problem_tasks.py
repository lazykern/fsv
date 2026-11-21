"""Golden change/problem time-entry and problem-task plans, readbacks and refusals.

    mkdir -p /tmp/fsv-py-51cd544
    git -C /Users/phusit.somb/lab/work/fsv archive 51cd544 | tar -x -C /tmp/fsv-py-51cd544
    cd /tmp/fsv-py-51cd544 && uv sync --offline
    FSV_PY_SRC=/tmp/fsv-py-51cd544 FSV_PY_COMMIT=$(git -C /Users/phusit.somb/lab/work/fsv rev-parse 51cd544) \\
        uv run --offline --project /tmp/fsv-py-51cd544 \\
        python /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_module_time_problem_tasks.py
"""
from __future__ import annotations

import copy
import json
import os
import sys
import tempfile
from pathlib import Path

BOOT = tempfile.mkdtemp(prefix="fsv-mtp-")
CFG = Path(BOOT) / "config"
CFG.mkdir()
(CFG / "config.json").write_text(json.dumps({"domain": "acme.freshservice.com"}))
os.environ["FSV_CONFIG_DIR"] = str(CFG)
os.environ["FSV_CACHE_DIR"] = str(Path(BOOT) / "cache")
os.environ["XDG_STATE_HOME"] = str(Path(BOOT) / "state")
os.environ["FSV_NO_KEYCHAIN"] = "1"

import httpx  # noqa: E402

from fsv import config, journal, module_time  # noqa: E402
from fsv import problem_task_update as tasks  # noqa: E402
from fsv.client import Client  # noqa: E402
from fsv.reviewed_write import ReviewedWriteError  # noqa: E402
from provenance import stamped  # noqa: E402

config.reload()


def fields():
    return [
        {"name": "status", "choices": [{"id": 1, "value": "Open"}, {"id": 2, "value": "In Progress"},
                                       {"id": 3, "value": "Completed"}]},
        {"name": "notify_before", "choices": [{"id": 0, "value": "Never"}, {"id": 900, "value": "15 Minutes"}]},
    ]


def world():
    journal.journal_path = lambda p=Path(tempfile.mkdtemp(dir=BOOT)) / "journal": p
    return {
        "entries": [],
        "task": {"id": 71, "title": "Acme task", "status": 1, "taskable_display_id": 123,
                 "taskable_type": "Itil::Problem", "owner_id": 901, "group_id": 81, "notify_before": 0,
                 "planned_start_date": "2026-01-02T09:00:00+00:00",
                 "planned_end_date": "2026-01-02T10:00:00+00:00", "custom_fields": {}},
        "writes": [], "omit": None, "fields": fields(), "deleted": False,
    }


def client(s):
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
                return httpx.Response(200, json={"problem_task_fields": s["fields"]})
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
        s["writes"].append([req.method, suffix, body])
        if "/time_entries" in suffix:
            if req.method == "POST":
                s["entries"].append({"id": 456, **body})
            elif req.method == "PUT":
                s["entries"][0].update(body)
            else:
                s["entries"] = []
            if req.method == "DELETE":
                return httpx.Response(204)
            return httpx.Response(200, json={"time_entry": {"id": 456}})
        if suffix.endswith("/tasks/update"):
            s["deleted"] = True
            return httpx.Response(200, content=b"", headers={"content-type": "application/json"})
        s["task"].update(body)
        if body == {"status": 3}:
            s["task"].update(planned_start_date="2026-01-01T09:00:00Z", planned_end_date="2026-01-01T09:15:00Z",
                             closed_at="2026-01-01T09:15:00Z")
        return httpx.Response(200, json={"task": {"id": 71}})

    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handler))
    c._csrf = "synthetic"
    return c


def snap(plan):
    return {"hash": plan.hash, "action": plan.action, "method": plan.method, "path": plan.path,
            "payload": plan.payload_json, "tenant": plan.tenant, "target": plan.target,
            "context": plan.context_hash}


def keep(result):
    return {k: v for k, v in result.items() if k in ("entry_id", "task_id", "fields", "verified",
                                                      "fields_verified", "warning", "readback")}


def time_create(c, module):
    return module_time.build_plan(c, module, 123, action="create", spent="00:01",
                                  executed_at="2026-01-01T09:00:00Z", note="Acme work", billable=True)


def time_flow(module):
    s = world()
    c = client(s)
    steps = []
    p = time_create(c, module)
    steps.append({"plan": snap(p), "result": keep(module_time.send(p, c, reviewed_hash=p.hash))})
    if module == "changes":
        q = module_time.build_plan(c, module, 123, action="update", entry_id=456, spent="00:02",
                                   note="Updated", billable=False)
        steps.append({"plan": snap(q), "result": keep(module_time.send(q, c, reviewed_hash=q.hash))})
    d = module_time.build_plan(c, module, 123, action="delete", entry_id=456)
    steps.append({"plan": snap(d), "result": keep(module_time.send(d, c, reviewed_hash=d.hash))})
    return {"steps": steps, "writes": s["writes"]}


FULL = dict(title="Acme updated", note="Acme task note", planned_effort="1h30m", due_date="2026-01-03T09:00:00Z",
            planned_start="2026-01-02T09:00:00Z", planned_end="2026-01-02T10:30:00Z",
            owner="901", group="81", status="In Progress", notify_before="15 Minutes")


def task_flow(kind, omit=None):
    s = world()
    c = client(s)
    if kind == "delete":
        p = tasks.delete_plan(c, 123, 71)
        result = tasks.delete(p, c, reviewed_hash=p.hash)
        return {"plan": snap(p), "result": keep(result), "writes": s["writes"]}
    inputs = FULL if kind == "full" else {"status": "Completed"} if kind == "complete" else {"notify_before": "15 Minutes"}
    p = tasks.build_plan(c, "PRB-123", 71, inputs)
    s["omit"] = omit
    result = tasks.send(p, c, reviewed_hash=p.hash)
    return {"plan": snap(p), "inputs": p.inputs_json, "result": keep(result), "writes": s["writes"]}


def time_missing():
    s = world()
    c = client(s)
    p = time_create(c, "changes")
    s["omit"] = "time_spent"
    return {"plan": snap(p), "result": keep(module_time.send(p, c, reviewed_hash=p.hash)), "writes": s["writes"]}


def refusal(setup, call):
    s = world()
    setup(s)
    try:
        call(client(s))
    except (ReviewedWriteError, ValueError) as exc:
        return {"kind": type(exc).__name__, "message": str(exc)}
    return None


def entry(**change):
    return lambda s: s.__setitem__("entries", [{"id": 456, "agent_id": 901, "timer_running": False, "billable": True,
                                                "time_spent": "00:01", **change}])


def closure(s):
    s["fields"].append({"name": "cf_answer", "label": "Closure answer",
                        "field_options": {"required_for_closure": "true"}})


def main():
    doc = {
        "time-changes": time_flow("changes"),
        "time-problems": time_flow("problems"),
        "time-missing": time_missing(),
        "task-full": task_flow("full"),
        "task-complete": task_flow("complete"),
        "task-delete": task_flow("delete"),
        "task-missing": task_flow("reminder", omit="notify_before"),
    }
    none = lambda s: None  # noqa: E731
    doc["refusals"] = {
        "other_owner": refusal(entry(agent_id=902), lambda c: module_time.build_plan(
            c, "changes", 123, action="delete", entry_id=456)),
        "running": refusal(entry(timer_running=True), lambda c: module_time.build_plan(
            c, "changes", 123, action="delete", entry_id=456)),
        "problem_update": refusal(entry(), lambda c: module_time.build_plan(
            c, "problems", 123, action="update", entry_id=456, note="No wire")),
        "bad_spent": refusal(none, lambda c: module_time.build_plan(
            c, "changes", 123, action="create", spent="00:00", executed_at="2026-01-01T09:00:00Z")),
        "naive_when": refusal(none, lambda c: module_time.build_plan(
            c, "changes", 123, action="create", spent="00:01", executed_at="2026-01-01T09:00:00")),
        "delete_billable": refusal(entry(), lambda c: module_time.build_plan(
            c, "changes", 123, action="delete", entry_id=456, billable=False)),
        "combined_completion": refusal(none, lambda c: tasks.build_plan(
            c, 123, 71, {"title": "Acme", "status": "Completed"})),
        "closure_missing": refusal(closure, lambda c: tasks.build_plan(c, 123, 71, {"status": "Completed"})),
        "null_owner": refusal(none, lambda c: tasks.build_plan(c, 123, 71, {"owner": "none"})),
        "unsupported": refusal(none, lambda c: tasks.build_plan(c, 123, 71, {"values": {}})),
        "end_before_start": refusal(none, lambda c: tasks.build_plan(
            c, 123, 71, {"planned_end": "2026-01-02T08:00:00Z"})),
        "delete_missing": refusal(none, lambda c: tasks.delete_plan(c, 123, 72)),
    }
    json.dump(stamped(doc), sys.stdout, indent=2)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
