"""Oracle for fsv-api views/schema loaders: runs the Python functions with a
recording fake client and dumps inputs + outputs as JSON."""
from __future__ import annotations

import copy
import json
import os
import sys
import tempfile
import zoneinfo

_TMP = tempfile.mkdtemp(prefix="fsv-oracle-")
os.environ["FSV_CONFIG_DIR"] = os.path.join(_TMP, "config")
os.environ["FSV_CACHE_DIR"] = os.path.join(_TMP, "cache")
os.environ["FSV_NO_KEYCHAIN"] = "1"
os.environ.pop("FSV_TZ", None)
sys.path.insert(0, "/Users/phusit.somb/lab/work/fsv/tests")

import acme_views as A  # noqa: E402
import acme_change_form as F  # noqa: E402
from fsv import cli, formmeta, views as V  # noqa: E402
from fsv.errors import APIError, SessionError  # noqa: E402
from fsv.resources import CHANGES, TICKETS  # noqa: E402


class Fake:
    """`routes`: path -> JSON | {"__status": n} | "__session" | "__other"."""

    def __init__(self, routes):
        self.routes = routes
        self.calls = []

    def int_get(self, path, params=None):
        self.calls.append([path, params])
        r = self.routes.get(path, {"__status": 404})
        if r == "__session":
            raise SessionError("expired")
        if r == "__other":
            raise RuntimeError("boom")
        if isinstance(r, dict) and "__status" in r:
            raise APIError(r["__status"], {"description": "nope"})
        return copy.deepcopy(r)


def miss(m):
    return None if m is None else {"what": m.what, "status": m.status, "note": m.note}


def run(fn):
    try:
        return {"ok": fn()}
    except SessionError as e:
        return {"session": str(e)}
    except Exception as e:  # noqa: BLE001
        return {"error": f"{type(e).__name__}: {e}"}


out = {}

# ------------------------------------------------------------ views
VIEW_PAYLOADS = {
    "acme": {"change_filters": [
        {"id": "my_open", "name": "New & My Open Changes", "default": True},
        {"id": "all", "name": "All Changes", "default": True},
        {"id": 9001, "name": "My Team's Changes", "user_id": 700001, "default": False}],
        "meta": {"editable_filter_ids": []}},
    "list": [{"id": 1, "name": "", "default": 1, "user_id": None}, {"id": ""}, "str", {"id": 0, "name": "Zero"},
             {"id": None, "name": "x"}, {"id": 2.5, "name": None, "default": [], "user_id": 3}],
    "my_views": {"meta": 1, "my_views": [{"id": "a", "name": "A", "default": False}]},
    "not_list": {"change_filters": {"id": "x"}},
    "plain_filters": {"filters": [{"id": "f", "name": "F"}]},
    "scalar": 5,
    "status403": {"__status": 403},
    "status500": {"__status": 500},
    "other": "__other",
    "session": "__session",
}
out["load_views"] = {}
for k, payload in VIEW_PAYLOADS.items():
    c = Fake({"change-filters": payload})
    got = run(lambda: (lambda r: {"views": [dict(v.to_json(), raw=dict(v.raw)) for v in r[0]], "missing": miss(r[1])})(
        V.load_views(c, CHANGES)))
    out["load_views"][k] = {"payload": payload, "result": got, "calls": c.calls}

SETTINGS = {
    "nested": {"change_filter": {"id": "my_open", "order_by": "created_at", "query_hash": []}},
    "first_mapping": {"meta": [1], "x": {"a": 1}, "y": {"b": 2}},
    "flat": {"order_by": "updated_at", "order_type": "asc"},
    "list": [{"a": 1}],
    "status403": {"__status": 403},
    "other": "__other",
    "session": "__session",
}
out["view_settings"] = {}
view = V.SavedView("my_open", "Mine")
for k, payload in SETTINGS.items():
    c = Fake({"change-filters/my_open": payload})
    out["view_settings"][k] = {"payload": payload, "result": run(lambda: V.view_settings(c, CHANGES, view)),
                               "calls": c.calls}

