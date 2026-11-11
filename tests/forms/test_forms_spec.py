from __future__ import annotations

import pytest

import acme_forms
from fsv.forms import FormSpec, UnknownFieldError, plain_text


def test_kinds_and_flags(change_spec: FormSpec) -> None:
    by = change_spec.by_name
    expect = {
        "guide": "notice",
        "requester": "lookup_user",
        "department": "department",
        "team": "dropdown",
        "group": "lookup_group",
        "agent": "lookup_agent",
        "change_type": "dropdown",
        "services": "multi_select",
        "reviewers": "lookup_user",
        "staging_start": "datetime",
        "description": "richtext",
        "needs_review": "checkbox",
        "planned_start_date": "datetime",
        "downtime_window": "text",
        "estimated_hours": "number",
        "category": "nested",
        "sub_category": "nested",
        "item_category": "nested",
        "change_window": "lookup",
    }
    for name, kind in expect.items():
        assert by[name].kind == kind, name
    assert by["reviewers"].multi and by["services"].multi
    assert by["reviewers"].lookup_entity == "user"
    assert by["change_window"].lookup_entity == "change_window"
    assert by["team"].is_custom and not by["priority"].is_custom
    assert by["requester"].required_base and by["requester"].hint
    assert by["requester"].helptext == "e.g. jane.doe@acme.example"
    assert by["guide"].content.startswith("<p>Read the <b>change guide")
    assert plain_text(by["guide"].content) == "Read the change guide first."


def test_planning_fields_are_off_the_create_form(change_spec: FormSpec) -> None:
    names = [f.api_name for f in change_spec.ordered()]
    assert "rollback_doc" not in names and "extra_notes" not in names
    assert not change_spec.by_name["rollback_doc"].on_form


def test_order_follows_position_with_nested_levels_after_parent(change_spec: FormSpec) -> None:
    names = [f.api_name for f in change_spec.ordered()]
    assert names[:4] == ["guide", "requester", "department", "team"]
    i = names.index("category")
    assert names[i : i + 3] == ["category", "sub_category", "item_category"]
    assert names.index("impact") < names.index("priority") < names.index("risk")


def test_defaults_first_visible_choice_for_default_dropdowns(change_spec: FormSpec) -> None:
    by = change_spec.by_name
    assert by["change_type"].default == "Normal"
    assert by["impact"].default == "Low" and by["risk"].default == "Low"
    assert by["status"].default == "Open"
    assert by["department"].default is None and by["team"].default is None


def test_choice_descriptions_parsed_from_helptext(change_spec: FormSpec) -> None:
    impact = {c.label: c.description for c in change_spec.by_name["impact"].choices}
    assert impact == {"Low": "Sandbox only", "Medium": "Internal tools", "High": "Customer facing"}
    prio = {c.label: c.description for c in change_spec.by_name["priority"].choices}
    assert prio["High"] == "Revenue at risk" and prio["Urgent"] == ""


def test_hidden_choice_kept_but_flagged(change_spec: FormSpec) -> None:
    major = change_spec.by_name["change_type"].choice("major")
    assert major is not None and major.visible is False and major.id == 3


def test_choice_lookup_by_value_label_id_and_display_id(change_spec: FormSpec) -> None:
    stage = change_spec.by_name["stage"]
    assert stage.choice("  staging ").value == "Staging"
    assert stage.choice("801").value == "Live"  # display id
    assert stage.choice(1402).value == "Staging"  # id
    assert stage.choice("prod") is None


def test_nested_choices_follow_upper_levels(change_spec: FormSpec) -> None:
    sub = change_spec.by_name["sub_category"]
    item = change_spec.by_name["item_category"]
    assert sub.parent == "category" and item.parent == "sub_category" and item.level == 3
    assert change_spec.choices_for(sub, {}) == ()
    assert [c.value for c in change_spec.choices_for(sub, {"category": "Hardware"})] == ["Laptop", "Monitor"]
    assert [c.value for c in change_spec.choices_for(item, {"category": "Hardware", "sub_category": "Laptop"})] == [
        "Battery"
    ]
    assert change_spec.choices_for(sub, {"category": "Software"}) == ()
    assert change_spec.by_name["category"].levels == ("sub_category", "item_category")


