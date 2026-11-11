"""Expected values for the fsv-api coverage tests (query, views, formmeta, schema).

Run from the Python checkout:

    cd /Users/phusit.somb/lab/work/fsv && uv run python \
        ../fsv-rust/port/golden/gen_fsv_api_coverage.py \
        > ../fsv-rust/crates/fsv-api/src/testdata/coverage_oracle.json
"""
from __future__ import annotations

import json
import os
import sys
import tempfile
import time
from pathlib import Path

from fsv import formmeta, query, schema, views
from fsv.resources import CHANGES, TICKETS

RES = {"changes": CHANGES, "tickets": TICKETS}


def err(fn, *a):
    try:
        return {"ok": fn(*a)}
    except ValueError as e:
        return {"err": str(e)}


SCHEMA = {"fields": [
    {"name": "subject", "label": "Subject", "default_field": True},
    {"name": "status", "label": "Status", "default_field": True, "field_type": "default_status",
     "choices": [{"id": 1, "value": "Open"}, {"id": 2, "value": "Planning"}, {"id": 3, "value": "Caf&eacute;"}]},
    {"name": "priority", "label": "Priority", "default_field": True, "choices": [{"id": 4, "value": "Urgent"}]},
    {"name": "department", "label": "Department", "default_field": True,
     "choices": [{"id": 7, "value": "IT"}, {"id": 8, "name": "HR"}]},
    {"name": "group", "label": "Group", "default_field": True, "choices": [{"id": 5, "value": "Ops"}]},
    {"name": "cf_env", "label": "Environment", "default_field": False, "field_type": "custom_dropdown",
     "choices": [{"id": 11, "value": "Prod"}, {"id": 12, "value": "Test"}]},
    {"name": "cf_note", "label": "Dup", "default_field": False},
    {"name": "cf_other", "label": "Dup", "default_field": False},
    {"name": "cf_strasse", "label": "Straße", "default_field": False},
    {"name": "requester", "label": "Requester &amp; Co", "default_field": True},
    {"name": "planned_start", "label": "", "default_field": True},
]}


