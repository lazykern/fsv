"""Golden outputs for fsv-app's reviewed TUI hooks (`crates/fsv-app/src/tui/tests_reviewed.rs`).

Runs the Python TUI helpers' bodies (`FsvApp._prepare_ticket_conversation`,
`_send_ticket_conversation`, `_load_ticket_property_fields`,
`_send_ticket_property`) over a mocked tenant that the Rust test routes the
same way. Writes `crates/fsv-app/src/tui/testdata/reviewed_expected.json`:
the tenant fixture, then per scenario the request lines, plans, results and
error texts.

    cd /Users/phusit.somb/lab/work/fsv && uv run python \
        /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_app_tui_reviewed.py
"""
from __future__ import annotations

import json
import os
import pathlib
import shutil
import tempfile
from urllib.parse import parse_qsl

ROOT = tempfile.mkdtemp(prefix="fsv-golden-")
os.environ["FSV_CONFIG_DIR"] = os.path.join(ROOT, "config")
os.environ["FSV_CACHE_DIR"] = os.path.join(ROOT, "cache")
os.environ["XDG_STATE_HOME"] = os.path.join(ROOT, "state")
os.environ["FSV_NO_KEYCHAIN"] = "1"
os.environ.pop("FSV_TZ", None)
os.makedirs(os.environ["FSV_CONFIG_DIR"])
with open(os.path.join(os.environ["FSV_CONFIG_DIR"], "config.json"), "w") as f:
    f.write('{"domain": "acme.freshservice.com"}')

import httpx  # noqa: E402

from fsv import asset_picker, config, planning_text, problem_known_error  # noqa: E402
from fsv import ticket_conversation as conversations, ticket_properties  # noqa: E402
from fsv.create import match_planning_row  # noqa: E402
from fsv import client as client_mod  # noqa: E402
from fsv.formmeta import load_meta  # noqa: E402
from provenance import stamped  # noqa: E402

config.reload()

OUT = pathlib.Path(__file__).resolve().parents[2] / "crates/fsv-app/src/tui/testdata/reviewed_expected.json"
DASHBOARD = '<html><head><meta name="csrf-token" content="tok"></head><body>app</body></html>'


def field(i, name, label, ft, extra=None):
    default = ft.startswith("default_")
    out = {"id": i, "name": name, "label": label, "description": "", "field_type": ft, "required": False,
           "required_for_closure": False, "position": i, "default_field": default, "choices": [],
           "nested_fields": [], "placeholder": "", "helptext": "", "hint": False, "content": None,
           "displayed_to_agents": True, "data_source": 0, "lookup_config": None,
           "workspace_id": 1 if default else 2, "updated_at": "2030-01-02T03:04:05Z"}
    out.update(extra or {})
    return out


FIELDS = [
    field(1, "subject", "Subject", "default_subject"),
    field(2, "description", "Description", "default_description"),
    field(3, "status", "Status", "default_status",
          {"choices": [{"id": 2, "value": "Open"}, {"id": 3, "value": "Pending"}, {"id": 5, "value": "Closed"}]}),
    field(4, "priority", "Priority", "default_priority",
          {"choices": [{"id": 1, "value": "Low"}, {"id": 3, "value": "High"}]}),
    field(5, "planned_start_date", "Start", "default_planned_start_date"),
    field(6, "cost_code", "Cost Code", "custom_text"),
    field(7, "notice", "Notice", "custom_content", {"content": "<p>Read me</p>"}),
    field(8, "requested_for", "Beneficiary", "default_requested_for"),
    field(9, "department", "Department", "default_department"),
    field(10, "region", "Region", "custom_dropdown",
          {"choices": [{"id": 31, "value": "North"}, {"id": 32, "value": "South"}]}),
]
CHANGE_FIELDS = [
    {"id": 41, "name": "change_reason", "label": "Reason for Change", "field_type": "default_change_reason"},
    {"id": 42, "name": "subject", "label": "Subject", "field_type": "default_subject"},
    {"id": 43, "name": "rollout_plan", "label": "Rollout Plan", "field_type": "default_change_plan"},
]
PROBLEM = {"id": 77, "subject": "Disk", "known_error": False}
PLANNING = {
    "problems/77": [{"name": "problem_cause", "label": "Root Cause", "description": "", "attachments": []}],
    "changes/5": [{"name": "rollout_plan", "label": "Rollout Plan", "description": "", "attachments": []}],
}
ASSETS = [{"display_id": 7, "name": "Acme DB"}, {"display_id": 8, "name": "Globex"}]
ROW = {"id": 123, "subject": "Test", "status": 2, "priority": 1, "tags": ["old"],
       "planned_start_date": None, "planned_end_date": None, "requested_for_id": None,
       "custom_fields": {"cost_code": None, "region": None}}


