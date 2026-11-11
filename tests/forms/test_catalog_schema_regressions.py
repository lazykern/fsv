from __future__ import annotations

import copy

import pytest

from fsv.formmeta import ClientResolver
from fsv.forms import FormSpec, FormState, ValueParseError, build_sr_payload


def field(name, code=1001, **extra):
    return {"id": name, "name": name, "label": name, "type": code,
            "position": 1, "field_options": {}, "choices": [], "fields": [], **extra}


def catalog(*fields):
    return {"service_item": {"id": 9000000001, "display_id": 9000000002,
                             "name": "Acme", "custom_fields": list(fields)}}


def tree(representation):
    leaf = field("leaf", required=True)
    middle = field("middle", 1003, fields=[leaf],
                   choices=[{"value": "X", "dependent_ids": {"field": ["leaf"]}}])
    shared = field("shared", required=True)
    only_a = field("only_a")
    only_b = field("only_b")
    choices = [{"value": "A", "dependent_ids": {"field": ["shared", "only_a", "middle"]}},
               {"value": "B", "dependent_ids": {"field": ["shared", "only_b", "middle"]}},
               {"value": "C", "dependent_ids": {"field": []}}]
    info = [{"sectionFields": ["shared", "middle"], "sectionChoices": ["A", "B"]},
            {"sectionFields": ["only_a"], "sectionChoices": ["A"]},
            {"sectionFields": ["only_b"], "sectionChoices": ["B"]}]
    if representation == "separate":
        info = [{"sectionFields": ["shared", "only_a", "middle"], "sectionChoices": ["A"]},
                {"sectionFields": ["shared", "only_b", "middle"], "sectionChoices": ["B"]}]
    opts = {} if representation == "dependent" else {"section_info": info}
    if representation == "info":
        for choice in choices:
            choice.pop("dependent_ids")
    controller = field("controller", 1003, choices=choices, field_options=opts,
                       fields=[shared, only_a, only_b, middle, field("unmapped")])
    return catalog(controller, field("after", position=2))


@pytest.mark.parametrize("representation", ["separate", "combined", "dependent", "info"])
def test_shared_sections_overlap_and_descendants(representation):
    spec = FormSpec.from_service_item(tree(representation))
    assert [f.api_name for f in spec.ordered()] == [
        "controller", "shared", "only_a", "only_b", "middle", "leaf", "unmapped", "after"]
    assert len(spec.fields) == len(spec.ordered()) == 8
    assert spec.section_parent("shared")[1] == ("a", "b")
    assert any("unmapped" in warning and "hidden" in warning for warning in spec.warnings)
    state = FormState(spec)
    assert state.evaluated.visible == ["controller", "after"]
    state.set("controller", "A")
    assert state.evaluated.visible == ["controller", "shared", "only_a", "middle", "after"]
    assert [f.api_name for f in state.missing_required()] == ["shared"]
    state.set("shared", "retained")
    state.set("middle", "X")
    assert state.evaluated.visible == ["controller", "shared", "only_a", "middle", "leaf", "after"]
    assert [f.api_name for f in state.missing_required()] == ["leaf"]
    state.set("leaf", "answer")
    state.set("controller", "B")
    assert state.evaluated.visible == ["controller", "shared", "only_b", "middle", "leaf", "after"]
    assert state.payload_values()["shared"] == "retained"
    assert state.payload_values()["leaf"] == "answer"
    state.set("controller", "C")
    assert state.evaluated.visible == ["controller", "after"]
    assert "shared" not in state.payload_values() and "leaf" not in state.payload_values()
    state.set("controller", "B")
    assert "shared" in state.evaluated.visible and "leaf" in state.evaluated.visible


@pytest.mark.parametrize("reverse", [False, True])
def test_inconsistent_section_sources_fail_closed(reverse):
    item = tree("separate")
    ctrl = item["service_item"]["custom_fields"][0]
    if reverse:
        ctrl["field_options"]["section_info"][0]["sectionFields"].remove("only_a")
    else:
        ctrl["choices"][0]["dependent_ids"]["field"].remove("only_a")
    with pytest.raises(ValueError, match="section_info and dependent_ids differ"):
        FormSpec.from_service_item(item)


def test_explicit_empty_section_source_conflicts():
    item = tree("separate")
    ctrl = item["service_item"]["custom_fields"][0]
    ctrl["field_options"]["section_info"] = []
    with pytest.raises(ValueError, match="section_info and dependent_ids differ"):
        FormSpec.from_service_item(item)


@pytest.mark.parametrize("malformed", ["{", {"wrong": "shape"}])
def test_bad_section_metadata_fails_closed(malformed):
    item = tree("info")
    item["service_item"]["custom_fields"][0]["field_options"]["section_info"] = malformed
    with pytest.raises(ValueError, match="unreadable section_info"):
        FormSpec.from_service_item(item)


def test_unknown_or_nonchild_section_id_fails_closed():
    item = tree("info")
    ctrl = item["service_item"]["custom_fields"][0]
    ctrl["field_options"]["section_info"][0]["sectionFields"].append("absent")
    with pytest.raises(ValueError, match="unknown child id"):
        FormSpec.from_service_item(item)
    ctrl["field_options"]["section_info"][0]["sectionFields"][-1] = "after"
    with pytest.raises(ValueError, match="non-child"):
        FormSpec.from_service_item(item)


def test_types_validate_and_notice_never_submits():
    item = catalog(field("confirm", 1006), field("amount", 1021),
                   field("selection", 1019, choices=[{"value": "One"}, {"value": "Two"}]),
                   field("notice", 1024, label="<p>Acme guidance</p>"))
    spec = FormSpec.from_service_item(item)
    assert [f.kind for f in spec.ordered()] == ["checkbox", "number", "dropdown", "notice"]
    state = FormState(spec)
    state.set("confirm", "yes")
    state.set("amount", "3.5")
    state.set("selection", "Two")
    for name, invalid in [("confirm", "maybe"), ("amount", "not numeric"), ("selection", "Three"),
                          ("notice", "text")]:
        with pytest.raises(ValueParseError):
            state.set(name, invalid)
    body = build_sr_payload(None, state, "user@example.com")
    assert body["custom_fields"] == {"confirm": True, "amount": 3.5, "selection": "Two"}
    assert "notice" not in body["custom_fields"]


def test_catalog_lookup_metadata_is_inert_not_unconstrained_text():
    opts = {"link": "/lookup_choices", "data_source": 4,
            "conditions": [{"field": "kind", "value": "Acme"}]}
    spec = FormSpec.from_service_item(catalog(field("single", 501, field_options=opts),
                                             field("multiple", 502, field_options=copy.deepcopy(opts))))
    assert len(spec.warnings) == 2
    for name, multi in [("single", False), ("multiple", True)]:
        f = spec.by_name[name]
        assert f.kind == "lookup" and f.multi is multi and not f.sent
        assert f.lookup_config == opts and f.lookup_link is None and f.lookup_entity == "catalog_lookup"
        state = FormState(spec, resolver=ClientResolver(object()))
        with pytest.raises(ValueParseError):
            state.set(name, "unresolved name")
    state = FormState(spec)
    state.set("single", 9000000003)
    state.set("multiple", [9000000004, 9000000005])
    assert "not sent" in state.to_review()[0][1]
    assert state.get("single").id == 9000000003
    assert len(state.get("multiple")) == 2