def gen_query():
    q = {}
    q["isdigit"] = {t: query._isdigit(t) for t in ["123", "", "\u0661\u0662", "\u00b2", "12a", "007"]}
    q["strip_quotes"] = {t: query._strip_quotes(t) for t in ['"a b"', "'x'", "'", '"a\'', "  'p'  ", '""', "plain"]}
    q["parse_where"] = {t: err(query._parse_where, t) for t in [
        "status=Open", " status != 'Closed' ", "a>=1", "a<=b=c", "x~=y", "=x", "==x", "noop", "f= \"q\" ", "a<b", ">=x"]}
    q["field_api_type"] = [[f, query._field_api_type(f)] for f in [
        {"default_field": True}, {"default_field": False}, {}, {"default_field": 1}, {"default_field": ""}]]
    q["schema_field_matches"] = {
        f"{t}|{s}": [f["name"] for f in query._schema_field_matches(SCHEMA, t, s)]
        for t, s in [("dup", None), ("DUP", "custom"), ("stat", None), ("stat", "custom"), ("cf_", "default"),
                     ("STRASSE", None), ("zzz", None)]
    }
    q["find_schema_field"] = {
        t: (lambda r: {"ok": r["ok"] and r["ok"]["name"]} if "ok" in r else r)(err(query._find_schema_field, SCHEMA, t))
        for t in ["status", "Status", "dup", "cf_note", "env", "custom:env", "default:env", "default:status",
                  "stat", "cf_", "nope", "STRASSE", "other:status"]
    }
    rows = [{"id": 1, "value": "Ann", "email": "ann@x.io"}, {"id": 2, "value": "Annie", "details": "annie@x.io"}]
    q["pick_lookup"] = [[v, r, k, err(query._pick_lookup, "agent", v, r, k)] for v, r, k in [
        ("ANN", rows, ("value",)),
        ("annie@x.io", rows, ("value", "details")),
        ("zz", rows[:1], ("value",)),
        ("zz", [], ("value",)),
        ("an", rows, ("value",)),
        ("STRASSE", [{"id": 3, "value": "straße"}, {"id": 4}], ("value",)),
        ("0", [{"id": 0, "value": None}, {"id": 5, "details": ""}], ("value", "id")),
        ("x", [{"id": 0}, {"value": "", "id": None}], ("value",)),
    ]]
    class Dir:
        def __init__(self, rows):
            self.rows = rows
            self.calls = []

        def autocomplete(self, kind, value, extra=None):
            self.calls.append([kind, value, extra or {}])
            return self.rows

    agents = {}
    for name, rows_, value in [
        ("digits", [], "42"),
        ("match", [{"id": 9, "user_id": 21, "value": "Ann"}, {"id": 10, "user_id": 22, "value": "Bob"}], "bob"),
        ("no_user_id", [{"id": 9, "value": "Ann"}], "Ann"),
        ("none", [], "Ann"),
    ]:
        d = Dir(rows_)
        agents[name] = {"value": value, "rows": rows_, "result": err(query._resolve_agent, d, value), "calls": d.calls}
    q["resolve_agent"] = agents
    reqs = {}
    for name, rows_, value in [
        ("digits", [], "42"),
        ("match", [{"id": 9, "value": "Ann", "email": "ann@x.io"}, {"id": 10, "value": "Bob"}], "ANN@x.io"),
        ("blank_value", [{"id": 9, "value": ""}], "zz"),
        ("none", [], "Ann"),
    ]:
        d = Dir(rows_)
        reqs[name] = {"value": value, "rows": rows_, "result": err(query._resolve_requester, d, value), "calls": d.calls}
    q["resolve_requester"] = reqs
    st = SCHEMA["fields"][1]
    env = SCHEMA["fields"][5]
    id_only = {"name": "x", "choices": [{"id": 5, "name": "Five"}]}
    zero = {"name": "z", "label": "", "choices": [{"id": 0, "value": None, "name": "Zero"}, {"id": 1, "value": ""}]}
    q["choice_value"] = [[f, raw, t, err(query._choice_value, f, raw, t)] for f, raw, t in [
        (st, "planning", "default"), (st, "2", "default"), (env, "prod", "custom_field"),
        (env, "12", "custom_field"), ({"name": "x"}, "17", "default"), ({"name": "x"}, "abc", "default"),
        (st, "Gone", "default"), (id_only, "five", "custom_field"), (zero, "0", "default"),
        (zero, "zero", "custom_field"), (zero, "none", "default"), (st, "CAF&EACUTE;", "default"),
    ]]
    q["where_operator"] = {op: err(query._where_operator, op) for op in ["=", "!=", ">", ">=", "<", "<=", "~=", "?"]}
    ex = []
    item = query._query_item("status", "is_in", [2], "default", ex, "status", "Planning")
    q["query_item"] = {"item": item, "explain": ex}
    q["merge_date_ranges"] = query._merge_date_ranges([
        {"condition": "created_at", "type": "default", "operator": "custom_date", "value": {"from": "a"}},
        {"condition": "status", "type": "default", "operator": "is_in", "value": [1]},
        {"condition": "created_at", "type": "default", "operator": "custom_date", "value": {"to": "b"}},
        {"condition": "created_at", "type": "custom_field", "operator": "custom_date", "value": {"to": "c"}},
        {"condition": "created_at", "type": "default", "operator": "custom_date", "value": {"from": "z"}},
    ])

    def bqh(where, raw=None, or_grouping=False):
        orig = query.__dict__.get("schema_mod")
        import fsv.schema as sm
        saved = sm.load
        sm.load = lambda res, c: SCHEMA
        try:
            return err(lambda: list(query.build_query_hash(None, CHANGES, where, raw, or_grouping)))
        finally:
            sm.load = saved
            del orig

    q["build_query_hash"] = {
        "empty": bqh([]),
        "where": bqh(["status=Open", "cf_env!=Test"]),
        "or": bqh(["status=Open", "created_at>=2026-01-01", "created_at<2026-02-01"], or_grouping=True),
        "raw": bqh(["status=2"], raw='[{"condition":"x","operator":"is_in","value":[1],"type":"default"}]'),
        "bad": bqh(["nope=1"]),
    }
    return q


