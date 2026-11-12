"""Expected values for the fsv-api query and state-flow matrices (Python
`bd6f6bd`: `tests/test_query_matrix.py`, `tests/test_strict_filter_contract.py`, `tests/test_state_flow_render.py`).

Each case runs the Python test's inputs through `fsv.query` /
`fsv.state_flow` with the same fakes. Renderers print into a recording
console, so the oracle holds the exact markup strings and table cells
(`fsv_api::state_flow::Printed`). The `denied` diagram case fails the flow
read itself (the Python test patches `get_state_flow`; both print nothing).

    cd /tmp/fsv-py-92d76ff && FSV_PY_SRC=$PWD FSV_PY_COMMIT=… uv run python \\
        /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_query_flow_matrix.py \\
        > /Users/phusit.somb/lab/work/fsv-rust/crates/fsv-api/src/testdata/query_flow_oracle.json
"""
from __future__ import annotations

import copy
import json
from types import SimpleNamespace
from urllib.parse import quote

from rich.table import Table

from fsv import query, schema, state_flow
from fsv.errors import APIError
from fsv.resources import CHANGES, TICKETS
from provenance import stamped


def outcome(fn):
    try:
        return {"ok": fn()}
    except ValueError as e:
        return {"err": str(e)}


# ------------------------------------------------------------ query

SCH = {"fields": [
    {"name": "status", "label": "State", "default_field": True, "field_type": "default_status",
     "choices": [{"id": 47, "value": "Awaiting review", "requester_display_value": "In review"}, {"id": 83, "name": "Done"}]},
    {"name": "cf_tier", "label": "Tier", "field_type": "custom_dropdown", "choices": [{"id": 701, "value": "Gold"}]},
    {"name": "cf_count", "label": "Count", "field_type": "custom_number"},
    {"name": "subject", "label": "Subject", "default_field": True, "field_type": "default_text"},
    {"name": "requester", "label": "Requester", "default_field": True},
    {"name": "agent", "label": "Agent", "default_field": True},
    {"name": "group", "label": "Group", "default_field": True, "choices": [{"id": 93, "value": "Acme Ops"}]},
]}

WIRE = ["State=In review", "status!=83", "custom:Tier=701", "cf_tier=gold", "Count=12", "Count=-12",
        "Subject='Acme = ready'", "group=Acme Ops", "group_id!=93", "asset_id=external", "requester=71", "agent=72"]
INVALID = ["status", "unknown=x", "status=missing", "status~=review",
           *["status" + op + "47" for op in (">", "<", ">=", "<=")],
           *["group_id" + op + "93" for op in (">", "<", ">=", "<=", "~=")],
           "created_at!=2026-01-01", *["created_at" + op + "2026-01-01" for op in (">", "<")],
           "created_at~=2026"]
DATE_FIELDS = ["created_at", "updated_at", "due_by", "planned_start_date", "planned_end_date"]
PRECEDENCE = [
    ([{"name": "status", "label": "State"}, {"name": "other", "label": "Status"}], "status"),
    ([{"name": "cf_zone", "label": "Acme Zone"}], "zone"),
    ([{"name": "state", "default_field": True}, {"name": "state"}], "default:state"),
    ([{"name": "state", "default_field": True}, {"name": "state"}], "custom:state"),
    ([{"name": "alpha", "label": "Zone"}, {"name": "beta", "label": "Zone"}], "Zone"),
    ([{"name": "cf_zone_a"}, {"name": "cf_zone_b"}], "zone"),
]


def from_schema(exprs):
    return outcome(lambda: query.build_query_hash_from_schema(None, CHANGES, SCH, exprs))


def people():
    out = {}
    for res in (CHANGES, TICKETS):
        calls = []

        def autocomplete(kind, value, params=None):
            calls.append([kind, value, params])
            return [{"id": 71, "user_id": 171, "value": "Alex Acme", "email": "alex@example.com"}]

        schema.load = lambda resource, client: SCH
        r = outcome(lambda: list(query.build_query_hash(SimpleNamespace(autocomplete=autocomplete), res,
                                                        ["requester=alex@example.com", "agent=Alex Acme"],
                                                        or_grouping=True)))
        out[res.name] = {**r, "calls": calls}
    return out


def groups():
    calls = []
    c = SimpleNamespace(int_get=lambda path: calls.append(path) or {"groups": [
        {"id": 91, "name": "Acme"}, {"id": 92, "name": "Acme Ops"}]})
    found = {v: outcome(lambda: query._resolve_group(c, v)) for v in ["Acme", "Ops", "93"]}
    missing = outcome(lambda: query._resolve_group(
        SimpleNamespace(int_get=lambda p: {"groups": [{"name": "Acme"}]}), "Acme"))
    return {"found": found, "calls": calls, "missing_id": missing}


