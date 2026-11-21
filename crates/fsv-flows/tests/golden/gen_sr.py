"""Golden fixtures for fsv-flows `sr` (cli_sr draft functions).

Run from the Python worktree:

    cd /Users/phusit.somb/lab/work/fsv && \
      uv run python /Users/phusit.somb/lab/work/fsv-rust/crates/fsv-flows/tests/golden/gen_sr.py

Writes ``sr.json`` next to this script: catalog items (the invented Acme
items from ``tests/acme_catalog.py`` plus a synthetic "Kitchen Sink" item),
annotated templates from ``render_sr_template``, ``parse_sr_template``
results, and ``resolve_sr_draft`` precedence/error cases. Temporary paths
are written as ``<TMP>``; no network.
"""
from __future__ import annotations

import json
import os
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parents[3] / "port" / "golden"))
from provenance import py_root, stamped  # noqa: E402

sys.path.insert(0, str(py_root() / "tests"))
os.environ.pop("FSV_TZ", None)

TMP = Path(os.path.realpath(tempfile.mkdtemp(prefix="fsv-sr-golden-")))
os.environ["FSV_CONFIG_DIR"] = str(TMP / "config")

import acme_catalog as A  # noqa: E402
from fsv import catalog as cat  # noqa: E402
from fsv import cli_sr  # noqa: E402
from fsv.drafts import Draft  # noqa: E402
from fsv.forms import LookupValue  # noqa: E402

FETCHED_AT = 1900000000.0
BASE = "/work/drafts"
JANE = LookupValue(9000001, "Jane Doe", A.JANE)
NOBODY = LookupValue(9000009, "No Email", None)


def kitchen_sink() -> dict:
    cf = A._cf
    env_choices = [{"id": f"e{i}", "value": v, "label": v, "position": i,
                    **({"description": f"About  {v}\n deploys"} if i in (1, 3) else {}),
                    **({"dependent_ids": {"field": ["k-host"]}} if v == "Staging" else {})}
                   for i, v in enumerate(["Production", "Staging", "Dev", "QA", "Perf", "Demo", "Sandbox", "Edge",
                                          "Canary", "Blue", "Green", "Lab", "Training", "Archive"], 1)]
    return {
        "id": 4400009, "display_id": 90, "name": "Kitchen Sink", "short_description": "Every kind",
        "category_id": 7003, "category_name": "Hardware", "workspace_id": A.WS,
        "updated_at": "2030-02-01T10:00:00+09:00", "description": "",
        "allow_attachments": True, "allow_quantity": False, "attachment_mandatory": False,
        "configs": {"attachment_mandatory": False, "subject": "", "auto_gen_document": False},
        "custom_fields": [
            cf("k-notice", "readme", 1024, 1, "&lt;p&gt;Read &lt;i&gt;this&lt;/i&gt; first.&lt;/p&gt;",
               field_options={"content_name": "Read me"}),
            cf("k-env", "environment", 1019, 2, "Environment", required=True, choices=env_choices,
               fields=[cf("k-host", "staging_host", 1001, 1, "Staging host", required=True, parent_id="k-env")]),
            cf("k-when", "start_at", 1007, 3, "Start at", field_options={"date_only": "false"}),
            cf("k-count", "count", 1021, 4, "Count", helptext="How many <b>units</b>\n are needed"),
            cf("k-ok", "confirmed", 1006, 5, "Confirmed"),
            cf("k-notes1", "notes_a", 1001, 6, "Notes"),
            cf("k-notes2", "notes_b", 1008, 7, "Notes"),
            cf("k-cc", "cost_center", 1001, 8, "Cost center: main", placeholder="e.g. &lt;b&gt;CC-1&lt;/b&gt;"),
            cf("k-files", "evidence", 1014, 9, "Evidence"),
        ],
    }


def kitchen_rules() -> dict:
    return {"business_rules": [{
        "id": 9100009, "name": "Production lock", "workspace_id": A.WS, "module_type": "service_request",
        "item_id": 4400009, "deleted": False, "user_type": "both", "form_type": "both", "state": "active",
        "rule_type": "client_side", "description": "",
        "condition_data": {"all": [{"name": "customField.sr_environment", "operator": "is", "value": ["Production"],
                                    "evaluate_on": "change"}]},
        "action_data": {"action": [{"name": ["customField.sr_cost_center"], "property": "disable"},
                                   {"name": ["Production changes need <b>approval</b>"], "property": "message"}]},
        "is_reversible": True, "is_validation": True,
    }], "user": {"first_name": "Test", "last_name": "Agent", "time_zone": "Tokyo"}}