class Tenant:
    def __init__(self) -> None:
        self.requests: list[httpx.Request] = []
        self.row = json.loads(json.dumps(ROW))
        self.convs: list[dict] = []
        self.problem = dict(PROBLEM)
        self.planning = json.loads(json.dumps(PLANNING))

    def __call__(self, req: httpx.Request) -> httpx.Response:
        self.requests.append(req)
        p = req.url.path
        if p.startswith("/a/"):
            return httpx.Response(200, text=DASHBOARD, headers={"content-type": "text/html"})
        if p == "/search/autocomplete/requesters":
            q = req.url.params["q"]
            return httpx.Response(200, json={"results": [{"id": 11, "value": "Acme User", "email": q}]})
        p = p.removeprefix("/api/_/")
        body = json.loads(req.content) if req.content else None
        if req.method == "GET" and p == "ticket_form_fields":
            return httpx.Response(200, json={"ticket_fields": FIELDS})
        if req.method == "GET" and p.endswith("business_rules"):
            return httpx.Response(200, json={"business_rules": []})
        if req.method == "GET" and p == "tickets/123":
            return httpx.Response(200, json={"ticket": self.row})
        if req.method == "GET" and p == "tickets/123/conversations":
            return httpx.Response(200, json={"conversations": self.convs, "meta": {"count": len(self.convs)}})
        if req.method == "POST" and p == "tickets/123/reply":
            self.convs.append({"id": 456, "body": body["body"], "cc_emails": body.get("cc_emails", []),
                               "bcc_emails": body.get("bcc_emails", [])})
            if "helpdesk_ticket" in body:
                self.row["status"] = body["helpdesk_ticket"]["status"]
            return httpx.Response(200, json={"conversation": {"id": 456}})
        if req.method == "POST" and p == "tickets/123/notes":
            self.convs.append({"id": 457, "body": body["body"], "private": body["private"],
                               "notify_emails": body["notify_emails"]})
            return httpx.Response(200, json={"note": {"id": 457}})
        if req.method == "PUT" and p == "tickets/123":
            self.row.update({k: v for k, v in body.items() if k not in ("br_validation_excludes", "custom_fields")})
            self.row["custom_fields"].update(body.get("custom_fields", {}))
            return httpx.Response(200, json={"ticket": self.row})
        if p == "problems/77":
            if req.method == "PUT":
                self.problem.update(body)
            return httpx.Response(200, json={"problem": self.problem})
        if p.endswith("/planning-fields"):
            rec = p.removesuffix("/planning-fields")
            if rec in self.planning:
                if req.method == "GET":
                    key = "change_planning_fields" if rec.startswith("changes") else "problem_planning_fields"
                    return httpx.Response(200, json={key: self.planning[rec]})
                for row in self.planning[rec]:
                    if row["name"] == req.url.params["id"]:
                        row["description"] = body["description"]
                return httpx.Response(200, json={"planning_field": {"id": 1}})
        if req.method == "GET" and p == "assets-to-associate":
            return httpx.Response(200, json={"assets": ASSETS})
        return httpx.Response(404, json={"description": "not found"})

    def client(self):
        c = client_mod.Client(cookies={"_x_w": "w", "fw-session-id": "fw1"})
        c._client.close()
        c._client = httpx.Client(transport=httpx.MockTransport(self), follow_redirects=False)
        return c

    def lines(self, start=0):
        out = []
        for r in self.requests[start:]:
            if r.url.path.startswith("/a/"):
                continue
            q = "&".join(f"{k}={v}" for k, v in parse_qsl(r.url.query.decode(), keep_blank_values=True))
            out.append(f"{r.method} {r.url.path}" + (f"?{q}" if q else ""))
        return out

    def bodies(self, start=0):
        return [r.content.decode() for r in self.requests[start:] if r.method in ("POST", "PUT")]


