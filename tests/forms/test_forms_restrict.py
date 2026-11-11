"""Default engine semantics: ``populate`` restricts a dropdown's choices,
every change re-runs every rule, ticket rules with system conditions and
nested fields, and the business-rules request builder. Synthetic Acme data."""
from __future__ import annotations

import json

import pytest

import acme_forms
from fsv.forms import (
    DEFAULT_EVENT_REVERSE,
    DEFAULT_EVENT_SCOPE,
    DEFAULT_POPULATE,
    FormRuleError,
    FormSpec,
    FormState,
    PayloadError,
    RuleContext,
    RulesEngine,
    ValueParseError,
    build_change_payload,
    build_ticket_payload,
    business_rules_query,
    decode_br_validation_excludes,
    parse_business_rules,
)

CF = acme_forms.CF


def change_state(**kw) -> FormState:
    spec = FormSpec.from_form_fields(acme_forms.change_fields(), module="change")
    eng = RulesEngine(spec, acme_forms.restrict_rules(), module="change", **kw)
    return FormState(spec, eng, event_order="history")


def labels(choices) -> list[str]:
    return [c.label for c in choices if c.visible]


# -- populate restricts the choices -----------------------------------------


def test_defaults() -> None:
    assert (DEFAULT_POPULATE, DEFAULT_EVENT_SCOPE, DEFAULT_EVENT_REVERSE) == ("restrict", "all", "always")


def test_populate_restricts_choices_and_does_not_set_the_value() -> None:
    st = change_state()
    assert labels(st.choices("stage")) == ["Live", "Staging"]
    d = st.set("Cloud Only", "No")
    assert st.get("stage") is None and st.evaluated.populated == {}
    assert labels(st.choices("Stage")) == ["Staging"]
    assert labels(st.evaluated.choices("stage")) == ["Staging"]
    assert d.restricted == {"stage": ["Staging"]}
    assert "choices limited: Stage -> Staging" in d.describe(st.spec)
    r, dec = st.evaluated.restriction_for("stage")
    assert r is not None and r.offers("Staging") and not r.offers("Live") and dec.rule_name == "Cloud only no"


def test_set_rejects_a_choice_outside_the_restriction() -> None:
    st = change_state()
    st.set("Cloud Only", "No")
    with pytest.raises(ValueParseError) as e:
        st.set("Stage", "Live")
    msg = str(e.value)
    assert "'Live' is not offered now" in msg and "'Cloud only no'" in msg and msg.endswith("to: Staging")
    assert e.value.options == ["Staging"]
    st.set("Stage", "Staging")
    assert st.get("stage") == "Staging"


def test_reversible_populate_restores_the_full_choices() -> None:
    st = change_state()
    st.set("Cloud Only", "Yes")
    assert labels(st.choices("stage")) == ["Live"]
    d = st.set("Cloud Only", None)  # rule 32 stops matching: reversed
    assert labels(st.choices("stage")) == ["Live", "Staging"]
    assert d.unrestricted == ["stage"] and "all choices offered again: Stage" in d.describe(st.spec)
    st.set("Cloud Only", "Yes")
    st.set("Cloud Only", "No")  # Yes reversed (full list), then No restricts
    assert labels(st.choices("stage")) == ["Staging"]


def test_value_given_before_the_restriction_is_reported_not_dropped() -> None:
    st = change_state()
    st.set("Stage", "Live")
    st.set("Cloud Only", "No")
    assert st.get("stage") == "Live"  # the value is kept ...
    assert "stage" in st.evaluated.unavailable  # ... but no longer on offer
    err = [e for e in st.errors() if e.startswith("Stage:")]
    assert err and "'Live' is not offered any more" in err[0] and "Staging" in err[0]
    assert any(k == "Not offered" and "Stage = Live" in v for k, v in st.to_review())
    with pytest.raises(PayloadError) as e:
        build_change_payload(st, "jane.doe@example.com", "Jane Doe")
    assert any("Stage" in p and "not offered" in p for p in e.value.problems)


