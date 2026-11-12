"""Golden closure conversations (Python 41783e2) for fsv-api
`conversations::{status_fields, build_plan, send}`: replies/notes that set
Resolved or Closed. Mirrors `test_ticket_closure_conversation.py` over the
`test_ticket_properties_http.wire` handler; the Rust test replays the ops.

    cd /tmp/fsv-py-41783e2 && FSV_PY_SRC=$PWD FSV_PY_COMMIT=41783e2... uv run python \
        /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_closure_conversation.py
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

from fsv import config, formmeta, journal, ticket_conversation as conv  # noqa: E402
from fsv.client import Client  # noqa: E402
from provenance import stamped  # noqa: E402

formmeta._save = lambda *a, **k: None
formmeta.configured_workspace = lambda: 1
TENANTS = {"acme": ("acme.example.com", 123, 2, 3, 9000008101),
           "globex": ("globex.example.com", 742, 17, 43, 9000008201)}


def world(tenant, label):
    domain, cid, opened, pending, fid = TENANTS[tenant]
    w = {"cid": cid, "failure": None, "conv_readback": None, "conversation": None,
         "conv_id": cid + 9000000000, "requests": [],
         "row": {"id": cid, "subject": "Acme review", "status": opened, "tags": ["before"],
                 "planned_start_date": None, "planned_end_date": None},
         "fields": [{"id": fid, "name": "subject", "label": "Subject", "field_type": "default_subject", "position": 1},
                    {"id": fid + 1, "name": "status", "label": "Status", "field_type": "default_status", "position": 2,
                     "choices": [{"id": opened, "value": "Open"}, {"id": pending, "value": "Pending"}]}]}
    if label is not None:
        w["fields"][1]["choices"].append({"id": pending + 100, "value": label})
        w["row"].update(group_id=None, responder_id=9000001234,
                        resolution_notes={"notes_html": "<p>Acme recovery verified</p>", "user": {"name": "Acme Operator"}},
                        custom_fields={"summary": "Existing fix"})
        w["fields"].append({"id": 9000008910, "name": "summary", "label": "Resolution Summary",
                            "field_type": "custom_text", "position": 3, "required_for_closure": True})
    return w


def handler_for(w):
    cid = w["cid"]

    def handler(req):
        path = req.url.path.removeprefix("/api/_/")
        w["requests"].append({"method": req.method, "path": path,
                              "raw": req.content.decode() if req.method in {"PUT", "POST"} else None})
        if req.method in {"PUT", "POST"}:
            if w["failure"] == "disconnect":
                raise httpx.ReadError("synthetic disconnect", request=req)
            if w["failure"]:
                return httpx.Response(w["failure"], json={"error": "synthetic denial"})
            payload = json.loads(req.content)
            w["conversation"] = {"id": w["conv_id"], "body": payload["body"]}
            for key in ("cc_emails", "bcc_emails", "private", "notify_emails", "from_email"):
                if key in payload:
                    w["conversation"][key] = payload[key]
            if "helpdesk_ticket" in payload:
                w["row"]["status"] = payload["helpdesk_ticket"]["status"]
                if "private" in payload:
                    w["conversation"]["to_emails"] = payload.get("notify_emails", [])
            return httpx.Response(200, json={"conversation": {"id": w["conv_id"]}})
        if path == "ticket_form_fields":
            return httpx.Response(200, json={"ticket_fields": w["fields"]})
        if path.endswith("business_rules"):
            return httpx.Response(200, json={"business_rules": []})
        if path == f"tickets/{cid}/conversations":
            c = dict(w["conversation"]) if w["conversation"] else None
            if c and w["conv_readback"] == "wrong-id":
                c["id"] += 1
            elif c and w["conv_readback"]:
                if w["conv_readback"] == "contradict":
                    c["body"] = "different"
                else:
                    c.pop(w["conv_readback"], None)
            return httpx.Response(200, json={"conversations": [c] if c else [], "meta": {"count": 1}})
        if path == f"tickets/{cid}":
            return httpx.Response(200, json={"ticket": dict(w["row"])})
        return httpx.Response(404, json={"error": "unexpected"})
    return handler


def client(handler):
    c = Client.__new__(Client)
    c._client = httpx.Client(transport=httpx.MockTransport(handler), follow_redirects=False)
    c._csrf = "synthetic-token"
    c._fw_domain = c._fw_session_id = None
    c._persist = False
    c._rl_rem = c._rl_tot = None
    c._probe_ok_until = 0
    c._probe_session = lambda: True
    return c


def run(case):
    os.environ["XDG_STATE_HOME"] = tempfile.mkdtemp(prefix="fsv-state-")
    tenant = case.get("tenant", "acme")
    config.DOMAIN = TENANTS[tenant][0]
    config.API_INT = f"https://{config.DOMAIN}/api/_"
    w = world(tenant, case.get("label"))
    c = client(handler_for(w))
    plan = None
    outputs = []
    for op in case["ops"]:
        try:
            if op["op"] == "build":
                plan = conv.build_plan(c, w["cid"], op["body"], kind=op["kind"], set_status=op["status"])
                out = {"payload_json": plan.payload_json, "record_hash": plan.record_hash,
                       "status_label": plan.status_label, "hash": plan.hash, "review": plan.review()}
            elif op["op"] == "send":
                out = {"result": conv.send(plan, c, reviewed_hash=plan.hash,
                                           public_confirmed=op.get("public", False))}
            elif op["op"] == "set":
                for k, v in op.get("world", {}).items():
                    w[k] = v
                for k, v in op.get("custom", {}).items():
                    w["row"]["custom_fields"][k] = v
                if "summary_label" in op:
                    w["fields"][-1]["label"] = op["summary_label"]
                out = {}
            else:
                raise AssertionError(op)
        except Exception as exc:  # noqa: BLE001
            out = {"error": {"type": type(exc).__name__, "msg": str(exc)}}
        out["posts"] = len([r for r in w["requests"] if r["method"] == "POST"])
        outputs.append(out)
    c.close()
    events = [e["event"] for e in journal.read() if plan is not None and e.get("plan") == plan.hash]
    return {**case, "outputs": outputs, "events": events,
            "writes": [r for r in w["requests"] if r["method"] != "GET"]}


CASES = {}
for tenant in TENANTS:
    for kind in ["note", "reply"]:
        for label in ["Resolved", "Closed"]:
            CASES[f"closure_{tenant}_{kind}_{label}"] = {"tenant": tenant, "label": label, "ops": [
                {"op": "build", "kind": kind, "status": label, "body": "Acme self-only update"},
                {"op": "send", "public": kind == "reply"}, {"op": "send", "public": True}]}
for missing in ["body", "to_emails", "wrong-id", "contradict"]:
    CASES[f"ambiguous_{missing}"] = {"label": "Resolved", "ops": [
        {"op": "build", "kind": "note", "status": "Resolved", "body": "Acme update"},
        {"op": "set", "world": {"conv_readback": missing}}, {"op": "send"}, {"op": "send"}]}
for failure in [403, 429, 500, "disconnect"]:
    CASES[f"transport_{failure}"] = {"tenant": "globex", "label": "Closed", "ops": [
        {"op": "build", "kind": "note", "status": "Closed", "body": "Acme update"},
        {"op": "set", "world": {"failure": failure}}, {"op": "send"}, {"op": "send"}]}
CASES["requirements_and_stale_metadata"] = {"label": "Resolved", "ops": [
    {"op": "set", "custom": {"summary": ""}},
    {"op": "build", "kind": "note", "status": "Resolved", "body": "Acme update"},
    {"op": "set", "custom": {"summary": "Fixed"}},
    {"op": "build", "kind": "note", "status": "Resolved", "body": "Acme update"},
    {"op": "set", "summary_label": "Changed requirement"}, {"op": "send"}]}
CASES["no_persisted_resolution"] = {"label": "Closed", "ops": [
    {"op": "set", "world": {"row": {"id": 123, "subject": "Acme review", "status": 2, "group_id": None,
                                    "responder_id": 9000001234, "custom_fields": {"summary": "x"}}}},
    {"op": "build", "kind": "reply", "status": "Closed", "body": "Acme update"}]}
CASES["pending_note_unchanged"] = {"ops": [
    {"op": "build", "kind": "note", "status": "Pending", "body": "Acme update"}, {"op": "send"}]}
CASES["numeric_status_refused"] = {"label": "Closed", "ops": [
    {"op": "build", "kind": "note", "status": "3", "body": "Acme update"}]}
json.dump(stamped({name: run(case) for name, case in CASES.items()}), sys.stdout, indent=1, ensure_ascii=False)
