"""Golden `ticket_note_mutation` (Python 9910ddc) for fsv-api
`ticket_note_mutation`: the PUT/DELETE wire, plan hashes, reviews, readback,
refusals and journal events. Mirrors `test_ticket_note_mutation.py` (the
non-CLI tests); the Rust test replays the ops.

    cd /tmp/fsv-py-9910ddc && FSV_PY_SRC=$PWD FSV_PY_COMMIT=9910ddc... uv run python \
        /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_ticket_note_mutation.py
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

from fsv import config, journal, ticket_note_mutation as flow  # noqa: E402
from fsv.client import Client  # noqa: E402
from provenance import stamped  # noqa: E402


def new_state():
    return {"rows": [{"id": 99001, "source": 2, "private": True, "body": "<div>Old</div>"}], "fail": None,
            "partial": False, "partial_after": False, "mismatch": False, "requests": []}


def handler_for(s):
    def handle(request):
        path = request.url.path.removeprefix("/api/_/")
        if request.method == "GET":
            if path != "tickets/123/conversations":
                return httpx.Response(404, json={"error": "unexpected"})
            if s["partial"]:
                return httpx.Response(403, json={"message": "denied"})
            return httpx.Response(200, json={"conversations": copy.deepcopy(s["rows"]), "meta": {"count": len(s["rows"])}})
        s["requests"].append({"method": request.method, "path": path, "raw": request.content.decode()})
        if s["fail"] == "disconnect":
            raise httpx.ReadError("disconnect", request=request)
        if s["fail"]:
            return httpx.Response(s["fail"], json={"message": "failure"})
        if s["partial_after"]:
            s["partial"] = True
        if request.method == "PUT" and s.get("drop_body"):
            s["rows"][0].pop("body")
            return httpx.Response(200, json={})
        if request.method == "PUT":
            s["rows"][0]["body"] = ("<div>Unexpected</div>" if s["mismatch"] else
                                    '<div class="server"><div>New &amp; text</div></div>')
            return httpx.Response(200, json={"conversation": copy.deepcopy(s["rows"][0])})
        s["rows"].clear()
        return httpx.Response(204)
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
                plan = flow.build_plan(c, op.get("ticket", "INC-123"), op.get("note", "99001"), body=op.get("body"))
                out = {"plan": dict(plan.__dict__), "hash": plan.hash}
            elif op["op"] == "review":
                out = {"review": flow.review(plan, c)}
            elif op["op"] == "send":
                reviewed = {"full": plan.hash, "none": None, "prefix": plan.hash[:16]}[op.get("hash", "full")]
                out = {"result": flow.send(plan, c, reviewed_hash=reviewed)}
            elif op["op"] == "set":
                s.update(op.get("state", {}))
                for k, v in op.get("row", {}).items():
                    s["rows"][0][k] = v
                if "rows" in op:
                    s["rows"] = op["rows"]
                out = {}
            else:
                raise AssertionError(op)
        except Exception as exc:  # noqa: BLE001
            out = {"error": {"type": type(exc).__name__, "msg": str(exc)}}
        out["writes"] = len(s["requests"])
        outputs.append(out)
    c.close()
    events = [[r["kind"], r["event"]] for r in journal.read() if str(r.get("kind", "")).startswith("plan:ticket_note")]
    return {**case, "outputs": outputs, "events": events, "writes": s["requests"]}


EDIT = {"op": "build", "body": "New & text"}
DEL = {"op": "build"}
R = {"op": "review"}
S = {"op": "send"}
CASES = {}
for tenant in ("acme.example.com", "globex.example.com"):
    t = tenant.split(".")[0]
    CASES[f"edit_{t}"] = {"tenant": tenant, "ops": [EDIT, R, S, S]}
    CASES[f"delete_{t}"] = {"tenant": tenant, "ops": [DEL, R, S, S]}
    for failure in [403, 429, 500, "disconnect", 400, 404]:
        CASES[f"unknown_{failure}_{t}"] = {"tenant": tenant, "ops": [
            {**EDIT, "ticket": 123}, {"op": "set", "state": {"fail": failure}}, S, S]}
    for mode in ["partial_after", "mismatch"]:
        CASES[f"unverified_{mode}_{t}"] = {"tenant": tenant, "ops": [
            {**EDIT, "ticket": 123}, {"op": "set", "state": {mode: True}}, S, S]}
    CASES[f"edit_no_body_readback_{t}"] = {"tenant": tenant, "ops": [
        EDIT, {"op": "set", "state": {"drop_body": True}}, S]}
    CASES[f"delete_partial_after_{t}"] = {"tenant": tenant, "ops": [
        DEL, {"op": "set", "state": {"partial_after": True}}, S]}
    CASES[f"preflight_{t}"] = {"tenant": tenant, "ops": [
        {**EDIT, "body": "New", "ticket": 123}, {"op": "set", "row": {"body": "changed"}}, R, S,
        {"op": "set", "row": {"source": 0}}, DEL, {"op": "set", "state": {"partial": True}}, DEL]}
CASES["invalid_ids"] = {"ops": [
    {**DEL, "note": "0"}, {**DEL, "note": "-1"}, {**DEL, "note": "x"}, {**DEL, "note": ""}, {**DEL, "note": "١٢"},
    {**DEL, "note": " 1"}, {**DEL, "ticket": "0"}, {**DEL, "ticket": "CHN-1"}, {**DEL, "note": "007"},
    {**EDIT, "body": "  "}, {**EDIT, "body": "a\u0000b"}, {**DEL, "hash": "none"}, {**S, "hash": "none"},
    {**S, "hash": "prefix"}]}
CASES["note_shapes"] = {"ops": [
    {"op": "set", "rows": [{"id": 99001, "source": True, "private": True}]}, DEL,
    {"op": "set", "rows": [{"id": 99001, "source": 2, "private": 1}]}, DEL,
    {"op": "set", "rows": [{"id": 99001, "source": 2.0, "private": False}]}, DEL,
    {"op": "set", "rows": [{"id": 99001, "source": 2, "private": False}, {"id": "99001", "source": 2, "private": False}]}, DEL,
    {"op": "set", "rows": [{"id": "99001", "source": 2, "private": False}]}, DEL,
    {"op": "set", "rows": [{"id": 99002, "source": 2, "private": False}]}, DEL]}
json.dump(stamped({name: run(case) for name, case in CASES.items()}), sys.stdout, indent=1, ensure_ascii=False)
