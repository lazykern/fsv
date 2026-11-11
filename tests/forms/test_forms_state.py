from __future__ import annotations

import datetime as dt
from zoneinfo import ZoneInfo

import pytest

from fsv.forms import (
    AmbiguousLookupError,
    FieldDisabledError,
    FormSpec,
    FormState,
    LookupValue,
    RulesEngine,
    UnknownFieldError,
    ValueParseError,
)

BKK = ZoneInfo("Asia/Bangkok")


def test_defaults_applied_on_new_only(change_spec: FormSpec, change_rules: RulesEngine) -> None:
    new = FormState(change_spec, change_rules)
    assert new.values == {"change_type": "Normal", "impact": "Low", "priority": "Low", "risk": "Low",
                          "status": "Open"}
    assert new.sources["status"] == "default"
    edit = FormState(change_spec, change_rules, mode="edit", values={"subject": "Old"})
    assert edit.values == {"subject": "Old"}
    with pytest.raises(ValueError):
        FormState(change_spec, change_rules, mode="draft")


def test_set_returns_diff_of_appeared_and_required(change_state: FormState) -> None:
    d = change_state.set("Cloud Only", "no")
    assert d.field == "cloud_only"
    assert d.appeared == ["rollout_tool", "reviewers", "staging_start", "staging_end"]
    assert d.became_required == ["rollout_tool", "reviewers", "staging_start", "staging_end"]
    assert d.populated == {"stage": "Staging"}
    lines = d.describe(change_state.spec)
    assert lines[0].startswith("added: Rollout Tool, Reviewers, Staging Start, Staging End (required:")
    assert "filled in: Stage = Staging" in lines
    back = change_state.set("cloud_only", "Yes")
    assert back.disappeared == ["rollout_tool", "reviewers", "staging_start", "staging_end"]
    assert back.populated == {"stage": "Live"}
    assert not change_state.set("estimated_hours", 3)  # nothing changes -> falsy diff


def test_diff_messages_and_no_longer_required(change_state: FormState) -> None:
    d = change_state.set("priority", "Urgent")
    assert [m.text for m in d.messages_added] == ["Urgent changes need the duty manager's approval"]
    assert any(line.startswith("message:") for line in d.describe(change_state.spec))
    d2 = change_state.set("priority", "Low")
    assert len(d2.messages_removed) == 1
    d3 = change_state.set("change_type", "Standard")
    assert "signoff_date" in d3.no_longer_required
    assert any(line.startswith("no longer required") for line in d3.describe(change_state.spec))


def test_user_value_wins_over_populate(change_state: FormState) -> None:
    change_state.set("stage", "Live")
    change_state.set("cloud_only", "No")
    assert change_state.get("stage") == "Live" and not change_state.is_populated("stage")
    change_state.unset("stage")
    assert change_state.get("stage") == "Staging" and change_state.is_populated("stage")


def test_choice_parsing_case_insensitive_exact(change_state: FormState) -> None:
    change_state.set("team", "  PAYMENTS ")
    assert change_state.values["team"] == "Payments"
    change_state.set("change_type", 4)  # id for default dropdowns
    assert change_state.values["change_type"] == "Emergency"
    with pytest.raises(ValueParseError) as e:
        change_state.set("team", "Pay")
    assert "Payments" in e.value.options
    with pytest.raises(ValueParseError):
        change_state.set("change_type", "Major")  # hidden choice


def test_multi_select_parsing(change_state: FormState) -> None:
    change_state.set("services", "billing, search")
    assert change_state.values["services"] == ("Billing", "Search")
    change_state.set("services", "Reports, Exports")  # a single label containing a comma
    assert change_state.values["services"] == ("Reports, Exports",)
    change_state.set("services", ["Search", "search", "Billing"])
    assert change_state.values["services"] == ("Search", "Billing")
    with pytest.raises(ValueParseError):
        change_state.set("services", "Billing, Nope")


@pytest.mark.parametrize("raw,expected", [("yes", True), ("No", False), ("TRUE", True), ("false", False),
                                          (True, True)])
def test_checkbox_strict(change_state: FormState, raw, expected) -> None:
    change_state.set("needs_review", raw)
    assert change_state.values["needs_review"] is expected


@pytest.mark.parametrize("raw", ["1", "on", "maybe", 1])
def test_checkbox_rejects_loose_values(change_state: FormState, raw) -> None:
    with pytest.raises(ValueParseError):
        change_state.set("needs_review", raw)


def test_number(change_state: FormState) -> None:
    change_state.set("estimated_hours", "1,200")
    assert change_state.values["estimated_hours"] == 1200
    change_state.set("estimated_hours", "2.5")
    assert change_state.values["estimated_hours"] == 2.5
    for bad in ("two", True):
        with pytest.raises(ValueParseError):
            change_state.set("estimated_hours", bad)


