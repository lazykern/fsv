"""Property tests over random answer sequences (both rule models)."""
from __future__ import annotations

import pytest

hypothesis = pytest.importorskip("hypothesis")
from hypothesis import HealthCheck, given, settings  # noqa: E402
from hypothesis import strategies as st  # noqa: E402

import acme_forms  # noqa: E402
from fsv.forms import (  # noqa: E402
    FieldDisabledError,
    FormSpec,
    FormState,
    LookupValue,
    RulesEngine,
    ValueParseError,
    build_change_payload,
    excludes_lists,
)

OPS_ANSWERS = {
    "tier": ["Gold", "Silver", "Bronze", None],
    "region": ["EU", "US", None],
    "notes": ["n1", None],
    "tags": [["A"], ["B", "C"], ["C"], None],
    "vip": [True, False, None],
    "code": ["X-1", None],
    "meeting": ["N/A", "01/12/2030", None],
    "area": ["Ops", "Dev", None],
    "area_team": ["Infra", "Other", None],
    "area_unit": ["Racks", "Misc", None],
    "priority": ["Low", "Urgent"],
}
CHANGE_ANSWERS = {
    "team": ["Platform", "Payments", "Labs", None],
    "change_type": ["Normal", "Standard", "Emergency"],
    "cloud_only": ["Yes", "No", None],
    "stage": ["Live", "Staging", None],
    "rollout_tool": ["Pipeline", "Script", "Manual", None],
    "needs_review": [True, False],
    "review_scope": [["Code"], ["Infra", "Code"], None],
    "downtime": ["Yes", "No", None],
    "downtime_window": ["22:00 - 22:30", None],
    "priority": ["Low", "High", "Urgent"],
    "category": ["Hardware", "Software", None],
    "sub_category": ["Laptop", "Monitor", None],
    "reviewers": [[LookupValue(9000006001, "Sam")], None],
}


def _answers(table):
    pair = st.sampled_from(sorted(table)).flatmap(lambda k: st.tuples(st.just(k), st.sampled_from(table[k])))
    return st.lists(pair, max_size=12)


def _spec(kind: str) -> tuple[FormSpec, dict]:
    if kind == "ops":
        return FormSpec.from_form_fields(acme_forms.ops_fields(), "change"), acme_forms.ops_rules()
    return FormSpec.from_form_fields(acme_forms.change_fields(), "change"), acme_forms.change_rules()


def _run(kind: str, model: str, order: str, seq):
    spec, rules = _spec(kind)
    eng = RulesEngine(spec, rules, module="change", workspace_id=2, model=model)
    s = FormState(spec, eng, event_order=order)
    for name, value in seq:
        try:
            s.set(name, value)
        except (FieldDisabledError, ValueParseError):
            pass
    return s


def _snapshot(s: FormState):
    e = s.evaluated
    return (e.visible, sorted(e.required), sorted(e.disabled), sorted(e.values.items(), key=lambda kv: kv[0]),
            sorted(e.populated.items()), [m.text for m in e.messages], excludes_lists(e, "900000777"))


SETTINGS = settings(max_examples=60, deadline=None, suppress_health_check=[HealthCheck.too_slow])
CASES = [(k, m, o) for k in ("ops", "change") for m in ("event", "replay") for o in ("position", "input", "history")]


@pytest.mark.parametrize("kind,model,order", CASES)
def test_invariants(kind: str, model: str, order: str) -> None:
    @SETTINGS
    @given(_answers(OPS_ANSWERS if kind == "ops" else CHANGE_ANSWERS))
    def check(seq):
        s = _run(kind, model, order, seq)
        e = s.evaluated
        vis = set(e.visible)
        assert e.required <= vis  # required implies visible
        assert set(s.payload_values()) <= vis  # hidden values never sent
        body = build_change_payload(s, "robin.ops@example.com", allow_incomplete=True)
        sent = set(body["custom_fields"]) | {k for k in body if k in s.spec.by_name}
        hidden = {f.api_name for f in s.spec.fields} - vis
        assert not (sent & hidden)
        # nested children always valid for their parent
        for f in s.spec.fields:
            if f.is_nested_level and f.api_name in e.values:
                assert f.choice(e.values[f.api_name], s.spec.choices_for(f, e.values)) is not None
        # replay is deterministic: the same answers in a fresh state give the same form
        again = _run(kind, model, order, seq)
        assert _snapshot(again) == _snapshot(s)

    check()


@pytest.mark.parametrize("model", ["event", "replay"])
def test_reversible_toggle_restores_state(model: str) -> None:
    """S4-style: answering No, then Yes, then No again ends where No alone does."""
    spec, rules = _spec("change")

    def form(*answers):
        eng = RulesEngine(spec, rules, module="change", model=model)
        s = FormState(spec, eng, event_order="history")  # every click is a change event
        for a in answers:
            s.set("cloud_only", a)
        return s.evaluated

    once, toggled = form("No"), form("No", "Yes", "No")
    assert (once.visible, once.required, once.values) == (toggled.visible, toggled.required, toggled.values)
