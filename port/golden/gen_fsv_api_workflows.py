"""Expected values for the fsv-api reviewed workflows: canned, scenarios,
private_views, list_preferences, problem_workflows (and the readback helpers
they use).

Each scenario drives a real `fsv.client.Client` over an httpx MockTransport,
with `me` and `_get_csrf` patched as in the Python tests and a fresh journal.
The Rust tests (`crates/fsv-api/src/workflows_tests.rs`) route an `FnApi`
the same way and compare request logs, plans, results and error texts.

    cd /Users/phusit.somb/lab/work/fsv && uv run python \
        ../fsv-rust/port/golden/gen_fsv_api_workflows.py \
        > ../fsv-rust/crates/fsv-api/src/testdata/workflows_oracle.json
"""
from __future__ import annotations

import copy
import json
import os
import pathlib
import shutil
import tempfile

ROOT = tempfile.mkdtemp(prefix="fsv-golden-")
os.environ["FSV_CONFIG_DIR"] = os.path.join(ROOT, "config")
os.environ["FSV_CACHE_DIR"] = os.path.join(ROOT, "cache")
os.environ["XDG_STATE_HOME"] = os.path.join(ROOT, "state")
os.environ["FSV_NO_KEYCHAIN"] = "1"
os.makedirs(os.environ["FSV_CONFIG_DIR"])
with open(os.path.join(os.environ["FSV_CONFIG_DIR"], "config.json"), "w") as f:
    f.write('{"domain": "acme.freshservice.com"}')

import httpx  # noqa: E402

from fsv import (canned, config, formmeta, journal, list_preferences, private_views,  # noqa: E402
                 problem_workflows, readback, scenarios, ticket_conversation as conv)
from fsv.client import Client  # noqa: E402
from provenance import stamped  # noqa: E402

config.reload()
DOMAIN = "acme.freshservice.com"
assert config.DOMAIN == DOMAIN
JOURNAL = pathlib.Path(ROOT) / "journal"
journal.journal_path = lambda: JOURNAL
formmeta.configured_workspace = lambda: 17
formmeta._save = lambda *a, **k: None
formmeta._cached = lambda *a, **k: None


class Disconnect(Exception):
    pass


class Wire:
    """`handle(method, path, body, params)` returns a JSON value, an int
    status (error body `{"error": "synthetic"}`), `("html", status)` or
    raises `Disconnect`. Paths are relative to `/api/_/`; autocomplete is
    `autocomplete/<kind>`."""

    def __init__(self, handle, user=901):
        self.handle = handle
        self.user = user
        self.log = []

    def __call__(self, req):
        path = req.url.path
        if path.startswith("/search/autocomplete/"):
            p = "autocomplete/" + path.rsplit("/", 1)[1]
        else:
            p = path.removeprefix("/api/_/")
        if req.method != "GET" and req.url.query:
            p += "?" + req.url.query.decode()
        body = json.loads(req.content) if req.content else None
        self.log.append([req.method, p, body])
        try:
            out = self.handle(req.method, p, body, dict(req.url.params))
        except Disconnect:
            raise httpx.ReadError("synthetic disconnect", request=req) from None
        if isinstance(out, tuple):
            return httpx.Response(out[1], text="<html>Not found</html>")
        if isinstance(out, int):
            if out == 204:
                return httpx.Response(204)
            return httpx.Response(out, json={"error": "synthetic"})
        return httpx.Response(200, json=out)

    def client(self):
        c = Client(cookies={})
        c._client.close()
        c._client = httpx.Client(transport=httpx.MockTransport(self))
        c._get_csrf = lambda: "synthetic"
        c.me = lambda: {"id": self.user}
        return c


def reset():
    if JOURNAL.exists():
        JOURNAL.unlink()
    shutil.rmtree(os.environ["FSV_CACHE_DIR"], ignore_errors=True)


def attempt(fn):
    try:
        return {"ok": fn()}
    except Exception as e:  # noqa: BLE001
        return {"error": str(e), "type": type(e).__name__}


def events():
    return [r["event"] for r in journal.read() if str(r.get("kind", "")).startswith("plan:")]


def write_plan(p):
    out = {"hash": p.hash, "review": p.review(), "action": p.action, "target": p.target,
           "method": p.method, "path": p.path, "payload_json": p.payload_json, "context_hash": p.context_hash}
    for extra in ("inputs_json", "source_id"):
        if hasattr(p, extra):
            out[extra] = getattr(p, extra)
    return out


def sent(w, start, fn):
    """Run a send; the result or error, the requests it made and the journal."""
    n = len(w.log)
    out = attempt(fn)
    out.update(calls=w.log[n:], events=events())
    return out


# ---------------------------------------------------------------- canned

def canned_wire():
    state = {"source": {"canned_response": {"id": 701, "content": "<div>Acme response</div>"}},
             "failure": None, "writes": []}

    def handle(method, path, body, params):
        if path == "canned_responses/701":
            return copy.deepcopy(state["source"])
        if path == "canned_response_folders":
            return {"canned_response_folders": [{"id": 702}]}
        if path == "canned_response_folders/702/canned_responses":
            return {"canned_responses": [{"id": 701}]}
        if path == "scenario_automations":
            return state.get("scenarios", {"scenario_automations": [{"id": 703, "name": "Globex"}]})
        if method == "GET" and path == "tickets/123":
            return {"ticket": {"id": 123, "subject": "Acme", "status": 42}}
        if method == "PUT" and path == "tickets/execute_scenario":
            state["writes"].append(body)
            if state["failure"] == "disconnect":
                raise Disconnect
            return state["failure"] or 204
        if method == "POST" and path == "tickets/123/reply":
            state["writes"].append(body)
            if state["failure"] == "disconnect":
                raise Disconnect
            return state["failure"] or {"conversation": {"id": 704}}
        if path == "tickets/123/conversations":
            return {"conversations": [{"id": 704, **state["writes"][0]}], "has_more": False}
        return 404

    w = Wire(handle)
    return w, state


