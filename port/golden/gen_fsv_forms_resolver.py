"""What FormState hands a LookupResolver, and what it makes of the answer.

Usage (from the Python worktree):
    cd /Users/phusit.somb/lab/work/fsv && uv run python \
        ../fsv-rust/port/golden/gen_fsv_forms_resolver.py \
        ../fsv-rust/crates/fsv-forms/testdata/resolver_expected.json
"""
import json
import sys
from pathlib import Path

from fsv.forms.errors import FormError
from fsv.forms.spec import FormSpec
from fsv.forms.state import FormState, LookupValue
from provenance import stamped

FIELDS = [
    {"name": "requester", "label": "Requester", "field_type": "default_requester"},
    {"name": "group", "label": "Group", "field_type": "default_group"},
    {"name": "agent", "label": "Agent", "field_type": "default_agent"},
    {"name": "change_window", "label": "Window", "field_type": "default_change_window"},
    {"name": "cf_vendor", "label": "Vendor", "field_type": "custom_lookup", "default_field": False,
     "lookup_config": {"data_source": 77, "link": "/api/_/vendors"}},
    {"name": "cf_person", "label": "Person", "field_type": "custom_lookup", "default_field": False},
]
RECORDS = [[1, "Jane Doe", "jane@x.io"], [2, "Jane Doerr", "doerr@x.io"], [42, "Answer", None]]
RAWS = ["Jane", "jane doe", " 42 ", 42, "nobody"]


class Recorder:
    def __init__(self):
        self.calls = []

    def lookup(self, entity, query, field):
        self.calls.append([entity, query, field.api_name])
        q = query.casefold()
        return [LookupValue(i, d, e) for i, d, e in RECORDS
                if q in d.casefold() or q in (e or "") or str(i) == query]


spec = FormSpec.from_form_fields(FIELDS, module="change")
rows = []
for f in FIELDS:
    for raw in RAWS:
        rec = Recorder()
        st = FormState(spec, resolver=rec)
        try:
            v = st.parse(f["name"], raw)
            out = {"id": v.id, "display": v.display, "email": v.email}
        except FormError as exc:
            out = {"error": str(exc)}
        rows.append({"field": f["name"], "raw": raw, "calls": rec.calls, "out": out})

Path(sys.argv[1]).write_text(json.dumps(stamped({"fields": FIELDS, "records": RECORDS, "rows": rows}),
                                        indent=1, ensure_ascii=False) + "\n")
