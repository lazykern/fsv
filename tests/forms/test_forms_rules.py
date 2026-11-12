from __future__ import annotations

import copy

import pytest

import acme_forms
from fsv.forms import FormRuleError, FormSpec, RulesEngine, UnsupportedRuleError

BASE = {"change_type": "Normal", "impact": "Low", "priority": "Low", "risk": "Low", "status": "Open"}


def ev(engine: RulesEngine, mode: str = "new", **values):
    return engine.evaluate({**BASE, **values}, mode)


def test_filters_and_exact_duplicate_dedupe(change_rules: RulesEngine) -> None:
    ids = [r.id for r in change_rules.rules]
    assert ids.count(9) == 1  # identical copy dropped
    assert change_rules.duplicates == [9]
    for gone in (21, 22, 23, 24):
        assert gone not in ids
    assert any("server_side" in s for s in change_rules.skipped)
    # rule 20 is requester-only: loaded but not applied to agents
    assert 20 in ids and 20 not in [r.id for r in change_rules.rules_for("new")]
    assert any("ghost_field" in w for w in change_rules.warnings)


def test_base_state(change_rules: RulesEngine) -> None:
    e = ev(change_rules)
    for hidden in ("rollout_tool", "reviewers", "staging_start", "staging_end", "emergency_reason", "review_scope",
                   "category", "sub_category", "legacy_flag"):
        assert not e.is_visible(hidden), hidden
    for shown in ("guide", "team", "cloud_only", "stage", "needs_review", "downtime", "signoff_date"):
        assert e.is_visible(shown), shown
    assert {"cloud_only", "stage", "signoff_date", "team"} <= e.required
    assert "needs_review" not in e.required  # unmandated by "team is_not Labs" (empty team)
    assert "guide" not in e.required
    assert e.messages == []


def test_is_with_reversible_show_and_populate(change_rules: RulesEngine) -> None:
    e = ev(change_rules, cloud_only="No")
    for f in ("rollout_tool", "reviewers", "staging_start", "staging_end"):
        assert e.is_visible(f) and e.is_required(f), f
    assert e.values["stage"] == "Staging" and e.populated == {"stage": 5}
    # reverting the answer reverts the reversible rule (pure function of values)
    e2 = ev(change_rules)
    assert not e2.is_visible("reviewers") and "stage" not in e2.populated


def test_later_rule_overrides_earlier_replay_model(change_spec: FormSpec) -> None:
    eng = RulesEngine(change_spec, acme_forms.change_rules(), module="change", model="replay", populate="set")
    # rule 2 (Live: hide staging dates) runs before rule 5 (Cloud no: show)
    e = ev(eng, cloud_only="No", stage="Live")
    assert e.is_visible("staging_start") and e.is_required("staging_start")
    e2 = ev(eng, cloud_only="Yes", stage="Staging")
    # rule 3 (Staging: show) runs before rule 4 (Cloud yes: hide)
    assert not e2.is_visible("staging_start")


def test_event_model_last_change_wins(change_rules: RulesEngine) -> None:
    assert change_rules.model == "event"
    # replayed in field order: cloud_only (shows dates), then stage=Live (hides them)
    e = ev(change_rules, cloud_only="No", stage="Live")
    assert not e.is_visible("staging_start")
    # the same answers in the other order: stage first, then cloud_only shows them
    e2 = change_rules.evaluate({**BASE, "cloud_only": "No", "stage": "Live"}, order=["stage", "cloud_only"])
    assert e2.is_visible("staging_start") and e2.values["stage"] == "Staging"
    assert e2.overridden == {"stage": ("Live", 5)}  # populate replaced the earlier answer


def test_populate_never_overwrites_user_value_and_tracks_provenance(change_rules: RulesEngine) -> None:
    e = ev(change_rules, cloud_only="Yes")
    assert e.values["stage"] == "Live" and e.populated["stage"] == 4
    e2 = ev(change_rules, cloud_only="Yes", stage="Staging")
    assert e2.values["stage"] == "Staging" and "stage" not in e2.populated