def gen_canned():
    out = {}
    reset()
    w, state = canned_wire()
    c = w.client()
    out["reads"] = {"folders": canned.folders(c), "responses": canned.responses(c, 702),
                    "show": canned.show(c, 701), "scenarios": canned.scenarios(c),
                    "scenario": canned.scenario(c, 703),
                    "scenario_missing": attempt(lambda: canned.scenario(c, 704)),
                    "calls": w.log[:]}
    state["scenarios"] = {"rows": []}
    out["reads"]["scenario_not_list"] = attempt(lambda: canned.scenario(c, 703))
    state["scenarios"] = [{"id": 703}, {"id": "703"}]
    out["reads"]["scenario_ambiguous"] = attempt(lambda: canned.scenario(c, 703))
    state["scenarios"] = [{"id": 703, "name": "bare list"}]
    out["reads"]["scenario_bare_list"] = attempt(lambda: canned.scenario(c, 703))

    contents = []
    for raw, pointer in [({"body": "{{name}}"}, "/body"), ({"body": "{% if user %}"}, "/body"),
                         ({"body": "${name}"}, "/body"), ({"body": "[[name]]"}, "/body"),
                         ({"body": ""}, "/body"), ({"body": "  "}, "/body"), ({"body": None}, "/body"),
                         ({"body": 123}, "/body"), ({"body": "Acme"}, "body"), ({"body": "Acme"}, "/missing"),
                         ({"body": "Acme"}, "/body/~2"), ({"body": {"x": "Acme"}}, "/body/x"),
                         ({"a/b": {"m~n": "Acme"}}, "/a~1b/m~0n"), ({"": "Acme"}, "/"),
                         ({"body": ["Acme"]}, "/body/0"), ({"body": "Acme"}, "/body~")]:
        contents.append({"raw": raw, "pointer": pointer, **attempt(lambda: canned.content(raw, pointer))})
    out["content"] = contents

    reset()
    w, state = canned_wire()
    c = w.client()
    n = len(w.log)
    p = canned.reply_plan(c, "INC-123", 701, "/canned_response/content", sender="support@example.com")
    out["reply"] = {"plan_calls": w.log[n:], "hash": p.hash, "payload_json": p.payload_json,
                    "review": p.review(), "canned_hash": p.canned_hash,
                    "unconfirmed": attempt(lambda: conv.send(p, c, reviewed_hash=p.hash))}
    out["reply"]["send"] = sent(w, n, lambda: conv.send(p, c, reviewed_hash=p.hash, public_confirmed=True))
    out["reply"]["resend"] = sent(w, n, lambda: conv.send(p, w.client(), reviewed_hash=p.hash,
                                                          public_confirmed=True))
    legacy = conv.build_plan(c, 123, "Acme")
    out["legacy_hash"] = legacy.hash

    reset()
    w, state = canned_wire()
    c = w.client()
    p = canned.reply_plan(c, "123", 701, "/canned_response/content", sender="support@example.com")
    state["source"]["canned_response"]["name"] = "Changed metadata"
    out["stale"] = sent(w, 0, lambda: conv.send(p, c, reviewed_hash=p.hash, public_confirmed=True))
    out["reply_refusals"] = {
        "no_sender": attempt(lambda: canned.reply_plan(c, "123", 701, "/canned_response/content",
                                                       sender="Help <x@example.com>")),
        "bad_pointer": attempt(lambda: canned.reply_plan(c, "123", 701, "/canned_response/body",
                                                         sender="support@example.com")),
        "bad_id": attempt(lambda: canned.reply_plan(c, "123", 0, "/canned_response/content",
                                                    sender="support@example.com")),
    }

    for failure in [500, 422, "disconnect"]:
        reset()
        w, state = canned_wire()
        c = w.client()
        p = canned.reply_plan(c, "123", 701, "/canned_response/content", sender="support@example.com")
        state["failure"] = failure
        out[f"reply_fail_{failure}"] = sent(w, 0, lambda: conv.send(p, c, reviewed_hash=p.hash,
                                                                     public_confirmed=True))
    return out


def gen_scenarios():
    out = {}
    reset()
    w, state = canned_wire()
    c = w.client()
    n = len(w.log)
    p = scenarios.build_plan(c, "INC-123", 703)
    out["plan"] = {"calls": w.log[n:], **write_plan(p)}
    out["cancel"] = scenarios.send(p, c, reviewed_hash=None)
    out["send"] = sent(w, 0, lambda: scenarios.send(p, c, reviewed_hash=p.hash))
    out["resend"] = sent(w, 0, lambda: scenarios.send(p, w.client(), reviewed_hash=p.hash))
    for failure in [500, 422, "disconnect"]:
        reset()
        w, state = canned_wire()
        c = w.client()
        p = scenarios.build_plan(c, "123", 703)
        state["failure"] = failure
        out[f"fail_{failure}"] = sent(w, 0, lambda: scenarios.send(p, c, reviewed_hash=p.hash))
    reset()
    w, state = canned_wire()
    c = w.client()
    out["refusals"] = {"missing_scenario": attempt(lambda: scenarios.build_plan(c, "123", 9)),
                       "bad_ticket": attempt(lambda: scenarios.build_plan(c, "CHN-1", 703)),
                       "zero_scenario": attempt(lambda: scenarios.build_plan(c, "123", 0))}
    return out


# ---------------------------------------------------------- private views

VIEW = {"id": 701, "user_id": 901, "default": False, "visibility": 81, "visibility_name": "Me",
        "name": "Acme source", "workspace_id": 17,
        "query_hash": [{"condition": "status", "operator": "is_in", "type": "default", "value": ["42"]}],
        "advanced_query_hash": None, "order_by": "created_at", "order_type": "desc"}


def view_wire(source=None):
    original = copy.deepcopy(source or VIEW)
    state = {"rows": {original["id"]: original}, "failure": None, "partial": False}

    def handle(method, path, body, params):
        leaf = path.split("?")[0].rsplit("/", 1)[-1]
        if method == "GET":
            if leaf == "ticket_filters":
                return {"ticket_filters": list(state["rows"].values()), "has_more": state["partial"]}
            row = state["rows"].get(int(leaf))
            if row is None:
                return ("html", 404)
            return {"ticket_filter": copy.deepcopy(row)}
        if state["failure"] == "disconnect":
            raise Disconnect
        if state["failure"]:
            return state["failure"]
        if method == "DELETE":
            del state["rows"][int(leaf)]
            return 204
        vid = 702 if method == "POST" else int(leaf)
        state["rows"][vid] = {**original, **body, "id": vid}
        return {"ticket_filter": {"id": vid}}

    return Wire(handle), state


