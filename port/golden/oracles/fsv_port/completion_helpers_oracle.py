"""Golden output for fsv.completion private helpers on the dynamic fixture caches.

Usage: uv run python port/golden/oracles/fsv_port/completion_helpers_oracle.py <input.json> <out.json>
"""
import json
import os
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from provenance import stamped  # noqa: E402

inp = json.loads(Path(sys.argv[1]).read_text())
OUT = Path(sys.argv[2])
tmp = Path(tempfile.mkdtemp())
os.environ["FSV_CONFIG_DIR"] = str(tmp / "cfg")
os.environ["FSV_CACHE_DIR"] = str(tmp / "cache")
(tmp / "cfg").mkdir()
(tmp / "cache" / "schema").mkdir(parents=True)
(tmp / "cache" / "filters").mkdir(parents=True)


def put(path, doc):
    doc = dict(doc)
    doc.setdefault("saved_at", time.time())
    path.write_text(json.dumps(doc))


for name, doc in inp["schema"].items():
    put(tmp / "cache" / "schema" / f"default--{name}.json", doc)
for name, doc in inp["filters"].items():
    put(tmp / "cache" / "filters" / f"default--{name}.json", doc)
put(tmp / "cache" / "groups--default.json", inp["groups"])

from fsv import completion as c  # noqa: E402
from fsv.resources import REGISTRY  # noqa: E402
import fsv.client  # noqa: E402

calls = []


class Fake:
    def autocomplete(self, kind, query, params=None):
        calls.append(["autocomplete", kind, query, params])
        return [dict(r) for r in inp["remote"][kind]]


fsv.client.get_client = lambda: Fake()

FIELDS = [
    {"name": "a", "label": "A", "field_type": "t", "default_field": True},
    {"name": "b"},
    {"label": "L"},
    {"name": 0, "label": None, "default_field": 0},
    {"name": "x", "field_type": None, "default_field": "yes"},
    {"name": 12, "label": ""},
    {"name": "n", "label": "Région", "field_type": "custom_dropdown", "default_field": False},
]
CHOICES = [
    {"value": "V", "name": "N"},
    {"name": "N", "label": "L"},
    {"label": "L"},
    {"id": 5},
    {"id": 0},
    {"value": "", "name": "N"},
    {},
    {"id": 1.5},
    {"value": True},
]
CASES = [
    ["alice@x.com", "AL"], ["Alice", "al"], ["bob", "al"], ["ALBERT", "al"],
    ["Straße", "STRASS"], ["\ufb01le", "FI"], ["x", ""], ["", "a"],
]
MATCHES = [
    ["Status", "st"], ["status", ""], ["Straße", "strasse"], ["x", "xy"], ["", ""], ["Région", "RÉG"],
]
PAIRS = [["table", "rich table"], ["Json", "JSON"], ["tsv", ""]]
FIND = [
    "status", "STATUS", "Status Note", "default:status", "custom:status", "custom:region",
    "plan", "stat", "nosuch", "default:", "custom:approved", "foo:bar", "", "Rollout",
]
CHOICE_FIELDS = ["status", "priority", "impact", "risk", "plan", "nosuch", "cf_region", "default:priority", "Région Straße"]
USERS = [["agents", "al"], ["agents", " a "], ["agents", " al "], ["requesters", "al"], ["agents", "A"], ["agents", "zz"]]

out = {
    "fields": FIELDS,
    "field_scope": [c._field_scope(f) for f in FIELDS],
    "field_name": [c._field_name(f) for f in FIELDS],
    "field_label": [c._field_label(f) for f in FIELDS],
    "field_help": [c._field_help(f) for f in FIELDS],
    "choices": CHOICES,
    "choice_text": [c._choice_text(x) for x in CHOICES],
    "completion_case": [[a, b, c._completion_case(a, b)] for a, b in CASES],
    "match": [[a, b, c._match(a, b)] for a, b in MATCHES],
    "complete_pairs": [[inc, [list(p) for p in c._complete_pairs(inc, PAIRS)]] for inc in ["", "t", "J", "z"]],
    "pairs": PAIRS,
    "cached_fields": {r: c._cached_fields(REGISTRY[r]) for r in REGISTRY},
    "fields_or_fallback": {r: c._fields_or_fallback(REGISTRY[r]) for r in REGISTRY},
    "find_field": {r: [[n, (c._find_field(REGISTRY[r], n) or {}).get("name")] for n in FIND] for r in REGISTRY},
    "choices_for": {r: [[n, c._choices_for(REGISTRY[r], n)] for n in CHOICE_FIELDS] for r in REGISTRY},
    "cached_filters": {r: c._cached_filters(REGISTRY[r]) for r in REGISTRY},
    "cached_groups": c._cached_groups(),
}
users = {}
for mode in ("offline", "network"):
    c._completion_network = (lambda m=mode: m == "network")
    rows = []
    for kind, q in USERS:
        calls.clear()
        rows.append({"args": [kind, q], "out": [list(p) for p in c._remote_user_values(kind, q)], "calls": list(calls)})
    users[mode] = rows
out["remote_user_values"] = users

OUT.write_text(json.dumps(stamped(out), indent=1, ensure_ascii=False) + "\n")
print("ok")
