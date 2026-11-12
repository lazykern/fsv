"""Golden `problem_status` (Python 5d7d777) for fsv-api `problem_status`:
the status PUT, plan hashes, reviews, closure preflight, server closure-rule
rejections, readback and journal events. Mirrors `test_problem_status.py`
(the non-CLI tests, for both status-id bases); metadata loads through the
real `load_meta` from routes. The Rust test replays the ops.

    cd /tmp/fsv-py-5d7d777 && FSV_PY_SRC=$PWD FSV_PY_COMMIT=5d7d777... uv run python \
        /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_problem_status.py
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

import httpx  # noqa: E402

from fsv import config, formmeta, journal, planning_text, problem_status as flow  # noqa: E402
from fsv.client import Client  # noqa: E402
from provenance import stamped  # noqa: E402

formmeta._save = lambda *a, **k: None
formmeta.configured_workspace = lambda: 17
LABELS = ["Open", "In Progress", "Resolve", "Closed"]


def problem_fields(n):
    return [{"id": 501, "name": "subject", "label": "Subject", "field_type": "default_subject", "position": 1,
             "required_for_closure": True},
            {"id": 502, "name": "status", "label": "Status", "field_type": "default_status", "position": 2,
             "choices": [{"id": n + i, "value": label} for i, label in enumerate(LABELS)]}]


def new_state(n):
    return {"row": {"id": 123, "subject": "Acme problem", "status": n, "planned_start_date": None, "planned_end_date": None},
            "analysis": [{"id": i + 1, "name": key, "description": "<div>Acme evidence</div>"}
                         for i, key in enumerate(planning_text.PROBLEM_SECTIONS.values())],
            "solutions": [{"id": 701, "title": "Acme solution"}], "fields": problem_fields(n), "rules": [],
            "fail": None, "mismatch": False, "partial": False, "drop_dates": False, "requests": []}


def handler_for(s):
    def handle(request):
        path = request.url.path.removeprefix("/api/_/")
        if request.method == "GET":
            if path == "problem_form_fields":
                return httpx.Response(200, json={"problem_fields": copy.deepcopy(s["fields"])})
            if path.endswith("business_rules"):
                return httpx.Response(200, json={"business_rules": copy.deepcopy(s["rules"])})
            if path.endswith("planning-fields"):
                return httpx.Response(200, json={"problem_planning_fields": copy.deepcopy(s["analysis"])})
            if path.endswith("solutions"):
                return httpx.Response(200, json={"solutions": copy.deepcopy(s["solutions"]), "has_more": s["partial"]})
            if path == "problems/123":
                return httpx.Response(200, json={"problem": copy.deepcopy(s["row"])})
            return httpx.Response(404, json={"error": "unexpected"})
        s["requests"].append({"method": request.method, "path": path, "raw": request.content.decode()})
        body = json.loads(request.content)
        if s["fail"] == "disconnect":
            raise httpx.ReadError("disconnect", request=request)
        if isinstance(s["fail"], dict):
            return httpx.Response(s["fail"]["status"], json=s["fail"]["body"])
        if s["fail"]:
            return httpx.Response(s["fail"], json={"error": "failed"})
        if not s["mismatch"]:
            s["row"].update(body)
        s["row"].update(planned_start_date="2030-01-02T10:00:00Z", planned_end_date="2030-01-02T11:00:00Z")
        if s["drop_dates"]:
            s["row"].pop("planned_end_date")
        return httpx.Response(200, json={"problem": {"id": 123}})
    return handle


def client(handler):
    c = Client.__new__(Client)
    c._client = httpx.Client(transport=httpx.MockTransport(handler), follow_redirects=False)
    c._csrf = "synthetic"
    c._fw_domain = c._fw_session_id = None
    c._persist = False
    c._rl_rem = c._rl_tot = None
    c._probe_ok_until = 0
    c._probe_session = lambda: True
    return c


def run(case):
    os.environ["XDG_STATE_HOME"] = tempfile.mkdtemp(prefix="fsv-state-")
    config.DOMAIN = "acme.example.com"
    config.API_INT = f"https://{config.DOMAIN}/api/_"
    s = new_state(case["n"])
    c = client(handler_for(s))
    plan = None
    outputs = []
    for op in case["ops"]:
        try:
            if op["op"] == "build":
                plan = flow.build_plan(c, op.get("target", "PRB-123"), op["to"])
                out = {"plan": dict(plan.__dict__), "hash": plan.hash, "review": plan.review()}
            elif op["op"] == "send":
                reviewed = {"full": plan.hash, "none": None, "prefix": plan.hash[:16]}[op.get("hash", "full")]
                out = {"result": flow.send(plan, c, reviewed_hash=reviewed)}
            elif op["op"] == "set":
                for k, v in copy.deepcopy(op.get("state", {})).items():
                    s[k] = v
                for k, v in op.get("row", {}).items():
                    s["row"][k] = v
                for k, v in op.get("analysis0", {}).items():
                    s["analysis"][0][k] = v
                out = {}
            else:
                raise AssertionError(op)
        except Exception as exc:  # noqa: BLE001
            out = {"error": {"type": type(exc).__name__, "msg": str(exc)}}
        out["writes"] = len(s["requests"])
        outputs.append(out)
    c.close()
    events = [r["event"] for r in journal.read() if r.get("kind") == "plan:problem_status"]
    return {**case, "outputs": outputs, "events": events, "writes": s["requests"]}


S = {"op": "send"}
CLOSURE = {"status": 400, "body": {"errors": [{"code": "violated_closure_rules", "description": {
    "check_analysis_completed": "Analysis required", "check_workaround_or_solution_added": "Solution required"}}]}}
CASES = {}
for n in (41, 81):
    for label in LABELS:
        CASES[f"transition_{label}_{n}"] = {"n": n, "ops": [{"op": "build", "to": label}, S, S]}
    CASES[f"closure_missing_{n}"] = {"n": n, "ops": [
        {"op": "set", "state": {"analysis": [], "solutions": []}}, {"op": "build", "target": 123, "to": "Closed"}]}
    CASES[f"closure_drift_required_{n}"] = {"n": n, "ops": [
        {"op": "build", "target": 123, "to": "Closed"}, {"op": "set", "analysis0": {"description": "<div> </div>"}}, S,
        {"op": "set", "analysis0": {"description": "Acme"}, "row": {"subject": ""}}, {"op": "build", "target": 123, "to": "Closed"}]}
    for failure in [403, 429, 500, "disconnect"]:
        CASES[f"ambiguous_{failure}_{n}"] = {"n": n, "ops": [
            {"op": "build", "target": 123, "to": "Resolve"}, {"op": "set", "state": {"fail": failure}}, S, S]}
    CASES[f"server_closure_{n}"] = {"n": n, "ops": [
        {"op": "build", "target": 123, "to": "Closed"}, {"op": "set", "state": {"fail": CLOSURE}}, S]}
    CASES[f"mismatch_{n}"] = {"n": n, "ops": [
        {"op": "build", "to": "Resolve"}, {"op": "set", "state": {"mismatch": True}}, S]}
REJECTIONS = [
    {"status": 400, "body": {"errors": [{"code": "violated_closure_rules", "description": {"check_analysis_completed": 1}}]}},
    {"status": 400, "body": {"errors": [{"code": "violated_closure_rules", "description": "x"}]}},
    {"status": 422, "body": {"errors": [{"code": "other"}, "x"]}},
    {"status": 400, "body": {"errors": "x"}},
    {"status": 409, "body": ["x"]},
]
for i, fail in enumerate(REJECTIONS):
    CASES[f"rejection_{i}"] = {"n": 41, "ops": [
        {"op": "build", "to": "Closed"}, {"op": "set", "state": {"fail": fail}}, S]}
CASES["refusals"] = {"n": 41, "ops": [
    {"op": "build", "to": "Done"}, {"op": "build", "to": " in progress "}, {"op": "build", "to": "42"},
    {"op": "build", "target": "0", "to": "Open"}, {"op": "build", "target": "INC-1", "to": "Open"},
    {"op": "set", "state": {"partial": True}}, {"op": "build", "to": "Closed"},
    {"op": "set", "state": {"partial": False, "solutions": [{"id": 1}, {"id": "1"}]}}, {"op": "build", "to": "Closed"},
    {"op": "set", "state": {"solutions": [{"id": 1}], "analysis": [{"id": 1, "name": "problem_cause", "description": "a"},
                                                                  {"id": 2, "name": "problem_cause", "description": "b"}]}},
    {"op": "build", "to": "Closed"},
    {"op": "set", "state": {"analysis": [{"id": 1, "name": "problem_cause", "description": "&nbsp;"},
                                         {"id": 2, "name": "problem_impact", "description_text": "Impact text"},
                                         {"id": 3, "name": "problem_symptom", "description": None}]}},
    {"op": "build", "to": "Closed"},
    {"op": "set", "state": {"analysis": [{"id": i + 1, "name": k, "description": "x"} for i, k in
                                         enumerate(planning_text.PROBLEM_SECTIONS.values())]},
     "row": {"subject": None}}, {"op": "build", "to": "Closed"},
    {"op": "set", "row": {"subject": []}}, {"op": "build", "to": "Closed"},
    {"op": "set", "row": {"subject": 0}}, {"op": "build", "to": "Closed"},
    {"op": "set", "row": {"subject": "Acme"}},
    {"op": "set", "state": {"row": {"id": 123, "subject": "Acme"}}}, {"op": "build", "to": "Open"},
    {"op": "set", "state": {"row": {"id": 124, "status": 41}}}, {"op": "build", "to": "Open"},
    {"op": "set", "state": {"row": {"id": 123, "status": 41, "subject": "Acme"},
                            "rules": [{"id": 1}], "fields": problem_fields(41)}}, {"op": "build", "to": "Open"},
    {"op": "set", "state": {"rules": [], "fields": problem_fields(41) + [
        {"id": 503, "name": "cf_reason", "label": "Reason", "field_type": "custom_text", "position": 3,
         "required_for_closure": True}]}},
    {"op": "build", "to": "Closed"},
    {"op": "set", "row": {"custom_fields": {"cf_reason": "Because"}}}, {"op": "build", "to": "Closed"},
    {"op": "build", "to": "Open"}, {**S, "hash": "none"}, {**S, "hash": "prefix"}]}
CASES["readback_dates"] = {"n": 41, "ops": [
    {"op": "build", "to": "Resolve"}, {"op": "set", "state": {"drop_dates": True}}, S]}
json.dump(stamped({name: run(case) for name, case in CASES.items()}), sys.stdout, indent=1, ensure_ascii=False)