def gen_views():
    v = {}
    v["missing"] = [{"status": s, "note": views.Missing("tasks", s).note, "json": views.Missing("tasks", s).to_json()}
                    for s in [None, 0, 403, 404, 500]]

    def ts(x):
        r = views.parse_dt(x)
        return None if r is None else r.timestamp()

    v["parse_dt"] = [[x, ts(x)] for x in [
        None, "", "2026-01-02T03:04:05Z", "2026-01-02T03:04:05+07:00", "2026-01-02", "2026-01-02 03:04",
        "nope", 5, "2026-01-02T03:04:05.250Z", "20260102T030405"]]
    v["is_date_only"] = [[x, views._is_date_only(x)] for x in [
        "2026-01-02", " 2026-01-02\n", "2026-1-2", 20260102, "2026-01-02T00:00", None, "2026-01-02x"]]
    v["html_to_text"] = [[x, views.html_to_text(x)] for x in [
        None, "", "<p>Hello&nbsp;<b>world</b></p><p></p><p>Two</p>",
        "<script>x()</script><style>.a{}</style>Line<br/>next<BR>end",
        "<ul><li>a</li><li>b</li></ul>", "a &amp; b &lt;c&gt;", "  spaced\t\tout  ", 42]]
    sv = views.SavedView("7", "Mine", system=False, marked_default=True)
    v["saved_view"] = sv.to_json()
    v["view_list"] = [[x, views._view_list(x)] for x in [
        [{"id": 1}, 2, "x", {"id": 3}],
        {"ticket_filters": [{"id": 1}, 5]},
        {"meta": {}, "x": [{"id": 9}], "my_views": [{"id": 2}]},
        {"a": 1},
        "str",
    ]]
    v["fold"] = [[x, views._fold(x)] for x in ["  Hello   World ", "Straße", "", "A\tB\nC"]]
    v["schema_label"] = [[n, views._schema_label(SCHEMA, n)] for n in ["requester", "status", "planned_start", "zzz"]]
    v["schema_label_none"] = views._schema_label(None, "status")
    headers = [
        {"name": "display_id", "label": "ID", "selected": True, "position": 0},
        {"name": "requester", "label": "Req &amp; X", "selected": True, "position": 3},
        {"name": "status", "label": "Status", "selected": True, "position": 2},
        {"name": "priority", "label": "Prio", "selected": False, "position": 1},
        {"name": "status", "label": "Again", "selected": True, "position": 4},
        {"name": "cf_x", "selected": True},
    ]
    with_subject = headers + [{"name": "ticket_subject", "label": "Title", "selected": True, "position": 1}]

    def cols(c):
        return [[x.key, x.label] for x in c]

    v["list_columns"] = {
        "changes_default": cols(views.list_columns(CHANGES)),
        "changes_schema": cols(views.list_columns(CHANGES, SCHEMA)),
        "tickets_default": cols(views.list_columns(TICKETS)),
        "tickets_headers": cols(views.list_columns(TICKETS, SCHEMA, headers)),
        "tickets_headers_subject": cols(views.list_columns(TICKETS, SCHEMA, with_subject)),
        "tickets_unselected": cols(views.list_columns(TICKETS, None, [{"name": "x", "selected": False}])),
    }
    v["choice_name"] = [[f, val, views.choice_name(SCHEMA, f, val)] for f, val in [
        ("status", 2), ("status", "2"), ("status", "Open"), ("status", 3), ("status", 9), ("status", None),
        ("status", ""), ("department", 8), ("nope", 1)]]
    items = [
        {"change_status": {"name": "Awaiting"}, "status": 1},
        {"status_name": "Named", "status": 1},
        {"status": 2},
        {"status": 99},
        {"status": None},
        {},
        {"ticket_status": {"value": "Pending"}},
        {"summary_status": {"email": "e@x"}},
    ]
    v["status_name"] = [[i, views.status_name(i, SCHEMA)] for i in items]
    deps = [{"department": {"name": "Eng"}}, {"department_name": "Ops"}, {"department_id": 7}, {"department_id": 8},
            {"department": {}, "department_id": 99}, {}]
    v["department_name"] = [[i, views.department_name(i, SCHEMA)] for i in deps]
    reqs = [{"requester": {"name": " Ann "}}, {"requester": {"email": "a@x"}}, {"requester_name": "Bob"},
            {"requester": "str", "requester_name": ""}, {}]
    v["requester_name"] = [[i, views.requester_name(i)] for i in reqs]
    ags = [
        {"agent": {"id": 5, "name": "Ann"}},
        {"agent": {"name": "Ann"}, "agent_id": 6},
        {"agent": {"id": None, "name": "Ann"}, "agent_id": 6},
        {"responder": {"id": 0}, "responder_id": 3},
        {"owner": {"value": "x"}, "agent_id": 0, "responder_id": 8, "owner_name": "Own"},
        {"agent_id": 4, "agent_name": "Agent"},
        {},
    ]
    v["agent_of"] = [[i, list(views.agent_of(i))] for i in ags]
    grps = [{"group": {"name": "Ops"}}, {"group_name": "Desk"}, {"group_id": 5}, {"group_id": 6}, {}]
    v["group_name_of"] = [[i, views.group_name_of(i, SCHEMA)] for i in grps]
    columns = views.list_columns(CHANGES, SCHEMA)
    row_items = [
        {"id": 12, "display_id": 12, "subject": "Patch", "requester": {"name": "Ann"}, "status": 2,
         "priority": 4, "agent": {"id": 21, "name": "Bob"}, "group": {"name": "Ops"}, "department_id": 7},
        {"id": 13, "subject": None, "status": None},
    ]
    v["list_row"] = [[i, views.list_row(i, CHANGES, columns, SCHEMA, 21, None)] for i in row_items]
    v["list_row_columns"] = cols(columns)
    v["sort_label"] = [[o, views.sort_label(o, SCHEMA)] for o in [
        None, "", "created_at", "updated_at", "status", "department_id", "requester", "cf_env", "cf_some_thing_id"]]
    v["tab"] = [views.Tab("notes", "Notes", 3).to_json(), views.Tab("tasks", "Tasks").to_json()]
    life = views.Lifecycle("Acme", [views.LifecycleState(1, "Open", "done"), views.LifecycleState("2", "Rev", "current")])
    v["lifecycle"] = life.to_json()
    v["planning"] = [views.PlanningEntry("reason", "Reason", "Why", "text", []).to_json(),
                     views.PlanningEntry("plan", "Plan", "", "", ["a.pdf"]).to_json(),
                     views.PlanningEntry("x", "X", "", "", []).to_json()]
    v["file_names"] = [[r, views._file_names(r)] for r in [
        None, [], [{"name": "a.pdf"}, {"content_file_name": "b.txt"}, {"file_name": "c"}, {"id": 9}, {}, "s",
                   {"name": "", "id": 0}]]]
    convs = [
        {"id": 1, "user": {"name": "Ann"}, "private": True, "body": "<p>Hi <b>there</b></p>",
         "attachments": [{"name": "a.png"}], "source": 2},
        {"id": 2, "user_id": 33, "body": "", "body_text": "plain"},
        {"id": 3, "incoming": True, "user": {}, "user_id": 0, "body": None},
    ]
    v["conversation_json"] = [[c, views.conversation_json(c, None)] for c in convs]
    return v