def test_populated_value_feeds_other_rules(change_rules: RulesEngine) -> None:
    # Emergency populates Sign-off Date = N/A (display id 700)
    e = ev(change_rules, change_type="Emergency")
    assert e.values["signoff_date"] == "N/A" and e.populated["signoff_date"] == 10


def test_includes_and_does_not_include(change_rules: RulesEngine) -> None:
    for tool in ("Pipeline", "Script"):
        e = ev(change_rules, cloud_only="No", rollout_tool=tool)
        assert "needs_review" not in e.required
    e = ev(change_rules, cloud_only="No", rollout_tool="Manual")
    assert "needs_review" in e.required
    # does_not_include (rule 15, new form): Labs lifts the mandate
    assert "cloud_only" in ev(change_rules, team="Platform").required
    assert "cloud_only" not in ev(change_rules, team="Labs").required


def test_is_not(change_rules: RulesEngine) -> None:
    # rule 1 unmandates needs_review unless team is Labs; reversed for Labs
    assert "needs_review" not in ev(change_rules, team="Payments").required
    assert "needs_review" in ev(change_rules, team="Labs").required


def test_selected_and_not_selected(change_rules: RulesEngine) -> None:
    e = ev(change_rules, needs_review=False)
    assert not e.is_visible("review_scope")
    e = ev(change_rules, needs_review=True)
    assert e.is_visible("review_scope") and e.is_required("review_scope")


def test_ids_and_labels_both_match_default_fields(change_rules: RulesEngine) -> None:
    # rule 9 uses change_type id "1"; rule 18 uses the label "Emergency"
    assert "signoff_date" in ev(change_rules, change_type="Normal").required
    assert "signoff_date" not in ev(change_rules, change_type="Standard").required
    msgs = [m.text for m in ev(change_rules, change_type="Emergency").messages]
    assert msgs == ["Tell the payments on-call before you start"]


def test_nested_any_all_conditions(change_rules: RulesEngine) -> None:
    hit = ev(change_rules, team="Payments", priority="High").messages
    assert [m.text for m in hit] == ["Tell the payments on-call before you start"]
    assert ev(change_rules, team="Payments", priority="Medium").messages == []
    assert ev(change_rules, team="Platform", priority="High").messages == []


def test_message_blocking_when_validation_rule(change_rules: RulesEngine) -> None:
    (m,) = ev(change_rules, priority="Urgent").messages
    assert m.blocking and m.rule_id == 13 and "duty manager" in m.text


def test_disable_and_reverse_enable(change_rules: RulesEngine) -> None:
    e = ev(change_rules, team="Labs")
    assert "department" in e.disabled and not e.is_visible("downtime")
    assert "department" not in ev(change_rules, team="Platform").disabled


def test_new_vs_edit_form_type(change_rules: RulesEngine) -> None:
    new = ev(change_rules, "new", team="Platform")
    edit = ev(change_rules, "edit", team="Platform")
    assert "cloud_only" in new.required and "cloud_only" not in edit.required  # new_form-only rule
    assert "subject" in edit.disabled and "subject" not in new.disabled  # edit_form-only rule


def test_condition_all_null_always_matches(change_rules: RulesEngine) -> None:
    assert not ev(change_rules).is_visible("legacy_flag")


def test_nested_levels_visible_only_with_options(change_rules: RulesEngine) -> None:
    e = ev(change_rules, change_type="Emergency")
    assert e.is_visible("category") and not e.is_visible("sub_category")
    e = ev(change_rules, change_type="Emergency", category="Hardware")
    assert e.is_visible("sub_category") and not e.is_visible("item_category")
    e = ev(change_rules, change_type="Emergency", category="Hardware", sub_category="Laptop")
    assert e.is_visible("item_category")
    assert not ev(change_rules, change_type="Emergency", category="Software").is_visible("sub_category")
    # hidden parent hides every level
    assert not ev(change_rules, category="Hardware", sub_category="Laptop").is_visible("sub_category")


def test_visible_is_in_form_order(change_rules: RulesEngine, change_spec: FormSpec) -> None:
    e = ev(change_rules, cloud_only="No")
    order = [f.api_name for f in change_spec.ordered()]
    assert e.visible == [n for n in order if n in set(e.visible)]


