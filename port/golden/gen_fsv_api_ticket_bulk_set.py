"""Golden `ticket_bulk_set` for fsv-api `ticket_bulk` (`bulk status --set`):
per-ticket reviewed PUTs over a ticket form with a Pending-needs-reason rule.
Each case builds (and, where present, sends and replays) over explicit ids
71 and 72; the Rust test replays the same inputs.

    cd /tmp/fsv-py-<sha> && FSV_PY_SRC=$PWD FSV_PY_COMMIT=<sha> python \
        port/golden/gen_fsv_api_ticket_bulk_set.py
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

from fsv import config, formmeta, journal, ticket_bulk as flow  # noqa: E402
from fsv.client import Client  # noqa: E402
from provenance import stamped  # noqa: E402

formmeta._save = lambda *a, **k: None
formmeta.configured_workspace = lambda: 1


def form_field(id_, name, label, field_type, position, **extra):
    default = field_type.startswith("default_")
    out = {"id": id_, "name": name, "label": label, "description": "", "field_type": field_type,
           "required": False, "required_for_closure": False, "position": position,
           "default_field": default, "choices": [], "nested_fields": [],
           "placeholder": "", "helptext": "", "hint": False,
           "content": None, "displayed_to_agents": True, "data_source": 0,
           "lookup_config": None, "workspace_id": 1 if default else 2,
           "updated_at": "2030-01-02T03:04:05Z"}
    out.update(extra)
    return out


FIELDS = [form_field(9000007101, "status", "Status", "default_status", 1,
                     choices=[{"id": 2, "value": "Open"}, {"id": 3, "value": "Pending"},
                              {"id": 4, "value": "Resolved"}, {"id": 5, "value": "Closed"}]),
          form_field(9000007103, "reason", "Pending Reason", "custom_text", 3)]
RULES = {"business_rules": [{
    "id": 9000007107, "name": "Pending needs reason", "workspace_id": 1, "module_type": "ticket",
    "item_id": None, "deleted": False, "user_type": "both", "form_type": "both", "state": "active",
    "rule_type": "client_side", "description": "",
    "condition_data": {"all": [{"name": "status", "operator": "is", "value": "3", "evaluate_on": "ticket"}]},
    "action_data": {"action": [{"name": ["customFields.reason"], "property": "mandate"}]},
    "is_reversible": False, "is_validation": False, "updated_at": "2030-01-03T00:00:00Z"}]}


def client(fail_on, writes):
    rows = {i: {"id": i, "subject": f"Acme {i}", "status": 2, "custom_fields": {"reason": None}} for i in (71, 72)}

    def handle(req):
        path = req.url.path
        if path == "/api/v2/agents/me":
            return httpx.Response(200, json={"id": 901})
        suffix = path.removeprefix("/api/_/")
        if suffix == "ticket_form_fields":
            return httpx.Response(200, json={"ticket_fields": FIELDS})
        if suffix.endswith("business_rules"):
            return httpx.Response(200, json=RULES)
        if suffix.startswith("tickets/"):
            tid = int(suffix.split("/")[1])
            if req.method == "GET":
                return httpx.Response(200, json={"ticket": copy.deepcopy(rows[tid])})
            if req.method == "PUT":
                writes.append({"method": "PUT", "id": tid, "raw": req.content.decode()})
                if tid == fail_on:
                    return httpx.Response(400, json={"errors": [{"field": "status", "message": "Synthetic rejection"}]})
                for k, v in json.loads(req.content).items():
                    if k == "custom_fields":
                        rows[tid][k].update(v)
                    elif k != "br_validation_excludes":
                        rows[tid][k] = v
                return httpx.Response(200, json={"ticket": copy.deepcopy(rows[tid])})
        return httpx.Response(404, json="missing")

    c = Client.__new__(Client)
    c._client = httpx.Client(transport=httpx.MockTransport(handle), follow_redirects=False)
    c._csrf = "synthetic"
    c._fw_domain = c._fw_session_id = None
    c._persist = False
    c._rl_rem = c._rl_tot = None
    c._probe_ok_until = 0
    c._probe_session = lambda: True
    return c


def err(exc):
    return {"error": {"type": type(exc).__name__, "msg": str(exc)}}


CASES = [
    ("no_set", "status", "Pending", None, None),
    ("set", "status", "Pending", {"Pending Reason": "Waiting on Acme vendor"}, None),
    ("set_unicode", "status", "Pending", {"Pending Reason": "Wartet auf Acme – Δ"}, None),
    ("set_internal_name", "status", "Pending", {"reason": "Waiting"}, None),
    ("fail_first", "status", "Pending", {"Pending Reason": "Waiting"}, 71),
    ("fail_second", "status", "Pending", {"Pending Reason": "Waiting"}, 72),
    ("status_key", "status", "Pending", {" Status ": "Open"}, None),
    ("spam_with_set", "spam", "Pending", {"Pending Reason": "x"}, None),
    ("empty_answers_open", "status", "Open", {}, None),
    ("plain_open", "status", "Open", None, None),
]


def run(name, action, status, answers, fail_on):
    os.environ["XDG_STATE_HOME"] = tempfile.mkdtemp(prefix="fsv-state-")
    config.DOMAIN = "acme.freshservice.com"
    config.API_INT = "https://acme.freshservice.com/api/_"
    config.API_V2 = "https://acme.freshservice.com/api/v2"
    writes = []
    c = client(fail_on, writes)
    case = {"name": name, "action": action, "status": status, "answers": answers, "fail_on": fail_on}
    try:
        plan = flow.build_plan(c, ["71", "72"], action=action, status=status if action == "status" else None,
                               answers=answers)
    except Exception as exc:
        case["build"] = err(exc)
        case["writes"] = writes
        return case
    w = plan.write
    case["build"] = {"write": {"action": w.action, "target": w.target, "tenant": w.tenant, "method": w.method,
                               "path": w.path, "payload_json": w.payload_json, "context_hash": w.context_hash},
                     "write_hash": w.hash, "intent_json": plan.intent_json, "ids": list(plan.ids),
                     "expected_json": plan.expected_json, "selected_json": plan.selected_json,
                     "hash": plan.hash, "review": plan.review()}
    if name in {"set", "fail_first", "fail_second"}:
        try:
            case["send"] = {"result": flow.send(plan, c, reviewed_hash=plan.hash)}
        except Exception as exc:
            case["send"] = err(exc)
        try:
            flow.send(plan, c, reviewed_hash=plan.hash)
            case["replay"] = {"result": "sent"}
        except Exception as exc:
            case["replay"] = err(exc)
        case["journal"] = [r["event"] for r in journal.read() if r.get("plan") == w.hash]
    case["writes"] = writes
    return case


json.dump(stamped({"cases": [run(*c) for c in CASES]}), sys.stdout, indent=1, ensure_ascii=False)