def test_datetime_defaults_to_bangkok(change_state: FormState) -> None:
    change_state.set("planned_start_date", "2030-11-20 10:00")
    v = change_state.values["planned_start_date"]
    assert v == dt.datetime(2030, 11, 20, 10, 0, tzinfo=BKK)
    assert v.isoformat() == "2030-11-20T10:00:00+07:00"


def test_datetime_keeps_explicit_offset(change_state: FormState) -> None:
    change_state.set("planned_start_date", "2030-11-20T03:00:00Z")
    assert change_state.values["planned_start_date"].utcoffset() == dt.timedelta(0)
    change_state.set("planned_end_date", "2030-11-20T10:00+09:00")
    assert change_state.values["planned_end_date"].utcoffset() == dt.timedelta(hours=9)
    aware = dt.datetime(2030, 1, 1, 8, tzinfo=dt.timezone.utc)
    change_state.set("planned_end_date", aware)
    assert change_state.values["planned_end_date"] is aware


def test_datetime_rejects_date_only_and_garbage(change_state: FormState) -> None:
    for bad in ("2030-11-20", dt.date(2030, 11, 20), "next tuesday"):
        with pytest.raises(ValueParseError):
            change_state.set("planned_start_date", bad)


def test_tz_override(change_spec: FormSpec, change_rules: RulesEngine, monkeypatch) -> None:
    st = FormState(change_spec, change_rules, tz="UTC")
    st.set("planned_start_date", "2030-01-01 09:00")
    assert st.values["planned_start_date"].utcoffset() == dt.timedelta(0)
    monkeypatch.setenv("FSV_TZ", "Asia/Tokyo")
    st2 = FormState(change_spec, change_rules)
    st2.set("planned_start_date", "2030-01-01 09:00")
    assert st2.values["planned_start_date"].utcoffset() == dt.timedelta(hours=9)


def test_lookup_resolution(change_state: FormState, resolver) -> None:
    change_state.set("agent", "alex")
    assert change_state.values["agent"] == LookupValue(701, "Alex Kim", "alex.kim@acme.example")
    assert resolver.calls[-1] == ("agent", "alex")
    change_state.set("group", "platform")  # two hits, one exact
    assert change_state.values["group"].id == 602
    change_state.set("group", 650)
    assert change_state.values["group"] == LookupValue(650)
    change_state.set("change_window", "weekend")
    assert resolver.calls[-1] == ("change_window", "weekend")


def test_lookup_ambiguous_and_not_found(change_state: FormState) -> None:
    with pytest.raises(AmbiguousLookupError) as e:
        change_state.set("requester", "jane")
    assert [c.id for c in e.value.candidates] == [9001, 9002]
    with pytest.raises(ValueParseError):
        change_state.set("requester", "nobody")
    change_state.set("requester", "jane.doe@acme.example")
    assert change_state.values["requester"].id == 9001


def test_multi_lookup(change_state: FormState) -> None:
    change_state.set("cloud_only", "No")
    change_state.set("reviewers", "sam.lee@acme.example, 9001")
    assert [v.id for v in change_state.values["reviewers"]] == [9003, 9001]


def test_lookup_without_resolver_needs_ids(change_spec: FormSpec, change_rules: RulesEngine) -> None:
    st = FormState(change_spec, change_rules)
    with pytest.raises(ValueParseError):
        st.set("agent", "alex")
    st.set("agent", "701")
    assert st.values["agent"] == LookupValue(701)
    with pytest.raises(ValueParseError):
        st.set("agent", [1, 2])


def test_nested_levels_validate_and_clear(change_state: FormState) -> None:
    change_state.set("change_type", "Emergency")
    with pytest.raises(ValueParseError):
        change_state.set("sub_category", "Laptop")  # choose Category first
    change_state.set("category", "Hardware")
    change_state.set("sub_category", "laptop")
    change_state.set("item_category", "Battery")
    with pytest.raises(ValueParseError):
        change_state.set("sub_category", "Battery")
    d = change_state.set("category", "Software")
    assert d.cleared == ["sub_category", "item_category"]
    assert "sub_category" not in change_state.values


def test_notice_and_unknown_and_disabled(change_state: FormState) -> None:
    with pytest.raises(ValueParseError):
        change_state.set("guide", "x")
    with pytest.raises(UnknownFieldError):
        change_state.set("nonexistent", "x")
    change_state.set("team", "Labs")
    with pytest.raises(FieldDisabledError):
        change_state.set("department", "Finance")


