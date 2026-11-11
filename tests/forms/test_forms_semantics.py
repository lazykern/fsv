"""Rule semantics from the design review: event vs replay model, context
filters, display-id populate, blocking messages, disabled/hidden handling,
nested fields, time zones, explain() and conflict reports."""
from __future__ import annotations

import datetime as dt

import pytest

import acme_forms
from fsv.forms import (
    FieldDisabledError,
    FormSpec,
    FormState,
    LookupValue,
    RuleContext,
    RulesEngine,
    ValueParseError,
    build_change_payload,
)

CF = acme_forms.CF
LEGACY_ENGINE = {"populate": "set", "event_scope": "referenced", "event_reverse": "transition"}


def engine(spec: FormSpec, rules=None, **kw) -> RulesEngine:
    kw.setdefault("module", "change")
    kw.setdefault("workspace_id", 2)
    for k, v in LEGACY_ENGINE.items():  # earlier semantics; defaults are covered in test_forms_restrict.py
        kw.setdefault(k, v)
    return RulesEngine(spec, rules if rules is not None else acme_forms.ops_rules(), **kw)


def state(spec: FormSpec, eng: RulesEngine | None = None, **kw) -> FormState:
    return FormState(spec, eng or engine(spec), **kw)


# -- event model --------------------------------------------------------------


def test_load_evaluates_every_rule_against_defaults(ops_spec: FormSpec, ops_engine: RulesEngine) -> None:
    ev = state(ops_spec, ops_engine).evaluated
    assert ev.model == "event"
    assert not ev.is_visible("region_notes")  # "always" rule ran at load
    assert "code" not in ev.required  # "tier not Silver" matches an empty tier
    assert ev.decision("code", "required").rule_id == 9000004005


def test_populate_does_not_cascade_in_event_model(ops_spec: FormSpec) -> None:
    st = state(ops_spec)
    d = st.set("tier", "Gold")
    assert st.get("region") == "EU" and st.evaluated.populated == {"region": 9000004001}
    assert d.populated == {"region": "EU"}
    assert st.evaluated.is_visible("notes")  # "EU region" did not fire: populate is not a change event
    replay = state(ops_spec, engine(ops_spec, model="replay"))
    replay.set("tier", "Gold")
    assert not replay.evaluated.is_visible("notes")  # replay model: every rule sees the populated value


def test_selecting_the_current_value_is_not_a_change_event(ops_spec: FormSpec) -> None:
    st = state(ops_spec)
    st.set("tier", "Gold")
    st.set("region", "EU")  # already EU (populated): the web fires no change
    assert st.evaluated.is_visible("notes")
    loud = state(ops_spec, engine(ops_spec, same_value_fires=True))
    loud.set("tier", "Gold")
    loud.set("region", "EU")
    assert not loud.evaluated.is_visible("notes")
    # FormState replays the *final* answers, not the click history: US then
    # EU again is the same as answering EU once (documented contract)
    st.set("region", "US")
    assert st.evaluated.is_visible("notes")  # US: the EU rule does not match
    st.set("region", "EU")
    assert st.evaluated.is_visible("notes")
    # event_order="history" replays every click instead: US then EU is a real change
    clicks = state(ops_spec, event_order="history")
    clicks.set("tier", "Gold")
    clicks.set("region", "US")
    clicks.set("region", "EU")
    assert not clicks.evaluated.is_visible("notes")
    # a record already in EU (load state) that the user moves to EU is a real change only if it differs
    edit = state(ops_spec, mode="edit", existing={"region": "US"})
    edit.set("region", "EU")
    assert not edit.evaluated.is_visible("notes")


def test_change_runs_only_rules_that_reference_the_field(ops_spec: FormSpec) -> None:
    st = state(ops_spec)
    st.set("tier", "Gold")
    # "Gold tier" shows Region Notes; the later "always hide" rule only ran at load
    assert st.evaluated.is_visible("region_notes")
    replay = state(ops_spec, engine(ops_spec, model="replay"))
    replay.set("tier", "Gold")
    assert not replay.evaluated.is_visible("region_notes")  # last rule wins in replay