def gen_formmeta():
    out = []
    for vals in [["12", 12, " 7 ", True, "x", None, 3.5, "\u0661\u0663", "", "-1", 0]]:
        acc: list = []
        steps = []
        for x in vals:
            formmeta._add(acc, x)
            steps.append(list(acc))
        out.append({"values": vals, "steps": steps})

    class Ck:
        def __init__(self, name, value):
            self.name, self.value = name, value

    class Jar:
        def __init__(self, cookies):
            self.jar = cookies

    class Client:
        def __init__(self, cookies):
            self._client = type("C", (), {"cookies": Jar(cookies)})()

    cookies = [["fs_workspace", "12"], ["WorkspaceId", " 4 "], ["other", "9"], ["ws_workspace", "x"],
               ["last_workspace", "12"]]
    cw = formmeta._cookie_workspace(Client([Ck(n, v) for n, v in cookies]))
    no_client = formmeta._cookie_workspace(object())

    class Boot:
        def __init__(self, reply):
            self.reply, self.calls = reply, 0

        def int_get(self, path, params=None):
            self.calls += 1
            return self.reply

    boots = {}
    for name, reply in [("doc", {"groups": [{"id": 4, "name": "Ops"}]}), ("empty", None), ("falsy", {})]:
        b = Boot(reply)
        r = formmeta.ClientResolver(b)
        first, second = r._bootstrap(), r._bootstrap()
        boots[name] = {"reply": reply, "first": first, "second": second, "calls": b.calls}
    return {"add": out, "cookie_cookies": cookies, "cookie_workspace": cw, "cookie_none": no_client,
            "bootstrap": boots}


