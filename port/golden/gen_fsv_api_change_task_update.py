"""Golden change-task updates (Python b310885) for fsv-api `change_task_update`.

    cd /tmp/fsv-py-be41032 && FSV_PY_SRC=$PWD FSV_PY_COMMIT=be41032... uv run --offline python \
        /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_change_task_update.py \
        > crates/fsv-api/src/testdata/change_task_update_oracle.json
"""
from __future__ import annotations

import copy
import json
import os
import sys
import tempfile

BOOT = tempfile.mkdtemp(prefix="fsv-gen-")
os.environ["FSV_CONFIG_DIR"] = os.path.join(BOOT, "config")
os.environ["FSV_CACHE_DIR"] = os.path.join(BOOT, "cache")
os.environ["FSV_NO_KEYCHAIN"] = "1"

from fsv import config, change_task_update as flow  # noqa: E402
from provenance import stamped  # noqa: E402

config.DOMAIN = "acme.example.com"

TASK = {"id": 17, "title": "Acme task", "owner_id": 81, "group_id": 91, "status": 1,
        "planned_start_date": "2026-12-01T10:00:00Z", "planned_end_date": "2026-12-01T11:00:00Z"}
FIELDS = [{"name": "status", "choices": [{"id": "1", "value": "Open"}, {"id": "7", "value": "In Progress"}]},
          {"name": "notify_before", "choices": [{"id": "0", "value": "Never"}, {"id": "900", "value": "15 Minutes"}]}]
CLOSURE = [{"name": "status", "choices": [{"id": "1", "value": "Open"}, {"id": "3", "value": "Completed"}]},
           {"name": "cf_evidence", "label": "Evidence", "required_for_closure": True,
            "custom": True, "default": False, "field_type": "custom_text"}]
CUSTOM = FIELDS + [{"name": "cf_system", "label": "System", "custom": True, "default": False,
                    "field_type": "custom_text"}]


class Client:
    def __init__(self):
        self.task = copy.deepcopy(TASK)
        self.fields = copy.deepcopy(FIELDS)
        self.persist = True
        self.after = None
        self.writes = []

    def int_get(self, path, params=None):
        if path == "change_task_fields":
            return {"change_task_fields": copy.deepcopy(self.fields)}
        return {"tasks": [copy.deepcopy(self.task)], "meta": {"has_next": False}}

    def _request(self, method, path, **kw):
        self.writes.append((method, path))
        body = json.loads(kw["content"])
        if self.persist:
            self.task.update(body)
            if "owner_id" in body:
                self.task["agent_id"] = self.task.pop("owner_id")
            if "description" in body:
                self.task["description"] = "<div style='font:inherit'>" + body["description"] + "</div>"
            if "planned_start_date" in body:
                self.task["planned_start_date"] = "2026-12-01T10:15:00Z"
        if self.after:
            self.task["planned_effort"] = "2h" if self.after == "effort-mismatch" else "1h"
            self.task.pop("notify_before", None)
            if self.after == "reminder-mismatch":
                self.task["notify_before"] = 0
            if self.after == "title-missing":
                self.task.pop("title", None)
        return {"task": body}

    def _get_csrf(self):
        return "synthetic"


def plan_out(plan):
    return {"action": plan.action, "target": plan.target, "tenant": plan.tenant, "method": plan.method,
            "path": plan.path, "payload": plan.payload, "context_hash": plan.context_hash,
            "hash": plan.hash, "review": plan.review(), "task_id": plan.task_id,
            "inputs": json.loads(plan.inputs_json)}


def run(ops):
    os.environ["XDG_STATE_HOME"] = tempfile.mkdtemp(prefix="fsv-state-")
    c = Client()
    plan = None
    outputs = []
    for op in ops:
        try:
            kind = op["op"]
            if kind == "set":
                if "persist" in op:
                    c.persist = op["persist"]
                if "task_title" in op:
                    c.task["title"] = op["task_title"]
                if "fields" in op:
                    c.fields = op["fields"]
                if "after" in op:
                    c.after = op["after"]
                out = {}
            elif kind == "build":
                plan = flow.build_plan(c, op.get("target", 23), op.get("task", 17), op["inputs"])
                out = {"plan": plan_out(plan)}
            elif kind == "send":
                reviewed = plan.hash if op.get("hash", "full") == "full" else plan.hash[:12]
                out = {"result": flow.send(plan, c, reviewed_hash=reviewed)}
            else:
                raise AssertionError(kind)
        except Exception as exc:  # noqa: BLE001
            out = {"error": {"type": type(exc).__name__, "msg": str(exc)}}
        out["writes"] = len(c.writes)
        outputs.append(out)
    return outputs


FULL = {"title": "Acme revised", "status": "In Progress", "owner": "82", "group": "92",
        "note": "Review evidence", "planned_start": "2026-12-01T05:15:00-05:00",
        "planned_end": "2026-12-01T06:15:00-05:00", "planned_effort": "01:30",
        "notify_before": "15 Minutes"}

CASES = {
    "full": [{"op": "build", "target": "CHN-23", "task": 17, "inputs": FULL}, {"op": "send"}],
    "echo": [{"op": "build", "inputs": {"title": "Acme revised"}}, {"op": "set", "persist": False},
             {"op": "send"}, {"op": "send"}],
    "validation": [{"op": "build", "inputs": {"notify_before": "15"}},
                   {"op": "build", "inputs": {"planned_end": "2026-12-01T09:00:00Z"}},
                   {"op": "build", "inputs": {"title": "Acme revised"}},
                   {"op": "set", "task_title": "Concurrent update"}, {"op": "send"}],
    "clear": [{"op": "build", "inputs": {"owner": "none", "group": "none"}}, {"op": "send"}],
    "custom": [{"op": "set", "fields": CUSTOM},
               {"op": "build", "inputs": {"values": {"System": "SAP"}}}, {"op": "send"}],
    "closure": [{"op": "set", "fields": CLOSURE},
                {"op": "build", "inputs": {"status": "Completed"}},
                {"op": "build", "inputs": {"status": "Completed", "values": {"Evidence": "photo"}}}],
    "unsupported": [{"op": "build", "inputs": {"due_date": "2026-01-01"}}],
    "empty": [{"op": "build", "inputs": {}}],
    "missing_task": [{"op": "build", "task": 99, "inputs": {"title": "Acme revised"}}],
    "bad_title": [{"op": "build", "inputs": {"title": "  "}},
                  {"op": "build", "inputs": {"title": "Acme\nreview"}}],
    "normalized": [{"op": "set", "after": "normalized"},
                   {"op": "build", "inputs": {"title": "Acme revised", "planned_effort": "1h", "notify_before": "15 Minutes"}},
                   {"op": "send"}],
    "effort_mismatch": [{"op": "set", "after": "effort-mismatch"},
                        {"op": "build", "inputs": {"title": "Acme revised", "planned_effort": "1h", "notify_before": "15 Minutes"}},
                        {"op": "send"}],
    "reminder_mismatch": [{"op": "set", "after": "reminder-mismatch"},
                          {"op": "build", "inputs": {"title": "Acme revised", "planned_effort": "1h", "notify_before": "15 Minutes"}},
                          {"op": "send"}],
    "title_missing": [{"op": "set", "after": "title-missing"},
                      {"op": "build", "inputs": {"title": "Acme revised", "planned_effort": "1h", "notify_before": "15 Minutes"}},
                      {"op": "send"}],
}

json.dump(stamped({"cases": {name: {"ops": ops, "outputs": run(ops)} for name, ops in CASES.items()}}),
          sys.stdout, indent=1, ensure_ascii=False)
print()
