"""Golden fixtures for fsv-flows (drafts, change_flow, ticket_flow).

Run from the Python worktree:

    cd /Users/phusit.somb/lab/work/fsv && \
      uv run python /Users/phusit.somb/lab/work/fsv-rust/crates/fsv-flows/tests/golden/gen_flows.py

Writes ``flows.json`` next to this script. Uses the invented Acme change
form from ``tests/acme_change_form.py``; no network.
"""
from __future__ import annotations

import copy
import json
import os
import sys
import tempfile
import textwrap
from pathlib import Path

HERE = Path(__file__).resolve().parent
# Update-plan hashes bind config.DOMAIN. Pin an empty tenant before provenance
# imports fsv, which reads the domain once at import.
TMP = Path(tempfile.mkdtemp(prefix="fsv-flows-golden-"))
os.environ["FSV_CONFIG_DIR"] = str(TMP)
os.environ["FSV_CACHE_DIR"] = str(TMP / "cache")
os.environ["XDG_STATE_HOME"] = str(TMP / "state")
os.environ.pop("FSV_TZ", None)

sys.path.insert(0, str(HERE.parents[3] / "port" / "golden"))
from provenance import py_root, stamped  # noqa: E402

sys.path.insert(0, str(py_root() / "tests"))
from fsv import config as fsv_config  # noqa: E402

fsv_config.reload()
(TMP / "hints.yaml").write_text(textwrap.dedent("""\
    changes:
      - when: {"Cloud Only": "No"}
        warn: staging runs first
      - when: {"Services": [Search, Mail]}
        warn: the search team is paged
      - when: {"Notify Ops Team": "*"}
        warn: ops is notified
      - when: {"Nope Field": "x"}
        warn: never
"""))

import acme_change_form as A  # noqa: E402
from fsv import change_flow as flow, drafts, ticket_flow, yamlio  # noqa: E402
from fsv.formmeta import ClientResolver, FormMeta  # noqa: E402
from fsv.forms import LookupValue  # noqa: E402

JANE = LookupValue(9000007001, "Jane Doe", "jane.doe@example.com")
FETCHED_AT = 1900000000.0
BASE = "/work/drafts"

DRAFT = textwrap.dedent("""\
    requester: jane.doe@example.com
    answers:
      Subject: "Billing: rotate keys"
      Cloud Only: No
      Stage: Staging
      Staging Start: 2030-11-18 09:00
      Staging End: 2030-11-18 12:00
      Services: [Billing, Search]
      Planned Start Date: 2030-11-26 19:00
      Planned End Date: 2030-11-26 22:00
    description_md: |
      Rotate the **billing** keys.

      - step one
      - step two
    """)


# Field ids come from a process-wide counter. Snapshot once so every plan's
# metadata fingerprint is the fields blob stored in this file.
FIELDS = A.fields()
RULES = A.rules()


def meta(module: str = "change", form_type: str = "new_form", module_id=None) -> FormMeta:
    fields = copy.deepcopy(FIELDS)
    rules = copy.deepcopy(RULES)
    if module == "ticket":
        fields = {"ticket_fields": fields["change_fields"]}
    return FormMeta(module, form_type, fields, rules, 2, FETCHED_AT, "server", module_id)


def resolver() -> ClientResolver:
    return ClientResolver(A.FakeClient())


def plan_out(plan: flow.Plan, title: str) -> dict:
    j = plan.to_json()
    j["review"].pop("metadata")
    return {
        "ok": plan.ok,
        "hash": plan.hash,
        "payload": plan.payload,
        "errors": plan.errors,
        "risks": plan.risks,
        "json": j,
        "template": flow.template_for(plan, plan.draft, title=title, errors=list(plan.errors), base_dir=BASE),
        "plan_text": flow.plan_text(plan),
    }


def create(text: str, **kw) -> dict:
    d = drafts.parse_template(text, Path(BASE))
    plan = flow.build_create_plan(meta(), d, resolver=resolver(), requester=kw.pop("requester", JANE), **kw)
    return {"draft": text, **plan_out(plan, "New change")}


def update(answers: dict, **over) -> dict:
    d = drafts.Draft(answers=answers)
    rec = A.record(**over)
    plan = flow.build_update_plan(meta(form_type="edit_form", module_id=501), d, rec, 501, resolver=resolver())
    return {"answers": answers, "record": rec, **plan_out(plan, "Update change 501")}