def raw_hash():
    raw_items = [{"condition": "subject", "operator": "is_in", "type": "default", "value": ["100%25 Acme"]}]
    schema.load = lambda *a: SCH
    out = []
    for encoded in (False, True):
        raw = json.dumps(raw_items)
        if encoded:
            raw = quote(raw)
        out.append({"raw": raw, **outcome(lambda: list(query.build_query_hash(None, CHANGES, ["status=47"], raw=raw)))})
    return out


MALFORMED = ["status==Open", "status=>Open", "status=<Open", "status<>Open", "status!==Open", "status~~=Open",
             "status!>Open", "status=", "=Open", "status='Open", "status=Open\npriority=High", "status=Open\x01",
             "  ", "status= ", " Subject = '=Acme' ", 'Subject="Acme >= Globex"', "Subject=''", "status!=Open",
             "a=b=c", '  "Due Date" >= \'2026-01-01\' ', "Sübject\u00a0=\u2003x\u3000", "x='", "x=\"a'"]
STRICT_SCH = {"fields": [{"name": "subject", "label": "Subject", "field_type": "default_text", "default_field": True},
                         {"name": "count", "label": "Count", "field_type": "custom_number"},
                         {"name": "status", "field_type": "default_status", "default_field": True}]}
STRICT_EXPRS = ["created_at>2030-01-01", "created_at<2030-01-01", "Subject~=Acme", "Count>12", "status~=Open",
                "created_at>=2030-01-01", "Subject='=Acme'"]


def strict():
    from fsv.resources import PROBLEMS
    return {
        "parse": {e: outcome(lambda e=e: list(query._parse_where(e))) for e in MALFORMED},
        "operator": {op: outcome(lambda op=op: query._where_operator(op))
                     for op in ["", "==", "contains", "gte", "is_greater_than", "=", "!=", ">", "~="]},
        "schema": STRICT_SCH,
        "builder": {r.name: {e: outcome(lambda r=r, e=e: query.build_query_hash_from_schema(None, r, STRICT_SCH, [e]))
                             for e in STRICT_EXPRS} for r in (CHANGES, TICKETS, PROBLEMS)},
    }


def gen_query():
    out = {
        "schema": SCH,
        "wire": {e: from_schema([e]) for e in WIRE},
        "invalid": {e: from_schema([e]) for e in INVALID},
        "date_bounds": [{"field": f, "lower": lo, "upper": hi,
                         **from_schema([f"{f}{lo}2026-01-01", "status=47", f"{f}{hi}2026-12-31"])}
                        for f in DATE_FIELDS for lo, hi in [(">=", "<=")]],
        "exact_date": from_schema(["due_by=2026-01-01", "due_by>=2026-02-01"]),
        "find_field": [{"fields": fields, "text": text,
                        **outcome(lambda: query._find_schema_field({"fields": fields}, text)["name"])}
                       for fields, text in PRECEDENCE],
        "people": people(),
        "agent_errors": [{"rows": rows, **outcome(lambda: query._resolve_agent(
            SimpleNamespace(autocomplete=lambda *a: rows), "Alex"))}
            for rows in [[], [{"id": 71, "value": "Alex"}],
                         [{"id": 71, "value": "Alex Acme"}, {"id": 72, "value": "Alex Globex"}]]],
        "groups": groups(),
        "strict": strict(),
        "raw": raw_hash(),
        "malformed_raw": {raw: outcome(lambda: list(query.build_query_hash(None, CHANGES, [], raw=raw)))
                          for raw in ["%ZZ", "%255B%255D", "{}", "null"]},
    }
    schema.load = lambda *a: (_ for _ in ()).throw(AssertionError("unneeded schema request"))
    out["empty"] = {"build": list(query.build_query_hash(None, CHANGES, [])),
                    "from_schema": query.build_query_hash_from_schema(None, CHANGES, SCH, [])}
    return out


# ------------------------------------------------------------ state flow

class Rec:
    """A console that records each `print` as `fsv_api::state_flow::Printed`."""

    def __init__(self):
        self.printed = []

    def print(self, *args):
        if not args:
            self.printed.append({"blank": True})
        elif isinstance(args[0], Table):
            t = args[0]
            rows = [list(r) for r in zip(*(col._cells for col in t.columns))]
            self.printed.append({"table": t.title, "rows": rows})
        else:
            self.printed.append({"text": args[0]})