def test_missing_required_tracks_rules(change_state: FormState) -> None:
    missing = {f.api_name for f in change_state.missing_required()}
    assert {"requester", "team", "cloud_only", "stage", "signoff_date", "subject"} <= missing
    assert "status" not in missing and "impact" not in missing  # defaults
    change_state.set("cloud_only", "No")
    change_state.set("rollout_tool", "Manual")
    names = {f.api_name for f in change_state.missing_required()}
    assert {"staging_start", "reviewers", "needs_review"} <= names
    assert "stage" not in names  # populated
    change_state.set("needs_review", False)
    assert "needs_review" in {f.api_name for f in change_state.missing_required()}  # required box must be ticked
    change_state.set("needs_review", True)
    names = {f.api_name for f in change_state.missing_required()}
    assert "needs_review" not in names and "review_scope" in names


def test_errors_include_blocking_messages(change_state: FormState) -> None:
    change_state.set("priority", "Urgent")
    assert "blocked by rule: Urgent changes need the duty manager's approval" in change_state.errors()


def test_hidden_values_dropped_from_payload_values(change_state: FormState) -> None:
    change_state.set("cloud_only", "No")
    change_state.set("staging_start", "2030-01-01 09:00")
    assert "staging_start" in change_state.payload_values()
    change_state.set("cloud_only", "Yes")
    assert "staging_start" in change_state.values  # kept so switching back restores it
    assert "staging_start" not in change_state.payload_values()


def test_to_review(change_state: FormState) -> None:
    change_state.set("team", "Payments")
    change_state.set("services", "Billing, Search")
    change_state.set("cloud_only", "No")
    change_state.set("planned_start_date", "2030-11-20 10:00")
    change_state.set("needs_review", "yes")
    change_state.set("description", "Line one\n\nLine two")
    change_state.set("priority", "High")
    rows = change_state.to_review()
    labels = [label for label, _ in rows]
    assert labels.index("Team") < labels.index("Services") < labels.index("Cloud Only") < labels.index("Stage")
    d = dict(rows)
    assert d["Services"] == "Billing, Search"
    assert d["Stage"] == "Staging (auto)"
    assert d["Planned Start Date"] == "20 Nov 2030 10:00 (+07:00)"
    assert d["Needs Security Review"] == "Yes"
    assert d["Description"] == "Line one Line two"
    assert d["Change Type"] == "Normal" and d["Status"] == "Open"
    assert rows[-1] == ("Message", "Tell the payments on-call before you start")
    assert "Guide" not in d


def test_initial_values_are_validated(change_spec: FormSpec, change_rules: RulesEngine) -> None:
    st = FormState(change_spec, change_rules, values={"Team": "labs", "category": "Hardware"})
    assert st.values["team"] == "Labs"
    with pytest.raises(ValueParseError):
        FormState(change_spec, change_rules, values={"team": "Nope"})


# -- service request: conditional reveal ----------------------------------


def test_sr_section_reveals_required_child(sr_spec: FormSpec) -> None:
    st = FormState(sr_spec)
    assert not st.evaluated.is_visible("goal_1_kpi")
    d = st.set("1. Expected benefit", "more revenue (total usd)")
    assert d.appeared == ["goal_1_kpi"] and d.became_required == ["goal_1_kpi"]
    assert "goal_1_kpi" in {f.api_name for f in st.missing_required()}
    d2 = st.set("goal", "Lower cost")
    assert d2.appeared == ["goal_2_kpi"] and d2.disappeared == ["goal_1_kpi"]
    assert not st.set("goal", "Other").appeared


def test_sr_date_and_attachment_parsing(sr_spec: FormSpec) -> None:
    st = FormState(sr_spec)
    st.set("date_103", "2030-12-03")
    assert st.values["date_103"] == dt.date(2030, 12, 3)
    assert st.display("date_103") == "03 Dec 2030"
    st.set("overview_deck", "/tmp/deck.pptx")
    assert st.values["overview_deck"] == ("/tmp/deck.pptx",)
    assert dict(st.to_review())["Overview deck (.pptx)"] == "deck.pptx"
    with pytest.raises(ValueParseError):
        st.set("overview_deck", "notes.docx")
    with pytest.raises(ValueParseError):
        st.set("overview_deck", ["a.pptx", "b.pptx"])
    with pytest.raises(ValueParseError):
        st.set("date_103", "03/12/2030")


def test_ticket_section_and_nested_custom(ticket_spec: FormSpec) -> None:
    st = FormState(ticket_spec)
    assert not st.evaluated.is_visible("customers_hit")
    d = st.set("ticket_type", "Major Incident")
    assert d.appeared == ["customers_hit"] and "customers_hit" in d.became_required
    st.set("issue", "Login")
    st.set("issue_system", "VPN")
    st.set("issue_detail", "Timeout")
    assert st.payload_values()["issue_detail"] == "Timeout"