def _toggle_rules() -> list[dict]:
    a, r, c = acme_forms._a, acme_forms.ops_rule, acme_forms._c
    return [
        r(1, "Silver hides when", {"all": [c(CF + "tier", "is", "Silver")]}, [a("hide", [CF + "when"])], rev=True),
        r(2, "Always hide when", {"all": None}, [a("hide", [CF + "when"])]),
    ]


def test_reversal_only_on_matched_to_unmatched_transition(ops_spec: FormSpec) -> None:
    st = state(ops_spec, engine(ops_spec, _toggle_rules()))
    assert not st.evaluated.is_visible("when")  # load: reverse (show) then rule 2 hides
    st.set("tier", "Gold")  # rule 1 did not match before either: nothing to undo
    assert not st.evaluated.is_visible("when")
    clicks = state(ops_spec, engine(ops_spec, _toggle_rules()), event_order="history")
    clicks.set("tier", "Silver")
    assert not clicks.evaluated.is_visible("when")
    clicks.set("tier", "Gold")  # matched -> unmatched: reversed
    assert clicks.evaluated.is_visible("when")
    # matched -> unmatched: the record is Silver (rule 1 matched at load), the user picks Gold
    edit = state(ops_spec, engine(ops_spec, _toggle_rules()), mode="edit", existing={"tier": "Silver"})
    assert not edit.evaluated.is_visible("when")
    edit.set("tier", "Gold")
    assert edit.evaluated.is_visible("when")
    always = state(ops_spec, engine(ops_spec, _toggle_rules(), event_reverse="always"))
    always.set("tier", "Gold")
    assert always.evaluated.is_visible("when")
    with pytest.raises(ValueError):
        engine(ops_spec, event_reverse="sometimes")


def test_answers_replay_in_field_order_or_input_order(ops_spec: FormSpec) -> None:
    rules = acme_forms.ops_rules()
    by_pos = state(ops_spec, engine(ops_spec, rules))
    by_pos.set("region", "US")
    by_pos.set("tier", "Gold")  # replayed in field order: tier (1) then region (2) -> US stays
    assert by_pos.get("region") == "US" and by_pos.evaluated.overridden == {}
    by_input = state(ops_spec, engine(ops_spec, rules), event_order="input")
    by_input.set("region", "US")
    by_input.set("tier", "Gold")  # in input order the populate replaces the earlier answer
    assert by_input.get("region") == "EU"
    assert by_input.evaluated.overridden == {"region": ("US", 9000004001)}
    assert any(r[0] == "Replaced" and "Region" in r[1] for r in by_input.to_review())
    with pytest.raises(ValueError):
        state(ops_spec, event_order="random")


def test_replay_is_deterministic_and_edit_replays_from_scratch(ops_spec: FormSpec) -> None:
    a = state(ops_spec, values={"tags": "A", "tier": "Silver", "region": "EU"})
    b = state(ops_spec, values={"region": "EU", "tier": "Silver", "tags": "A"})
    assert (a.evaluated.visible, a.evaluated.required, a.evaluated.values) == (
        b.evaluated.visible, b.evaluated.required, b.evaluated.values)
    a.set("tier", "Gold")  # editing an earlier answer replays the whole sequence
    fresh = state(ops_spec, values={"tags": "A", "tier": "Gold", "region": "EU"})
    assert a.evaluated.visible == fresh.evaluated.visible and a.evaluated.required == fresh.evaluated.required


# -- operators and actions -----------------------------------------------------


def test_includes_on_single_and_multi_select(ops_spec: FormSpec) -> None:
    st = state(ops_spec)
    st.set("tags", ["C"])
    assert "notes" not in st.evaluated.required
    st.set("tags", ["C", "B"])  # multi-select: any-of
    assert "notes" in st.evaluated.required
    # single select: value in list (does_not_include = not in list)
    st.set("tier", "Silver")
    assert "code" in st.evaluated.required
    st.set("tier", "Bronze")
    assert "code" not in st.evaluated.required
    a, r, c = acme_forms._a, acme_forms.ops_rule, acme_forms._c
    none_of = [r(1, "No A", {"all": [c(CF + "tags", "does_not_include", ["A"])]}, [a("hide", [CF + "when"])],
                 rev=True)]
    s2 = state(ops_spec, engine(ops_spec, none_of), values={"tags": ["B", "C"]})
    assert not s2.evaluated.is_visible("when")
    s3 = state(ops_spec, engine(ops_spec, none_of), values={"tags": ["B", "A"]})
    assert s3.evaluated.is_visible("when")


