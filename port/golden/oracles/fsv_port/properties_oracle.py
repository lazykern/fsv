"""Golden output for fsv-views properties (load_properties / load_edit_rules).

Usage: uv run python /tmp/fsv_port/properties_oracle.py <out.json>
"""
import copy
import datetime as dt
import json
import sys
from pathlib import Path

sys.path.insert(0, "/Users/phusit.somb/lab/work/fsv/tests")
import acme_change_form as A  # noqa: E402

from fsv import formmeta, views  # noqa: E402
from fsv.errors import APIError, SessionError  # noqa: E402
from fsv.resources import REGISTRY  # noqa: E402

OUT = Path(sys.argv[1])
ACME = json.loads(Path("/Users/phusit.somb/lab/work/fsv-rust/crates/fsv-forms/testdata/acme.json").read_text())


class Fake:
    """Routes: path -> {"ok": data} | {"api": status} | {"session": msg}; others 404."""

    def __init__(self, routes):
        self.routes = routes
        self.calls = []

    def _answer(self, r):
        if "api" in r:
            raise APIError(r["api"], {})
        if "session" in r:
            raise SessionError(r["session"])
        return copy.deepcopy(r["ok"])

    def int_get(self, path, params=None):
        self.calls.append(["int_get", path, dict(params) if params else None])
        if path not in self.routes:
            raise APIError(404, {})
        return self._answer(self.routes[path])

    def autocomplete(self, kind, query, params=None):
        self.calls.append(["autocomplete", kind, query, params])
        raise APIError(404, {})

    def lookup_choices(self, link, query):
        self.calls.append(["lookup_choices", link, query, None])
        raise APIError(404, {})


class Store:
    def __init__(self, seed, configured):
        self.data = dict(seed)
        self.configured = configured
        self.saves = []

    def install(self):
        formmeta._cached = lambda name: copy.deepcopy(self.data.get(name))
        formmeta._save = self.save
        formmeta.configured_workspace = lambda: self.configured

    def save(self, name, kind, data):
        self.saves.append([name, kind])
        self.data[name] = copy.deepcopy(data)


change_fields = A.fields()["change_fields"]
change_rules = A.rules()
change_record = A.record()
ticket_fields = ACME["ticket_fields"]["ticket_fields"]
ticket_rules = ACME["ticket_rules"]
ticket_record = {
    "id": 42, "display_id": 42, "workspace_id": 3, "subject": "Printer", "description": "<p>Broken</p>",
    "requester_id": 900, "responder_id": None, "type": "Incident", "priority": 2, "status": 2, "source": 2,
    "group_id": None, "custom_fields": {"customers_hit": 4, "due_on": "2026-03-04"},
}
BKK = {"time_zone": "Asia/Bangkok"}

CASES = [
    {"name": "change_ws2", "resource": "changes", "record": change_record, "fields": change_fields,
     "routes": {"ws/2/business_rules": {"ok": change_rules}}, "user": BKK},
    {"name": "change_no_rules", "resource": "changes", "record": change_record, "fields": change_fields,
     "routes": {}, "user": BKK},
    {"name": "change_rules_500", "resource": "changes", "record": change_record, "fields": change_fields,
     "routes": {"ws/2/business_rules": {"api": 500}}, "user": BKK},
    {"name": "change_session", "resource": "changes", "record": change_record, "fields": change_fields,
     "routes": {"ws/2/business_rules": {"session": "expired"}}, "user": BKK},
    {"name": "change_empty_rules", "resource": "changes", "record": change_record, "fields": change_fields,
     "routes": {"ws/2/business_rules": {"ok": {"business_rules": [], "user": None}}}, "user": None},
    {"name": "change_cached", "resource": "changes", "record": change_record, "fields": change_fields,
     "routes": {}, "user": BKK, "seed": {"change-rules-edit_form-ws2-900000501": change_rules}},
    {"name": "change_configured_ws", "resource": "changes", "record": change_record, "fields": change_fields,
     "routes": {"ws/5/business_rules": {"ok": change_rules}}, "user": BKK, "configured": 5},
    {"name": "change_bad_tz", "resource": "changes", "record": change_record, "fields": change_fields,
     "routes": {"ws/2/business_rules": {"ok": change_rules}}, "user": {"time_zone": "Mars/Olympus"}},
    {"name": "no_fields", "resource": "changes", "record": change_record, "fields": [], "routes": {}, "user": None},
    {"name": "ticket", "resource": "tickets", "record": ticket_record, "fields": ticket_fields,
     "routes": {"ws/3/business_rules": {"ok": ticket_rules}}, "user": BKK},
    {"name": "ticket_no_ws", "resource": "tickets", "record": {**ticket_record, "workspace_id": None}, "fields": ticket_fields,
     "routes": {"business_rules": {"ok": ticket_rules}}, "user": None},
]

TZ7 = dt.timezone(dt.timedelta(hours=7))
out = []
for case in CASES:
    store = Store(case.get("seed", {}), case.get("configured"))
    store.install()
    c = Fake(case["routes"])
    schema = {"fields": case["fields"]}
    try:
        props, note = views.load_properties(c, REGISTRY[case["resource"]], case["record"], schema, TZ7, case["user"])
        result = {"props": [p.to_json() for p in props], "note": note}
    except SessionError as e:
        result = {"session": str(e)}
    out.append({**case, "result": result, "calls": c.calls, "saves": store.saves})
    print(case["name"], json.dumps(result)[:200])

OUT.write_text(json.dumps(out, indent=1, ensure_ascii=False, default=str) + "\n")