# cli._saved_views (+ _load_views)
out["saved_views"] = {}
for k, payload in {**VIEW_PAYLOADS, "first_key": {"x_filters": [{"id": 1, "name": "One"}],
                                                   "y_filters": [{"id": 2}]},
                   "key_not_list": {"change_filters": "abc"}, "empty_dict": {}}.items():
    c = Fake({"change-filters": payload})
    got = run(lambda: [dict(v.to_json(), raw=v.raw) for v in cli._saved_views(c, CHANGES)])
    out["saved_views"][k] = {"payload": payload, "result": got}

out["resolve_view_name"] = []
for text in [None, "", "ALL", "  my_open ", "new & my open changes", "My team's changes", "nope", "9001"]:
    c = Fake({"change-filters": VIEW_PAYLOADS["acme"]})
    out["resolve_view_name"].append({"text": text, "result": run(lambda: cli._resolve_view_name(c, CHANGES, text))})

# ------------------------------------------------------------ edit rules + properties
STORE: dict = {}
CONFIGURED = {"ws": None}
formmeta._cached = lambda name: copy.deepcopy(STORE.get(name)) if formmeta._non_empty(STORE.get(name)) else None
formmeta._save = lambda name, kind, data: STORE.__setitem__(name, copy.deepcopy(data)) if formmeta._non_empty(data) else None
formmeta.configured_workspace = lambda: CONFIGURED["ws"]

change_fields = A.acme_change_fields()["change_fields"]
ACME_ROUTES = {"bootstrap/agents_groups": {"groups": A.GROUPS, "agents": A.AGENTS}}
CHANGE_RULES = F.rules()
TICKET_RULES = {"business_rules": [], "user": None}

out["edit_rules"] = []


def edit_case(name, module, record, fields, routes, configured=None, pre=None):
    STORE.clear()
    STORE.update(pre or {})
    CONFIGURED["ws"] = configured
    c = Fake(routes)
    got = run(lambda: list(V.load_edit_rules(c, module, record, fields)))
    out["edit_rules"].append({"name": name, "module": module, "record": record, "fields": fields, "routes": routes,
                              "configured": configured, "pre": pre or {}, "result": got,
                              "store": copy.deepcopy(STORE), "calls": c.calls})


rec501 = A.ACME_CHANGES[501]
edit_case("change_rules", "change", rec501, change_fields, {"ws/2/business_rules": CHANGE_RULES})
edit_case("cached", "change", rec501, change_fields, {},
          pre={"change-rules-edit_form-ws2-501": CHANGE_RULES})
edit_case("empty_rules", "ticket", A.ACME_TICKETS[601], A.ACME_TICKET_FIELDS["ticket_fields"],
          {"ws/2/business_rules": TICKET_RULES})
edit_case("all_404", "change", A.GLOBEX_CHANGES[77], A.globex_change_fields()["change_fields"], {})
edit_case("status_500", "change", rec501, change_fields, {"ws/2/business_rules": {"__status": 500}})
edit_case("listing_only", "change", rec501, change_fields,
          {"ws/2/business_rules": {"business_rules": [{"id": 1, "name": "r"}]}})
edit_case("other_error", "change", rec501, change_fields, {"ws/2/business_rules": "__other"})
edit_case("session", "change", rec501, change_fields, {"ws/2/business_rules": "__session"})
edit_case("configured", "change", rec501, change_fields, {"ws/9/business_rules": CHANGE_RULES}, configured=9)
edit_case("no_ws_uses_fields", "change", {"id": 5}, change_fields, {"ws/2/business_rules": CHANGE_RULES})
edit_case("no_ids", "change", {}, [{"name": "x", "default_field": True}], {"business_rules": CHANGE_RULES})

out["properties"] = []
TOKYO = zoneinfo.ZoneInfo("Asia/Tokyo")


def prop_case(name, res, record, schema, routes, user=A.ME, tz=True):
    STORE.clear()
    CONFIGURED["ws"] = None
    c = Fake(routes)
    got = run(lambda: (lambda r: {"props": [p.to_json() for p in r[0]], "note": r[1]})(
        V.load_properties(c, res, record, schema, TOKYO if tz else None, user)))
    out["properties"].append({"name": name, "res": res.name, "record": record, "schema": schema, "routes": routes,
                              "user": user, "tz": "Asia/Tokyo" if tz else None, "result": got, "calls": c.calls,
                              "store_keys": sorted(STORE)})