def test_explain_names_the_restricting_rule() -> None:
    st = change_state()
    st.set("Cloud Only", "No")
    ex = [e for e in st.explain(["stage"]) if e.property == "choices restricted"]
    assert len(ex) == 1 and ex[0].value and "Cloud only no" in str(ex[0])


def test_nested_restriction_tree() -> None:
    st = change_state()
    assert labels(st.choices("category")) == ["Hardware", "Software"]
    st.set("Team", "Platform")  # rule 35: Hardware -> Laptop only (and everything below Laptop)
    assert labels(st.choices("category")) == ["Hardware"]
    with pytest.raises(ValueParseError):
        st.set("Category", "Software")
    st.set("Category", "Hardware")
    assert labels(st.choices("sub_category")) == ["Laptop"]
    with pytest.raises(ValueParseError) as e:
        st.set("Sub-Category", "Monitor")
    assert "to: Laptop" in str(e.value)
    st.set("Sub-Category", "Laptop")
    assert labels(st.choices("item_category")) == ["Battery"]  # null nested_options: no limit below


def test_legacy_set_mode_still_fills_the_value() -> None:
    st = change_state(populate="set")
    st.set("Cloud Only", "No")
    assert st.get("stage") == "Staging" and st.evaluated.populated == {"stage": 33}
    assert st.evaluated.restricted == {}


# -- every change re-runs every rule (default) ------------------------------


def test_every_change_reruns_every_rule_and_reverses_non_matching() -> None:
    # "Staging stage" (non-reversible) hides Review Scope; "Labs hides review
    # scope" (reversible, not matching) comes later and shows it again on
    # every change, so it stays visible: what the web does.
    st = change_state()
    st.set("Needs Security Review", "yes")
    st.set("Stage", "Staging")
    assert st.evaluated.is_visible("review_scope")
    old = change_state(event_scope="referenced", event_reverse="transition")
    old.set("Needs Security Review", "yes")
    old.set("Stage", "Staging")
    assert not old.evaluated.is_visible("review_scope")  # earlier model: only rule 31 ran


def test_engine_rejects_unknown_knobs() -> None:
    spec = FormSpec.from_form_fields(acme_forms.change_fields(), module="change")
    for kw in ({"populate": "fill"}, {"event_scope": "some"}, {"event_reverse": "never"}):
        with pytest.raises(ValueError):
            RulesEngine(spec, [], **kw)


# -- tickets ----------------------------------------------------------------


def ticket(system=None, **kw):
    spec = FormSpec.from_form_fields(acme_forms.ticket_internal_fields(), module="ticket")
    rules = parse_business_rules(acme_forms.ticket_rules())
    ctx = RuleContext(module="ticket", form="new", user_type="agent", user=rules.user,
                      system=system if system is not None else {"portal": acme_forms.PORTAL})
    eng = RulesEngine(spec, rules.raw, context=ctx, **kw)
    return spec, eng, FormState(spec, eng)


def test_ticket_internal_fields_nested_sections_and_account() -> None:
    spec, _, st = ticket()
    assert spec.account_id == acme_forms.TICKET_ACCOUNT
    assert spec.by_name["symptom_system"].parent == "symptom"
    assert spec.by_name["delivery_team"].parent == "request_item"  # section child
    assert not spec.by_name["orphan_note"].visible_base  # in no section: never revealed
    assert any("orphan_note" in w for w in spec.warnings)
    assert not st.evaluated.is_visible("orphan_note")


def test_ticket_system_condition_and_nested_populate() -> None:
    spec, eng, st = ticket()
    ev = st.evaluated
    assert not ev.is_visible("priority") and "attachment_check" in ev.disabled
    assert labels(st.choices("symptom")) == ["Cannot log in", "Other"]
    st.set("Symptom", "Cannot log in")
    assert labels(st.choices("symptom_system")) == ["VPN"]
    with pytest.raises(ValueParseError):
        st.set("Symptom", "Slow")