def gen_private_views():
    out = {}
    reset()
    w, state = view_wire()
    c = w.client()
    life = []
    for source, action, name in [(701, "create", "Globex clone"), (702, "edit", "Globex renamed"),
                                 (702, "delete", None)]:
        n = len(w.log)
        p = private_views.build_plan(c, source, action=action, name=name)
        entry = {"source": source, "action": action, "name": name, "plan_calls": w.log[n:], **write_plan(p)}
        entry["send"] = sent(w, 0, lambda: private_views.send(p, c, reviewed_hash=p.hash))
        life.append(entry)
    out["lifecycle"] = life

    reset()
    w, state = view_wire()
    c = w.client()
    p = private_views.build_plan(c, 701, action="delete")
    state["partial"] = True
    out["delete_partial"] = sent(w, 0, lambda: private_views.send(p, c, reviewed_hash=p.hash))

    reset()
    w, state = view_wire()
    c = w.client()
    p = private_views.build_plan(c, 701, action="create", name="Globex")
    state["failure"] = 500
    out["create_fail_500"] = sent(w, 0, lambda: private_views.send(p, c, reviewed_hash=p.hash))

    refusals = []
    for mods, action, name in [({}, "rename", "x"), ({}, "delete", "x"), ({}, "create", " "),
                               ({}, "edit", None), ({"user_id": 902}, "create", "x"),
                               ({"default": None}, "create", "x"), ({"system": True}, "create", "x"),
                               ({"visibility_name": "Everyone"}, "create", "x"),
                               ({"visibility_name": None}, "create", "x"),
                               ({"visibility": {"id": 5, "label": " Only Me "}}, "create", "x"),
                               ({"visibility": {"id": 5, "label": "Agents"}}, "create", "x"),
                               ({"advanced_query_hash": []}, "create", "x"),
                               ({"query_hash": "x"}, "create", "x"), ({"order_by": "updated_at"}, "edit", "x"),
                               ({"order_type": "asc"}, "create", "x"), ({"id": 9}, "delete", None),
                               ({"workspace_id": None}, "create", "x"), ({"system": False}, "create", "x")]:
        reset()
        w, state = view_wire({**VIEW, **mods} if "id" not in mods else {**VIEW})
        if "id" in mods:
            state["rows"][701]["id"] = mods["id"]
        c = w.client()
        got = attempt(lambda: private_views.build_plan(c, 701, action=action, name=name))
        if "ok" in got:
            got = {"ok": write_plan(got["ok"])}
        refusals.append({"mods": mods, "action": action, "name": name, **got})
    out["refusals"] = refusals
    return out


# ------------------------------------------------------- list preferences

HEADERS = [{"name": key, "label": label, "position": i + 1, "selected": True, "frozen": key == "display_id"}
           for i, (key, label) in enumerate([("ticket_subject", "Acme subject"), ("requester", "Acme requester"),
                                             ("status", "Acme state"), ("priority", "Acme priority"),
                                             ("display_id", "Acme ID"), ("assigned_to", "Acme agent")])]
FILTER = {"id": 701, "name": "Acme queue",
          "query_hash": [{"condition": "status", "operator": "is_in", "value": ["47"], "type": "default"}],
          "advanced_query_hash": None}
FILTERS = {"tickets": "ticket_filters", "changes": "change-filters", "problems": "problem-filters"}


def list_wire(module):
    state = {"headers": copy.deepcopy(HEADERS), "row": copy.deepcopy(FILTER), "failure": None,
             "readback": "ok", "writes": 0}
    filters = FILTERS[module]

    def handle(method, path, body, params):
        if method == "GET":
            if path == f"{module}/headers":
                rows = copy.deepcopy(state["headers"])
                if state["writes"] and state["readback"] == "missing":
                    rows[0].pop("frozen")
                return {"headers": rows}
            if path == filters:
                return {"filters": [state["row"]]}
            if path == filters + "/701":
                return {module[:-1] + "_filter": state["row"]}
            return 404
        state["writes"] += 1
        if state["failure"] == "disconnect":
            raise Disconnect
        if state["failure"]:
            return state["failure"]
        if path.endswith("/headers"):
            for header in state["headers"]:
                header.update(selected=header["name"] in body["columns"],
                              position=body["columns"].get(header["name"]),
                              frozen=header["name"] in body["frozen_columns"])
        return 204

    return Wire(handle), state


def gen_list_preferences():
    out = {}
    for module in ["tickets", "changes", "problems"]:
        m = {}
        reset()
        w, state = list_wire(module)
        c = w.client()
        p = list_preferences.columns_plan(c, module, ["ticket_subject", "requester", "status"], [])
        m["columns"] = {"plan_calls": w.log[:], **write_plan(p)}
        m["columns"]["send"] = sent(w, 0, lambda: list_preferences.send(p, c, reviewed_hash=p.hash))
        m["columns"]["resend"] = sent(w, 0, lambda: list_preferences.send(p, w.client(), reviewed_hash=p.hash))

        reset()
        w, state = list_wire(module)
        c = w.client()
        fields = ["subject", "status_name", "display_id"]
        p = list_preferences.export_plan(c, module, "701", fields)
        m["export"] = {"plan_calls": w.log[:], **write_plan(p)}
        m["export"]["send"] = sent(w, 0, lambda: list_preferences.send(p, c, reviewed_hash=p.hash))

        reset()
        w, state = list_wire(module)
        c = w.client()
        p = list_preferences.columns_plan(c, module, ["ticket_subject", "status"], [])
        state["readback"] = "missing"
        m["columns_missing"] = sent(w, 0, lambda: list_preferences.send(p, c, reviewed_hash=p.hash))

        reset()
        w, state = list_wire(module)
        c = w.client()
        p = list_preferences.columns_plan(c, module, ["ticket_subject"], [])
        state["headers"][0]["label"] = "Drifted"
        m["columns_drift"] = sent(w, 0, lambda: list_preferences.send(p, c, reviewed_hash=p.hash))

        reset()
        w, state = list_wire(module)
        c = w.client()
        p = list_preferences.export_plan(c, module, "701", ["subject"])
        state["failure"] = 429
        m["export_fail_429"] = sent(w, 0, lambda: list_preferences.send(p, c, reviewed_hash=p.hash))

        refusals = []
        for kind, args in [("columns", ([], [])), ("columns", (["ticket_subject", "ticket_subject"], [])),
                           ("columns", (["ticket_subject", "nope"], [])),
                           ("columns", (["ticket_subject", "display_id"], [])), ("columns", (["status"], [])),
                           ("columns", (["ticket_subject"], ["status"])),
                           ("columns", (["ticket_subject", "requester"], ["requester", "requester"])),
                           ("columns", (["ticket_subject", "requester"], ["requester"])),
                           ("columns", (["ticket_subject", "status"], ["status"])),
                           ("export", ("701", [])), ("export", ("701", ["subject", "subject"])),
                           ("export", ("701", ["owner"])), ("export", ("701", ["responder_name"])),
                           ("export", ("701", ["group_name"])), ("export", ("701", ["requester_info"])),
                           ("export", ("Acme queue", ["display_id"])), ("export", ("nope", ["display_id"]))]:
            reset()
            w, state = list_wire(module)
            c = w.client()
            fn = list_preferences.columns_plan if kind == "columns" else list_preferences.export_plan
            got = attempt(lambda: fn(c, module, *args))
            if "ok" in got:
                got = {"ok": write_plan(got["ok"])}
            refusals.append({"kind": kind, "args": list(args), **got})
        for name, mutate in [("advanced", lambda s: s["row"].update(advanced_query_hash=[])),
                             ("query_not_list", lambda s: s["row"].update(query_hash="x")),
                             ("id_mismatch", lambda s: s.update(row={**s["row"], "id": 702})),
                             ("headers_empty", lambda s: s.update(headers=[])),
                             ("headers_dup", lambda s: s["headers"].append({"name": "status"})),
                             ("headers_no_name", lambda s: s["headers"].append({"label": "x"}))]:
            reset()
            w, state = list_wire(module)
            mutate(state)
            c = w.client()
            got = attempt(lambda: list_preferences.export_plan(c, module, "701", ["display_id"]))
            if "ok" in got:
                got = {"ok": write_plan(got["ok"])}
            refusals.append({"kind": "export_source", "args": [name], **got})
        m["refusals"] = refusals
        out[module] = m
    out["unsupported"] = attempt(lambda: list_preferences.headers(Wire(lambda *a: 404).client(), "assets"))
    return out