chg_routes = {**ACME_ROUTES, "ws/2/business_rules": CHANGE_RULES}
tkt_routes = {**ACME_ROUTES, "ws/2/business_rules": TICKET_RULES}
prop_case("change_501", CHANGES, rec501, {"fields": change_fields}, chg_routes)
prop_case("change_502", CHANGES, A.ACME_CHANGES[502], {"fields": change_fields}, chg_routes)
prop_case("change_501_no_user", CHANGES, rec501, {"fields": change_fields}, chg_routes, user=None)
prop_case("ticket_601", TICKETS, A.ACME_TICKETS[601], {"fields": A.ACME_TICKET_FIELDS["ticket_fields"]}, tkt_routes)
prop_case("ticket_602", TICKETS, A.ACME_TICKETS[602], {"fields": A.ACME_TICKET_FIELDS["ticket_fields"]}, tkt_routes)
prop_case("globex_77", CHANGES, A.GLOBEX_CHANGES[77], A.globex_change_fields() and
          {"fields": A.globex_change_fields()["change_fields"]}, {"bootstrap/agents_groups": {"__status": 403}},
          user=None)
prop_case("globex_88", TICKETS, A.GLOBEX_TICKETS[88], {"fields": A.GLOBEX_TICKET_FIELDS["ticket_fields"]}, {},
          user=None)
prop_case("no_fields", CHANGES, rec501, {"fields": []}, {})
prop_case("schema_none", CHANGES, rec501, None, {})
prop_case("bad_tz_user", CHANGES, rec501, {"fields": change_fields}, chg_routes, user={"time_zone": "Nowhere/Land"})
prop_case("session_rules", CHANGES, rec501, {"fields": change_fields}, {"ws/2/business_rules": "__session"})
prop_case("unavailable_rules", CHANGES, rec501, {"fields": change_fields}, {"ws/2/business_rules": {"__status": 500}})
prop_case("bootstrap_session", CHANGES, {**rec501, "agent": None, "group": None},
          {"fields": change_fields}, {"bootstrap/agents_groups": "__session", "ws/2/business_rules": CHANGE_RULES})

extra_fields = change_fields + [F._f("notes_text", "Notes", "custom_paragraph", 12.5)]
cloud_no = {**rec501, "custom_fields": {**rec501["custom_fields"], "cloud_only": "No", "staging_start": "2030-06-20",
                                        "staging_end": "2030-06-21T10:00:00Z", "reviewer": 123,
                                        "notes_text": "line one\n<b>line</b>   two\n\n"}}
prop_case("cloud_no_extras", CHANGES, cloud_no, {"fields": extra_fields}, chg_routes)
prop_case("reviewer_object", CHANGES, {**cloud_no, "custom_fields": {**cloud_no["custom_fields"],
                                                                     "reviewer": {"id": 5, "name": "Rev"}}},
          {"fields": extra_fields}, chg_routes, user=None)

FIXTURES = {"change_fields": change_fields, "change_rules": CHANGE_RULES,
            "ticket_fields": A.ACME_TICKET_FIELDS["ticket_fields"],
            "globex_change_fields": A.globex_change_fields()["change_fields"],
            "bootstrap": {"groups": A.GROUPS, "agents": A.AGENTS}}
FIXTURES = json.loads(json.dumps(FIXTURES, default=str))
out = json.loads(json.dumps(out, default=str))


def dedupe(v, top=False):
    if not top:
        for name, fx in FIXTURES.items():
            if v == fx:
                return {"$ref": name}
    if isinstance(v, dict):
        return {k: dedupe(x) for k, x in v.items()}
    if isinstance(v, list):
        return [dedupe(x) for x in v]
    return v


out = dedupe(out, top=True)
out["$fixtures"] = FIXTURES
json.dump(out, open("/tmp/fsv_api_views/views_oracle.json", "w"), separators=(",", ":"), default=str)
print("ok")
