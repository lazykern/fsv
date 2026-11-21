"""Golden `problem_incidents` (Python 17ada1b) for fsv-api
`problem_incidents`: the link/unlink wire, plan hashes, inventory readback,
refusals and journal events. Mirrors `test_problem_incidents.py` (the
non-CLI tests); the Rust test replays the ops.

    cd /tmp/fsv-py-17ada1b && FSV_PY_SRC=$PWD FSV_PY_COMMIT=17ada1b... uv run python \
        /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_problem_incidents.py
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

from fsv import config, journal, problem_incidents as flow  # noqa: E402
from fsv.client import Client  # noqa: E402
from provenance import stamped  # noqa: E402


def new_state():
    return {"linked": False, "fail": None, "partial": False, "partial_after": False, "mismatch": False,
            "type": "Incident", "display": "INC-123", "problem": {"problem": {"id": 7, "subject": "Acme problem"}},
            "ticket_id": 123, "rows": None, "requests": []}


def handler_for(s):
    def handle(request):
        path = request.url.path.removeprefix("/api/_/")
        if request.method == "GET":
            if path == "problems/7":
                return httpx.Response(200, json=s["problem"])
            if path == "tickets/123":
                t = {"id": s["ticket_id"], "type": s["type"]}
                if s["display"] is not None:
                    t["human_display_id"] = s["display"]
                return httpx.Response(200, json={"ticket": t})
            if path != "problems/7/tickets":
                return httpx.Response(404, json={"error": "unexpected"})
            if s["partial"]:
                return httpx.Response(403, json={"error": "denied"})
            rows = s["rows"] if s["rows"] is not None else (
                [{"id": 123, "human_display_id": "INC-123", "subject": "Acme incident"}] if s["linked"] else [])
            return httpx.Response(200, json={"tickets": copy.deepcopy(rows), "meta": {"count": len(rows)}})
        s["requests"].append({"method": request.method, "path": path, "raw": request.content.decode()})
        if s["fail"] == "disconnect":
            raise httpx.ReadError("disconnect", request=request)
        if s["fail"]:
            return httpx.Response(s["fail"], json={"error": "failure"})
        if s["partial_after"]:
            s["partial"] = True
        if not s["mismatch"]:
            s["linked"] = request.method == "PUT"
        return httpx.Response(200, json={"tickets": []}) if request.method == "PUT" else httpx.Response(204)
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
    config.DOMAIN = case.get("tenant", "acme.example.com")
    config.API_INT = f"https://{config.DOMAIN}/api/_"
    s = new_state()
    c = client(handler_for(s))
    plan = None
    outputs = []
    for op in case["ops"]:
        try:
            if op["op"] == "build":
                plan = flow.build_plan(c, op.get("problem", "PRB-7"), op.get("incident", "INC-123"),
                                       unlink=op.get("unlink", False))
                out = {"plan": dict(plan.__dict__), "hash": plan.hash, "review": plan.review()}
            elif op["op"] == "send":
                reviewed = {"full": plan.hash, "none": None, "prefix": plan.hash[:16]}[op.get("hash", "full")]
                out = {"result": flow.send(plan, c, op.get("incident", "INC-123"), reviewed_hash=reviewed)}
            elif op["op"] == "set":
                s.update(op["state"])
                out = {}
            else:
                raise AssertionError(op)
        except Exception as exc:  # noqa: BLE001
            out = {"error": {"type": type(exc).__name__, "msg": str(exc)}}
        out["writes"] = len(s["requests"])
        outputs.append(out)
    c.close()
    events = [[r["kind"], r["event"]] for r in journal.read() if str(r.get("kind", "")).startswith("plan:problem_")]
    return {**case, "outputs": outputs, "events": events, "writes": s["requests"]}


B = {"op": "build"}
S = {"op": "send"}
CASES = {}
for tenant in ("acme.example.com", "globex.example.com"):
    t = tenant.split(".")[0]
    CASES[f"link_{t}"] = {"tenant": tenant, "ops": [B, S, {**S, "incident": 123}]}
    CASES[f"unlink_{t}"] = {"tenant": tenant, "ops": [
        {"op": "set", "state": {"linked": True}}, {**B, "unlink": True}, S, {**S, "incident": 123}]}
    for failure in [403, 429, 500, "disconnect", 400]:
        CASES[f"ambiguous_{failure}_{t}"] = {"tenant": tenant, "ops": [
            {**B, "problem": 7, "incident": 123}, {"op": "set", "state": {"fail": failure}}, S, S]}
    CASES[f"unavailable_readback_{t}"] = {"tenant": tenant, "ops": [
        B, {"op": "set", "state": {"partial_after": True}}, S, S]}
    CASES[f"mismatch_{t}"] = {"tenant": tenant, "ops": [B, {"op": "set", "state": {"mismatch": True}}, S]}
    CASES[f"preflight_{t}"] = {"tenant": tenant, "ops": [
        B, {"op": "set", "state": {"type": "Service Request"}}, S, {**B, "incident": "SR-123"},
        {**B, "incident": "sr-123"}, {"op": "set", "state": {"type": "Incident", "partial": True}}, B]}
CASES["refusals"] = {"ops": [
    {**B, "unlink": True}, {"op": "set", "state": {"linked": True}}, B,
    {"op": "set", "state": {"linked": False, "display": "SR-123"}}, B,
    {"op": "set", "state": {"display": None}}, B,
    {"op": "set", "state": {"ticket_id": "124"}}, B,
    {"op": "set", "state": {"ticket_id": 123, "problem": {"problem": {"id": 8}}}}, B,
    {"op": "set", "state": {"problem": {"problem": "x"}}}, B,
    {"op": "set", "state": {"problem": {"id": "7"}}}, B,
    {"op": "set", "state": {"rows": [{"id": 5}, {"id": "5"}]}}, B,
    {"op": "set", "state": {"rows": [{"id": 0}]}}, B,
    {"op": "set", "state": {"rows": [{"id": 5}]}}, B,
    {**B, "problem": "0"}, {**B, "incident": "0"}, {**B, "problem": "INC-7"}, {**B, "incident": "CHN-1"},
    {**S, "hash": "none"}, {**S, "hash": "prefix"}]}
json.dump(stamped({name: run(case) for name, case in CASES.items()}), sys.stdout, indent=1, ensure_ascii=False)