def test_unconditional_rules_and_empty_name_lists(ops_spec: FormSpec, ops_engine: RulesEngine) -> None:
    rule = next(r for r in ops_engine.rules if r.id == 9000004008)
    assert rule.unconditional and all(a.all_targets == () for a in rule.actions)
    ev = state(ops_spec, ops_engine).evaluated
    assert 9000004008 in ev.matched and 9000004003 in ev.matched  # {"any": null} / {"all": null}
    assert not any("Empty lists" in w for w in ops_engine.warnings)


def test_populate_uses_choice_display_id(ops_spec: FormSpec) -> None:
    a, r = acme_forms._a, acme_forms.ops_rule
    # GOLD_DID is Gold's display id and also Bronze's choice id: display id wins
    rules = [r(1, "Default tier", {"all": None}, [a("populate", [CF + "tier"], [acme_forms.GOLD_DID])])]
    st = state(ops_spec, engine(ops_spec, rules))
    assert st.get("tier") == "Gold"
    assert ops_spec.by_name["tier"].choice_by_display_id(str(acme_forms.GOLD_DID)).value == "Gold"


def test_rule_filters_use_the_context(ops_spec: FormSpec) -> None:
    eng = engine(ops_spec)
    ids = {r.id for r in eng.rules}
    assert 9000004010 not in ids and any("workspace 3" in s for s in eng.skipped)
    assert 9000004011 not in ids  # an item's rules apply to that item only
    scoped = RulesEngine(ops_spec, acme_forms.ops_rules(),
                         context=RuleContext(module="change", workspace_id=[1, 2], item_id=9000008888))
    assert 9000004011 not in {r.id for r in scoped.rules}
    mine = RulesEngine(ops_spec, acme_forms.ops_rules(), context=RuleContext(module="change", item_id=9000009999))
    assert 9000004011 in {r.id for r in mine.rules}
    ev = state(ops_spec, scoped).evaluated
    assert ev.is_visible("subject")  # requester-only and other-item rules not applied to agents
    portal = RulesEngine(ops_spec, acme_forms.ops_rules(), context=RuleContext(module="change", user_type="requester",
                                                                               workspace_id=2, item_id=1))
    assert not state(ops_spec, portal).evaluated.is_visible("subject")
    assert RulesEngine(ops_spec, acme_forms.ops_rules(), module="ticket").rules == []


def test_current_user_conditions_read_the_rules_user_object(ops_spec: FormSpec) -> None:
    eng = engine(ops_spec)  # rules response user: department 9000005002
    assert state(ops_spec, eng).evaluated.is_visible("when")
    staff = engine(ops_spec, user={"department_ids": [9000005001, 9000005003]})
    assert not state(ops_spec, staff).evaluated.is_visible("when")
    data = acme_forms.ops_rules()
    del data["user"]
    blind = engine(ops_spec, data)
    assert state(ops_spec, blind).evaluated.is_visible("when")
    assert any("user record" in w for w in blind.warnings)


def test_requester_conditions_read_the_requester_context(ops_spec: FormSpec) -> None:
    a, r = acme_forms._a, acme_forms.ops_rule
    rules = [r(1, "VIP requester", {"all": [{"name": "vip_user", "operator": "selected", "evaluate_on": "requester"}]},
               [a("mandate", [CF + "notes"])])]
    ctx = RuleContext(module="change", requester={"vip_user": True})
    assert "notes" in state(ops_spec, RulesEngine(ops_spec, rules, context=ctx)).evaluated.required
    assert "notes" not in state(ops_spec, engine(ops_spec, rules)).evaluated.required


# -- messages, disabled, hidden -------------------------------------------------


def test_validation_message_blocks_until_acknowledged(ops_spec: FormSpec) -> None:
    st = state(ops_spec)
    st.set("priority", "Urgent")
    (m,) = st.evaluated.blocking_messages
    assert m.text == "Call the duty lead first" and m.rule_id == 9000004007
    assert st.unacknowledged() == [m]
    assert "blocked by rule (acknowledge to continue): Call the duty lead first" in st.errors()
    assert ("Blocked", "Call the duty lead first") in st.to_review()
    with pytest.raises(ValueError):
        st.acknowledge("no such message")
    st.acknowledge(9000004007)
    assert st.unacknowledged() == [] and ("Acknowledged", "Call the duty lead first") in st.to_review()
    st.set("priority", "Low")
    assert st.evaluated.blocking_messages == []