def test_ticket_token_from_engine_state() -> None:
    _, _, st = ticket()
    tok = build_ticket_payload(st, "robin.desk@example.com", "Robin Desk")["br_validation_excludes"]
    d = decode_br_validation_excludes(tok)
    acct = acme_forms.TICKET_ACCOUNT
    # the hidden custom nested field takes its lower level along, listed after
    # the rule actions, as hidden and as optional
    assert d["hidden"] == ["priority", f"delivery_team_{acct}", f"delivery_squad_{acct}"]
    assert d["optional"] == [f"delivery_squad_{acct}"]
    assert d["disabled"] == [f"attachment_check_{acct}"]


def test_ticket_without_system_values_warns_and_does_not_match() -> None:
    spec = FormSpec.from_form_fields(acme_forms.ticket_internal_fields(), module="ticket")
    eng = RulesEngine(spec, acme_forms.ticket_rules(), module="ticket")
    st = FormState(spec, eng)
    assert st.evaluated.is_visible("priority")
    assert any("system values" in w for w in eng.warnings)


def test_ticket_description_html_target_resolves_to_description() -> None:
    _, _, st = ticket()
    st.set("Symptom", "Other")  # restricted list still offers Other
    spec, _, st2 = ticket(system={"portal": "other"})
    st2.set("Symptom", "Slow")
    assert st2.evaluated.is_visible("priority") and st2.evaluated.is_required("priority")
    assert not st2.evaluated.is_visible("description")


# -- business rules request and response ------------------------------------


def test_business_rules_query_for_a_new_ticket() -> None:
    q = business_rules_query("ticket", workspace_id=2)
    assert q.path == "/api/_/ws/2/business_rules"
    assert q.url("https://acme.freshservice.com/") == (
        "https://acme.freshservice.com/api/_/ws/2/business_rules?ember_api=true&module_type=ticket&item_id=null"
        "&requester=&requested_for=&include_workspace=false&filter_by_default_form_action=true"
        "&form_type=new_form&user_type=null&module_id=&cf_name_prefix=customFields.")


def test_business_rules_query_for_an_item_and_an_edit_form() -> None:
    sr = business_rules_query("service_request", workspace_id=3, item_id=9000009001, requester=9000005001)
    assert sr.query_string() == (
        "ember_api=true&module_type=service_request&item_id=9000009001&requester=9000005001&requested_for="
        "&include_workspace=false&form_type=new_form&user_type=null&module_id=&cf_name_prefix=customField.")
    edit = business_rules_query("change", workspace_id=2, form_type="edit", module_id=4242)
    params = dict(edit.params)
    assert params["form_type"] == "edit_form" and params["module_id"] == "4242"
    assert params["filter_by_default_form_action"] == "true"
    with pytest.raises(ValueError):
        business_rules_query("change", workspace_id=None)
    with pytest.raises(ValueError):
        business_rules_query("change", workspace_id=2, form_type="draft")


def test_parse_both_response_shapes() -> None:
    form = parse_business_rules(acme_forms.ticket_rules())
    assert (form.shape, form.evaluable, len(form.rules)) == ("form", True, 3)
    assert form.user["email"] == "robin.desk@example.com"
    listing = parse_business_rules(acme_forms.business_rules_listing())
    assert (listing.shape, listing.evaluable, listing.user) == ("listing", False, None)
    bare = parse_business_rules(acme_forms.ticket_rules()["business_rules"])
    assert bare.shape == "list" and bare.evaluable
    with pytest.raises(ValueError):
        parse_business_rules({"rules": []})
    json.dumps(form.raw)  # plain data, serialisable


def test_engine_refuses_the_rules_listing() -> None:
    spec = FormSpec.from_form_fields(acme_forms.ticket_internal_fields(), module="ticket")
    with pytest.raises(FormRuleError) as e:
        RulesEngine(spec, acme_forms.business_rules_listing(), module="ticket")
    assert "business_rules_query" in str(e.value)
