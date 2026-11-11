"""Planning fields are resolved by label (then id/position), never by a
fixed internal name: the same label has different names on different
accounts. Invented Acme/Globex data; no network."""
from __future__ import annotations

import pytest

import acme_change_form as A
from fsv import create, views
from fsv.drafts import Draft
from fsv.forms import LookupValue
from fsv.formmeta import FormMeta
from fsv.formui.session import PLANNING, change_session

# Globex: the labels Acme uses, with other internal names (one name even
# looks like another field's label).
GLOBEX_PLANNING = [
    {"id": 9000013001, "name": "rollout_plan", "label": "Backout Plan", "field_type": "default_backout_plan"},
    {"id": 9000013002, "name": "cp_gx_17", "label": "Rollout Plan", "field_type": "default_change_plan"},
    {"id": 9000013003, "name": "cfp_gx_checklist", "label": "Go-Live Checklist", "field_type": "planning_field"},
]


def test_label_wins_over_another_fields_internal_name():
    assert create.resolve_planning_field("Rollout Plan", GLOBEX_PLANNING) == "cp_gx_17"
    assert create.resolve_planning_field("rollout plan", GLOBEX_PLANNING) == "cp_gx_17"
    assert create.resolve_planning_field("Backout Plan", GLOBEX_PLANNING) == "rollout_plan"
    assert create.resolve_planning_field("9000013003", GLOBEX_PLANNING) == "cfp_gx_checklist"


def test_ambiguous_labels_and_helptext_never_select_first():
    rows = [dict(GLOBEX_PLANNING[0], label="Review", helptext="Shared help"),
            dict(GLOBEX_PLANNING[1], label="Review", helptext="Shared help")]
    with pytest.raises(Exception, match="ambiguous"):
        create.resolve_planning_definition("Review", rows)
    with pytest.raises(Exception, match="ambiguous"):
        create.resolve_planning_definition("Shared help", rows)
    assert create.resolve_planning_definition(rows[1]["name"], rows) == rows[1]


def test_reordered_opaque_rows_never_match_position_or_conflicting_id():
    rows = [{"id": 111}, {"id": 222}, {"id": 333}]
    assert create.match_planning_row(GLOBEX_PLANNING[0], rows[::-1], GLOBEX_PLANNING) is None
    conflicting = [{"field_id": 123, "label": "Backout Plan"}]
    assert create.match_planning_row(GLOBEX_PLANNING[0], conflicting) is None
    with pytest.raises(Exception, match="ambiguous"):
        create.match_planning_row({"label": "Review"}, [{"label": "Review"}, {"label": "Review"}])


def test_opaque_existing_rows_block_before_writes():
    class C:
        def int_get(self, path):
            return {"change_planning_fields": [{"id": 111}]}

        def int_post(self, *args):
            pytest.fail("must not write")

    with pytest.raises(Exception, match="unidentified"):
        create.update_planning_field(1, "rollout_plan", description="x", c=C())


def test_no_hardcoded_name_aliases():
    with pytest.raises(Exception, match="planning field not found"):
        create.resolve_planning_field("reason_for_change", GLOBEX_PLANNING)


def test_a_draft_written_on_one_account_works_on_another():
    fields = A.fields()
    rows = [f for f in fields["change_fields"] if f["field_type"] != "planning_field"]
    rows += [A._f(p["name"], p["label"], p["field_type"], None) for p in GLOBEX_PLANNING]
    meta = FormMeta("change", "new_form", {"change_fields": rows}, A.rules(), 2, 0.0, "server")
    from fsv.change_flow import _resolve_planning

    draft = Draft(planning={"Go-Live Checklist": {"files": ["a.txt"], "description_html": None},
                            "Rollout Plan": {"files": ["b.txt"], "description_html": None}})
    out, errors = _resolve_planning(meta, draft)
    assert not errors
    assert [(p["label"], p["field_id"]) for p in out] == [("Go-Live Checklist", "cfp_gx_checklist"),
                                                         ("Rollout Plan", "cp_gx_17")]


def test_detail_page_matches_rows_by_id_or_label_when_names_differ():
    class C:
        def int_get(self, path, params=None):
            assert path.endswith("/planning-fields")
            return {"change_planning_fields": [
                {"name": "other_name", "field_id": 9000013002, "description": "<p>step 1</p>", "attachments": []},
                {"name": "x", "label": "Go-Live Checklist", "description": "<p>all green</p>",
                 "attachments": [{"name": "runbook.pdf"}]},
            ]}

    entries, miss = views.load_planning(C(), {"display_id": 5}, {"fields": GLOBEX_PLANNING})
    assert miss is None
    by_label = {e.label: e for e in entries}
    assert by_label["Rollout Plan"].text == "step 1"
    assert by_label["Go-Live Checklist"].files == ["runbook.pdf"]
    assert by_label["Backout Plan"].empty


def test_update_finds_the_row_by_label_when_its_name_differs(tmp_path):
    class C:
        def __init__(self):
            self.puts = []

        def int_get(self, path, params=None):
            return {"change_planning_fields": [{"name": "cfp_other", "label": "Go-Live Checklist",
                                                "description": "<p>x</p>", "attachments": []}]}

        def int_put(self, path, body):
            self.puts.append((path, body))
            return {"ok": True}

    c = C()
    create.update_planning_field(1, "cfp_gx_checklist", description="<p>y</p>", c=c, label="Go-Live Checklist")
    assert c.puts == [("changes/1/planning-fields/cfp_other", {"description": "<p>y</p>"})]


def test_interactive_form_offers_default_planning_types_by_label():
    fake = A.FakeClient()
    fake.fields["change_fields"] += [A._f(p["name"], p["label"], p["field_type"], None) for p in GLOBEX_PLANNING[:2]]
    s = change_session(fake, requester=LookupValue(9000007001, "Jane Doe", "jane.doe@example.com"))
    keys = [it.key for it in s.items if it.key.startswith(PLANNING)]
    assert keys == [PLANNING + "Go-Live Checklist", PLANNING + "Backout Plan", PLANNING + "Rollout Plan"]