def test_disabled_field_refuses_input_but_keeps_populated_value(ops_spec: FormSpec) -> None:
    st = state(ops_spec)
    st.set("vip", True)
    assert st.get("meeting") == "N/A" and "meeting" in st.evaluated.disabled
    with pytest.raises(FieldDisabledError, match="VIP"):
        st.set("meeting", "01/12/2030")
    assert st.payload_values()["meeting"] == "N/A"
    # an answer the replay reaches after the field was disabled is refused
    with pytest.raises(FieldDisabledError, match="Review Meeting"):
        state(ops_spec, values={"meeting": "01/12/2030", "vip": "yes"})
    # setting the field that disables an earlier answer is refused and rolled back
    s2 = state(ops_spec)
    s2.set("meeting", "01/12/2030")
    with pytest.raises(FieldDisabledError, match="disables Review Meeting"):
        s2.set("vip", True)  # field order: VIP comes first, so the meeting answer would be refused
    assert "vip" not in s2.values and s2.get("meeting") == "01/12/2030"
    # in input order the VIP populate replaces the earlier answer instead (as on the web)
    s3 = state(ops_spec, event_order="input")
    s3.set("meeting", "01/12/2030")
    s3.set("vip", True)
    assert s3.get("meeting") == "N/A" and s3.evaluated.overridden["meeting"] == ("01/12/2030", 9000004006)


def test_hidden_values_are_dropped_and_reported(ops_spec: FormSpec) -> None:
    st = state(ops_spec)
    st.set("notes", "keep me?")
    st.set("region", "EU")  # hides Notes
    assert "notes" not in st.payload_values()
    (f, v, d) = st.dropped()[0]
    assert (f.api_name, v, d.rule_name) == ("notes", "keep me?", "EU region")
    assert ("Dropped", "Notes = keep me? (hidden by rule 'EU region'; not sent)") in st.to_review()
    body = build_change_payload(st, "robin.ops@example.com", allow_incomplete=True)
    assert "notes" not in body["custom_fields"]


def test_nested_children_validated_and_cleared_when_parent_changes(ops_spec: FormSpec) -> None:
    st = state(ops_spec)
    with pytest.raises(ValueParseError):
        st.set("area_team", "Infra")  # parent first
    st.set("area", "Ops")
    with pytest.raises(ValueParseError):
        st.set("area_team", "Misc")  # not a child of Ops
    st.set("area_team", "Infra")
    assert "area_unit" in st.evaluated.required  # rule condition on a lower level
    st.set("area_team", "Other")
    st.set("area", "Dev")  # "Other" exists under Dev too, but the web clears children anyway
    assert "area_team" not in st.values
    d = st.set("area", "Ops")
    assert d.cleared == []  # nothing left to clear
    st.set("area_team", "Infra")
    d = st.set("area", "Dev")
    assert d.cleared == ["area_team"]


def test_date_like_dropdowns_are_not_parsed_as_dates(ops_spec: FormSpec) -> None:
    st = state(ops_spec)
    st.set("meeting", "08/12/2030")
    assert st.get("meeting") == "08/12/2030"
    for bad in ("2030-12-08", dt.date(2030, 12, 8), "8/12/2030"):
        with pytest.raises(ValueParseError):
            st.set("meeting", bad)
    assert build_change_payload(st, "robin.ops@example.com", allow_incomplete=True)["custom_fields"]["meeting"] == (
        "08/12/2030")


# -- time zones -------------------------------------------------------------------