FLOW = {"name": "Acme release", "state_list": [{"id": i, "value": f"Phase {i}"} for i in range(1, 8)]}
APPROVALS = [{"level_id": 1, "status": {"name": "approved"}, "member": {"name": "Alex Acme"}},
             {"level_id": 2, "status": {"name": "rejected"}, "member_id": 72},
             {"level_id": 3, "member_id": 73}]
CHANGE = {"id": 123, "human_display_id": "CHN-123", "state_flow_id": 51, "workspace_id": 17,
          "status": 4, "status_name": "Phase 4", "state_traversal": [1, 2, 3], "change_type": 2}
RENDERERS = {"compact": state_flow.render_compact, "diagram": state_flow.render_state_diagram}


def flow_env(flow=None, deny_flow=None):
    flow = flow if flow is not None else copy.deepcopy(FLOW)
    calls = []

    def get(path, params=None):
        calls.append([path, params])
        if path == "ws/17/change_management/51":
            if deny_flow:
                raise APIError(deny_flow, "synthetic failure")
            return {"state_flow": flow}
        assert path == "changes/123/approvals"
        return {"approvals": APPROVALS}

    state_flow.get_client = lambda: SimpleNamespace(int_get=get)
    return flow, copy.deepcopy(CHANGE), calls


def render(renderer, change, **kw):
    rec = Rec()
    try:
        renderer(change, rec, **kw)
        return {"printed": rec.printed}
    except APIError as e:
        return {"printed": rec.printed, "status": e.status}


def gen_flow():
    out = {"flow": FLOW, "approvals": APPROVALS, "change": CHANGE}
    workspace = []
    for ws, wrapped in [(None, True), ("", False), (17, True)]:
        calls = []
        flow = {"name": "Acme lifecycle", "state_list": []}

        def get(path, params=None, wrapped=wrapped, flow=flow, calls=calls):
            calls.append([path, params])
            return {"state_flow": flow} if wrapped else flow

        state_flow.get_client = lambda get=get: SimpleNamespace(int_get=get)
        workspace.append({"workspace": ws, "wrapped": wrapped,
                          "result": state_flow.get_state_flow(51, workspace_id=ws), "calls": calls})
    out["workspace"] = workspace
    fallback = []
    for status in [403, 404, 429, 500]:
        calls = []

        def get(path, params=None, status=status, calls=calls):
            calls.append(path)
            if path.startswith("ws/"):
                raise APIError(status, "synthetic denial")
            return {"state_flow": {"name": "Acme fallback"}}

        try:
            r = {"ok": state_flow.get_state_flow(51, SimpleNamespace(int_get=get), workspace_id=17)}
        except APIError as e:
            r = {"status": e.status}
        fallback.append({"status": status, **r, "calls": calls})
    out["fallback"] = fallback

    _, change, calls = flow_env()
    out["full"] = {**render(state_flow.render_flow, change), "calls": calls}

    windows = []
    for name, renderer in RENDERERS.items():
        for status, context in [(1, 1), (4, 1), (7, 1), (4, 10)]:
            _, change, calls = flow_env()
            change["status"] = status
            windows.append({"renderer": name, "status": status, "context": context,
                            **render(renderer, change, context=context), "calls": calls})
    out["windows"] = windows

    escaped = []
    for name, renderer in RENDERERS.items():
        flow, change, _ = flow_env()
        flow["state_list"][3]["value"] = "Review [Acme] → Δ"
        escaped.append({"renderer": name, **render(renderer, change)})
    out["escaped"] = escaped

    unavailable = []
    for name, renderer in RENDERERS.items():
        for case in ["missing-flow", "empty-states", "unknown-current", "denied"]:
            flow, change, calls = flow_env(deny_flow=500 if case == "denied" else None)
            if case == "missing-flow":
                change.pop("state_flow_id")
            elif case == "empty-states":
                flow["state_list"] = []
            elif case == "unknown-current":
                change["status"] = 999
            unavailable.append({"renderer": name, "case": case, **render(renderer, change), "calls": calls})
    out["unavailable"] = unavailable

    absent = {}
    for case in ["missing", "empty"]:
        flow, change, _ = flow_env()
        if case == "missing":
            change.pop("state_flow_id")
        else:
            flow["state_list"] = []
        absent[case] = render(state_flow.render_flow, change)["printed"]
    out["absent"] = absent

    def denied(path, params=None):
        raise APIError(403, "synthetic denial")

    state_flow.get_client = lambda: SimpleNamespace(int_get=denied)
    out["denied"] = {"approvals": state_flow._try_get_approvals(123, SimpleNamespace(int_get=denied)),
                     **render(state_flow.render_flow, copy.deepcopy(CHANGE))}
    return out


print(json.dumps(stamped({"query": gen_query(), "state_flow": gen_flow()}), indent=1, ensure_ascii=False))
