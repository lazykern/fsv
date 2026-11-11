"""Unknown tenant widget types are not writable text fields."""
import pytest
from fsv.forms import FormSpec, FormState, PayloadError, build_ticket_payload
from fsv.forms.payload import encode_value, _status_id
from fsv.forms.errors import ValueParseError


def spec(ftype="custom_future_reference", required=False):
    return FormSpec.from_form_fields([{"name": "future", "label": "Future", "field_type": ftype,
                                      "required": required, "position": 1}], "ticket")


def test_unknown_answer_cannot_parse_or_encode():
    s = FormState(spec())
    f = s.spec.field("future")
    assert f.kind == "unsupported" and not f.sent
    with pytest.raises(ValueParseError):
        s.set("future", "raw text")
    with pytest.raises(PayloadError, match="unsupported"):
        encode_value(s.spec, f, "raw text", {})


@pytest.mark.parametrize("required,existing", [(True, {}), (False, {"future": "server value"})])
def test_required_or_answered_unsupported_blocks_even_incomplete(required, existing):
    s = FormState(spec(required=required), existing=existing)
    assert s.unsupported_errors()
    with pytest.raises(PayloadError, match="unsupported"):
        build_ticket_payload(s, "jane@example.com", allow_incomplete=True)


def test_optional_unanswered_unsupported_does_not_block():
    s = FormState(spec())
    assert not s.unsupported_errors()
    assert "future" not in build_ticket_payload(s, "jane@example.com", allow_incomplete=True).get("custom_fields", {})


def test_unknown_catalog_type_and_reference_are_unsupported():
    sr = FormSpec.from_service_item({"id": 123, "name": "Acme", "custom_fields": [
        {"id": 321, "name": "future", "type": 99999, "position": 1, "required": True}]})
    assert sr.field("future").kind == "unsupported"
    reference = FormSpec.from_form_fields([{"name": "asset", "field_type": "custom_lookup", "position": 1,
                                            "lookup_config": {"data_source": 99}}], "ticket")
    assert reference.field("asset").kind == "unsupported"


def test_missing_status_schema_and_ambiguous_choices_block():
    with pytest.raises(PayloadError, match="status schema"):
        _status_id(spec(), 99)
    choices = FormSpec.from_form_fields([{"name": "priority", "field_type": "default_priority", "position": 1,
        "choices": [{"id": 71, "value": "High"}, {"id": 81, "value": "high"}]}], "ticket")
    with pytest.raises(ValueError, match="ambiguous"):
        choices.field("priority").choice("HIGH")


def test_explicit_default_wins_over_first_choice():
    s = FormSpec.from_form_fields([{"name": "priority", "field_type": "default_priority", "position": 1,
        "default_value": 81, "choices": [{"id": 71, "value": "Low"}, {"id": 81, "value": "High"}]}], "ticket")
    assert s.field("priority").default == "High"


def test_invalid_boolean_is_not_false():
    s = FormState(spec("custom_checkbox"))
    with pytest.raises(ValueParseError):
        s.set("future", "truue")