# ------------------------------------------------------ problem workflows

def problem_fields(base):
    fields = [{"id": 501, "name": "subject", "label": "Subject", "field_type": "default_subject", "required": True},
              {"id": 502, "name": "description", "label": "Description", "field_type": "default_description",
               "required": True},
              {"id": 503, "name": "requester", "label": "Requester", "field_type": "default_requester",
               "required": True}]
    for i, name in enumerate(["status", "priority", "impact"]):
        fields.append({"id": 510 + i, "name": name, "label": name.title(), "field_type": "default_" + name,
                       "required": True, "choices": [{"id": base + i, "value": "Open" if name == "status" else "Low"}]})
    for index, field in enumerate(fields):
        field["position"] = index + 1
    return fields


def problem_wire(base=41):
    state = {"fields": problem_fields(base), "rows": {123: {"id": 123, "subject": "Acme"}}, "notes": [],
             "tasks": [], "failure": None, "rules_denied": False, "readback": "ok", "omit_id": False,
             "normalize": None, "writes": 0,
             "task_fields": [{"name": "status", "choices": [{"id": base + 10, "value": "Open"}]},
                             {"name": "notify_before", "choices": [{"id": base + 11, "value": "Never"}]}]}

    def handle(method, path, body, params):
        if method == "GET":
            if path == "problem_form_fields":
                return {"problem_fields": state["fields"]}
            if path == "problem_task_fields":
                return {"problem_task_fields": state["task_fields"]}
            if path.endswith("business_rules"):
                return 403 if state["rules_denied"] else {"business_rules": []}
            if path == "autocomplete/requesters":
                return {"results": [{"id": 901, "value": "Acme Operator", "email": "person@example.com"}]}
            if path in {"problems/123/tasks", "problems/123/notes"}:
                key = path.rsplit("/", 1)[1]
                rows = copy.deepcopy(state[key])
                if rows and state["readback"] == "missing":
                    rows[0].pop("body" if key == "notes" else "title")
                return {key: rows, "has_more": False}
            if path.startswith("problems/"):
                row = state["rows"].get(int(path.split("/")[1]))
                if row is None:
                    return 404
                row = copy.deepcopy(row)
                if state["writes"] and state["normalize"]:
                    state["normalize"](row)
                if state["writes"] and state["readback"] == "missing":
                    row.pop("email", None)
                return {"problem": row}
            return 404
        state["writes"] += 1
        if state["failure"] == "disconnect":
            raise Disconnect
        if state["failure"]:
            return state["failure"]
        if path == "problems":
            state["rows"][456] = {"id": 456, **body}
            key = "problem"
        else:
            key = "note" if path.endswith("/notes") else "task"
            state[key + "s"].append({"id": 456, **body})
        return {} if state["omit_id"] else {key: {"id": 456}}

    return Wire(handle), state


ANSWERS = {"Subject": "Acme problem", "Description": "Acme details", "Status": "Open", "Priority": "Low",
           "Impact": "Low"}


