"""br_validation_excludes: an unsigned JWT-shaped token built from engine state."""
from __future__ import annotations

import base64

import pytest

import acme_forms
from fsv.forms import (
    BR_VALIDATION_EXCLUDES,
    EvaluatedForm,
    FormSpec,
    FormState,
    PayloadError,
    RulesEngine,
    build_br_validation_excludes,
    build_change_payload,
    build_sr_payload,
    build_ticket_payload,
    decode_br_validation_excludes,
    excludes_lists,
)
from fsv.forms.rules import ExcludeName

HEADER = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9"  # {"alg":"HS256","typ":"JWT"}
CF = acme_forms.CF


def ops_state(spec: FormSpec, rules=None, **values) -> FormState:
    eng = RulesEngine(spec, rules if rules is not None else acme_forms.ops_rules(), module="change", workspace_id=2,
                      populate="set", event_scope="referenced", event_reverse="transition")
    return FormState(spec, eng, values=values)


def test_account_id_is_derived_from_metadata(ops_spec: FormSpec, ticket_spec: FormSpec) -> None:
    assert ops_spec.account_id == acme_forms.ACCOUNT  # from a lookup link: field_name=owner_900000777
    assert ticket_spec.account_id is None
    item = acme_forms.service_item()
    for f in item["service_item"]["custom_fields"]:
        f["account_id"] = 900000778
    assert FormSpec.from_service_item(item).account_id == "900000778"


def test_flat_token_byte_for_byte(ops_spec: FormSpec) -> None:
    st = ops_state(ops_spec, tier="Gold", region="US", vip=True)
    lists = excludes_lists(st.evaluated)
    assert lists == {"optional": ["code_900000777"], "disabled": ["meeting_900000777"], "hidden": []}
    token = build_br_validation_excludes(st.evaluated, "change")
    assert token == (
        "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltcImNvZGVfOTAwMDAwNzc3XCJdIiwiZGlzYWJsZWQiOiJbXC"
        "JtZWV0aW5nXzkwMDAwMDc3N1wiXSIsImhpZGRlbiI6IltdIn0="
    )
    raw = base64.b64decode(token.split(".")[1])
    assert raw == b'{"optional":"[\\"code_900000777\\"]","disabled":"[\\"meeting_900000777\\"]","hidden":"[]"}'
    assert token.split(".")[0] == HEADER and token.count(".") == 1  # no signature segment


def test_list_strings_use_comma_space_and_default_fields_stay_bare(ops_spec: FormSpec) -> None:
    a, r = acme_forms._a, acme_forms.ops_rule
    rules = [r(1, "Hide two", {"all": None}, [a("hide", ["priority", CF + "notes", CF + "not_on_form"])])]
    st = ops_state(ops_spec, rules)
    token = build_br_validation_excludes(st.evaluated, "change")
    raw = base64.b64decode(token.split(".")[1]).decode()
    assert '"hidden":"[\\"priority\\", \\"notes_900000777\\"]"' in raw  # unknown fields are not listed
    assert decode_br_validation_excludes(token)["hidden"] == ["priority", "notes_900000777"]


def test_order_is_first_touch_and_membership_is_final_state(ops_spec: FormSpec) -> None:
    st = ops_state(ops_spec, tier="Silver", region="EU", vip=True)
    # region_notes touched at load (hidden), notes touched by "Silver shows" then hidden by "EU region";
    # code was unmandated at load, re-mandated by the Silver transition -> not optional
    assert excludes_lists(st.evaluated) == {
        "optional": [], "disabled": ["meeting_900000777"], "hidden": ["region_notes_900000777", "notes_900000777"]}


def test_optional_lists_only_statically_required_fields(ops_spec: FormSpec) -> None:
    a, r = acme_forms._a, acme_forms.ops_rule
    rules = [r(1, "Relax", {"all": None}, [a("unmandate", [CF + "notes", CF + "code", "subject"])])]
    st = ops_state(ops_spec, rules)
    assert excludes_lists(st.evaluated)["optional"] == ["code_900000777", "subject"]  # notes was never required