ITEMS = {
    "feature": (A.feature_request()["service_item"], A.rules_for(4400001)),
    "vpn": (A.vpn_access()["service_item"], A.rules_for(4400002)),
    "sink": (kitchen_sink(), kitchen_rules()),
}


def meta(key: str) -> cat.ItemMeta:
    item, rules = ITEMS[key]
    return cat.ItemMeta(item, rules, A.WS, FETCHED_AT, source="server")


def draft_json(d: Draft) -> dict:
    out = {"answers": d.answers, "requester": d.requester, "requested_for": d.requested_for,
           "description_html": d.description_html, "attachments": d.attachments, "planning": d.planning,
           "ack": d.ack, "preset": d.preset, "base_dir": None if d.base_dir is None else str(d.base_dir)}
    return json.loads(json.dumps(out, default=str).replace(str(TMP), "<TMP>"))


def who(r):
    return None if r is None else {"id": r.id, "display": r.display, "email": r.email}


def template(item: str, text: str, *, req=JANE, flag=None, errors=False, base_dir=BASE, ack=False) -> dict:
    m = meta(item)
    d = cli_sr.parse_sr_template(text, Path(BASE))
    d.answers = cat.file_answers(m.spec, d.answers, d.base_dir)
    plan = cat.build_sr_plan(m, d, resolver=None, requester=req, ack_all=ack)
    out = cli_sr.render_sr_template(plan, errors=list(plan.errors) if errors else None, requester=flag,
                                    base_dir=base_dir)
    return {"item": item, "draft": text, "requester": who(req), "flag": flag, "with_errors": errors,
            "base_dir": base_dir, "ack": ack, "plan_errors": list(plan.errors), "template": out,
            "reparsed": draft_json(cli_sr.parse_sr_template(out, Path(BASE)))}


FEATURE_FULL = """\
requester: sam.lee@example.com
requested_for: "  sam.leeds@example.com  "
answers:
  Title: Dark mode
  Details: |
    Please add
    dark mode.
  Needed by: 2030-12-03
  1. Expected benefit: More revenue
  1.1 Revenue target: +5%
  Priority: Medium - planned work
  Overview deck (.pptx): deck.pptx
attachments: [brief.pdf, /abs/spec.pdf]
"""

FEATURE_HIDDEN = """\
answers:
  Title: Dark mode
  1. Expected benefit: Lower cost
  1.1 Revenue target: +5%
  Colour: blue
  paragraph_102: "#legacy: yes"
"""

SINK = """\
answers:
  Environment: Production
  Start at: 2030-12-03 09:30
  Count: 3
  Confirmed: true
  notes_a: one
  notes_b: "two\\nlines"
  Evidence: [a.png, sub/b.png]
ack: [all]
"""

TEMPLATES = {
    "feature_empty": template("feature", "", req=None, base_dir=None),
    "feature_full": template("feature", FEATURE_FULL),
    "feature_full_errors": template("feature", FEATURE_FULL, req=None, errors=True, base_dir="/work"),
    "feature_flag": template("feature", FEATURE_FULL, flag="flag.user@example.com"),
    "feature_hidden": template("feature", FEATURE_HIDDEN, req=NOBODY, errors=True),
    "vpn_admin": template("vpn", "requester: sam.lee@example.com\nanswers:\n  Access level: Admin\n"
                                 "  Reason: travel\n", req=None, base_dir=None, errors=True),
    "vpn_read": template("vpn", "answers:\n  Access level: Read  write\n", base_dir=None),
    "sink_empty": template("sink", ""),
    "sink_staging": template("sink", "answers:\n  Environment: Staging\n  Staging host: st-1\n"),
    "sink_prod": template("sink", SINK, errors=True),
    "sink_prod_ack": template("sink", SINK, ack=True),
}


def parse_case(text: str) -> dict:
    try:
        return {"text": text, "draft": draft_json(cli_sr.parse_sr_template(text, Path(BASE)))}
    except ValueError as e:
        return {"text": text, "error": str(e)}


PARSE = [parse_case(t) for t in [
    "",
    "requested_for: target@example.com\n",
    "requested_for: '  t@example.com  '\nrequester: jane\n",
    "requested_for: '   '\n",
    "requested_for: ''\n",
    "requested_for: 12\n",
    "requested_for: [a]\n",
    "requested_for: null\nanswers: {A: 1}\n",
    "colour: x\n",
    "colour: x\nrequested_for: 12\n",
    "requested_for: 12\nanswers: [1]\n",
    "answers: [1]\n",
    "- a\n",
    "attachments: [a.pdf, /b.pdf]\nack: all\npreset: p\n",
    "description_md: '*x*'\nplanning:\n  P: [f.txt]\n",
]]