def gen_problem_workflows():
    out = {"fields": problem_fields(41), "answers": ANSWERS}
    reset()
    w, state = problem_wire()
    c = w.client()
    p = problem_workflows.create_plan(c, ANSWERS, "person@example.com")
    out["create"] = {"plan_calls": w.log[:], **write_plan(p)}
    out["create"]["send"] = sent(w, 0, lambda: problem_workflows.send(p, c, reviewed_hash=p.hash))
    out["create"]["resend"] = sent(w, 0, lambda: problem_workflows.send(p, w.client(), reviewed_hash=p.hash))

    customs = []
    for actual in [{"acme_count": "7", "default_flag": None}, {"acme_count": 7, "default_flag": False},
                   {"acme_count": "8"}, {"default_flag": None}, {"acme_count": True}, None,
                   {"acme_count": "7.0"}, {"acme_count": " 7 "}, {"acme_count": "1_0"}]:
        reset()
        w, state = problem_wire()
        state["fields"].append({"id": 601, "name": "acme_count", "label": "Acme count",
                                "field_type": "custom_number", "position": 20})
        c = w.client()
        p = problem_workflows.create_plan(c, {**ANSWERS, "Acme count": 7}, "person@example.com")

        def normalize(row, actual=actual):
            row["custom_fields"] = actual
            row["description"] = "Acme details"
        state["normalize"] = normalize
        customs.append({"actual": actual, "hash": p.hash, "payload_json": p.payload_json,
                        "inputs_json": p.inputs_json,
                        **sent(w, 0, lambda: problem_workflows.send(p, c, reviewed_hash=p.hash))})
    out["create_custom"] = customs

    for kind in ["note", "task"]:
        reset()
        w, state = problem_wire()
        c = w.client()
        p = problem_workflows.child_plan(c, "PRB-123", kind=kind, text="Acme")
        out[kind] = {"plan_calls": w.log[:], **write_plan(p)}
        out[kind]["send"] = sent(w, 0, lambda: problem_workflows.send(p, c, reviewed_hash=p.hash))
        for readback in ["missing", "no-id"]:
            reset()
            w, state = problem_wire()
            c = w.client()
            p = problem_workflows.child_plan(c, 123, kind=kind, text="Acme")
            state["readback"], state["omit_id"] = readback, readback == "no-id"
            out[kind][readback] = sent(w, 0, lambda: problem_workflows.send(p, c, reviewed_hash=p.hash))

    for readback in ["missing", "no-id"]:
        reset()
        w, state = problem_wire()
        c = w.client()
        p = problem_workflows.create_plan(c, ANSWERS, "person@example.com")
        state["readback"], state["omit_id"] = readback, readback == "no-id"
        out["create"][readback] = sent(w, 0, lambda: problem_workflows.send(p, c, reviewed_hash=p.hash))

    reset()
    w, state = problem_wire()
    c = w.client()
    p = problem_workflows.create_plan(c, ANSWERS, "person@example.com")
    state["fields"][0]["label"] = "Changed subject"
    out["create_drift"] = sent(w, 0, lambda: problem_workflows.send(p, c, reviewed_hash=p.hash))

    reset()
    w, state = problem_wire()
    c = w.client()
    p = problem_workflows.create_plan(c, ANSWERS, "person@example.com")
    state["failure"] = 500
    out["create_fail_500"] = sent(w, 0, lambda: problem_workflows.send(p, c, reviewed_hash=p.hash))

    refusals = []

    def refuse(name, mutate, fn):
        reset()
        w, state = problem_wire()
        mutate(state)
        c = w.client()
        got = attempt(lambda: fn(c))
        if "ok" in got:
            got = {"ok": write_plan(got["ok"])}
        refusals.append({"name": name, **got})

    create = lambda answers=ANSWERS, who="person@example.com": (  # noqa: E731
        lambda c: problem_workflows.create_plan(c, answers, who))
    refuse("rules_denied", lambda s: s.update(rules_denied=True), create())
    refuse("requester_answer", lambda s: None, create({**ANSWERS, "Requester": "x"}))
    refuse("unknown_answer", lambda s: None, create({**ANSWERS, "Nope": "x"}))
    refuse("no_requester_field", lambda s: s["fields"].pop(2), create())
    refuse("bad_choice", lambda s: None, create({**ANSWERS, "Status": "Closed"}))
    refuse("missing_impact", lambda s: None, create({k: v for k, v in ANSWERS.items() if k != "Impact"}))
    refuse("extra_top", lambda s: s["fields"].append(
        {"id": 520, "name": "urgency", "label": "Urgency", "field_type": "default_urgency", "position": 9,
         "choices": [{"id": 9, "value": "Low"}]}), create({**ANSWERS, "Urgency": "Low"}))
    refuse("no_requester_match", lambda s: None, create(who="nobody@example.com"))
    child = lambda kind="note", text="Acme", target="123": (  # noqa: E731
        lambda c: problem_workflows.child_plan(c, target, kind=kind, text=text))
    refuse("child_kind", lambda s: None, child(kind="comment"))
    refuse("child_text", lambda s: None, child(text="  "))
    refuse("child_target", lambda s: None, child(target="CHN-1"))
    refuse("child_missing_parent", lambda s: s["rows"].clear(), child())
    refuse("task_no_meta", lambda s: s.update(task_fields="x"), child(kind="task"))
    refuse("task_no_status", lambda s: s["task_fields"].pop(0), child(kind="task"))
    refuse("task_deleted_choice", lambda s: s["task_fields"][0]["choices"][0].update(deleted=True),
           child(kind="task"))
    refuse("task_str_id", lambda s: s["task_fields"][0]["choices"][0].update(id="51"), child(kind="task"))
    refuse("task_required", lambda s: s["task_fields"].append({"name": "agent", "required": True}),
           child(kind="task"))
    refuse("task_label_match", lambda s: s["task_fields"][0]["choices"][0].update(value=None, label="OPEN"),
           child(kind="task"))
    refuse("task_dup_rows", lambda s: s["tasks"].extend([{"id": 5}, {"id": 5}]), child(kind="task"))
    out["refusals"] = refusals
    return out


def gen_readback():
    cases = []
    for actual, expected in [("7", 7), ("7.0", 7), (" 7 ", 7), ("1_0", 10), ("07", 7), ("x", 7), ("inf", 7.5),
                             ("7.5", 7.5), ("1e1", 10), ("-0", 0), (7, "7"), (7.0, "7"), (7.0, "7.0"),
                             ("true", True), ("True", True), ("1", True), (True, 1), (None, None), (None, 0),
                             (0, None), ([1, 2], [1, 2]), ([1], [1, 2]), ({"a": 1, "b": 2}, {"a": "1"}),
                             ({"a": 1}, {"a": 1, "b": None}), (1, 1.0), (1.0, 1), ("a", "a"),
                             ("1__0", 10), ("_1", 1), ("1.", 1), (".5", 0.5), ("+5", 5), ("5e", 5)]:
        cases.append({"actual": actual, "expected": expected,
                      "submitted": readback.submitted_value_matches(actual, expected)})
    texts = []
    for a, b in [("<div>Acme details</div>", "Acme details"), ("<p>a&amp;b</p>", "a&b"), ("a  b", "a b"),
                 (None, "x"), ("x", None), ("<p>x</p>", "<p>y</p>")]:
        texts.append({"actual": a, "expected": b, "same": readback.same_text(a, b)})
    return {"submitted": cases, "same_text": texts}


# --------------------------------------------------------- planning text

class PlanningClient:
    """`tests/test_planning_text.py::Client`; `after` replaces the rows a
    write leaves behind (the native detail shape of test_planning_readback,
    or `test_planning_edit_clear.py`'s readback client). `failure` is raised
    by the write after it is recorded; `methods` logs every request."""

    def __init__(self, module="changes"):
        self.module = module
        self.rows = []
        self.writes = []
        self.methods = []
        self.after = None
        self.failure = None

    def int_get(self, path):
        self.methods.append("GET")
        key = "change_planning_fields" if self.module == "changes" else "problem_planning_fields"
        return {key: copy.deepcopy(self.rows)}

    def _get_csrf(self):
        return "synthetic"

    def _request(self, method, path, **kw):
        from urllib.parse import parse_qs, urlsplit
        self.methods.append(method)
        payload = json.loads(kw["content"]) if "content" in kw else None
        self.writes.append([method, path.split("/api/_/", 1)[1], payload])
        if self.failure is not None:
            raise self.failure
        key = (parse_qs(urlsplit(path).query)["id"][0] if method == "POST" else
               urlsplit(path).path.rsplit("/", 1)[1])
        self.rows = copy.deepcopy(self.after) if self.after is not None else [] if method == "DELETE" else [
            {"name": key, "description": payload["description"], "attachments": []}]
        return {}