def reset():
    shutil.rmtree(os.environ["XDG_STATE_HOME"], ignore_errors=True)
    shutil.rmtree(os.environ["FSV_CACHE_DIR"], ignore_errors=True)


def prepare(c, ticket_id, body, public, reply, options):
    """`FsvApp._prepare_ticket_conversation` without the worker."""
    def emails(key):
        return [v.strip() for v in options.get(key, "").split(",") if v.strip()]
    cc = conversations.recipients(c, emails("cc"))
    notify = conversations.recipients(c, emails("notify"))
    bcc = tuple(emails("bcc"))
    conversations.recipients(c, list(bcc))
    return conversations.build_plan(c, ticket_id, body, kind="reply" if reply else "note", public=public,
                                    cc=cc, notify=notify, bcc=bcc, sender=options.get("sender") or None,
                                    set_status="Pending" if options.get("pending") else None)


def conversation(name, body, public, reply, options, public_confirmed):
    reset()
    t = Tenant()
    c = t.client()
    plan = prepare(c, 123, body, public, reply, options)
    n = len(t.requests)
    result = conversations.send(plan, c, reviewed_hash=plan.hash, public_confirmed=public_confirmed)
    return {"plan_lines": t.lines(0)[:len(t.lines(0)) - len(t.lines(n))], "payload_json": plan.payload_json,
            "hash": plan.hash, "review": plan.review(), "send_lines": t.lines(n), "send_bodies": t.bodies(n),
            "result": result}


out = {"fields": FIELDS, "row": ROW}
out["reply"] = conversation("reply", "Acme update", False, True,
                            {"cc": "person@example.com", "pending": True}, True)
out["reply_bcc"] = conversation("reply_bcc", "Acme bcc", False, True,
                                {"bcc": "x@example.com", "sender": "help@acme.example.com"}, True)
out["note"] = conversation("note", "Acme note", False, False, {"notify": "b@example.com"}, False)

refusals = []
for reply, options in [
    (False, {"sender": "help@acme.example.com"}),
    (False, {"bcc": "x@example.com"}),
    (True, {"sender": "Help <help@acme.example.com>"}),
    (True, {"bcc": "x@example.com", "cc": "person@example.com"}),
    (True, {"bcc": "x@example.com", "pending": True}),
    (True, {"bcc": "x@example.com"}),
    (True, {"bcc": "Ann <x@example.com>", "sender": "help@acme.example.com"}),
    (True, {"bcc": "x@example.com", "sender": "help@acme.example.com"}),
    (True, {"sender": "help@acme.example.com"}),
]:
    reset()
    t = Tenant()
    try:
        plan = prepare(t.client(), 123, "Acme", False, reply, options)
        error = None
        payload = plan.payload
    except Exception as e:  # noqa: BLE001
        error, payload = str(e), None
    refusals.append({"reply": reply, "options": options, "error": error, "payload": payload, "lines": t.lines(),
                     "hash": None if error else plan.hash, "review": None if error else plan.review()})
out["refusals"] = refusals

# `_load_ticket_property_fields`
reset()
t = Tenant()
meta = load_meta(t.client(), "ticket", "edit", fresh=True, module_id=123)
core = {"subject", "description", "requester", "requested_for", "group", "agent", "status",
        "priority", "urgency", "impact", "source", "category", "ticket_type"}
fields = [f for f in meta.spec.fields if (f.sent or f.field_type == "default_requested_for") and
          not f.synthetic and f.is_input and f.kind != "unsupported" and
          (f.is_custom or f.api_name in core)]
out["property_fields"] = {
    "lines": t.lines(),
    "fields": [{"name": f.api_name, "label": f.label,
                "choices": [[ch.value, ch.label] for ch in f.choices if ch.visible] if f.kind == "dropdown" else None}
               for f in fields],
}

# `_send_ticket_property` on a flag-built plan (the plan the CLI's --tag makes).
reset()
t = Tenant()
c = t.client()
plan = ticket_properties.build_plan(c, 123, tags=["Acme", "one"])
n = len(t.requests)
result = ticket_properties.send(plan, c, reviewed_hash=plan.hash)
out["property_send"] = {"plan_lines": t.lines(0)[:len(t.lines(0)) - len(t.lines(n))], "hash": plan.hash,
                        "review": plan.review(), "body_json": plan.body_json, "before_json": plan.before_json,
                        "record_hash": plan.record_hash, "send_lines": t.lines(n), "send_bodies": t.bodies(n),
                        "result": result}