LIFECYCLE_FLOW = {
    "state_list": [{"id": 1, "value": "Open"}, {"id": 2, "value": "Planning"}, {"id": 3, "value": "Closed"}],
    "transitions": {
        "1": [{"to_state": 2, "entry_criteria": [
            {"name": "planned_start_date", "operator": "is_not", "value": ""},
            {"fields": {"7": {"name": "customFields.cab_ref", "operator": "is_not", "value": None}}},
            {"name": "risk", "operator": "is", "value": 2}]}],
        "2": [{"to_state": 3, "entry_criteria": []}],
    },
    "state_properties": {"field_lock_config": {"1": [{"1": "subject", "2": "customFields.notify_ops"}]}},
}


def update_lifecycle(answers: dict, flow_data=LIFECYCLE_FLOW, **over) -> dict:
    """``build_update_plan(lifecycle=...)``; ``flow_data`` None raises APIError."""
    from fsv.errors import APIError

    calls = []

    def lifecycle():
        calls.append(1)
        if flow_data is None:
            raise APIError(503, "unavailable")
        return flow_data

    d = drafts.Draft(answers=answers)
    rec = A.record(**over)
    plan = flow.build_update_plan(meta(form_type="edit_form", module_id=501), d, rec, 501, resolver=resolver(),
                                  lifecycle=lifecycle)
    return {"answers": answers, "record": rec, "flow": flow_data, "lifecycle_reads": len(calls),
            **plan_out(plan, "Update change 501")}


def ticket(answers: dict, cc=(), **kw) -> dict:
    d = drafts.Draft(answers=answers, **kw)
    plan = ticket_flow.build_create_plan(meta("ticket"), d, resolver=None, requester=JANE, cc_emails=tuple(cc))
    j = plan.to_json()
    j["review"].pop("metadata")
    return {"answers": answers, "cc": list(cc), "extra": {k: v for k, v in kw.items()},
            "ok": plan.ok, "hash": plan.hash, "payload": plan.payload, "errors": plan.errors, "json": j}


def unexpand(path):
    """`~/...` for a path under HOME, so the fixture does not depend on it."""
    home = str(Path.home())
    return "~" + path[len(home):] if isinstance(path, str) and path.startswith(home + "/") else path


def draft_case(text: str) -> dict:
    try:
        d = drafts.parse_template(text, Path(BASE))
    except ValueError as e:
        return {"text": text, "error": str(e)}
    return {"text": text, "draft": {
        "answers": d.answers, "requester": d.requester, "description_html": d.description_html,
        "attachments": [unexpand(a) for a in d.attachments], "planning": d.planning, "ack": d.ack, "preset": d.preset}}


ROUND2 = textwrap.dedent("""\
    requester: jane.doe@example.com
    answers:
      Subject: "Billing: rotate keys"
      Cloud Only: No
      Planned Start Date: 2030-11-26 19:00
      Planned End Date: 2030-11-26 22:00
    description_md: |
      Rotate keys.
    """)