@pytest.mark.parametrize(
    "name,expected",
    [
        ("customFields.team", "team"),
        ("changeType", "change_type"),
        ("change_type", "change_type"),
        ("department_id", "department"),
        ("departmentId", "department"),
        ("groupId", "group"),
        ("responderId", "agent"),
        ("requesterId", "requester"),
        ("plannedStartDate", "planned_start_date"),
        ("changeWindowId", "change_window"),
        ("subCategory", "sub_category"),
        ("customFields.nope", None),
    ],
)
def test_rule_and_dom_names_map_to_api_names(change_spec: FormSpec, name: str, expected: str | None) -> None:
    assert change_spec.resolve_name(name) == expected


def test_find_by_label_and_unknown_field(change_spec: FormSpec) -> None:
    assert change_spec.field("cloud only").api_name == "cloud_only"
    with pytest.raises(UnknownFieldError) as e:
        change_spec.field("Stagee")
    assert e.value.name == "Stagee"
    with pytest.raises(UnknownFieldError) as e2:
        change_spec.field("Staging")
    assert "Staging Start" in e2.value.suggestions


def test_accepts_bare_list_payload() -> None:
    spec = FormSpec.from_form_fields(acme_forms.change_fields()["change_fields"])
    assert "team" in spec.by_name
    with pytest.raises(ValueError):
        FormSpec.from_form_fields({"nothing": 1})


# -- service catalog item -------------------------------------------------------


def test_service_item_fields(sr_spec: FormSpec) -> None:
    by = sr_spec.by_name
    assert sr_spec.module == "service_request"
    assert by["notice"].kind == "notice" and by["notice"].label == "Notice"
    assert plain_text(by["notice"].content) == "Attach the overview deck."
    assert by["title_100"].kind == "text" and by["paragraph_101"].kind == "textarea"
    assert by["date_103"].kind == "date"
    assert by["goal"].kind == "dropdown"
    deck = by["overview_deck"]
    assert deck.kind == "attachment" and deck.allowed_file_types == (".pptx",) and deck.max_files == 1
    assert deck.placeholder == "Upload files or drop here"
    assert "old_field" not in by  # deleted


def test_service_item_sections_and_children(sr_spec: FormSpec) -> None:
    goal = sr_spec.by_name["goal"]
    assert [s.fields for s in goal.sections] == [("goal_1_kpi",), ("goal_2_kpi",)]
    assert sr_spec.by_name["goal_1_kpi"].parent == "goal"
    parent, choices = sr_spec.section_parent("goal_1_kpi")
    assert parent is goal and choices == ("more revenue (total usd)",)
    names = [f.api_name for f in sr_spec.ordered()]
    assert names.index("goal") + 1 == names.index("goal_1_kpi")
    assert names.index("goal_2_kpi") + 1 == names.index("priority")


def test_service_item_sections_fallback_to_dependent_ids() -> None:
    data = acme_forms.service_item()
    goal = next(f for f in data["service_item"]["custom_fields"] if f["name"] == "goal")
    del goal["field_options"]["section_info"]
    spec = FormSpec.from_service_item(data)
    assert [s.fields for s in spec.by_name["goal"].sections] == [("goal_1_kpi",), ("goal_2_kpi",)]


def test_service_item_dom_names(sr_spec: FormSpec) -> None:
    assert sr_spec.resolve_name("customField.sr_goal") == "goal"
    assert sr_spec.resolve_name("customField.sr_title_100") == "title_100"


def test_service_item_meta_and_subject_template(sr_spec: FormSpec) -> None:
    item = sr_spec.item
    assert item is not None
    assert item.display_id == 77 and item.attachment_mandatory and item.allow_attachments
    assert item.render_subject({"title_100": "Dark mode"}) == "Dark mode"
    assert item.render_subject({}) == ""


# -- tickets (v2 shape) -------------------------------------------------------


def test_ticket_sections_nested_custom_and_unknown_type(ticket_spec: FormSpec) -> None:
    by = ticket_spec.by_name
    assert by["requester"].required_base  # required_for_agents
    assert by["due_on"].kind == "date"
    assert by["issue"].kind == "nested" and by["issue"].levels == ("issue_system", "issue_detail")
    assert by["issue_system"].is_custom
    assert by["customers_hit"].parent == "ticket_type"
    assert by["ticket_type"].sections[0].choices == ("major incident",)
    assert by["source"].default == "Phone"  # first choice, as the web shows
    assert by["mystery"].kind == "text"
    assert any("custom_hologram" in w for w in ticket_spec.warnings)