def test_time_zone_comes_from_the_freshservice_user(ops_spec: FormSpec, monkeypatch: pytest.MonkeyPatch) -> None:
    st = state(ops_spec)  # rules user time_zone "Tokyo" (Rails name)
    assert "Tokyo" in st.tz_source
    st.set("when", "2030-01-15 09:00")
    assert st.get("when").utcoffset() == dt.timedelta(hours=9)
    st2 = state(ops_spec, engine(ops_spec, user={"time_zone": "Europe/Paris"}))
    st2.set("when", "2030-01-15 09:00")
    assert st2.get("when").utcoffset() == dt.timedelta(hours=1)
    monkeypatch.setenv("FSV_TZ", "America/New_York")
    st3 = state(ops_spec)
    st3.set("when", "2030-01-15 09:00")
    assert st3.get("when").utcoffset() == dt.timedelta(hours=-5) and st3.tz_source == "FSV_TZ"
    monkeypatch.setenv("FSV_TZ", "Atlantis/Nowhere")
    with pytest.raises(ValueError):
        state(ops_spec)


def test_time_zone_falls_back_to_the_system(ops_spec: FormSpec, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("TZ", "America/Sao_Paulo")
    data = acme_forms.ops_rules()
    del data["user"]
    st = state(ops_spec, engine(ops_spec, data))
    assert st.tz_source == "system time zone"
    st.set("when", "2030-01-15 09:00")
    assert st.get("when").utcoffset() == dt.timedelta(hours=-3)


# -- explain and conflicts ------------------------------------------------------------


def test_explain_names_the_rule_behind_each_decision(ops_spec: FormSpec) -> None:
    st = state(ops_spec)
    by = {(e.field, e.property): e for e in st.explain()}
    rn = by[("region_notes", "visible")]
    assert (rn.value, rn.source, rn.decision.rule_id) == (False, "rule", 9000004003)
    assert by[("region", "visible")].source == "metadata"
    assert by[("region", "required")].value is True and by[("region", "required")].source == "metadata"
    assert by[("code", "required")].decision.rule_name == "Tier not Silver"
    assert by[("area_team", "visible")].source == "nested"
    st.set("tier", "Gold")
    e = st.explain(["Region Notes"])[0]
    assert e.value and e.decision.rule_id == 9000004001 and e.decision.event == "tier"
    assert "Gold tier" in str(e) and "on change of tier" in str(e)
    st.set("vip", True)
    dis = [x for x in st.explain(["meeting"]) if x.property == "disabled"][0]
    assert dis.value and dis.decision.rule_name == "VIP"


def test_conflict_report_lists_disagreeing_matched_rules(ops_spec: FormSpec) -> None:
    st = state(ops_spec)
    st.set("tier", "Silver")  # shows Notes
    st.set("region", "EU")  # hides Notes
    (c,) = [c for c in st.conflicts if c.field == "notes"]
    assert c.property == "visible"
    assert [(d.rule_name, d.action) for d in c.decisions] == [("Silver shows notes", "show"), ("EU region", "hide")]
    assert c.winner.rule_name == "EU region" and "hide wins" in c.describe()


# -- edit form --------------------------------------------------------------------------


def test_clearing_a_load_value_is_a_change_event(ops_spec: FormSpec) -> None:
    st = state(ops_spec, mode="edit", existing={"tier": "Silver", "code": "X-1"})
    assert "code" in st.evaluated.required
    st.unset("tier")  # tier becomes empty: "tier not Silver" matches again
    assert "code" not in st.evaluated.required and "tier" not in st.values


def test_edit_form_runs_edit_rules_on_existing_values(ops_spec: FormSpec) -> None:
    existing = {"subject": "Rotate keys", "tier": "Silver", "code": "X-1", "region": "US", "priority": "Low"}
    st = state(ops_spec, mode="edit", existing=existing)
    assert st.sources["tier"] == "existing" and "code" in st.evaluated.required
    assert "subject" in st.evaluated.disabled
    with pytest.raises(FieldDisabledError):
        st.set("subject", "Something else")
    d = st.set("tier", "Gold")  # an update --set runs the change event on top of the record
    assert "code" in d.no_longer_required


def test_lookup_answer_round_trip(ops_spec: FormSpec) -> None:
    st = state(ops_spec)
    class Resolver:
        def lookup(self, entity, query, field):
            assert query == "9000006001"
            return [LookupValue(9000006001, "Robin Ops", "robin.ops@example.com")]
    st.resolver = Resolver()
    st.set("owner", LookupValue(9000006001, "Robin Ops", "robin.ops@example.com"))
    assert build_change_payload(st, "robin.ops@example.com", allow_incomplete=True)["custom_fields"]["owner"] == (
        9000006001)
