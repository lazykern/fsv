"""Golden `ticket_tasks` (Python c46d3b0, metadata-driven create 6e598b2,
unoffered-choice message 39e4853, owner/group update 4626e12) for fsv-api
`ticket_tasks`: the add/status/delete/update wires, plan hashes, inputs,
readback, refusals and journal events. Mirrors `test_ticket_tasks_reviewed.py`
(the non-CLI tests, for both status-id bases); the Rust test replays the ops.

    cd /tmp/fsv-py-4626e12 && FSV_PY_SRC=$PWD FSV_PY_COMMIT=4626e12... uv run python \
        /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_ticket_tasks.py
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

from fsv import config, journal, ticket_tasks as flow  # noqa: E402
from fsv.client import Client  # noqa: E402
from provenance import stamped  # noqa: E402


def task_fields(n):
    return [{"name": "title", "required_for_create": True},
            {"name": "status", "choices": [{"id": n, "value": "Open"}, {"id": n + 1, "value": "Completed"},
                                           {"id": n + 2, "value": "Canceled"}]},
            {"name": "notify_before", "choices": [{"id": 0, "value": "Never"}]},
            {"name": "tier", "label": "Tier", "field_type": "custom_dropdown", "required_for_create": True,
             "choices": [{"id": n + 3, "value": "Standard"}]}]


def new_state(n):
    return {"rows": [], "fail": None, "partial": False, "partial_after": False, "mismatch": False,
            "fields": task_fields(n), "fields_key": "ticket_task_fields", "response": None, "requests": [],
            "agents": []}


def handler_for(s):
    def handle(request):
        if request.url.path == "/search/autocomplete/agents":
            return httpx.Response(200, json={"results": copy.deepcopy(s.get("agents", []))})
        path = request.url.path.removeprefix("/api/_/")
        if request.method == "GET":
            if path == "ticket_task_fields":
                return httpx.Response(200, json={s["fields_key"]: copy.deepcopy(s["fields"])})
            if path != "tickets/123/tasks":
                return httpx.Response(404, json={"error": "unexpected"})
            if s["partial"]:
                return httpx.Response(403, json={"error": "denied"})
            return httpx.Response(200, json={"tasks": copy.deepcopy(s["rows"]), "meta": {"count": len(s["rows"])}})
        body = json.loads(request.content)
        s["requests"].append({"method": request.method, "path": path, "raw": request.content.decode()})
        if s["fail"] == "disconnect":
            raise httpx.ReadError("disconnect", request=request)
        if s["fail"]:
            return httpx.Response(s["fail"], json={"error": "failure"})
        if path.endswith("/update"):
            s["rows"] = []
            result = {"tasks": []}
        elif request.method == "POST":
            row = {"id": 701, "human_display_id": "TSK-701", **body}
            s["rows"] = [copy.deepcopy(row)]
            result = {"task": row}
        else:
            s["rows"][0].update(body)
            result = {"task": copy.deepcopy(s["rows"][0])}
        if s["mismatch"] and s["rows"]:
            s["rows"][0]["status"] = 999
        if s["partial_after"]:
            s["partial"] = True
        if s["response"] is not None:
            result = s["response"]
        return httpx.Response(201 if request.method == "POST" else 200, json=result)
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


ARGS = ("action", "task", "title", "status", "values", "owner", "group")


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
                kw = {k: op[k] for k in ARGS if k in op}
                plan = flow.build_plan(c, op.get("target", "INC-123"), **kw)
                out = {"plan": dict(plan.__dict__), "hash": plan.hash, "review": plan.review()}
            elif op["op"] == "send":
                reviewed = {"full": plan.hash, "none": None, "prefix": plan.hash[:16]}[op.get("hash", "full")]
                out = {"result": flow.send(plan, c, reviewed_hash=reviewed)}
            elif op["op"] == "custom":
                out = {"custom": flow.custom_values(op["fields"], op["values"])}
            elif op["op"] == "set":
                s.update(copy.deepcopy(op.get("state", {})))
                if "bump_open" in op:
                    s["fields"][1]["choices"][0]["id"] += op["bump_open"]
                out = {}
            else:
                raise AssertionError(op)
        except Exception as exc:  # noqa: BLE001
            out = {"error": {"type": type(exc).__name__, "msg": str(exc)}}
        out["writes"] = len(s["requests"])
        outputs.append(out)
    c.close()
    events = [[r["kind"], r["event"]] for r in journal.read() if str(r.get("kind", "")).startswith("plan:ticket_task")]
    return {**case, "outputs": outputs, "events": events, "writes": s["requests"]}


ADD = {"op": "build", "action": "add", "title": "Acme task", "values": {"Tier": "Standard"}}
S = {"op": "send"}
CF = [{"name": "tier", "label": "Tier", "field_type": "custom_dropdown", "choices": [{"id": 5, "value": "Standard"}]},
      {"name": "note", "label": "Note", "field_type": "custom_text"},
      {"name": "memo", "label": "Memo", "custom": True, "field_type": "custom_paragraph"},
      {"name": "plain", "label": "Plain", "field_type": "default_text"},
      {"name": "gone", "label": "Gone", "field_type": "custom_text", "deleted": True},
      {"name": "num", "label": "Num", "is_custom": True, "field_type": "custom_number"},
      {"name": "twin", "label": "Twin", "field_type": "custom_text"}, {"name": "twin2", "label": "twin", "field_type": "custom_text"},
      {"name": None, "label": "Nameless", "field_type": "custom_text"}]
CUSTOM = [{"Tier": "standard"}, {"tier": "5"}, {"Note": "x", "memo": "y"}, {"Note": 5}, {"Plain": "x"}, {"Gone": "x"},
          {"Num": 1}, {"Twin": "x"}, {"Missing": "x"}, {"Tier": "Gold"}, {"Nameless": "x"}, {"note": "a", "NOTE": "b"}, {}]
CASES = {}
for n in (21, 61):
    CASES[f"lifecycle_{n}"] = {"n": n, "ops": [
        ADD, S, {"op": "build", "target": 123, "action": "status", "task": 701, "status": "Canceled"}, S,
        {"op": "build", "target": 123, "action": "delete", "task": 701}, S, S]}
    for failure in [403, 429, 500, "disconnect", 422]:
        CASES[f"unknown_{failure}_{n}"] = {"n": n, "ops": [ADD, {"op": "set", "state": {"fail": failure}}, S, S]}
    CASES[f"validation_drift_{n}"] = {"n": n, "ops": [
        {"op": "build", "target": 123, "action": "add", "title": "Acme"},
        {"op": "build", "target": 123, "action": "status", "task": "bad", "status": "Open"},
        ADD, {"op": "set", "bump_open": 100}, S]}
    for mode in ["mismatch", "partial_after"]:
        CASES[f"readback_{mode}_{n}"] = {"n": n, "ops": [ADD, {"op": "set", "state": {mode: True}}, S]}
CASES["refusals"] = {"n": 21, "ops": [
    {"op": "build", "action": "bogus"}, {"op": "build", "target": "0", "action": "add"},
    {"op": "build", "target": "SR-5", "action": "status", "task": 1}, {**ADD, "title": " "}, {**ADD, "title": "a\tb"},
    {**ADD, "title": None}, {**ADD, "status": "Done"}, {**ADD, "status": " completed "}, {**ADD, "status": "22"},
    {"op": "build", "action": "status", "task": 701}, {"op": "build", "action": "delete", "task": None},
    {"op": "build", "action": "delete", "task": True}, {**ADD, "values": {"tier": "Standard", "Tier": "Standard"}},
    {**ADD, "values": {"Tier": ""}}, {"op": "set", "state": {"rows": [{"id": 9}, {"id": "9"}]}}, ADD,
    {"op": "set", "state": {"rows": [{"id": 0}]}}, ADD,
    {"op": "set", "state": {"rows": [{"id": "x"}]}}, ADD,
    {"op": "set", "state": {"rows": [{"id": 9, "status": 21}, {"id": 3, "status": 22}]}},
    {"op": "build", "action": "status", "task": "9", "status": "Open"},
    {"op": "build", "action": "delete", "task": "003"},
    {"op": "set", "state": {"partial": True}}, ADD, {"op": "set", "state": {"partial": False}},
    {"op": "set", "state": {"fields_key": "task_fields"}}, ADD,
    {"op": "set", "state": {"fields_key": "other"}}, ADD,
    {"op": "set", "state": {"fields_key": "fields", "fields": [{"name": "status", "choices": [{"id": 21, "value": "Open"}]}, "x"]}}, ADD,
    {"op": "set", "state": {"fields_key": "fields", "fields": [{"name": "status", "choices": [{"id": 21, "value": "Open"}]},
                                                              {"name": "notify_before", "choices": [{"id": 0, "value": "Never"}]},
                                                              {"label": "Need", "required": True}]}}, ADD,
    {"op": "set", "state": {"fields": [{"name": "status", "choices": [{"id": 21, "value": "Open"}]}]}}, ADD,
    {"op": "set", "state": {"fields": [{"name": "status", "choices": [{"id": 21, "value": "Open"}]},
                                       {"name": "notify_before", "choices": [{"id": 0, "value": "Never"}]}]}},
    {**ADD, "values": None}, {**S, "hash": "none"}, {**S, "hash": "prefix"}]}
BASIC = [{"name": "status", "choices": [{"id": 21, "value": "Open"}]},
         {"name": "notify_before", "choices": [{"id": 0, "value": "Never"}]}]
for label, response in [("string_task", {"task": "x"}), ("list", [1]), ("bad_id", {"task": {"id": "x"}}),
                        ("no_id", {"task": {}})]:
    CASES[f"response_{label}"] = {"n": 21, "ops": [
        {"op": "set", "state": {"fields": BASIC, "response": response}}, {**ADD, "values": None}, S]}
CASES["response_old_id"] = {"n": 21, "ops": [
    {"op": "set", "state": {"rows": [{"id": 701, "status": 21}],
                            "fields": [{"name": "status", "choices": [{"id": 21, "value": "Open"}]},
                                       {"name": "notify_before", "choices": [{"id": 0, "value": "Never"}]}]}},
    {**ADD, "values": None}, S]}
CASES["response_bare"] = {"n": 21, "ops": [
    {"op": "set", "state": {"response": {"id": 702}, "fields": [{"name": "status", "choices": [{"id": 21, "value": "Open"}]},
                                                                {"name": "notify_before", "choices": [{"id": 0, "value": "Never"}]}]}},
    {**ADD, "values": None}, S]}
CASES["custom_values"] = {"n": 21, "ops": [{"op": "custom", "fields": CF, "values": v} for v in CUSTOM]}
NATIVE = [{"name": "cf_area", "label": "Area", "default": False, "dom_type": "text"},
          {"name": "cf_memo", "label": "Memo", "default": False, "dom_type": "textarea"},
          {"name": "cf_pick", "label": "Pick", "default": False, "dom_type": "dropdown", "choices": [{"id": "7", "value": "Acme"}]},
          {"name": "plain_area", "label": "Plain", "default": True, "dom_type": "text"},
          {"name": "cf_num", "label": "Num", "default": False, "dom_type": "number"},
          {"name": "area", "label": "Twin", "default": False, "dom_type": "text"}]
CASES["custom_native"] = {"n": 21, "ops": [{"op": "custom", "fields": NATIVE, "values": v} for v in [
    {"cf_area": "x", "Memo": "y"}, {"Area": "x"}, {"Pick": "acme"}, {"Pick": "Globex"}, {"Pick": 5}, {"Plain": "x"}, {"Num": 1},
    {"cf_area": "x", "Twin": "y"}, {"Tier": "x"}, {"cf_memo": 3}]]}
for n in (21, 61):
    no_reminder = [f for f in task_fields(n) if f["name"] != "notify_before"]
    CASES[f"reminder_optional_{n}"] = {"n": n, "ops": [{"op": "set", "state": {"fields": no_reminder}}, ADD, S]}
    native = task_fields(n)
    native[2]["choices"][0]["id"] = "0"
    native[-1] = {"name": "cf_tier", "label": "Tier", "default": False, "type": 1003, "dom_type": "dropdown",
                  "field_options": {"required_for_create": "true", "required_for_closure": "false"},
                  "choices": [{"id": str(n + 3), "value": "Standard"}]}
    native.append({"name": "workspace_id", "default": True, "field_options": {"required_for_create": "true"}})
    CASES[f"native_required_{n}"] = {"n": n, "ops": [
        {"op": "set", "state": {"fields": native}}, {"op": "build", "action": "add", "title": "Acme task"}, ADD, S]}
    unknown = task_fields(n) + [{"name": "unproven_a", "label": "Acme dependency", "required": True},
                                {"name": "unproven_b", "label": "Globex dependency", "field_options": {"required_for_create": "true"}},
                                {"name": "closure_only", "field_options": {"required_for_closure": "true"}},
                                {"name": "flag_text", "required_on_create": "TRUE"}, {"required_for_agents": True}]
    CASES[f"required_listed_{n}"] = {"n": n, "ops": [{"op": "set", "state": {"fields": unknown}}, ADD]}
    CASES[f"unoffered_choice_{n}"] = {"n": n, "ops": [{**ADD, "values": {"Tier": "Globex"}}, {**ADD, "values": {"Tier": 7}}]}
reminders = []
for choice in [True, -1, "-1", "bad", None, "\u0663", 1.0, "007", 3]:
    fields = task_fields(21)
    fields[2]["choices"][0]["id"] = choice
    reminders += [{"op": "set", "state": {"fields": fields}}, ADD]
for choices in [[{"id": 0, "value": "Never"}, {"id": 1, "label": "never"}],
                [{"id": 0, "value": "Never", "deactivated": True}, {"id": 4, "value": "", "label": "NEVER"}],
                [{"id": 0, "value": "Never", "deleted": True}], [], ["x", {"id": 2, "value": "Never"}]]:
    fields = task_fields(21)
    fields[2]["choices"] = choices
    reminders += [{"op": "set", "state": {"fields": fields}}, ADD]
for extra in [{"name": "notify_before", "choices": []}, None]:
    fields = task_fields(21)
    if extra is None:
        fields[2]["deleted"] = True
    else:
        fields.append(extra)
    reminders += [{"op": "set", "state": {"fields": fields}}, ADD]
CASES["reminders"] = {"n": 21, "ops": reminders}
PARTY = task_fields(21) + [{"name": "owner_id"}, {"name": "group_id"}]
AGENT = [{"id": "agent@example.com", "value": "Alex Acme", "user_id": 81}]
ROW = [{"id": 701, "status": 21}]
CASES["assign_add"] = {"n": 21, "ops": [
    {"op": "set", "state": {"fields": PARTY, "agents": AGENT}},
    {"op": "build", "action": "add", "title": "Acme task", "values": {"Tier": "Standard"},
     "owner": "agent@example.com", "group": "91"}]}
CASES["assign_update"] = {"n": 21, "ops": [
    {"op": "set", "state": {"fields": PARTY, "agents": AGENT, "rows": ROW}},
    {"op": "build", "action": "update", "task": 701, "status": None,
     "owner": "agent@example.com", "group": "91"}, S]}
CASES["assign_missing"] = {"n": 21, "ops": [
    {"op": "set", "state": {"rows": ROW}},
    {"op": "build", "action": "update", "task": 701, "status": None, "owner": "81"},
    {"op": "build", "action": "update", "task": 701, "status": None, "group": "91"}]}
CASES["assign_deleted"] = {"n": 21, "ops": [
    {"op": "set", "state": {"fields": task_fields(21) + [{"name": "owner_id", "deleted": True},
                                                        {"name": "group_id", "is_deleted": True}],
                            "rows": ROW}},
    {"op": "build", "action": "update", "task": 701, "status": None, "owner": "81"},
    {"op": "build", "action": "update", "task": 701, "status": None, "group": "91"}]}
CASES["assign_duplicate"] = {"n": 21, "ops": [
    {"op": "set", "state": {"fields": PARTY + [{"name": "owner_id"}, {"name": "group_id"}], "rows": ROW}},
    {"op": "build", "action": "update", "task": 701, "status": None, "owner": "81"},
    {"op": "build", "action": "update", "task": 701, "status": None, "group": "91"}]}
CASES["assign_owner_bad"] = {"n": 21, "ops": [
    {"op": "set", "state": {"fields": PARTY, "agents": AGENT, "rows": ROW}}] + [
    {"op": "build", "action": "update", "task": 701, "status": None, "owner": bad}
    for bad in ("me", "Acme Agent", "0", "-1", "81x", " agent@example.com")]}
CASES["assign_drift"] = {"n": 21, "ops": [
    {"op": "set", "state": {"fields": PARTY, "rows": ROW}},
    {"op": "build", "action": "update", "task": 701, "status": None, "owner": "81"},
    {"op": "set", "state": {"fields": task_fields(21)}}, S]}
CASES["assign_empty"] = {"n": 21, "ops": [
    {"op": "set", "state": {"rows": ROW}},
    {"op": "build", "action": "update", "task": 701, "status": None}]}
CASES["assign_group_blank"] = {"n": 21, "ops": [
    {"op": "set", "state": {"fields": PARTY, "rows": ROW}},
    {"op": "build", "action": "update", "task": 701, "status": None, "group": "  "}]}
json.dump(stamped({name: run(case) for name, case in CASES.items()}), sys.stdout, indent=1, ensure_ascii=False)