PLANNING_TARGETS = [("changes", "Rollout Plan", "acme_plan"), ("problems", "Root Cause", "problem_cause")]
TEXT_KEYS = ["description", "content", "description_html", "description_text", "body"]


def gen_planning():
    from fsv import planning_text as planning
    planning.schema.load = lambda *a, **kw: {"fields": [
        {"name": "acme_plan", "label": "Rollout Plan", "field_type": "planning_field", "id": 901}]}

    def plan_out(p):
        return {**write_plan(p), "before": p.before}

    out = {"sections": [], "create": [], "existing": [], "unknown": [], "files": [], "race": [], "readback": []}
    for module, section, key in PLANNING_TARGETS + [("problems", "Impact", "problem_impact"),
                                                    ("problems", "Symptoms", "problem_symptom")]:
        reset()
        c = PlanningClient(module)
        p = planning.build_plan(c, module, 123, section, "<div>Acme evidence</div>")
        entry = {"module": module, "section": section, **plan_out(p)}
        entry["send"] = attempt(lambda: planning.send(p, c, reviewed_hash=p.hash))
        entry["resend"] = attempt(lambda: planning.send(p, c, reviewed_hash=p.hash))
        entry["writes"] = c.writes
        out["sections"].append(entry)
    for module, section, key in PLANNING_TARGETS:
        for shape in TEXT_KEYS:
            reset()
            c = PlanningClient(module)
            c.rows = [{"name": key, shape: "", "files": [], "empty": True}]
            p = planning.build_plan(c, module, 123, section, "Acme replacement")
            out["create"].append({"module": module, "section": section, "rows": c.rows, **plan_out(p),
                                  "send": attempt(lambda: planning.send(p, c, reviewed_hash=p.hash)),
                                  "writes": c.writes})
        for shape in TEXT_KEYS:
            for stored in ["Acme original", " ", "<div><br></div>"]:
                reset()
                c = PlanningClient(module)
                c.rows = [{"name": key, shape: stored, "files": []}]
                rows = copy.deepcopy(c.rows)
                p = planning.build_plan(c, module, 123, section, "Acme replacement")
                out["existing"].append({"module": module, "section": section, "rows": rows, **plan_out(p),
                                        "send": attempt(lambda: planning.send(p, c, reviewed_hash=p.hash)),
                                        "writes": c.writes})
        for fields in [{"content": None}, {"content": 42}, {}, {"empty": True},
                       {"description": "", "content": "Acme original"}, {"description": "", "content": 42},
                       {"description": "", "content": None}, {"content": "", "empty": False}]:
            reset()
            c = PlanningClient(module)
            c.rows = [{"name": key, "files": [], **fields}]
            out["unknown"].append({"module": module, "section": section, "rows": c.rows,
                                   **attempt(lambda: plan_out(planning.build_plan(
                                       c, module, 123, section, "Acme replacement")))})
        reset()
        c = PlanningClient(module)
        p = planning.build_plan(c, module, 123, section, "Acme")
        c.rows = [{"name": key, "content": "Someone else's update", "files": []}]
        out["race"].append({"module": module, "section": section, "rows": c.rows, "hash": p.hash,
                            **attempt(lambda: planning.send(p, c, reviewed_hash=p.hash)), "writes": c.writes})
        for stored in ["Acme & Globex details", "<p>Acme &amp; Globex&nbsp; details</p>", "Acme wrong details",
                       None, 42]:
            reset()
            c = PlanningClient(module)
            c.rows = [{"name": key, "label": section, "content": "", "files": [], "empty": True}]
            p = planning.build_plan(c, module, 123, section, "<div>Acme &amp; Globex\n details</div>")
            c.after = [{"name": key, "label": section, "content": stored, "files": [], "empty": False}]
            out["readback"].append({"module": module, "section": section, "stored": stored, "hash": p.hash,
                                    **attempt(lambda: planning.send(p, c, reviewed_hash=p.hash))})
    for files in [{}, {"files": None}, {"attachments": [], "files": [{"id": 456}]}]:
        reset()
        c = PlanningClient()
        c.rows = [{"name": "acme_plan", "content": "", **files}]
        out["files"].append({"rows": c.rows, **attempt(lambda: plan_out(planning.build_plan(
            c, "changes", 123, "Rollout Plan", "Acme")))})
    out["same_text"] = [{"actual": a, "expected": b, "same": readback.same_text(a, b)} for a, b in [
        ("Acme Globex", "<div>Acme</div><div>Globex</div>"), ("AcmeGlobex", "<div>Acme</div><div>Globex</div>"),
        ("acme", "Acme"), (None, "")]]
    out["section_content"] = [{"row": r, "content": planning.section_content(r)} for r in [
        None, {}, {"description": 1, "content": "c"}, {"body": "b", "description_text": "t"},
        {"description_html": "h"}]]
    out["editable_content"] = []
    for r in [None, {}, {"attachments": []}, {"files": [], "content": "x"},
              {"attachments": [], "description": "<p>a</p>"}, {"attachments": [{"id": 1}], "content": ""},
              {"files": None, "content": ""}, {"attachments": [], "files": [], "description": "d"},
              {"attachments": [], "description": 1, "content": "c"},
              {"attachments": [], "description": None, "content": "c"},
              {"attachments": [], "body": "b", "content": "c"}]:
        out["editable_content"].append({"row": r, **attempt(lambda: planning.editable_content(r))})
    for r in [{"attachments": [], "description": "<div>Acme original</div>", "description_text": "Acme original"},
              {"attachments": [], "description": "<div>Acme original</div>", "description_text": "Conflicting"},
              {"attachments": [], "content": "", "empty": False}, {"attachments": [], "content": "x", "empty": True},
              {"attachments": [], "content": "x", "empty": False}, {"attachments": [], "content": "", "empty": True},
              {"attachments": [], "content": "", "empty": "yes"}, {"attachments": [], "content": "", "empty": None},
              {"attachments": [], "content": "", "description_text": " "},
              {"attachments": [], "description": "<div><br></div>", "body": ""}]:
        out["editable_content"].append({"row": r, **attempt(lambda: planning.editable_content(r))})
    out["definitions"] = {m: attempt(lambda: planning.definitions(None, m)) for m in ["changes", "problems", "tickets"]}
    out["edit_clear"] = gen_planning_edit_clear(planning, plan_out)
    return out


