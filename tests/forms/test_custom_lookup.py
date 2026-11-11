"""Metadata-backed custom references must resolve before serialization."""
import pytest

from fsv.formmeta import ClientResolver
from fsv.forms import FormSpec, FormState
from fsv.forms.errors import AmbiguousLookupError, ValueParseError
from fsv.forms.payload import encode_value
from fsv.forms.state import LookupValue


class Client:
    def __init__(self, rows):
        self.rows = rows
        self.calls = []

    def lookup_choices(self, link, query):
        self.calls.append((link, query))
        return self.rows

    def autocomplete(self, *args):
        raise AssertionError("custom references must not use requester autocomplete")


def state(rows, multi=False, source=99):
    spec = FormSpec.from_form_fields([{
        "name": "reference", "label": "Reference", "position": 1,
        "field_type": "custom_multi_lookup" if multi else "custom_lookup",
        "lookup_config": {"data_source": source, "link": "/lookup_choices?field_name=reference&module=change"},
    }], "change")
    client = Client(rows)
    return FormState(spec, resolver=ClientResolver(client)), client


@pytest.mark.parametrize("answer", [901, "901", "Acme", LookupValue(901, "untrusted")])
def test_resolves_selected_reference_and_encodes_id(answer):
    s, c = state([{"id": 901, "value": "Acme"}])
    s.set("reference", answer)
    f = s.spec.field("reference")
    assert f.kind == "lookup" and f.sent
    assert s.get("reference").display == "Acme"
    assert encode_value(s.spec, f, s.get("reference"), {}) == 901
    assert c.calls[0][0] == f.lookup_link


@pytest.mark.parametrize("answer", [901, "901", LookupValue(901)])
def test_unresolved_numeric_id_cannot_bypass_resolver(answer):
    s, _ = state([{"id": 902, "name": "Globex"}])
    with pytest.raises(ValueParseError, match="no match"):
        s.set("reference", answer)


def test_duplicate_names_fail_closed_and_deleted_rows_are_ignored():
    s, c = state([{"id": 901, "name": "Acme"}, {"id": 902, "name": "Acme"}])
    with pytest.raises(AmbiguousLookupError):
        s.set("reference", "Acme")
    c.rows = [{"id": 901, "name": "Acme", "deleted": True}]
    with pytest.raises(ValueParseError, match="no match"):
        s.set("reference", "Acme")


def test_multi_lookup_verifies_every_record():
    s, _ = state([{"id": 901, "name": "Acme"}, {"id": 902, "name": "Globex"}], multi=True)
    s.set("reference", [901, 902])
    f = s.spec.field("reference")
    assert encode_value(s.spec, f, s.get("reference"), {}) == [901, 902]


def test_user_lookup_metadata_link_takes_precedence():
    s, c = state([{"id": 901, "name": "Acme"}], source=5)
    s.set("reference", 901)
    assert c.calls and s.get("reference").id == 901


def test_missing_resolver_refuses_even_numeric_reference():
    s, _ = state([])
    s.resolver = None
    with pytest.raises(ValueParseError):
        s.set("reference", 901)
