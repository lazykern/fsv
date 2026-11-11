"""Legacy choice writes must use the current tenant's schema."""
import pytest
import typer

from fsv import cli


def schema(choices):
    return {"fields": [{"name": "priority", "field_type": "default_priority", "choices": choices}]}


@pytest.mark.parametrize("choices,expected", [
    ([{"id": 1, "value": "High"}], 1),
    ([{"id": 87, "value": "High"}], 87),
    ([{"id": 97, "value": "Critical"}], 97),
])
def test_choices_are_tenant_specific(choices, expected):
    sch = schema(choices)
    label = choices[0]["value"]
    assert cli._resolve_update_choice(sch, "priority", label) == expected
    assert cli._resolve_set_field(None, sch, "priority", label) == ("priority", expected)
    assert cli._resolve_update_choice(sch, "priority", str(expected)) == expected


@pytest.mark.parametrize("choices,raw", [
    ([], "High"), ([], "1"), ([{"id": 87, "value": "High"}], "1"),
    ([{"id": 87, "value": "High", "deleted": True}], "87"),
    ([{"id": 87, "value": "High"}, {"id": 97, "value": "high"}], "High"),
])
def test_missing_deleted_or_ambiguous_choices_fail(choices, raw):
    with pytest.raises(typer.Exit):
        cli._resolve_update_choice(schema(choices), "priority", raw)


def test_checkbox_typo_does_not_become_false():
    sch = {"fields": [{"name": "checked", "field_type": "custom_checkbox"}]}
    assert cli._resolve_set_field(None, sch, "checked", "false") == ("checked", False)
    with pytest.raises(typer.Exit):
        cli._resolve_set_field(None, sch, "checked", "truue")
