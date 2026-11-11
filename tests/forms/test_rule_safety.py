"""Synthetic catalog rule names and write-safety dependencies."""
from __future__ import annotations

import copy

import pytest
import acme_forms as a
from fsv.forms import FormSpec, FormState, RuleContext, RulesEngine


def engine(spec, condition, targets, *, context=None, form="both", item_id=None):
    rule = a._rule(9000004998, "Sample rule", {"all": condition}, [a._a("hide", targets)],
                   module_type="service_request", item_id=item_id, form=form, rev=False)
    return RulesEngine(spec, {"business_rules": [rule]}, context=context or RuleContext(
        module="service_request", workspace_id=2, item_id=item_id))


def test_catalog_cf_alias_uses_exact_names_first():
    data = a.service_item()
    spec = FormSpec.from_service_item(data)
    assert spec.resolve_name("customField.cf_goal") == "goal"
    assert spec.resolve_name("cf_goal") == "goal"
    assert spec.resolve_name("customField.cf_missing") is None
    change = FormSpec.from_form_fields(a.change_fields(), "change")
    assert change.resolve_name("customField.cf_team") is None
    original = data["service_item"]["custom_fields"][1]
    literal = copy.deepcopy(original)
    literal.update(id="f-literal", name="cf_goal", label="Literal")
    data["service_item"]["custom_fields"].append(literal)
    colliding = FormSpec.from_service_item(data)
    assert colliding.resolve_name("customField.cf_goal") == "cf_goal"
    assert colliding.resolve_name("cf_goal") == "cf_goal"


@pytest.mark.parametrize("operator,selected", [("is", "Other"), ("is_not", "Lower cost")])
def test_catalog_cf_rule_condition_and_targets_resolve(operator, selected):
    spec = FormSpec.from_service_item(a.service_item())
    rule_condition = [{"name": "customField.cf_goal", "operator": operator,
                       "evaluate_on": "service_request", "value": "Other"}]
    eng = engine(spec, rule_condition, ["customField.cf_priority"], item_id=spec.item.id)
    assert eng.unavailable_dependencies() == []
    assert not eng.warnings
    state = FormState(spec, eng)
    state.set("goal", selected)
    assert not state.evaluated.is_visible("priority")


@pytest.mark.parametrize("source", ["current_user", "requester", "requested_for", "system"])
def test_missing_context_differs_from_present_empty_context(source):
    spec = FormSpec.from_service_item(a.service_item())
    condition = [{"any": [
        {"name": "customField.cf_goal", "evaluate_on": "service_request", "operator": "not_selected"},
        {"all": [{"name": "region", "evaluate_on": source, "operator": "is_not", "value": "west"}]},
    ]}]
    key = "user" if source == "current_user" else source
    missing = engine(spec, condition, ["customField.cf_priority"])
    assert len(missing.unavailable_dependencies()) == 1
    partial = RuleContext(module="service_request", **{key: {"other": "x"}})
    assert len(engine(spec, condition, ["customField.cf_priority"], context=partial).unavailable_dependencies()) == 1
    for empty in (None, "", []):
        known = RuleContext(module="service_request", **{key: {"region": empty}})
        assert engine(spec, condition, ["customField.cf_priority"], context=known).unavailable_dependencies() == []


def test_unavailable_dependencies_cover_unknown_conditions_targets_and_applicability():
    spec = FormSpec.from_service_item(a.service_item())
    cond = [{"name": "customField.cf_unknown", "evaluate_on": "service_request", "operator": "is_not", "value": "x"}]
    eng = engine(spec, cond, ["customField.cf_missing", "attachment_ids"])
    problems = eng.unavailable_dependencies()
    assert len(problems) == 2
    assert "unknown condition field" in problems[0]
    assert "unknown action field" in problems[1]
    assert engine(spec, cond, ["customField.cf_missing"], form="edit_form").unavailable_dependencies() == []
    wrong_item = engine(spec, cond, ["customField.cf_missing"], item_id=9000009002,
                        context=RuleContext(module="service_request", item_id=9000009003))
    assert wrong_item.unavailable_dependencies() == []