out = {
    "base_dir": BASE,
    "fields": FIELDS,
    "rules": RULES,
    "hints_yaml": (TMP / "hints.yaml").read_text(),
    "create": {
        "full": create(DRAFT),
        "empty": create("", requester=None),
        "round2": create(ROUND2),
        "urgent": create(DRAFT.replace("answers:\n", "answers:\n  Priority: Urgent\n")),
        "urgent_ack": create(DRAFT.replace("answers:\n", "answers:\n  Priority: Urgent\n") + "ack: [all]\n"),
        "status": create(DRAFT.replace("answers:\n", "answers:\n  Status: Planning\n  Notify Ops Team: Yes\n"
                                       "  Services: [Search, Mail, Docs]\n")),
        "unknown": create(DRAFT.replace("answers:\n", "answers:\n  Colour: red\n  Legacy Flag: true\n")),
        "yes_cloud": create(DRAFT.replace("Cloud Only: No", "Cloud Only: Yes")),
        "no_validate": create(ROUND2, allow_incomplete=True),
        "planning": create(DRAFT + "planning:\n  Go-Live Checklist:\n    description_md: see *plan*\n"
                                   "  Nope Plan: [x.txt]\n"),
        "missing_file": create(DRAFT + "attachments: [missing.pdf]\n"),
    },
    "update": {
        "subject": update({"Subject": "Billing: rotate all keys", "Services": "Billing"}),
        "status": update({"Status": "Planning", "Notify Ops Team": "Yes"}),
        "downtime": update({"Downtime Window": ""}),
        "nothing": update({"Subject": "Billing: rotate keys"}),
        "dates": update({"Planned Start Date": "2030-11-26 21:00", "Agent": "Alex Kim", "Group": "Platform Ops"}),
    },
    "update_lifecycle": {
        "locked": update_lifecycle({"Subject": "Billing: rotate all keys", "Services": "Billing"}),
        "edge_criteria": update_lifecycle({"Status": "Planning"}),
        "edge_criteria_lock": update_lifecycle({"Status": "Planning", "Notify Ops Team": "Yes"}),
        "not_allowed": update_lifecycle({"Status": "Closed"}),
        "from_planning": update_lifecycle({"Status": "Closed"}, status=2),
        "read_failure": update_lifecycle({"Status": "Planning"}, flow_data=None),
        "nothing": update_lifecycle({"Subject": "Billing: rotate keys"}, flow_data=None),
    },
    "ticket": {
        "basic": ticket({"Subject": "Example", "Description": "Help", "Cloud Only": "Yes", "Stage": "Live",
                         "Change Type": "Normal", "Planned Start Date": "2030-11-26 19:00",
                         "Planned End Date": "2030-11-26 22:00"}),
        "cc": ticket({"Subject": "Example", "Description": "Help", "Cloud Only": "Yes", "Stage": "Live",
                      "Change Type": "Normal", "Planned Start Date": "2030-11-26 19:00",
                      "Planned End Date": "2030-11-26 22:00"}, cc=["a@example.com", "b@example.com"]),
        "bad_cc": ticket({"Subject": "Example"}, cc=["Name <a@example.com>"]),
        "status_attach": ticket({"Subject": "Example", "Status": "Closed", "Requester": "x"},
                                attachments=["a.txt"]),
    },
    "drafts": [draft_case(t) for t in [
        "",
        "answers: {}\n",
        "preset: acme\nrequester: ''\nanswers:\n  true: x\n  1: 2\n  Services: [a, 3]\n",
        "description_md: '   '\n",
        "description_html: '<p>x</p>'\n",
        "description_md: a\ndescription_html: b\n",
        "colour: red\nanswers: {}\n",
        "- a\n- b\n",
        "answers: [1]\n",
        "attachments: ~/a.pdf\n",
        "attachments: [./a/../b.pdf, /abs/c.pdf, 7, true]\n",
        "attachments: [1.5]\n",
        "planning:\n  Rollout Plan: [r.xlsx]\n  Backout: b.txt\n  Notes:\n    description_md: '**x**'\n"
        "    files: [n.txt]\n  Html:\n    description_html: <b>x</b>\n",
        "planning:\n  X: \"a\\nb\"\n",
        "planning:\n  X: {files: [a], colour: red}\n",
        "planning:\n  X: 5\n",
        "planning: [a]\n",
        "ack: all\n",
        "ack: [1, x]\n",
        "requester: 0\npreset: 0\n",
        "requester: false\n",
    ]],
    "relpath": [[p, s, os.path.relpath(p, s)] for p, s in [
        ("/a/b/c.txt", "/a"), ("/a/b/c.txt", "/a/d"), ("/a/b", "/a/b"), ("/x/y", "/a/b/c"), ("/a//b/./c", "/a/"),
    ]],
    "yaml_kv": [[k, v, drafts._kv(k, v)] for k, v in [
        ("Subject", "Billing: rotate keys"), ("A", None), ("B", True), ("C", ["x", "y: z"]), ("D", "line1\nline2"),
        ("E: f", 3), ("G", "No"), ("H", "#tag"),
    ]],
    "scalar": [[v, yamlio.scalar(v)] for v in ["Services", "a b", "x: y", ""]],
}

(HERE / "flows.json").write_text(json.dumps(stamped(out), indent=1, ensure_ascii=False, default=str) + "\n")
print("wrote", HERE / "flows.json")