def gen_schema():
    now = time.time()
    cases = {}
    specs = {
        "fresh": [{"saved_at": now - 60, "data": [{"name": "a"}]}],
        "old": [{"saved_at": now - 8 * 86400, "data": [{"name": "a"}]}],
        "future": [{"saved_at": now + 3600, "data": [{"name": "a"}]}],
        "bool": [{"saved_at": True, "data": [{"name": "a"}]}],
        "string": [{"saved_at": "1", "data": [{"name": "a"}]}],
        "mtime": [{"data": [{"name": "a"}]}],
        "skip_empty": [{"saved_at": now, "data": []}, {"saved_at": now - 60, "fields": [{"name": "b"}]}],
        "none": [{"saved_at": now, "data": []}],
    }
    with tempfile.TemporaryDirectory() as d:
        for name, docs in specs.items():
            paths = []
            for i, doc in enumerate(docs):
                p = Path(d) / f"{name}{i}.json"
                p.write_text(json.dumps(doc))
                paths.append(p)
            schema.schema_cache_candidates = lambda _n, ps=paths: ps
            schema._path = lambda _r, ps=paths: Path(d) / "primary.json"
            path, doc, stale = schema._cached_path(CHANGES)
            cases[name] = {
                "docs": [{k: ("NOW" + str(round(v - now)) if k == "saved_at" and isinstance(v, float) else v)
                          for k, v in doc_.items()} for doc_ in docs],
                "index": paths.index(path) if path in paths else None,
                "found": doc is not None,
                "stale": stale,
            }
    return cases


def gen_misc():
    from fsv import change_approvals, change_associations, change_tasks, task_status
    from fsv.ticket_conversation import ConversationPlan
    from fsv.ticket_properties import PropertyPlan

    ids = [5, "5", "007", 0, "0", -3, "-3", True, False, 3.0, "3.0", "", " 4", "\u0661", None, [], 2**40, "x"]
    id_rows = [[v, change_approvals._id(v), change_associations._id(v), change_tasks._id(v)] for v in ids]

    def status(choices):
        return [{"name": "status", "choices": choices}]

    samples = {
        "plain": status([{"id": 7, "value": " Open "}, {"id": "8", "label": "Done"}]),
        "falsy_value": status([{"id": 7, "value": "", "label": "Done"}]),
        "int_value": status([{"id": 7, "value": 5, "label": "Done"}]),
        "skip_deleted": status([{"id": 7, "value": "Old", "deleted_at": "2026"}, {"id": 8, "value": "New"},
                                {"id": 9, "value": "Gone", "deactivated": True}]),
        "casefold_dup": status([{"id": 1, "value": "Straße"}, {"id": 2, "value": "STRASSE"}]),
        "id_dup": status([{"id": 1, "value": "A"}, {"id": "1", "value": "B"}]),
        "bad_id": status([{"id": True, "value": "A"}]),
        "zero_id": status([{"id": 0, "value": "A"}]),
        "unicode_id": status([{"id": "\u0661\u0662", "value": "A"}]),
        "blank_label": status([{"id": 1, "value": "  "}]),
        "not_dict": status(["x"]),
        "two_fields": status([]) + status([]),
        "none": [{"name": "other"}],
        "empty": status([]),
        "non_dict_field": ["status", {"name": "status", "choices": [{"id": 3, "name": "Named"}]}],
    }
    active = {k: err(lambda f=f: [list(x) for x in task_status.active_choices(f)]) for k, f in samples.items()}
    conv = ConversationPlan("note", 5, "acme", '{"body": "<p>x</p>", "private": true}', None, None)
    prop = PropertyPlan(5, "acme", '{"status": 2, "custom_fields": {"a": null}}', "{}", "h")
    return {"ids": id_rows, "active_choices": {"cases": samples, "want": active},
            "conversation_payload": conv.payload, "property_payload": prop.payload}


json.dump({"misc": gen_misc(), "schema": SCHEMA, "query": gen_query(), "views": gen_views(), "formmeta": gen_formmeta(), "schema_cache": gen_schema()},
          sys.stdout, indent=1, ensure_ascii=False, default=str)
sys.stdout.write("\n")
os.environ.pop("_", None)