# The TUI's `answers={name: value}` path refuses the fields the old flags
# cover, so a flag-built plan cannot stand in for it.
answers = {}
for name, value in [("planned_start_date", "2030-01-02T09:00:00-05:00"), ("tags", "Acme")]:
    reset()
    t = Tenant()
    try:
        answers[name] = {"hash": ticket_properties.build_plan(t.client(), 123, answers={name: value}).hash}
    except Exception as e:  # noqa: BLE001
        answers[name] = {"error": str(e)}
out["answers_for_flag_fields"] = answers



def reviewed(name, build, send):
    """A `WritePlan` hook pair: build, review, send with the reviewed hash."""
    reset()
    t = Tenant()
    c = t.client()
    try:
        plan = build(c)
    except Exception as e:  # noqa: BLE001
        return {"error": str(e), "lines": t.lines()}
    n = len(t.requests)
    try:
        result, error = send(plan, c), None
    except Exception as e:  # noqa: BLE001
        result, error = None, str(e)
    fields = {k: getattr(plan, k) for k in ("action", "target", "method", "path", "payload_json", "context_hash")
              if hasattr(plan, k)}
    return {"plan_lines": t.lines(0)[:len(t.lines(0)) - len(t.lines(n))], "hash": plan.hash,
            "review": plan.review(), **fields,
            "send_lines": t.lines(n), "send_bodies": t.bodies(n), "result": result, "send_error": error}


# `_prepare_ticket_property` / `_send_ticket_property` with the form's answers.
choice = {f["name"]: dict((label, value) for value, label in f["choices"] or [])
          for f in out["property_fields"]["fields"]}
property_answers = {}
for name, value in [("subject", "Globex"), ("priority", choice["priority"]["High"]), ("cost_code", "CC-9"),
                    ("region", choice["region"]["South"]), ("requested_for", "person@example.com"),
                    ("status", choice["status"]["Closed"]), ("planned_start_date", "2030-01-02T09:00:00-05:00")]:
    property_answers[name] = reviewed(
        name, lambda c, name=name, value=value: ticket_properties.build_plan(c, 123, answers={name: value}),
        lambda plan, c: ticket_properties.send(plan, c, reviewed_hash=plan.hash))
    property_answers[name]["value"] = value
out["property_answers"] = property_answers

# `_prepare_known_error` / `_send_known_error`.
out["known_error"] = reviewed("known_error", lambda c: problem_known_error.build_plan(c, 77),
                              lambda plan, c: problem_known_error.send(plan, c, reviewed_hash=plan.hash))

# `_load_planning_sections` / `_load_planning_editor` / `_prepare_planning_text` / `_send_planning_text`.
schema_load = planning_text.schema.load
planning_text.schema.load = lambda res, client=None, force=False: {"fields": CHANGE_FIELDS}
planning = {}
for module, target, section, description in [("problems", 77, "problem_cause", "<p>new cause</p>"),
                                             ("changes", 5, "rollout_plan", "<p>v2</p>")]:
    reset()
    t = Tenant()
    c = t.client()
    defs = planning_text.definitions(c, module)
    rows = planning_text.inventory(c, module, target)
    entry = {"definitions": defs, "row": match_planning_row({"name": section}, rows),
             "missing_row": match_planning_row({"name": "nope"}, rows), "read_lines": t.lines()}
    entry.update(reviewed(module, lambda c, m=module, i=target, s_=section, d=description:
                          planning_text.build_plan(c, m, i, s_, d),
                          lambda plan, c: planning_text.send(plan, c, reviewed_hash=plan.hash)))
    planning[module] = entry
try:
    planning_text.definitions(Tenant().client(), "tickets")
except Exception as e:  # noqa: BLE001
    planning["tickets_error"] = str(e)
planning_text.schema.load = schema_load
out["planning"] = planning
out["change_fields"] = CHANGE_FIELDS

# `_load_asset_candidates`.
reset()
t = Tenant()
out["assets"] = {"result": asset_picker.candidates(t.client(), 5, "acme"), "lines": t.lines()}

OUT.parent.mkdir(parents=True, exist_ok=True)
OUT.write_text(json.dumps(stamped(out), indent=2, ensure_ascii=False) + "\n")
print(OUT)