def test_differing_duplicates_are_both_evaluated_by_default(change_spec: FormSpec) -> None:
    data = acme_forms.change_rules()
    first = next(r for r in data["business_rules"] if r["id"] == 5)
    newer = copy.deepcopy(first)
    newer["is_reversible"] = False
    newer["action_data"]["action"].append({"name": ["customFields.estimated_hours"], "property": "hide"})
    data["business_rules"].insert(data["business_rules"].index(first) + 1, newer)

    keep = RulesEngine(change_spec, data, module="change")
    assert [r.id for r in keep.rules].count(5) == 2
    e = keep.evaluate({**BASE})
    assert not e.is_visible("reviewers")  # reversible copy still reverses
    assert e.is_visible("estimated_hours")  # non-reversible copy does nothing
    assert not keep.evaluate({**BASE, "cloud_only": "No"}).is_visible("estimated_hours")

    last = RulesEngine(change_spec, data, module="change", dedupe="last")
    assert [r.id for r in last.rules].count(5) == 1
    assert last.evaluate({**BASE}).is_visible("reviewers")  # only the non-reversible copy kept
    first_only = RulesEngine(change_spec, data, module="change", dedupe="first")
    assert first_only.evaluate({**BASE, "cloud_only": "No"}).is_visible("estimated_hours")
    with pytest.raises(ValueError):
        RulesEngine(change_spec, data, dedupe="bogus")


def _with_rule(rule: dict) -> dict:
    data = acme_forms.change_rules()
    data["business_rules"].append(rule)
    return data


def test_unsupported_operator_is_skipped_with_warning_or_raises_in_strict(change_spec: FormSpec) -> None:
    bad = {"id": 99, "name": "Contains", "module_type": "change", "state": "active", "rule_type": "client_side",
           "form_type": "both", "user_type": "both", "is_reversible": False,
           "condition_data": {"all": [{"name": "subject", "operator": "contains", "value": "x"}]},
           "action_data": {"action": [{"name": ["subject"], "property": "hide"}]}}
    with pytest.raises(FormRuleError, match="doesn't understand.*'contains'") as e:
        RulesEngine(change_spec, _with_rule(bad), module="change")  # strict by default
    assert isinstance(e.value, UnsupportedRuleError) and e.value.rule_id == 99
    eng = RulesEngine(change_spec, _with_rule(bad), module="change", strict=False)  # lenient
    assert 99 not in [r.id for r in eng.rules]
    assert any("contains" in u for u in eng.unsupported)


def test_unsupported_action_is_skipped(change_spec: FormSpec) -> None:
    bad = {"id": 98, "name": "Odd action", "module_type": "change", "state": "active", "rule_type": "client_side",
           "form_type": "both", "user_type": "both", "is_reversible": False, "condition_data": {"all": None},
           "action_data": {"action": [{"name": ["subject"], "property": "blink"}]}}
    with pytest.raises(FormRuleError, match="blink"):
        RulesEngine(change_spec, _with_rule(bad), module="change")
    eng = RulesEngine(change_spec, _with_rule(bad), module="change", strict=False)
    assert any("blink" in u for u in eng.unsupported)


def test_unknown_evaluate_on_raises(change_spec: FormSpec) -> None:
    bad = {"id": 96, "name": "Odd source", "module_type": "change", "state": "active", "rule_type": "client_side",
           "form_type": "both", "user_type": "both", "is_reversible": False,
           "condition_data": {"all": [{"name": "subject", "operator": "is", "value": "x", "evaluate_on": "moon"}]},
           "action_data": {"action": [{"name": ["subject"], "property": "hide"}]}}
    with pytest.raises(FormRuleError, match="evaluate_on 'moon'"):
        RulesEngine(change_spec, _with_rule(bad), module="change")
    assert RulesEngine(change_spec, _with_rule(bad), module="change", strict=False).unsupported


