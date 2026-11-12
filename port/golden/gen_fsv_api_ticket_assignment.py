"""Golden `ticket_assignment` (Python cc41912) for fsv-api
`ticket_assignment`: assignment/pickup wires, plan hashes, reviews, intent,
readback and refusals, plus the `ticket_properties` explicit-null routing
encoding. Mirrors `test_ticket_assignment.py`; metadata loads through the real
`load_meta` from routes and lookups use `ClientResolver`. The Rust test
replays the ops.

    cd /tmp/fsv-py-cc41912 && FSV_PY_SRC=$PWD FSV_PY_COMMIT=cc41912... uv run python \
        /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_ticket_assignment.py
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

from fsv import config, formmeta, journal, ticket_assignment as flow  # noqa: E402
from fsv.client import Client  # noqa: E402
from provenance import stamped  # noqa: E402

formmeta._save = lambda *a, **k: None
formmeta.configured_workspace = lambda: 1


def form_field(id_, name, label, field_type, position):
    """Mirror of fsv-api `service::testing::form_field`."""
    default = field_type.startswith("default_")
    return {"id": id_, "name": name, "label": label, "description": "", "field_type": field_type,
            "required": False, "required_for_closure": False, "position": position,
            "default_field": default, "choices": [], "nested_fields": [],
            "placeholder": "", "helptext": "", "hint": False,
            "content": None, "displayed_to_agents": True, "data_source": 0,
            "lookup_config": None, "workspace_id": 1 if default else 2,
            "updated_at": "2030-01-02T03:04:05Z"}


FIELDS = [form_field(9000007100, "group", "Group", "default_group", 1),
          form_field(9000007101, "agent", "Agent", "default_agent", 2)]


def new_state():
    return {"boot": {"agents": [{"id": 71, "name": "Acme Agent", "email": "agent@example.com", "group_ids": [81], "deleted": False},
                                {"id": 72, "name": "Globex Agent", "group_ids": [], "deleted": False}],
                     "groups": [{"id": 81, "name": "Acme Desk", "agent_ids": [71]},
                                {"id": 82, "name": "Globex Desk", "agent_ids": []}]},
            "tickets": {"123": {"id": 123, "responder_id": 71, "group_id": 81},
                        "124": {"id": 124, "responder_id": None, "group_id": 81}},
            "me": {"agent": {"id": 71}}, "fail": None, "mismatch": False, "writes": []}


def handler_for(s):
    def handle(req):
        path = req.url.path
        if path == "/api/v2/agents/me":
            return httpx.Response(200, json=s["me"])
        suffix = path.removeprefix("/api/_/")
        if suffix == "bootstrap/agents_groups":
            return httpx.Response(200, json=copy.deepcopy(s["boot"]))
        if suffix == "ticket_form_fields":
            return httpx.Response(200, json={"ticket_fields": FIELDS})
        if suffix.endswith("business_rules"):
            return httpx.Response(200, json={"business_rules": []})
        if req.method == "GET" and suffix.startswith("tickets/"):
            return httpx.Response(200, json={"ticket": copy.deepcopy(s["tickets"][suffix.split("/")[1]])})
        if req.method == "PUT":
            s["writes"].append({"method": "PUT", "path": suffix, "raw": req.content.decode()})
            body = json.loads(req.content)
            if s["fail"] == "disconnect":
                raise httpx.ReadError("connection lost", request=req)
            if s["fail"]:
                return httpx.Response(s["fail"], json={"message": "synthetic failure"})
            if suffix == "tickets/pick_up":
                for cid in body["ids"]:
                    if not s["mismatch"]:
                        s["tickets"][str(cid)]["responder_id"] = 71
                return httpx.Response(204)
            cid = suffix.split("/")[1]
            if not s["mismatch"]:
                s["tickets"][cid].update({k: v for k, v in body.items() if k != "br_validation_excludes"})
            return httpx.Response(200, json={"ticket": copy.deepcopy(s["tickets"][cid])})
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
    config.DOMAIN = "acme.example.com"
    config.API_INT = "https://acme.example.com/api/_"
    config.API_V2 = "https://acme.example.com/api/v2"
    s = new_state()
    c = client(handler_for(s))
    plan = None
    outputs = []
    for op in case["ops"]:
        try:
            if op["op"] == "build":
                plan = flow.build_plan(c, op["ids"], agent=op.get("agent"), group=op.get("group"),
                                       unassign=op.get("unassign", False), pickup=op.get("pickup", False))
                out = {"write": dict(plan.write.__dict__), "intent_json": plan.intent_json,
                       "before_json": plan.before_json, "hash": plan.hash, "review": plan.review()}
            elif op["op"] == "send":
                reviewed = {"full": plan.hash, "prefix": plan.hash[:16]}[op.get("hash", "full")]
                out = {"result": flow.send(plan, c, reviewed_hash=reviewed)}
            elif op["op"] == "set":
                for k, v in op.get("state", {}).items():
                    s[k] = v
                for k, v in op.get("boot", {}).items():
                    s["boot"][k] = v
                for k, v in op.get("group0", {}).items():
                    if v is None:
                        s["boot"]["groups"][0].pop(k)
                    else:
                        s["boot"]["groups"][0][k] = v
                for k, v in op.get("ticket123", {}).items():
                    if v == "<pop>":
                        s["tickets"]["123"].pop(k)
                    else:
                        s["tickets"]["123"][k] = v
                out = {}
            else:
                raise AssertionError(op)
        except Exception as exc:  # noqa: BLE001
            out = {"error": {"type": type(exc).__name__, "msg": str(exc)}}
        out["writes"] = len(s["writes"])
        outputs.append(out)
    c.close()
    events = [[r["kind"], r["event"]] for r in journal.read() if str(r.get("kind", "")).startswith("plan:ticket_")]
    return {**case, "outputs": outputs, "events": events, "writes": s["writes"]}


B = {"op": "build"}
S = {"op": "send"}
CASES = {
    "single_unassign": {"ops": [{**B, "ids": ["123"], "unassign": True}, S, S]},
    "group_agent_then_bulk_pickup": {"ops": [
        {**B, "ids": ["123"], "group": "Acme Desk", "agent": "agent@example.com"}, S,
        {**B, "ids": ["123", "124"], "pickup": True}, S]},
    "group_only_clears_responder": {"ops": [{**B, "ids": ["123"], "group": "Globex Desk"}, S]},
    "agent_only_infers_group": {"ops": [{**B, "ids": ["INC-124"], "agent": " ACME agent "}, S]},
    "group_by_id_text": {"ops": [{**B, "ids": ["123"], "group": "81"}]},
    "membership": {"ops": [
        {**B, "ids": ["123"], "agent": "Globex Agent"}, {"op": "set", "group0": {"agent_ids": None}},
        {**B, "ids": ["123"], "pickup": True}, {"op": "set", "group0": {"agent_ids": [71, "x"]}},
        {**B, "ids": ["123"], "pickup": True}]},
    "argument_errors": {"ops": [
        {**B, "ids": []}, {**B, "ids": [str(i) for i in range(1, 102)]}, {**B, "ids": ["123", "INC-123"]},
        {**B, "ids": ["0"]}, {**B, "ids": ["123"], "pickup": True, "agent": "71"},
        {**B, "ids": ["123", "124"], "agent": "71"}, {**B, "ids": ["123"], "unassign": True, "group": "81"},
        {**B, "ids": ["123"]}]},
    "metadata_errors": {"ops": [
        {"op": "set", "boot": {"agents": None}}, {**B, "ids": ["123"], "unassign": True},
        {"op": "set", "boot": {"agents": [{"id": 71, "name": "Acme Agent", "deleted": True},
                                          {"id": 73, "name": "Dup"}, {"id": 74, "name": "dup"}]}},
        {**B, "ids": ["123"], "agent": "Acme Agent"}, {**B, "ids": ["123"], "agent": "Dup"},
        {**B, "ids": ["123"], "agent": "   "}, {"op": "set", "state": {"me": {"agent": {"id": True}}}},
        {**B, "ids": ["123"], "pickup": True}]},
    "routing_unavailable": {"ops": [{"op": "set", "ticket123": {"group_id": "<pop>"}},
                                    {**B, "ids": ["123"], "unassign": True}]},
    "changed_metadata_and_mismatch": {"ops": [
        {**B, "ids": ["123"], "unassign": True}, {"op": "set", "group0": {"name": "Changed"}}, S,
        {**B, "ids": ["123"], "unassign": True}, {"op": "set", "state": {"mismatch": True}}, S]},
    "pickup_mismatch": {"ops": [{"op": "set", "ticket123": {"responder_id": None}},
                                {**B, "ids": ["123"], "pickup": True}, {"op": "set", "state": {"mismatch": True}}, S]},
    "stale_hash": {"ops": [{**B, "ids": ["123"], "unassign": True}, {**S, "hash": "prefix"}]},
}
for failure in [403, 429, 500, "disconnect"]:
    CASES[f"ambiguous_{failure}"] = {"ops": [{**B, "ids": ["123"], "unassign": True},
                                             {"op": "set", "state": {"fail": failure}}, S, S]}


def outcome(fn):
    try:
        return {"ok": fn()}
    except Exception as exc:  # noqa: BLE001
        return {"error": str(exc)}


POSITIVE = [71, "71", 0, -1, True, 1.0, "0", "007", "x", "", None, [1], " 7"]
ROWS = [{"id": 71, "name": "Acme Agent", "email": "agent@example.com"}, {"id": 72, "name": "Gone", "deleted": True},
        {"id": 73, "name": "Off", "deactivated": 1}, {"id": "x", "name": "BadId"}, {"id": 74, "name": "Twin"},
        {"id": 75, "name": "twin"}, "junk", {"id": 76, "email": None}]
RESOLVE = [(ROWS, "acme agent"), (ROWS, "AGENT@EXAMPLE.COM"), (ROWS, "71"), (ROWS, "Gone"), (ROWS, "Off"),
           (ROWS, "BadId"), (ROWS, "twin"), (ROWS, " "), (None, "x"), (ROWS, "None")]
MEMBER = [({"id": 71, "group_ids": [81]}, {"id": 81, "agent_ids": [71]}),
          ({"id": 71, "group_ids": ["81"]}, {"id": "81", "agent_ids": ["71"]}),
          ({"id": 71}, {"id": 81, "agent_ids": [71]}),
          ({"id": 71, "group_ids": [81]}, {"id": 81, "agent_ids": None}),
          ({"id": 71, "group_ids": [81, 0]}, {"id": 81, "agent_ids": [71]}),
          ({"id": 71, "group_ids": [82]}, {"id": 81, "agent_ids": [71]}),
          ({"id": 71, "group_ids": [81]}, {"id": 81, "agent_ids": [72]})]
helpers = {"positive": [{"value": v, "expected": flow._positive(v)} for v in POSITIVE],
           "resolve": [{"rows": r, "value": v, **outcome(lambda r=r, v=v: flow._resolve(r, v, "agent"))} for r, v in RESOLVE],
           "member": [{"agent": a, "group": g, **outcome(lambda a=a, g=g: flow._member(a, g))} for a, g in MEMBER]}
json.dump(stamped({**{name: run(case) for name, case in CASES.items()}, "helpers": helpers}),
          sys.stdout, indent=1, ensure_ascii=False)