# ---------------------------------------------------------------------------
# resolve_sr_draft: files under TMP, cwd TMP/cwd, presets in TMP/config/presets

def write(rel: str, text: str) -> None:
    p = TMP / rel
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(text)


FILES = {
    "config/presets/acme.yaml": "requester: preset.user@example.com\nrequested_for: pre.for@example.com\n"
                                "answers:\n  Title: From preset\n  Priority: Low - cosmetic, reports\n"
                                "  Overview deck (.pptx): preset-deck.pptx\nattachments: [p.pdf]\nack: [p]\n",
    "config/presets/other.yml": "answers:\n  Details: other preset\n",
    "config/presets/nested.yaml": "preset: acme\n",
    "config/presets/badfor.yaml": "requested_for: 7\n",
    "drafts/full.yaml": "preset: acme\nrequester: file.user@example.com\nanswers:\n  Title: From file\n"
                        "  Details: file details\n  Overview deck (.pptx): [sub/deck.pptx, /abs/d2.pptx]\n"
                        "attachments: [brief.pdf]\nack: [f]\n",
    "drafts/plain.yaml": "requested_for: file.for@example.com\nanswers:\n  Title: Plain\n  goal_1_kpi: x\n",
    "drafts/empty.yaml": "",
    "drafts/emptyfor.yaml": "requested_for: ''\npreset: acme\n",
    "drafts/bad.yaml": "colour: red\n",
    "drafts/badfor.yaml": "requested_for: 7\n",
    "drafts/nested.yaml": "preset: nested\n",
    "drafts/missing_preset.yaml": "preset: nope\n",
}
for rel, text in FILES.items():
    write(rel, text)
(TMP / "cwd").mkdir(exist_ok=True)
os.chdir(TMP / "cwd")


def resolve_case(file, preset, sets, attach) -> dict:
    f = None if file is None else (file if file == "" else str(TMP / file))
    case = {"file": file, "preset": None if preset is None else preset.replace(str(TMP), "<TMP>"),
            "sets": sets, "attach": attach}
    try:
        d = cli_sr.resolve_sr_draft(meta("feature").spec, f, preset, sets, attach)
    except ValueError as e:
        return {**case, "error": str(e).replace(str(TMP), "<TMP>")}
    return {**case, "draft": draft_json(d)}


RESOLVE = [
    resolve_case(None, None, None, None),
    resolve_case("", None, ["Title=x"], None),
    resolve_case("drafts/plain.yaml", None, None, None),
    resolve_case("drafts/full.yaml", None, None, None),
    resolve_case("drafts/full.yaml", None, ["Title=From set", "Overview deck (.pptx)=set.pptx", " Details = a=b"],
                 ["extra.pdf", "/abs/x.pdf", "../up.pdf"]),
    resolve_case("drafts/full.yaml", "other", None, None),
    resolve_case("drafts/plain.yaml", "acme", ["Priority=Critical"], None),
    resolve_case(None, "acme", None, ["a.pdf"]),
    resolve_case(None, "acme", ["Overview deck (.pptx)="], None),
    resolve_case("drafts/emptyfor.yaml", None, None, None),
    resolve_case("drafts/empty.yaml", None, None, []),
    resolve_case(None, str(TMP / "config/presets/other.yml"), None, None),
    resolve_case("drafts/nested.yaml", None, None, None),
    resolve_case(None, "nested", None, None),
    resolve_case("drafts/missing_preset.yaml", None, None, None),
    resolve_case("drafts/bad.yaml", None, None, None),
    resolve_case("drafts/badfor.yaml", None, None, None),
    resolve_case(None, "badfor", None, None),
    resolve_case("drafts/nope.yaml", None, None, None),
    resolve_case(None, "missing.yaml", None, None),
    resolve_case(None, None, ["novalue"], None),
    resolve_case(None, None, [" =v"], None),
]

out = {
    "items": {k: {"item": v[0], "rules": v[1]} for k, v in ITEMS.items()},
    "workspace_id": A.WS,
    "fetched_at": FETCHED_AT,
    "base": BASE,
    "templates": TEMPLATES,
    "parse": PARSE,
    "files": FILES,
    "resolve": RESOLVE,
}

(HERE / "sr.json").write_text(json.dumps(stamped(out), indent=1, ensure_ascii=False, default=str) + "\n")
print("wrote", HERE / "sr.json")