def gen_planning_edit_clear(planning, plan_out):
    """`tests/test_planning_edit_clear.py` (e638d85) over `PlanningClient`."""
    from fsv.errors import APIError
    import httpx

    def populated(c, key):
        c.rows = [{"id": 987, "name": key, "description": "<div>Acme original</div>",
                   "description_text": "Acme original", "attachments": []}]

    out = {"wire": [], "race": [], "transport": [], "envelope": [], "delete_readback": [],
           "clear_refused": [], "aliases": [], "inputs": []}
    for module, section, key in PLANNING_TARGETS:
        for clear in (False, True):
            desc = None if clear else "Acme new"
            reset()
            c = PlanningClient(module)
            populated(c, key)
            p = planning.build_plan(c, module, 123, section, desc, clear=clear)
            entry = {"module": module, "section": section, "clear": clear, **plan_out(p), "payload": p.payload,
                     "cancel": attempt(lambda: planning.send(p, c, reviewed_hash=None))}
            entry["cancel_writes"] = len(c.writes)
            entry["send"] = attempt(lambda: planning.send(p, c, reviewed_hash=p.hash))
            entry["resend"] = attempt(lambda: planning.send(p, c, reviewed_hash=p.hash))
            entry["writes"] = c.writes
            out["wire"].append(entry)
            for race in ("text", "files", "removed"):
                reset()
                c = PlanningClient(module)
                populated(c, key)
                p = planning.build_plan(c, module, 123, section, desc, clear=clear)
                if race == "removed":
                    c.rows = []
                elif race == "files":
                    c.rows[0]["attachments"] = [{"id": 321}]
                else:
                    c.rows[0]["description"] = "Changed by someone else"
                    c.rows[0]["description_text"] = "Changed by someone else"
                out["race"].append({"module": module, "section": section, "clear": clear, "race": race,
                                    "rows": copy.deepcopy(c.rows),
                                    **attempt(lambda: planning.send(p, c, reviewed_hash=p.hash)),
                                    "writes": c.writes})
            for failure in (403, 429, "timeout"):
                reset()
                c = PlanningClient(module)
                c.rows = [{"id": 987, "name": key, "description": "Acme original", "attachments": []}]
                p = planning.build_plan(c, module, 123, section, desc, clear=clear)
                c.failure = (httpx.ReadTimeout("synthetic timeout") if failure == "timeout" else
                             APIError(failure, {"message": "synthetic rejection"}))
                out["transport"].append({"module": module, "section": section, "clear": clear,
                                         "failure": failure, "hash": p.hash,
                                         "first": attempt(lambda: planning.send(p, c, reviewed_hash=p.hash)),
                                         "second": attempt(lambda: planning.send(p, c, reviewed_hash=p.hash)),
                                         "writes": c.writes})
            reset()
            c = PlanningClient(module)
            c.rows = [{"id": 987, "name": key, "description": "Acme original", "attachments": []}]
            c.after = [] if clear else [{"id": 987, "name": key, "description": "<div>Acme new</div>",
                                         "attachments": []}]
            p = planning.build_plan(c, module, 123, section, desc, clear=clear)
            out["envelope"].append({"module": module, "section": section, "clear": clear, "rows": c.rows,
                                    "after": c.after, "hash": p.hash,
                                    **attempt(lambda: planning.send(p, c, reviewed_hash=p.hash)),
                                    "methods": c.methods, "writes": c.writes})
    for after in [[], [{"name": "acme_plan", "content": "", "files": [], "empty": True}],
                  [{"name": "acme_plan", "content": "<div><br></div>", "files": []}],
                  [{"name": "acme_plan", "content": "", "files": None}],
                  [{"name": "acme_plan", "content": "", "files": [], "empty": False}],
                  [{"name": "acme_plan", "content": "", "files": [], "id": 987}],
                  [{"name": "acme_plan", "files": []}]]:
        reset()
        c = PlanningClient()
        populated(c, "acme_plan")
        c.after = after
        p = planning.build_plan(c, "changes", 123, "Rollout Plan", clear=True)
        out["delete_readback"].append({"after": after, "hash": p.hash,
                                       **attempt(lambda: planning.send(p, c, reviewed_hash=p.hash))})
    for rows, desc, clear in [([], None, True),
                              ([{"name": "acme_plan", "description": "", "attachments": []}], None, True),
                              ([{"id": 987, "name": "acme_plan", "description": "", "attachments": []}], None, True),
                              ([{"id": 987, "name": "acme_plan", "description": "", "attachments": []}], "Acme", False)]:
        reset()
        c = PlanningClient()
        c.rows = rows
        out["clear_refused"].append({"rows": rows, "description": desc, "clear": clear,
                                     **attempt(lambda: planning.build_plan(c, "changes", 123, "Rollout Plan",
                                                                           desc, clear=clear)),
                                     "writes": c.writes})
    for clear in (False, True):
        reset()
        c = PlanningClient()
        populated(c, "acme_plan")
        c.rows[0]["description_text"] = "Conflicting original"
        out["aliases"].append({"clear": clear, "rows": c.rows,
                               **attempt(lambda: planning.build_plan(c, "changes", 123, "Rollout Plan",
                                                                     None if clear else "Acme new", clear=clear))})
    for desc, clear in [(None, False), ("Acme", True), ("", False), ("  ", False)]:
        reset()
        c = PlanningClient()
        populated(c, "acme_plan")
        out["inputs"].append({"description": desc, "clear": clear,
                              **attempt(lambda: planning.build_plan(c, "changes", 123, "Rollout Plan",
                                                                    desc, clear=clear))})
    return out


# ------------------------------------------------- helpers called directly