def test_service_request_token_keyed_by_item_id(sr_spec: FormSpec) -> None:
    rules = {"business_rules": [acme_forms.ops_rule(
        1, "Staff may skip attachments",
        {"all": [{"name": "department_ids", "value": ["9000005001"], "operator": "includes",
                  "field_type": "multiselect_choicelist", "evaluate_on": "current_user"}]},
        [acme_forms._a("unmandate", ["attachment_ids"])], module_type="service_request", item_id=4400001)],
        "user": {"department_ids": [9000005001]}}
    eng = RulesEngine(sr_spec, rules, module="service_request", item_id=4400001)
    st = FormState(sr_spec, eng)
    for k, v in {"title_100": "T", "paragraph_101": "D", "date_103": "2030-01-02", "goal": "Other",
                 "priority": "Low - cosmetic, reports"}.items():
        st.set(k, v)
    assert st.evaluated.attachments_required is False
    body = build_sr_payload(None, st, "jane.doe@acme.example", field_attachments={"overview_deck": [1]})
    assert body["attachments"] == []  # the rule lifted the item's attachment_mandatory
    dec = decode_br_validation_excludes(body[BR_VALIDATION_EXCLUDES])
    assert dec["items"] == {"4400001": {"optional": ["attachment_ids"], "disabled": [], "hidden": []}}
    plain = FormState(sr_spec, RulesEngine(sr_spec, [], module="service_request"))
    tok = build_br_validation_excludes(plain.evaluated, "service_request", item_id=4400001)
    # 64 payload bytes: no padding, like the captured SR token
    assert tok == HEADER + ".eyI0NDAwMDAxIjp7Im9wdGlvbmFsIjoiW10iLCJkaXNhYmxlZCI6IltdIiwiaGlkZGVuIjoiW10ifX0="
    with pytest.raises(PayloadError):
        build_br_validation_excludes(plain.evaluated, "service_request")


def test_account_id_parameter_and_errors(ticket_spec: FormSpec, ops_spec: FormSpec) -> None:
    rules = [acme_forms.ops_rule(1, "Hide due", {"all": None}, [acme_forms._a("hide", [CF + "due_on"])],
                                 module_type="ticket")]
    st = FormState(ticket_spec, RulesEngine(ticket_spec, rules, module="ticket"))
    with pytest.raises(PayloadError, match="account id"):
        build_br_validation_excludes(st.evaluated, "ticket")
    assert decode_br_validation_excludes(build_br_validation_excludes(st.evaluated, "ticket", 900000779))[
        "hidden"] == ["due_on_900000779"]
    body = build_ticket_payload(st, "sam.lee@acme.example", account_id=900000779, allow_incomplete=True)
    assert decode_br_validation_excludes(body[BR_VALIDATION_EXCLUDES])["hidden"] == ["due_on_900000779"]
    ops = ops_state(ops_spec)
    with pytest.raises(PayloadError, match="does not match"):
        build_br_validation_excludes(ops.evaluated, "change", account_id="900000001")
    with pytest.raises(ValueError):
        build_br_validation_excludes(ops.evaluated, "invoice")
    odd = EvaluatedForm([], set(), set(), {}, {}, excludes={"optional": [], "disabled": [],
                                                            "hidden": [ExcludeName("bad name", False)]})
    with pytest.raises(PayloadError, match="unexpected field name"):
        build_br_validation_excludes(odd, "change")


def test_payload_builders_always_compute_the_token(ops_spec: FormSpec) -> None:
    st = ops_state(ops_spec, tier="Gold", region="US", vip=True)
    body = build_change_payload(st, "robin.ops@example.com", allow_incomplete=True)
    assert body[BR_VALIDATION_EXCLUDES] == build_br_validation_excludes(st.evaluated, "change")
    with pytest.raises(TypeError):
        build_change_payload(st, "robin.ops@example.com", br_validation_excludes="x")  # type: ignore[call-arg]
