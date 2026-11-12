"""Golden `ticket_merge` (Python f402e61) for fsv-api `ticket_merge`: the
complete-merge wire, plan hashes, reviews, readback checks, refusals and
journal events. Mirrors `test_ticket_merge.py`; metadata loads through the
real `load_meta` from routes. The Rust test replays the ops.

    cd /tmp/fsv-py-f402e61 && FSV_PY_SRC=$PWD FSV_PY_COMMIT=f402e61... uv run python \
        /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_ticket_merge.py
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

from fsv import config, formmeta, journal, ticket_merge as flow  # noqa: E402
from fsv.client import Client  # noqa: E402
from provenance import stamped  # noqa: E402

formmeta._save = lambda *a, **k: None
formmeta.configured_workspace = lambda: 1


def status_field(closed=True):
    choices = [{"id": 31, "value": "Open"}] + ([{"id": 89, "value": "Closed"}] if closed else [])
    return {"id": 9000007101, "name": "status", "label": "Status", "description": "", "field_type": "default_status",
            "required": False, "required_for_closure": False, "position": 1, "default_field": True,
            "choices": choices, "nested_fields": [], "placeholder": "", "helptext": "", "hint": False,
            "content": None, "displayed_to_agents": True, "data_source": 0, "lookup_config": None,
            "workspace_id": 1, "updated_at": "2030-01-02T03:04:05Z"}


def new_state():
    return {"records": {str(i): {"id": i, "status": 31, "subject": f"Acme {i}"} for i in (123, 124, 125)},
            "histories": {"123": [{"id": 10, "body": "earlier"}], "124": [{"id": 11, "body": "source A"}],
                          "125": [{"id": 12, "body": "source B"}]},
            "has_next": False, "closed_choice": True, "fail": None, "bad_readback": None, "requests": []}


def handler_for(s):
    def handle(request):
        path = request.url.path.removeprefix("/api/_/")
        if request.method == "POST":
            s["requests"].append({"method": "POST", "path": path, "raw": request.content.decode()})
            if s["fail"] == "disconnect":
                raise httpx.ReadError("connection lost", request=request)
            if s["fail"]:
                return httpx.Response(s["fail"], json={"message": "synthetic failure"})
            body = json.loads(request.content)
            for i in body["source"]["ticket_ids"]:
                s["records"][str(i)]["status"] = 89
                s["histories"]["123"].extend(s["histories"][str(i)])
                s["histories"][str(i)] = [{"id": 100 + i, "body": "<div>Secondary note</div>", "private": body["source"]["is_private"]}]
            s["histories"]["123"].append({"id": 20, "body": "<p>Primary note</p>", "private": body["target"]["is_private"]})
            if s["bad_readback"] == "status":
                s["records"]["124"].pop("status")
            elif s["bad_readback"] == "note":
                s["histories"]["123"][-1].pop("body")
            elif s["bad_readback"] == "moved":
                s["histories"]["123"] = [row for row in s["histories"]["123"] if row["id"] != 11]
            elif s["bad_readback"] == "primary_status":
                s["records"]["123"]["status"] = 89
            return httpx.Response(200, json={"count": 2, "target": body["target"], "source": body["source"]})
        if path == "ticket_form_fields":
            return httpx.Response(200, json={"ticket_fields": [status_field(s["closed_choice"])]})
        if path.endswith("business_rules"):
            return httpx.Response(200, json={"business_rules": []})
        parts = path.split("/")
        if len(parts) == 2 and parts[0] == "tickets":
            return httpx.Response(200, json={"ticket": copy.deepcopy(s["records"][parts[1]])})
        if len(parts) == 3 and parts[2] == "conversations":
            rows = s["histories"][parts[1]]
            return httpx.Response(200, json={"conversations": copy.deepcopy(rows), "meta": {"has_next": s["has_next"]}})
        return httpx.Response(404, json={"error": "unexpected"})
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
                plan = flow.build_plan(c, op.get("primary", "INC-123"), op.get("secondaries", ["INC-124", "INC-125"]),
                                       note_primary=op.get("np", "Primary note"),
                                       note_secondary=op.get("ns", "Secondary note"),
                                       private_primary=op.get("pp", True), private_secondary=op.get("ps", True))
                out = {"plan": dict(plan.__dict__), "hash": plan.hash, "review": flow.review(plan)}
            elif op["op"] == "send":
                reviewed = {"full": plan.hash, "none": None, "prefix": plan.hash[:16]}[op.get("hash", "full")]
                out = {"result": flow.send(plan, c, reviewed_hash=reviewed)}
            elif op["op"] == "set":
                for k, v in op.get("state", {}).items():
                    s[k] = v
                for i, v in op.get("subject", {}).items():
                    s["records"][i]["subject"] = v
                for i, v in op.get("histories", {}).items():
                    s["histories"][i] = v
                out = {}
            else:
                raise AssertionError(op)
        except Exception as exc:  # noqa: BLE001
            out = {"error": {"type": type(exc).__name__, "msg": str(exc)}}
        out["posts"] = len(s["requests"])
        outputs.append(out)
    c.close()
    events = [r["event"] for r in journal.read() if r.get("kind") == "plan:ticket_merge"]
    return {**case, "outputs": outputs, "events": events, "writes": s["requests"]}


B = {"op": "build"}
S = {"op": "send"}
CASES = {
    "exact_wire": {"ops": [B, S, S]},
    "globex_public_notes": {"tenant": "globex.example.com", "ops": [
        {**B, "secondaries": ["124"], "np": "Primary note", "ns": "Secondary note", "pp": False, "ps": False}, S]},
    "stale_and_invalid": {"ops": [
        B, {**S, "hash": "prefix"}, {"op": "set", "subject": {"124": "changed"}}, S,
        {**B, "primary": "123", "secondaries": ["123"]}, {**B, "primary": "123", "secondaries": ["124"], "np": ""},
        {**B, "secondaries": []}, {**B, "secondaries": ["124", "INC-124"]}, {**B, "primary": "0"},
        {**B, "ns": "a\u0000b"}, {**B, "ns": "  "}, {**S, "hash": "none"}]},
    "incomplete_history": {"ops": [{"op": "set", "state": {"has_next": True}}, B]},
    "duplicate_history_ids": {"ops": [{"op": "set", "histories": {"124": [{"id": 11}, {"id": "11"}]}}, B]},
    "zero_history_id": {"ops": [{"op": "set", "histories": {"125": [{"id": 0}]}}, B]},
    "bool_history_id": {"ops": [{"op": "set", "histories": {"125": [{"id": True}]}}, B]},
    "no_closed_status": {"ops": [{"op": "set", "state": {"closed_choice": False}}, B]},
}
for failure in [403, 429, 500, "disconnect"]:
    CASES[f"ambiguous_{failure}"] = {"ops": [B, {"op": "set", "state": {"fail": failure}}, S,
                                             {"op": "set", "state": {"fail": None}}, S]}
for bad in ["status", "note", "moved", "primary_status"]:
    CASES[f"readback_{bad}"] = {"ops": [B, {"op": "set", "state": {"bad_readback": bad}}, S, S]}
json.dump(stamped({name: run(case) for name, case in CASES.items()}), sys.stdout, indent=1, ensure_ascii=False)