def test_populate_with_unknown_choice_warns(change_spec: FormSpec) -> None:
    rule = {"id": 97, "name": "Bad populate", "module_type": "change", "state": "active",
            "rule_type": "client_side", "form_type": "both", "user_type": "both", "is_reversible": False,
            "condition_data": {"all": None},
            "action_data": {"action": [{"name": ["customFields.team"], "values": [123456], "property": "populate"}]}}
    eng = RulesEngine(change_spec, _with_rule(rule), module="change")
    e = eng.evaluate({**BASE})
    assert "team" not in e.values
    assert any("123456" in w for w in eng.warnings)


def test_rules_accept_plain_list(change_spec: FormSpec) -> None:
    eng = RulesEngine(change_spec, acme_forms.change_rules()["business_rules"])
    assert eng.rules
    assert RulesEngine(change_spec, None).rules == []


def _entity_rule(rid: int, cond: dict, actions: list, *, reversible: bool = False, form_type: str = "edit_form") -> dict:
    return {"id": rid, "name": f"Entity rule {rid}", "module_type": "change", "state": "active",
            "rule_type": "client_side", "form_type": form_type, "user_type": "both", "is_reversible": reversible,
            "condition_data": {"all": [cond]}, "action_data": {"action": actions}}


def test_current_entity_reads_the_saved_record_not_live_answers(change_spec: FormSpec) -> None:
    # production shape: {"name": "status", "value": ["3"], "operator": "is", "evaluate_on": "current_entity"}
    rule = _entity_rule(80, {"name": "status", "operator": "is", "value": ["3"], "evaluate_on": "current_entity"},
                        [{"name": ["downtime"], "property": "disable"}])
    eng = RulesEngine(change_spec, _with_rule(rule), module="change")  # strict: no UnsupportedRuleError
    saved = {**BASE, "status": "Approval"}
    e = eng.evaluate({"status": "Open"}, "edit", base=saved)
    assert "downtime" in e.disabled  # the record is in status 3, whatever the form now says
    e = eng.evaluate({"status": "Approval"}, "edit", base={**BASE, "status": "Open"})
    assert "downtime" not in e.disabled  # live answer 3 does not count: the saved record is Open
    assert eng.unavailable_dependencies("edit") == [] or all("current_entity" not in d for d in eng.unavailable_dependencies("edit"))


def test_current_entity_includes_and_non_form_record_fields(change_spec: FormSpec) -> None:
    from fsv.forms.rules import RuleContext
    rule = _entity_rule(81, {"name": "requested_catalog_items", "operator": "includes", "value": ["700", "701"],
                             "evaluate_on": "current_entity"}, [{"name": ["subject"], "property": "hide"}])
    eng = RulesEngine(change_spec, _with_rule(rule), context=RuleContext(
        module="change", entity={"requested_catalog_items": ["701"]}))
    assert not eng.evaluate(dict(BASE), "edit", base=dict(BASE)).is_visible("subject")
    # no record context for a non-form field: writes must see the gap
    eng = RulesEngine(change_spec, _with_rule(rule), module="change")
    eng.evaluate(dict(BASE), "edit", base=dict(BASE))
    assert any("current_entity" in d and "requested_catalog_items" in d for d in eng.unavailable_dependencies("edit"))


def test_enable_action_and_its_reverse(change_spec: FormSpec) -> None:
    disable = _entity_rule(82, {"name": "status", "operator": "is", "value": ["1"], "evaluate_on": "change"},
                           [{"name": ["downtime"], "property": "disable"}], form_type="both")
    enable = _entity_rule(83, {"name": "status", "operator": "includes", "value": ["2", "3"],
                               "evaluate_on": "current_entity"},
                          [{"name": ["downtime"], "property": "enable"}], reversible=True)
    data = _with_rule(disable)
    data["business_rules"].append(enable)
    eng = RulesEngine(change_spec, data, module="change")
    assert "downtime" not in eng.evaluate({"status": "Open"}, "edit", base={**BASE, "status": "Planning"}).disabled
    # reversible enable that does not match disables (enable <-> disable)
    assert "downtime" in eng.evaluate({"status": "Planning"}, "edit", base={**BASE, "status": "Open"}).disabled