def gen_direct():
    """Reads and checks the workflows call internally, called on their own."""
    out = {}

    reset()
    w, state = canned_wire()
    c = w.client()
    p = canned.reply_plan(c, "123", 701, "/canned_response/content", sender="support@example.com")
    n = len(w.log)
    fresh = {"same": attempt(lambda: canned.freshen(p, c))}
    state["source"]["canned_response"]["name"] = "Changed metadata"
    fresh["metadata"] = attempt(lambda: canned.freshen(p, c))
    state["source"] = {"canned_response": {"id": 701, "content": "<div>Globex response</div>"}}
    fresh["content"] = attempt(lambda: canned.freshen(p, c))
    fresh["calls"] = w.log[n:]
    out["canned_freshen"] = fresh

    reset()
    w, state = canned_wire()
    c = w.client()
    out["scenarios_context"] = {"ok": attempt(lambda: scenarios.context(c, 123, 703)),
                                "missing_scenario": attempt(lambda: scenarios.context(c, 123, 9)),
                                "missing_ticket": attempt(lambda: scenarios.context(c, 999, 703)),
                                "calls": w.log[:]}

    reset()
    w, state = view_wire()
    c = w.client()
    detail = {"ok": attempt(lambda: private_views.detail(c, 701)),
              "missing": attempt(lambda: private_views.detail(c, 9))}
    state["rows"][701]["id"] = 702
    detail["other_id"] = attempt(lambda: private_views.detail(c, 701))
    state["rows"][701]["id"] = "x"
    detail["bad_id"] = attempt(lambda: private_views.detail(c, 701))
    detail["calls"] = w.log[:]
    out["private_detail"] = detail
    vis = []
    for mods in [{}, {"user_id": 902}, {"user_id": "901"}, {"default": None}, {"system": True},
                 {"system": None}, {"visibility_name": " ONLY ME "}, {"visibility_name": "private"},
                 {"visibility_name": "Everyone"}, {"visibility_name": 5},
                 {"visibility": {"id": 5, "label": "me"}}, {"visibility": {"id": 0, "label": "me"}},
                 {"visibility": {"id": 5}}, {"visibility": "81"}, {"visibility": None}]:
        vis.append({"mods": mods, **attempt(lambda: private_views.private_visibility({**VIEW, **mods}, 901))})
    out["private_visibility"] = vis

    exports = []
    for module, view, mutate in [("tickets", "701", None), ("changes", "Acme queue", None),
                                 ("problems", "701", None), ("tickets", "nope", None),
                                 ("tickets", "701", lambda s: s["row"].update(advanced_query_hash=[])),
                                 ("tickets", "701", lambda s: s.update(row={**s["row"], "id": 702}))]:
        reset()
        w, state = list_wire(module)
        if mutate:
            mutate(state)
        c = w.client()
        got = attempt(lambda: list_preferences.export_source(c, module, view))
        if "ok" in got:
            selected, row = got["ok"]
            got = {"ok": {"id": selected.id, "name": selected.name, "row": row}}
        exports.append({"module": module, "view": view, **got, "calls": w.log[:]})
    out["export_source"] = exports

    reset()
    w, state = problem_wire()
    c = w.client()
    state["notes"] = [{"id": 9, "body": "b"}, {"id": "3", "body": "a"}]
    prob = {"record": attempt(lambda: problem_workflows.record(c, 123)),
            "record_missing": attempt(lambda: problem_workflows.record(c, 9)),
            "notes": attempt(lambda: problem_workflows.inventory(c, 123, "notes")),
            "tasks": attempt(lambda: problem_workflows.inventory(c, 123, "tasks"))}
    state["tasks"] = [{"id": 5}, {"id": "5"}]
    prob["tasks_dup"] = attempt(lambda: problem_workflows.inventory(c, 123, "tasks"))
    state["tasks"] = [{"id": "x"}]
    prob["tasks_bad_id"] = attempt(lambda: problem_workflows.inventory(c, 123, "tasks"))
    state["rows"][123] = {"id": "123"}
    prob["record_str_id"] = attempt(lambda: problem_workflows.record(c, 123))
    state["rows"][123] = {"id": 124}
    prob["record_other_id"] = attempt(lambda: problem_workflows.record(c, 123))
    prob["calls"] = w.log[:]
    out["problem_reads"] = prob
    status = {"name": "status", "choices": [{"id": 51, "value": "Open"}]}
    choices = []
    for fields, label in [([status], "open"), ([status], "Closed"), ([], "Open"), ([status, status], "Open"),
                          ([{"name": "status", "choices": [{"id": 51, "label": "OPEN"}]}], "open"),
                          ([{"name": "status", "choices": [{"id": 51, "name": "Open"}]}], "Open"),
                          ([{"name": "status", "choices": [{"id": 51, "value": "Open", "deleted": True}]}], "Open"),
                          ([{"name": "status", "choices": [{"id": 51, "value": "Open", "deleted_at": "x"}]}], "Open"),
                          ([{"name": "status", "choices": [{"id": 51, "value": "Open"}, {"id": 52, "value": "open"}]}],
                           "Open"),
                          ([{"name": "status", "choices": [{"id": "51", "value": "Open"}]}], "Open"),
                          ([{"name": "status", "choices": [{"id": True, "value": "Open"}]}], "Open"),
                          ([{"name": "status", "choices": [{"id": -1, "value": "Open"}]}], "Open"),
                          ([{"name": "status", "choices": [{"id": 0, "value": "Open"}]}], "Open"),
                          ([{"name": "status", "choices": ["Open", {"id": 51, "value": "Open"}]}], "Open"),
                          ([{"name": "status"}], "Open")]:
        choices.append({"fields": fields, "label": label,
                        **attempt(lambda: problem_workflows._choice(fields, "status", label))})
    out["problem_choice"] = choices

    from fsv import ticket_time
    entries = []
    for reply in [{"time_entries": [{"id": 9, "a": 1}, {"id": "3"}], "has_more": False},
                  {"time_entries": [], "has_more": False},
                  {"time_entries": [{"id": 3}, {"id": "3"}], "has_more": False},
                  {"time_entries": [{"id": 0}], "has_more": False}, 403]:
        reset()
        w = Wire(lambda m, p, b, q, reply=reply: reply if p == "tickets/123/time_entries" else 404)
        entries.append({"reply": reply, **attempt(lambda: ticket_time.inventory(w.client(), 123)), "calls": w.log})
    out["time_inventory"] = entries
    return out


def main():
    doc = {"canned": gen_canned(), "scenarios": gen_scenarios(), "private_views": gen_private_views(),
           "list_preferences": gen_list_preferences(), "problem_workflows": gen_problem_workflows(),
           "readback": gen_readback(), "planning_text": gen_planning(), "direct": gen_direct()}
    print(json.dumps(stamped(doc), indent=1, ensure_ascii=False))


if __name__ == "__main__":
    main()
