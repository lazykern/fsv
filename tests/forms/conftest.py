from __future__ import annotations

from typing import Any, Sequence

import pytest

import acme_forms
from fsv.forms import FormSpec, FormState, LookupValue, RulesEngine


class FakeResolver:
    """In-memory LookupResolver: substring search over a fixed directory."""

    def __init__(self) -> None:
        self.calls: list[tuple[str, str]] = []
        self.records: dict[str, list[LookupValue]] = {
            "user": [
                LookupValue(9001, "Jane Doe", "jane.doe@acme.example"),
                LookupValue(9002, "Jane Doerr", "jane.doerr@acme.example"),
                LookupValue(9003, "Sam Lee", "sam.lee@acme.example"),
            ],
            "group": [LookupValue(601, "Platform Ops"), LookupValue(602, "Platform")],
            "agent": [LookupValue(701, "Alex Kim", "alex.kim@acme.example")],
            "change_window": [LookupValue(801, "Weekend window")],
        }

    def lookup(self, entity: str, query: str, field: Any) -> Sequence[LookupValue]:
        self.calls.append((entity, query))
        q = query.casefold()
        return [r for r in self.records.get(entity, []) if q in str(r).casefold()]


@pytest.fixture
def change_spec() -> FormSpec:
    return FormSpec.from_form_fields(acme_forms.change_fields(), module="change")


@pytest.fixture
def change_rules(change_spec: FormSpec) -> RulesEngine:
    return RulesEngine(change_spec, acme_forms.change_rules(), module="change")


@pytest.fixture
def resolver() -> FakeResolver:
    return FakeResolver()


@pytest.fixture
def change_state(change_spec: FormSpec, change_rules: RulesEngine, resolver: FakeResolver) -> FormState:
    return FormState(change_spec, change_rules, mode="new", resolver=resolver)


@pytest.fixture
def sr_spec() -> FormSpec:
    return FormSpec.from_service_item(acme_forms.service_item())


@pytest.fixture
def ticket_spec() -> FormSpec:
    return FormSpec.from_form_fields(acme_forms.ticket_fields(), module="ticket")
