"""Regenerate testdata/acme.json and testdata/py_golden.json from the Python
reference implementation (fsv.forms, fsv.formui.fuzzy, fsv.formui.when).

    cd <python fsv checkout> && uv run python <this file> <python fsv checkout>

acme.json holds the synthetic form metadata of the Python test suite
(tests/forms/acme_forms.py), frozen once so ids stay stable. py_golden.json
holds what the Python functions return for the cases below; the Rust tests
in src/*/py_tests.rs run the same cases and compare.
"""
from __future__ import annotations

import datetime as dt
import json
import os
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
PY_ROOT = Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()
sys.path.insert(0, str(PY_ROOT / "src"))
sys.path.insert(0, str(PY_ROOT / "tests" / "forms"))
sys.path.insert(0, str(HERE.parents[2] / "port" / "golden"))
from provenance import stamped  # noqa: E402
os.environ.pop("FSV_TZ", None)
os.environ["TZ"] = "UTC"

import acme_forms  # noqa: E402
from fsv.formui import fuzzy, when  # noqa: E402
from fsv.forms import errors, fetch, payload, plan, rules, spec, state, tz  # noqa: E402
from fsv.forms.rules import (  # noqa: E402
    ChoiceRestriction,
    Conflict,
    Decision,
    EvaluatedForm,
    ExcludeName,
    Message,
    RuleContext,
    RulesEngine,
    Target,
    _State,
)
from fsv.forms.spec import Choice, Field, FormSpec  # noqa: E402
from fsv.forms.state import FormState, LookupValue  # noqa: E402

LEGACY = {"populate": "set", "event_scope": "referenced", "event_reverse": "transition"}


# ---------------------------------------------------------------------------
# fixtures


def dump_acme() -> dict:
    data = {
        "change_fields": acme_forms.change_fields(),
        "change_rules": acme_forms.change_rules(),
        "service_item": acme_forms.service_item(),
        "ticket_fields": acme_forms.ticket_fields(),
        "ops_fields": acme_forms.ops_fields(),
        "ops_rules": acme_forms.ops_rules(),
        "restrict_rules": acme_forms.restrict_rules(),
        "ticket_internal_fields": acme_forms.ticket_internal_fields(),
        "ticket_rules": acme_forms.ticket_rules(),
        "business_rules_listing": acme_forms.business_rules_listing(),
    }
    path = HERE / "acme.json"
    if path.exists():  # keep the frozen ids, and their commit, unless they still reproduce
        frozen = json.loads(path.read_text())
        if {k: v for k, v in frozen.items() if k != "_python"} != json.loads(json.dumps(data)):
            return frozen
    path.write_text(json.dumps(stamped(data), indent=1, ensure_ascii=False) + "\n")
    return data


ACME = dump_acme()


class FakeResolver:
    """tests/forms/conftest.py FakeResolver."""

    def __init__(self) -> None:
        self.records = {
            "user": [
                LookupValue(9001, "Jane Doe", "jane.doe@acme.example"),
                LookupValue(9002, "Jane Doerr", "jane.doerr@acme.example"),
                LookupValue(9003, "Sam Lee", "sam.lee@acme.example"),
            ],
            "group": [LookupValue(601, "Platform Ops"), LookupValue(602, "Platform")],
            "agent": [LookupValue(701, "Alex Kim", "alex.kim@acme.example")],
            "change_window": [LookupValue(801, "Weekend window")],
        }

    def lookup(self, entity, query, field):
        q = query.casefold()
        return [r for r in self.records.get(entity, []) if q == str(r.id) or q in str(r).casefold()]


def change_spec() -> FormSpec:
    return FormSpec.from_form_fields(ACME["change_fields"], module="change")


def change_engine(s: FormSpec | None = None, **kw) -> RulesEngine:
    return RulesEngine(s or change_spec(), ACME["change_rules"], module="change", **{**LEGACY, **kw})


def change_state(**kw) -> FormState:
    s = change_spec()
    return FormState(s, change_engine(s), mode=kw.pop("mode", "new"), resolver=FakeResolver(), tz="UTC", **kw)


def ops_spec() -> FormSpec:
    return FormSpec.from_form_fields(ACME["ops_fields"], module="change")


def ops_engine(**kw) -> RulesEngine:
    return RulesEngine(ops_spec(), ACME["ops_rules"], module="change", workspace_id=2, **{**LEGACY, **kw})


def sr_spec() -> FormSpec:
    return FormSpec.from_service_item(ACME["service_item"])


def ticket_spec() -> FormSpec:
    return FormSpec.from_form_fields(ACME["ticket_fields"], module="ticket")


def ticket_internal_spec() -> FormSpec:
    return FormSpec.from_form_fields(ACME["ticket_internal_fields"], module="ticket")


# ---------------------------------------------------------------------------
# projections (mirrored in src/py_golden.rs)


def V(v):
    if isinstance(v, float) and v != v or v in (float("inf"), float("-inf")):
        return {"float": json.dumps(v)}
    if v is None or isinstance(v, (bool, int, float, str)):
        return v
    if isinstance(v, LookupValue):
        return {"lookup": [v.id, v.display, v.email]}
    if isinstance(v, dt.datetime):
        return {"datetime": v.isoformat()}
    if isinstance(v, dt.date):
        return {"date": v.isoformat()}
    if isinstance(v, (list, tuple)):
        return [V(x) for x in v]
    raise TypeError(type(v))


def VS(values):
    return {k: V(v) for k, v in values.items()}


def C(c: Choice | None):
    if c is None:
        return None
    return {"value": c.value, "label": c.label, "id": c.id, "display_id": c.display_id,
            "description": c.description, "visible": c.visible, "children": [C(x) for x in c.children]}


def F(f: Field | None):
    if f is None:
        return None
    return {
        "api_name": f.api_name, "label": f.label, "kind": f.kind, "choices": [C(c) for c in f.choices],
        "required_base": f.required_base, "position": f.position, "helptext": f.helptext, "help": f.help,
        "hint": f.hint, "placeholder": f.placeholder, "description": f.description, "default": V(f.default),
        "is_custom": f.is_custom, "field_type": f.field_type, "id": f.id, "multi": f.multi,
        "visible_base": f.visible_base, "on_form": f.on_form, "content": f.content, "parent": f.parent,
        "level": f.level, "levels": list(f.levels),
        "sections": [{"choices": list(s.choices), "fields": list(s.fields)} for s in f.sections],
        "date_only": f.date_only, "lookup_entity": f.lookup_entity, "lookup_link": f.lookup_link,
        "lookup_config": f.lookup_config, "allowed_file_types": list(f.allowed_file_types),
        "max_files": f.max_files, "sent": f.sent, "synthetic": f.synthetic, "is_input": f.is_input,
        "is_nested_level": f.is_nested_level,
    }


def SPEC(s: FormSpec):
    item = s.item
    return {
        "module": s.module, "account_id": s.account_id, "warnings": list(s.warnings),
        "fields": [F(f) for f in s.fields],
        "item": None if item is None else {
            "id": item.id, "display_id": item.display_id, "name": item.name,
            "short_description": item.short_description, "category_name": item.category_name,
            "description_html": item.description_html, "allow_attachments": item.allow_attachments,
            "attachment_mandatory": item.attachment_mandatory, "allow_quantity": item.allow_quantity,
            "subject_template": item.subject_template},
    }


def D(d: Decision | None):
    if d is None:
        return None
    return [d.rule_id, d.rule_name, d.action, d.matched, d.event, d.rule_index]


def M(m: Message):
    return [m.text, m.rule_id, m.rule_name, m.blocking]


def X(x: ExcludeName):
    return [x.base, x.custom]


def CR(r: ChoiceRestriction | None):
    if r is None:
        return None
    return {"allowed": {k: CR(v) for k, v in r.allowed.items()}, "rule_id": r.rule_id, "rule_name": r.rule_name}


def CONFLICT(c: Conflict):
    return {"field": c.field, "property": c.property, "decisions": [D(d) for d in c.decisions],
            "winner": D(c.winner), "describe": c.describe()}


def EXPL(e):
    return [e.field, e.label, e.property, e.value, e.source, D(e.decision), str(e)]


def EV(ev: EvaluatedForm):
    return {
        "visible": list(ev.visible), "required": sorted(ev.required), "disabled": sorted(ev.disabled),
        "values": VS(ev.values), "populated": dict(ev.populated), "messages": [M(m) for m in ev.messages],
        "matched": list(ev.matched), "model": ev.model,
        "excludes": {k: [X(x) for x in v] for k, v in ev.excludes.items()},
        "decisions": [[n, p, D(d)] for (n, p), d in ev.decisions.items()],
        "conflicts": [CONFLICT(c) for c in ev.conflicts],
        "refused": [[n, D(d)] for n, d in ev.refused],
        "overridden": {k: [V(a), b] for k, (a, b) in ev.overridden.items()},
        "attachments_required": ev.attachments_required, "account_id": ev.account_id,
        "restricted": {k: CR(v) for k, v in ev.restricted.items()},
        "restricted_by": {k: D(v) for k, v in ev.restricted_by.items()},
        "unavailable": {k: [V(a), D(b)] for k, (a, b) in ev.unavailable.items()},
        "populate_mode": ev.populate_mode,
    }


def RULE(r):
    def cond(g):
        return {"mode": g.mode, "items": [
            cond(c) if isinstance(c, rules.ConditionGroup)
            else {"name": c.name, "field": c.field, "operator": c.operator, "values": list(c.values),
                  "source": c.source}
            for c in g.items]}
    return {
        "id": r.id, "name": r.name, "condition": cond(r.condition),
        "actions": [{"property": a.property, "targets": list(a.targets), "values": list(a.values),
                     "all_targets": [[t.api, t.raw, t.base, t.custom] for t in a.all_targets]} for a in r.actions],
        "reversible": r.reversible, "form_type": r.form_type, "user_type": r.user_type,
        "is_validation": r.is_validation, "index": r.index, "fields": sorted(r.fields),
        "unconditional": r.unconditional,
    }


def ENGINE(e: RulesEngine):
    return {"rules": [RULE(r) for r in e.rules], "skipped": list(e.skipped), "duplicates": list(e.duplicates),
            "unsupported": list(e.unsupported), "warnings": list(e.warnings)}


ERR_NAMES = {
    "UnknownFieldError", "ValueParseError", "AmbiguousLookupError", "FieldDisabledError", "FormRuleError",
    "UnsupportedRuleError", "PayloadError", "ValueError", "TypeError", "FormError", "AttributeError", "KeyError",
}


def run(fn):
    try:
        return {"ok": fn()}
    except Exception as e:  # noqa: BLE001
        name = type(e).__name__
        if name not in ERR_NAMES:
            raise
        return {"err": [name, str(e)]}


def DIFF(d: state.Diff, s: FormSpec):
    return {"field": d.field, "appeared": d.appeared, "disappeared": d.disappeared,
            "became_required": d.became_required, "no_longer_required": d.no_longer_required,
            "messages_added": [M(m) for m in d.messages_added], "messages_removed": [M(m) for m in d.messages_removed],
            "populated": VS(d.populated), "cleared": d.cleared, "restricted": d.restricted,
            "unrestricted": d.unrestricted, "describe": d.describe(s)}


# ---------------------------------------------------------------------------
# sections


def gen_errors():
    lv = [LookupValue(i, f"Person {i}") for i in range(1, 12)] + [LookupValue(99, "")]
    cases = {
        "unknown_plain": errors.UnknownFieldError("Prio"),
        "unknown_suggest": errors.UnknownFieldError("pri", ["Priority", "Primary Contact"]),
        "parse": errors.ValueParseError("Team", "zz", "'zz' is not a valid choice", ["Platform", "Labs"]),
        "ambiguous": errors.AmbiguousLookupError("Owner", "per", lv),
        "ambiguous_blank": errors.AmbiguousLookupError("Owner", "x", [LookupValue(7, ""), LookupValue("a-1", "")]),
        "rule": errors.FormRuleError("rule 'X' is odd", 5, "X"),
        "rule_bare": errors.FormRuleError("odd"),
        "payload_empty": errors.PayloadError([]),
        "payload_two": errors.PayloadError(["Team is required", "Stage is required"]),
    }
    out = {}
    for k, e in cases.items():
        out[k] = {"message": str(e)}
        for attr in ("name", "suggestions", "field", "raw", "options", "query", "rule_id", "rule_name", "problems"):
            if hasattr(e, attr):
                out[k][attr] = getattr(e, attr)
        if hasattr(e, "candidates"):
            out[k]["candidates"] = len(e.candidates)
    return out


def gen_fetch():
    out = {}
    for ws in (2, "7", 0):
        q = fetch.business_rules_query("change", workspace_id=ws)
        out[str(ws)] = {"path": q.path, "url": q.url("https://acme.example.com/")}
    blank = [[v, fetch._blank(v), fetch._null(v)] for v in (None, "", "x", 0, 7, 2.5, True, False, [1], {"a": 1})]
    return {"queries": out, "blank": blank}


def gen_plan(tmp: Path):
    docs = {
        "nested": {"b": [1, 2.5, None, True, False], "a": "é\u2028</x>", "c": {"z": 1, "y": "ü\n\t\""}},
        "empty": {},
        "list": [3, "x", {"k": -0.5, "j": 1e20}],
        "unicode_keys": {"ß": 1, "a": 2, "Z": 3, "é": 4},
        "big": {"n": 12345678901234567890, "f": 0.1},
    }
    canonical = {k: plan.canonical_json(v).decode("utf-8") for k, v in docs.items()}
    files = {
        "a.txt": b"hello\n" * 1000,
        "empty.bin": b"",
        "big.bin": bytes(range(256)) * 300,
    }
    for name, data in files.items():
        (tmp / name).write_bytes(data)
    digests = {name: plan.file_digest(tmp / name) for name in files}
    groups = {"zeta": [str(tmp / "a.txt")], "alpha": [str(tmp / "big.bin"), str(tmp / "empty.bin")]}
    entries_fields = plan._file_entries(groups)
    entries_list = plan._file_entries([str(tmp / "empty.bin"), str(tmp / "a.txt")])
    for e in entries_fields + entries_list:
        e.pop("name_path", None)
    ts_cases = ["2026-01-02T03:04:05Z", " 2026-01-02 10:00 ", "2026-01-02T03:04:05.123456+07:00",
                "2026-01-02", "nope", "", "2026-13-01T00:00:00", "2026-01-02T03:04:05-05:30", "20260102T030405"]
    parse_ts = {s: (None if (t := plan._parse_ts(s)) is None else t.isoformat()) for s in ts_cases}
    walk_doc = {"form_fields": {"fields": [{"updated_at": "2026-01-02T03:04:05Z", "x": {"updated_at": "bad"}},
                                           {"updated_at": 5}, {"nested": [{"updated_at": "2025-06-01 00:00"}]}]},
                "rules": None, "item": {"updated_at": "2026-02-01T00:00:00+09:00"}}
    walk = [t.isoformat() for t in plan._walk_updated(walk_doc)]
    return {
        "canonical": {"docs": docs, "out": canonical},
        "digests": digests,
        "entries_fields": entries_fields,
        "entries_list": entries_list,
        "parse_ts": parse_ts,
        "walk_doc": walk_doc,
        "walk": walk,
    }


def gen_tz():
    out = {}
    sys_cases = {}
    for value in ("Asia/Tokyo", ":Europe/Paris", "Bangkok", "", "Not/AZone"):
        os.environ["TZ"] = value
        if value in ("", "Not/AZone"):
            real = os.path.realpath
            os.path.realpath = lambda p: "/usr/share/zoneinfo/Europe/Berlin"  # noqa: E731
            try:
                sys_cases[value] = tz.tz_name(tz.system_timezone())
            finally:
                os.path.realpath = real
        else:
            sys_cases[value] = tz.tz_name(tz.system_timezone())
    os.environ["TZ"] = "UTC"
    out["system_timezone"] = sys_cases
    res = {}
    for name, explicit, fsv_tz, user in (
        ("explicit", "Asia/Tokyo", "Europe/Paris", {"time_zone": "Bangkok"}),
        ("explicit_rails", "Eastern Time (US & Canada)", None, None),
        ("fsv_tz", None, "Europe/Paris", {"time_zone": "Bangkok"}),
        ("user", None, None, {"time_zone": "Bangkok"}),
        ("user_unknown", None, None, {"time_zone": "Atlantis"}),
        ("system", None, None, None),
        ("empty_explicit", "", "Europe/Paris", None),
        ("bad_explicit", "Mars/Base", None, None),
        ("bad_fsv_tz", None, "Mars/Base", None),
    ):
        if fsv_tz is None:
            os.environ.pop("FSV_TZ", None)
        else:
            os.environ["FSV_TZ"] = fsv_tz
        r = run(lambda: (lambda t: [tz.tz_name(t[0]), t[1]])(tz.resolve_timezone(explicit, user)))
        res[name] = {"explicit": explicit, "fsv_tz": fsv_tz, "user": user, "out": r}
    os.environ.pop("FSV_TZ", None)
    out["resolve_timezone"] = res
    names = {}
    for z in ("Asia/Bangkok", "Bangkok", "UTC", "Etc/UTC", "Hawaii"):
        names[z] = tz.tz_name(tz.zone(z))
    names["fixed+7"] = tz.tz_name(dt.timezone(dt.timedelta(hours=7)))
    names["fixed-5:30"] = tz.tz_name(dt.timezone(-dt.timedelta(hours=5, minutes=30)))
    names["fixed0"] = tz.tz_name(dt.timezone.utc)
    out["tz_name"] = names
    loc = {}
    for zname, wall in (("Asia/Bangkok", "2026-11-26 19:00"), ("America/New_York", "2026-03-08 02:30"),
                        ("America/New_York", "2026-11-01 01:30"), ("America/New_York", "2026-07-04 12:00"),
                        ("Europe/London", "2026-03-29 01:15"), ("Europe/London", "2026-10-25 01:59"),
                        ("Australia/Lord_Howe", "2026-04-05 01:45"), ("UTC", "2026-03-08 02:30")):
        naive = dt.datetime.strptime(wall, "%Y-%m-%d %H:%M")
        loc[f"{zname} {wall}"] = run(lambda: tz.localize(naive, tz.zone(zname)).isoformat())
    out["localize"] = loc
    offs = {}
    for secs in (0, 7 * 3600, -(5 * 3600 + 30 * 60), 5 * 3600 + 45 * 60, -12 * 3600, 14 * 3600, 30):
        d = dt.datetime(2026, 1, 1, tzinfo=dt.timezone(dt.timedelta(seconds=secs)))
        offs[str(secs)] = [tz._off(d), state._offset(d), when.offset_text(d)]
    out["offsets"] = offs
    return out


def gen_when():
    today = dt.date(2026, 11, 12)
    month = {w: when._month(w) for w in ("Jan", "january", "SEP", "Sept", "mar.", "xyz", "", "Ma", "DECEMBER", "ﬀeb")}
    pdate = {}
    for t in ("", "  ", "today", "NOW", "tomorrow", "yesterday", "+3d", "+ 2 w", "fri", "Friday", "thu 26 nov 2026",
              "Thu, 26 Nov 2026", "2026-11-26", "2026/1/5", "2026.02.30", "26 Nov 2026", "26 nov", "5 jan",
              "nov 26 2026", "Nov 26", "jan 5", "26 Foo 2026", "garbage", "mon tue", "12", "+0d", "31 feb 2027"):
        pdate[t] = run(lambda: (lambda d: None if d is None else d.isoformat())(when._parse_date(t, today)))
    split = {}
    for t in ("2026-11-26 19:00", "19:00", "7pm", "7 PM", "at 9am", "2026-11-26T19:00:30+07:00", "x 10:00Z",
              "2026-11-26 19:00 -0530", "2026-11-26", "13pm", "0am", "25:00", "10:61", "12am", "12pm", "tomorrow 9",
              "2026-11-26 19:00 +99:00", "y 1:02:03"):
        def go(t=t):
            date_part, time, off = when._split_time(t)
            return [date_part, None if time is None else time.isoformat(),
                    None if off is None else int(off.utcoffset(None).total_seconds())]
        split[t] = run(go)
    phrases = {s: when.tz_phrase(s) for s in ("Freshservice user time zone (Bangkok)", "FSV_TZ", "tz argument",
                                               "system time zone", "", "fsv_tz")}
    ok = {}
    bkk = tz.zone("Asia/Bangkok")
    for t in ("2026-11-26 19:00", "", "nonsense", "2026-11-26"):
        p = when.parse_when(t, tz=bkk, tz_source="FSV_TZ", today=today)
        ok[t] = [p.ok, p.error]
    return {"month": month, "parse_date": pdate, "split_time": split, "tz_phrase": phrases, "parsed_ok": ok}


def gen_fuzzy():
    cases = [("stg", "staging"), ("", "abc"), ("abc", ""), ("ac", "abc"), ("ca", "abc"), ("aa", "a"), ("aa", "aba"),
             ("é", "e"), ("xyz", "xxyyzz"), ("abc", "abc"), ("bd", "abcd")]
    return {f"{a}|{b}": fuzzy._subsequence(a, b) for a, b in cases}


def malformed_cases():
    base = {"name": "f", "field_type": "custom_text", "label": "F", "position": 1}
    dd = {"name": "d", "field_type": "custom_dropdown", "label": "D", "position": 2,
          "choices": [{"id": 1, "value": "A"}, {"id": 2, "value": "B"}]}
    nest = {"name": "n", "field_type": "nested_field", "label": "N", "position": 3}

    def with_(f, **kw):
        return {**f, **kw}

    cases = [
        ("data_none", None), ("data_int", 5), ("data_str", "fields"), ("data_empty_list", []),
        ("data_empty_dict", {}), ("data_fields_str", {"fields": "abc"}), ("data_fields_dict", {"fields": {"a": 1}}),
        ("data_guess_nonmap_first", {"x": [5]}), ("data_guess_empty", {"x": []}),
        ("data_guess_str_first", {"x": ["field_type"]}),
        ("entry_int", [5]), ("entry_str", ["abc"]), ("entry_none", [None]), ("entry_list", [[1]]),
        ("entry_empty_list", [[]]), ("entry_bool", [True]), ("entry_float", [1.5]),
        ("entry_after_ok", [base, 7]), ("entry_empty_dict", [{}]),
        ("ftype_int", [with_(base, field_type=5)]), ("ftype_none", [with_(base, field_type=None)]),
        ("ftype_list", [with_(base, field_type=["custom_text"])]),
        ("name_int", [with_(base, name=7)]), ("name_none", [with_(base, name=None)]),
        ("name_list", [with_(base, name=[1])]), ("name_dict_empty", [with_(base, name={})]),
        ("label_int", [with_(base, label=5)]), ("label_none", [with_(base, label=None)]),
        ("label_list", [with_(base, label=[1, 2])]),
        ("default_field_str", [with_(base, default_field="no")]),
        ("lookup_cfg_str", [with_(base, lookup_config="abc")]),
        ("lookup_cfg_empty_str", [with_(base, lookup_config="")]),
        ("lookup_cfg_list", [with_(base, lookup_config=[1])]),
        ("lookup_cfg_empty_list", [with_(base, lookup_config=[])]),
        ("lookup_cfg_int", [with_(base, lookup_config=5)]), ("lookup_cfg_zero", [with_(base, lookup_config=0)]),
        ("lookup_cfg_true", [with_(base, lookup_config=True)]),
        ("lookup_cfg_link_int", [with_(base, lookup_config={"link": 5})]),
        ("lookup_cfg_link_list", [with_(base, lookup_config={"link": ["x"]})]),
        ("lookup_cfg_link_empty", [with_(base, lookup_config={"link": ""})]),
        ("lookup_cfg_link_zero", [with_(base, lookup_config={"link": 0})]),
        ("lookup_src_other_cfg_str", [with_(base, field_type="custom_lookup", lookup_config="abc")]),
        ("lookup_src_list", [with_(base, field_type="custom_lookup", lookup_config={"data_source": [1], "link": "x"})]),
        ("lookup_src_str", [with_(base, field_type="custom_lookup", data_source="9", lookup_config={"link": "x"})]),
        ("lookup_src_float", [with_(base, field_type="custom_lookup", data_source=1.5)]),
        ("lookup_src_user_float", [with_(base, field_type="custom_lookup", data_source=1.0)]),
        ("lookup_src_bool", [with_(base, field_type="custom_lookup", data_source=True)]),
        ("choices_str", [with_(dd, choices="abc")]), ("choices_int", [with_(dd, choices=5)]),
        ("choices_true", [with_(dd, choices=True)]), ("choices_dict", [with_(dd, choices={"a": 1, "b": 2})]),
        ("choices_zero", [with_(dd, choices=0)]), ("choices_float", [with_(dd, choices=1.5)]),
        ("choices_empty_str", [with_(dd, choices="")]), ("choices_none", [with_(dd, choices=None)]),
        ("choices_nested_options_int", [with_(dd, choices=[{"value": "A", "nested_options": 5}])]),
        ("choices_nested_options_str", [with_(dd, choices=[{"value": "A", "nested_options": "xy"}])]),
        ("choices_nested_options_dict", [with_(dd, choices=[{"value": "A", "nested_options": {"k": 1}}])]),
        ("choices_nested_options_true", [with_(dd, choices=[{"value": "A", "nested_options": True}])]),
        ("choices_entry_none", [with_(dd, choices=[None])]),
        ("choices_entry_empty_list", [with_(dd, choices=[[]])]),
        ("choices_entry_list_id", [with_(dd, choices=[["A", 3], ["B"]])]),
        ("choices_entry_bool", [with_(dd, choices=[True, 1.5])]),
        ("choices_entry_nested_list", [with_(dd, choices=[[["x"]]])]),
        ("choice_value_list", [with_(dd, choices=[{"value": ["a"], "label": {"x": 1}}])]),
        ("choice_desc_int", [with_(dd, choices=[{"value": "A", "description": 5}])]),
        ("choice_desc_list", [with_(dd, choices=[{"value": "A", "description": [1]}])]),
        ("choice_visible_str", [with_(dd, choices=[{"value": "A", "visible": "no"}])]),
        ("helptext_int", [with_(dd, helptext=5)]), ("helptext_list", [with_(dd, helptext=["A - x"])]),
        ("helptext_bars", [with_(dd, helptext="A - first || B - second")]),
        ("position_str", [with_(base, position="3")]), ("position_str_mixed", [base, with_(dd, position="3")]),
        ("position_list", [with_(base, position=[1])]), ("position_bool", [with_(base, position=True)]),
        ("position_float", [with_(base, position=1.5)]), ("position_none", [with_(base, position=None)]),
        ("position_dict", [with_(base, position={})]),
        ("required_str", [with_(base, required="no")]), ("required_list", [with_(base, required=[])]),
        ("hint_str", [with_(base, hint="yes")]),
        ("placeholder_int", [with_(base, placeholder=5)]), ("placeholder_list", [with_(base, placeholder=[1])]),
        ("description_int", [with_(base, description=5)]), ("description_none", [with_(base, description=None)]),
        ("default_value_int", [with_(dd, default_value=1)]), ("default_value_no_match", [with_(dd, default_value="Z")]),
        ("default_value_list", [with_(dd, default_value=["A"])]),
        ("default_value_dict", [with_(dd, default_value={"a": 1})]),
        ("default_value_text_list", [with_(base, default_value=[1, 2])]),
        ("default_value_text_float", [with_(base, default_value=1.5)]),
        ("default_value_text_bool", [with_(base, default_value=True)]),
        ("default_str", [with_(dd, default="a")]),
        ("id_str", [with_(base, id="7")]), ("id_list", [with_(base, id=[1])]),
        ("displayed_str", [with_(base, displayed_to_agents="false")]),
        ("displayed_none", [with_(base, displayed_to_agents=None)]),
        ("date_only_str", [with_(base, field_type="custom_date_time", date_only="TRUE")]),
        ("date_only_int", [with_(base, field_type="custom_date_time", date_only=1)]),
        ("content_int", [with_(base, field_type="custom_content", content=5)]),
        ("content_field_options_list", [with_(base, field_type="custom_content", field_options=[1])]),
        ("content_field_options_str", [with_(base, field_type="custom_content", field_options="x")]),
        ("content_field_options_content_int", [with_(base, field_type="custom_content", field_options={"content": 5})]),
        ("content_html", [with_(base, field_type="custom_content", field_options={"content_html": "<p>x</p>"})]),
        ("nested_str", [with_(nest, nested_fields="abc")]), ("nested_int", [with_(nest, nested_fields=5)]),
        ("nested_true", [with_(nest, nested_fields=True)]), ("nested_float", [with_(nest, nested_fields=1.5)]),
        ("nested_zero", [with_(nest, nested_fields=0)]), ("nested_empty_str", [with_(nest, nested_fields="")]),
        ("nested_none", [with_(nest, nested_fields=None)]), ("nested_empty_dict", [with_(nest, nested_fields={})]),
        ("nested_dict", [with_(nest, nested_fields={"a": {"name": "x"}})]),
        ("nested_dict_empty_key", [with_(nest, nested_fields={"": 1})]),
        ("nested_entry_int", [with_(nest, nested_fields=[5])]),
        ("nested_entry_str", [with_(nest, nested_fields=["x"])]),
        ("nested_entry_none", [with_(nest, nested_fields=[None])]),
        ("nested_entry_list", [with_(nest, nested_fields=[[1]])]),
        ("nested_entry_single_int", [with_(nest, nested_fields=[7])]),
        ("nested_entry_second_bad", [with_(nest, nested_fields=[{"name": "a", "level": 2}, 5])]),
        ("nested_ok", [with_(nest, nested_fields=[{"name": "b", "level": 3, "label": "B"}, {"name": "a", "level": 2}])]),
        ("nested_level_str", [with_(nest, nested_fields=[{"name": "a", "level": "3"}])]),
        ("nested_level_str_two", [with_(nest, nested_fields=[{"name": "a", "level": "3"}, {"name": "b", "level": "2"}])]),
        ("nested_level_str_mixed", [with_(nest, nested_fields=[{"name": "a", "level": "3"}, {"name": "b", "level": 2}])]),
        ("nested_level_bad_str", [with_(nest, nested_fields=[{"name": "a", "level": "x"}])]),
        ("nested_level_bad_str_two", [with_(nest, nested_fields=[{"name": "a", "level": "x"}, {"name": "b", "level": "y"}])]),
        ("nested_level_float", [with_(nest, nested_fields=[{"name": "a", "level": 2.7}])]),
        ("nested_level_float_sort", [with_(nest, nested_fields=[{"name": "a", "level": 3.5}, {"name": "b", "level": 2.5}])]),
        ("nested_level_list", [with_(nest, nested_fields=[{"name": "a", "level": [1]}])]),
        ("nested_level_list_two", [with_(nest, nested_fields=[{"name": "a", "level": [1]}, {"name": "b", "level": [0]}])]),
        ("nested_level_empty_list", [with_(nest, nested_fields=[{"name": "a", "level": []}])]),
        ("nested_level_none", [with_(nest, nested_fields=[{"name": "a", "level": None}])]),
        ("nested_level_zero", [with_(nest, nested_fields=[{"name": "a", "level": 0}])]),
        ("nested_level_bool", [with_(nest, nested_fields=[{"name": "a", "level": True}])]),
        ("nested_level_dict", [with_(nest, nested_fields=[{"name": "a", "level": {"a": 1}}])]),
        ("nested_level_missing", [with_(nest, nested_fields=[{"name": "a"}, {"name": "b"}])]),
        ("nested_level_neg_str", [with_(nest, nested_fields=[{"name": "a", "level": " 4 "}])]),
        ("nested_name_none", [with_(nest, nested_fields=[{"level": 2}])]),
        ("nested_name_int", [with_(nest, nested_fields=[{"name": 5, "level": 2}])]),
        ("nested_name_list", [with_(nest, nested_fields=[{"name": [1], "level": 2}])]),
        ("nested_label_int", [with_(nest, nested_fields=[{"name": "a", "label": 5}])]),
        ("nested_id_str", [with_(nest, nested_fields=[{"name": "a", "id": "9"}])]),
        ("nested_placeholder_int", [with_(nest, nested_fields=[{"name": "a", "placeholder": 5}])]),
        ("nested_description_list", [with_(nest, nested_fields=[{"name": "a", "description": [1]}])]),
        ("sections_str", [with_(dd, sections="abc")]), ("sections_int", [with_(dd, sections=5)]),
        ("sections_true", [with_(dd, sections=True)]), ("sections_dict", [with_(dd, sections={"a": 1})]),
        ("sections_empty_dict", [with_(dd, sections={})]), ("sections_zero", [with_(dd, sections=0)]),
        ("sections_entry_int", [with_(dd, sections=[5])]), ("sections_entry_str", [with_(dd, sections=["x"])]),
        ("sections_entry_none", [with_(dd, sections=[None])]), ("sections_entry_list", [with_(dd, sections=[[1]])]),
        ("sections_ok", [with_(dd, sections=[{"choice_ids": [1], "ticket_field_ids": [10]}]),
                         with_(base, id=10)]),
        ("sections_ids_int", [with_(dd, sections=[{"choice_ids": [1], "ticket_field_ids": 5}])]),
        ("sections_ids_str", [with_(dd, sections=[{"choice_ids": [1], "ticket_field_ids": "10"}]), with_(base, id=1)]),
        ("sections_ids_dict", [with_(dd, sections=[{"choice_ids": [1], "ticket_field_ids": {"10": 1}}]), with_(base, id=10)]),
        ("sections_ids_true", [with_(dd, sections=[{"choice_ids": [1], "ticket_field_ids": True}])]),
        ("sections_ids_zero", [with_(dd, sections=[{"choice_ids": [1], "ticket_field_ids": 0, "field_ids": [10]}]),
                               with_(base, id=10)]),
        ("sections_field_ids", [with_(dd, sections=[{"choice_ids": [1], "field_ids": [10]}]), with_(base, id=10)]),
        ("sections_change_field_ids", [with_(dd, sections=[{"choice_ids": [2], "change_field_ids": [10]}]),
                                       with_(base, id=10)]),
        ("sections_choice_ids_int", [with_(dd, sections=[{"choice_ids": 1, "ticket_field_ids": []}])]),
        ("sections_choice_ids_str", [with_(dd, sections=[{"choice_ids": "12", "ticket_field_ids": []}])]),
        ("sections_choice_ids_true", [with_(dd, sections=[{"choice_ids": True}])]),
        ("sections_choice_ids_dict", [with_(dd, sections=[{"choice_ids": {"1": 1}}])]),
        ("sections_choice_ids_none", [with_(dd, sections=[{"choice_ids": None}])]),
        ("sections_id_types", [with_(dd, sections=[{"choice_ids": ["1", 2.0, None], "ticket_field_ids": ["10", 11.0]}]),
                               with_(base, id=10), with_(nest, id=11)]),
        ("sections_unknown_ctrl_bad", [with_(dd, name=None, sections=[5])]),
        ("sections_missing_ctrl_str", [{"name": "", "field_type": "custom_dropdown", "sections": "abc"}]),
        ("belongs_str", [with_(base, belongs_to_section="no")]),
        ("belongs_ok", [with_(base, belongs_to_section=True)]),
        ("belongs_list", [with_(base, belongs_to_section=[1])]),
        ("belongs_dup_name", [with_(base, belongs_to_section=True), with_(base, belongs_to_section=True)]),
        ("dup_names", [base, base]),
        ("dup_names_diff_kind", [base, with_(dd, name="f")]),
        ("dup_ids", [with_(base, id=1), with_(dd, id=1)]),
        ("id_list_dup", [with_(base, id=[1]), with_(dd, id=[1])]),
        ("account_link_int", [with_(base, lookup_config={"link": 5})]),
        ("account_link_ok", [with_(base, lookup_config={"link": "field_name=f_77"})]),
        ("account_name_int", [with_(base, name=5, lookup_config={"link": "field_name=5_77"})]),
        ("account_cfg_list", [with_(base, lookup_config=["field_name=f_77"])]),
        ("requester_pos_str", [{"name": "requester", "field_type": "default_requester", "position": "1"}]),
        ("requester_pos_float", [{"name": "requester", "field_type": "default_requester", "position": 1.5}]),
        ("requester_pos_none", [{"name": "requester", "field_type": "default_requester"}]),
        ("requester_pos_bool", [{"name": "requester", "field_type": "default_requester", "position": True}]),
        ("requester_pos_list", [{"name": "requester", "field_type": "default_requester", "position": [1]}]),
    ]
    return cases


def gen_malformed():
    out = []
    for name, data in malformed_cases():
        for module in ("change",) if not name.startswith("requester") else ("change", "ticket"):
            r = run(lambda: SPEC(FormSpec.from_form_fields(data, module=module)))
            row = {"name": name, "module": module, "data": data, "out": r}
            if "ok" in r:
                # what any engine/state use of the spec does (Python builds
                # the spec first and fails here)
                sp = FormSpec.from_form_fields(data, module=module)
                row["engine"] = run(lambda: RulesEngine(sp, {"business_rules": []}, module=module) and None)
            out.append(row)
    return out


def gen_spec():
    out: dict = {}
    out["is_content_type"] = {t: spec.is_content_type(t) for t in (
        "custom_content", "CUSTOM_STATIC_TEXT", "nested_content", "content_block", "static", "my_static_field",
        "contented", "custom_text", "", "staticx", "x_content", "default_content_html")}
    out["is_planning_type"] = {t: spec.is_planning_type(t) for t in (
        "planning_field", "default_change_reason", "default_backout_plan", "default_change_plan", "custom_text", "",
        "Planning_Field")}
    out["camel_to_snake"] = {t: spec.camel_to_snake(t) for t in (
        "plannedStartDate", "already_snake", "HTTPServer", "A", "aB", "x1Y2", "requesterId", "", "lower")}
    out["plain_text"] = {t: spec.plain_text(t) for t in (
        "", "plain", "&lt;p&gt;Read the &lt;b&gt;guide&lt;/b&gt; first.&lt;/p&gt;", "<p>a</p><p>b</p>",
        "line1<br>line2<BR/>line3", "<div>x&nbsp;y</div>\n\n  <li>z</li>", "a &amp;lt; b", "  spaced   out  ",
        "<span class=\"btn-link\">Upload files</span> or drop here", "x\r\ny\u2028z", "&#60;b&#62;bold&#60;/b&#62;")}
    raw_choices = [
        {"id": 1, "value": "Low", "label": "Low &amp; slow", "description": "  very\n slow  "},
        {"value": None, "label": "LabelOnly"},
        {"value": "ValueOnly"},
        {"id": 7, "display_id": 70, "value": "Hidden", "visible": False},
        {"id": 8, "value": "Tree", "nested_options": [{"id": 9, "value": "Leaf", "nested_options": None}]},
        ["Pair", 42],
        ["Solo"],
        "bare",
        5,
    ]
    out["parse_choices"] = {"raw": raw_choices, "out": [C(c) for c in spec._parse_choices(raw_choices)],
                            "none": [C(c) for c in spec._parse_choices(None)]}
    base = spec._parse_choices([{"value": "Very High"}, {"value": "High"}, {"value": "Low", "description": "kept"},
                                {"value": "Medium"}, {"value": ""}])
    sch = {}
    for name, ht in (("none", ""), ("no_bars", "  just help  "),
                     ("bars", "High - Revenue || Very High - Outage  ||  Low - nice || general words || Medium-ish"),
                     ("dash_spaces", "Medium   -   Planned work || high-lower"),
                     ("dupes", "High - first || HIGH - second || || trailing")):
        chs, rest = spec.split_choice_help(base, ht)
        sch[name] = {"helptext": ht, "choices": [C(c) for c in chs], "rest": rest,
                     "attach": [C(c) for c in spec.attach_choice_descriptions(base, ht)]}
    out["split_choice_help"] = {"base": [C(c) for c in base], "cases": sch}
    keys = {}
    for c in spec._parse_choices([{"id": 1, "display_id": 11, "value": "Low", "label": "Low  Prio"},
                                  {"value": "Only"}, {"id": "X", "value": "X"}]):
        keys[c.value] = sorted(c.keys())
    out["choice_keys"] = keys
    content = {}
    for name, f in (("content", {"content": "<p>A</p>", "description": "d"}),
                    ("opts", {"content": "  ", "field_options": {"content": "<b>B</b>"}}),
                    ("opts_html", {"field_options": {"content_html": "<i>C</i>"}}),
                    ("description", {"field_options": "bad", "description": "desc"}),
                    ("helptext", {"helptext": "help"}),
                    ("none", {"content": 5}),
                    ("empty", {})):
        content[name] = {"in": f, "out": spec._content_of(f)}
    out["content_of"] = content
    out["opt_true"] = [[o, k, spec._opt_true(o, k)] for o, k in (
        ({"a": "true"}, "a"), ({"a": "TRUE"}, "a"), ({"a": True}, "a"), ({"a": "false"}, "a"), ({}, "a"),
        ({"a": None}, "a"), ({"a": 1}, "a"))]
    unwrap = {}
    for name, d in (("list", [{"name": "a", "field_type": "custom_text"}]),
                    ("change_fields", {"change_fields": [{"name": "b"}], "fields": [{"name": "c"}]}),
                    ("fields", {"meta": 1, "fields": [{"name": "c"}]}),
                    ("form_fields", {"form_fields": []}),
                    ("guess", {"weird": [{"field_type": "custom_text", "name": "d"}]}),
                    ("guess_miss", {"weird": [{"name": "d"}]}),
                    ("scalar", 5)):
        unwrap[name] = {"in": d, "out": run(lambda: spec._unwrap_fields(d))}
    out["unwrap_fields"] = unwrap
    out["malformed"] = gen_malformed()
    acc = {}
    for name, raw in (
        ("one", [{"name": "owner", "lookup_config": {"link": "/lookup_choices?field_name=owner_900000777&module=change"}}]),
        ("mismatch_name", [{"name": "other", "lookup_config": {"link": "/x?field_name=owner_900000777"}}]),
        ("two", [{"name": "a", "lookup_config": {"link": "?field_name=a_1&x"}},
                 {"name": "b", "lookup_config": {"link": "?field_name=b_2"}}]),
        ("same_twice", [{"name": "a", "lookup_config": {"link": "?field_name=a_1"}},
                        {"name": "b", "lookup_config": {"link": "?field_name=b_1&m=1"}}]),
        ("none", [{"name": "a"}, {"name": "b", "lookup_config": None}, {"name": "c", "lookup_config": "str"}]),
        ("underscored", [{"name": "my_field", "lookup_config": {"link": "?field_name=my_field_42"}}]),
    ):
        acc[name] = {"in": raw, "out": spec._account_from_form_fields(raw)}
    out["account_from_form_fields"] = acc
    acc_item = {}
    for name, si in (
        ("top", {"account_id": 900, "custom_fields": []}),
        ("fields", {"custom_fields": [{"account_id": "901", "fields": [{"account_id": 901}]}]}),
        ("conflict", {"account_id": 1, "custom_fields": [{"account_id": 2}]}),
        ("blank", {"account_id": "", "custom_fields": [{"account_id": None}]}),
        ("none", {}),
    ):
        acc_item[name] = {"in": si, "out": spec._account_from_item(si)}
    out["account_from_item"] = acc_item
    fff = {}
    for name, raw in (
        ("lookup_other_source", {"name": "ref", "label": "Ref", "field_type": "custom_lookup", "position": 1,
                                 "lookup_config": {"data_source": 99, "link": "/l?field_name=ref_5"}}),
        ("lookup_other_nolink", {"name": "ref", "field_type": "custom_lookup", "position": 1,
                                 "lookup_config": {"data_source": 99}}),
        ("lookup_user_top_source", {"name": "u", "field_type": "custom_multi_lookup", "position": 1, "data_source": 5}),
        ("date_only_str", {"name": "d", "field_type": "custom_date", "position": 2, "date_only": "True"}),
        ("explicit_default", {"name": "p", "field_type": "default_priority", "position": 3, "default_value": "2",
                              "choices": [{"id": 1, "value": "Low"}, {"id": 2, "value": "High"}]}),
        ("bad_default", {"name": "p", "field_type": "custom_dropdown", "position": 3, "default": "Nope",
                         "choices": [{"id": 1, "value": "Low"}]}),
        ("text_default", {"name": "t", "field_type": "custom_text", "position": 4, "default": "hi"}),
        ("content", {"name": "c", "field_type": "custom_static_rich_text", "position": 5,
                     "field_options": {"content": "&lt;b&gt;x&lt;/b&gt;"}}),
        ("unsupported", {"name": "m", "field_type": "custom_hologram", "position": 6}),
        ("planning", {"name": "pl", "label": "Plan &amp; more", "field_type": "planning_field", "position": 7}),
        ("hidden", {"name": "h", "field_type": "custom_text", "position": None, "displayed_to_agents": False,
                    "required": None, "required_for_agents": True, "hint": 1, "helptext": True}),
        ("nested", {"name": "n", "field_type": "nested_field", "position": 8, "default_field": False,
                    "nested_fields": [{"name": "n3", "level": 3, "id": 3}, {"name": "n2", "level": 2, "label": "L2",
                                                                              "placeholder": "<b>pick</b>"}]}),
        ("requested_for", {"name": "requested_for", "field_type": "default_requested_for", "position": 2}),
    ):
        w: list = []
        fff[name] = {"in": raw, "out": run(lambda: [F(f) for f in spec._fields_from_form_field(raw, w)]),
                     "warnings": w}
    out["fields_from_form_field"] = fff
    arf = {}
    for name, fields in (
        ("adds", [Field("requester", "Requester", "lookup_user", position=2, field_type="default_requester")]),
        ("no_pos", [Field("requester", "Requester", "lookup_user", position=None, field_type="default_requester",
                          visible_base=False)]),
        ("present", [Field("requester", "R", "lookup_user", position=2, field_type="default_requester"),
                     Field("requested_for", "For", "lookup_user", position=3)]),
        ("no_requester", [Field("subject", "Subject", "text", position=1)]),
    ):
        spec._add_requested_for(fields)
        arf[name] = [F(f) for f in fields]
    out["add_requested_for"] = arf
    sr = {}
    for name, c in (
        ("unsupported", {"id": 1, "name": "u", "type": 9999, "label": "U"}),
        ("str_type", {"id": 2, "type": "1001", "label": "No name"}),
        ("datetime", {"id": 3, "name": "d", "type": 1007, "field_options": {"date_only": "false"}}),
        ("date", {"id": 4, "name": "d", "type": 1007, "field_options": {"date_only": "TRUE"}}),
        ("date_missing", {"id": 5, "name": "d", "type": 1007, "field_options": {}}),
        ("notice", {"id": 6, "name": "n", "type": 1024, "label": "&lt;b&gt;Hi&lt;/b&gt;", "field_options": {}}),
        ("lookup", {"id": 7, "name": "l", "type": 502, "label": "L", "field_options": {"x": 1}, "required": True}),
        ("choices", {"id": 8, "name": "c", "type": 1003, "label": "C", "helptext": "B - bee || A - ay",
                     "choices": [{"id": "b", "value": "B", "position": 2, "description": " x  y "},
                                 {"id": "a", "value": "A", "label": "Ay", "position": 1, "dependent_ids": {"field": [11]}}]}),
        ("section_info", {"id": 9, "name": "s", "type": 1003, "field_options": {
            "section_info": [{"sectionFields": [11], "sectionChoices": ["A"]}]},
            "choices": [{"value": "A", "dependent_ids": {"field": [11]}}]}),
        ("section_mismatch", {"id": 10, "name": "s", "type": 1003, "field_options": {
            "section_info": [{"sectionFields": [12], "sectionChoices": ["A"]}]},
            "choices": [{"value": "A", "dependent_ids": {"field": [11]}}]}),
        ("section_bad", {"id": 11, "name": "s", "type": 1003, "field_options": {"section_info": "{not json"}}),
        ("files", {"id": 12, "name": "f", "type": 1014, "hint": True, "visible": False, "placeholder": "&lt;i&gt;up&lt;/i&gt;",
                   "field_options": {"allowed_file_types": "[\".pdf\", \"png\"]", "max_attachment_limit": "3"}}),
        ("files_bad", {"id": 13, "name": "f", "type": 1014,
                       "field_options": {"allowed_file_types": "nope", "max_attachment_limit": "x"}}),
        ("files_list", {"id": 14, "name": "f", "type": 1014, "field_options": {"allowed_file_types": [".doc"]}}),
    ):
        w = []
        def go(c=c, w=w):
            f, secs = spec._field_from_sr(c, "parent_x", w)
            return {"field": F(f), "sections": secs}
        sr[name] = {"in": c, "out": run(go), "warnings": w}
    out["field_from_sr"] = sr

    specs = {"change": change_spec(), "ticket": ticket_spec(), "ticket_internal": ticket_internal_spec(),
             "ops": ops_spec(), "sr": sr_spec()}
    out["specs"] = {k: SPEC(s) for k, s in specs.items()}
    out["ordered"] = {k: [f.api_name for f in s.ordered()] for k, s in specs.items()}
    out["inputs"] = {k: [f.api_name for f in s.inputs()] for k, s in specs.items()}
    out["section_parent"] = {k: {f.api_name: (lambda p: None if p is None else [p[0].api_name, list(p[1])])(
        s.section_parent(f.api_name)) for f in s.fields} for k, s in specs.items()}
    names = ["priority", "Priority", "customFields.team", "customField.sr_team", "custom_fields.team", "plannedStartDate",
             "department_id", "responder_id", "agent_id", "requester_id", "change_window_id", "type", "description_html",
             "group_id", "cf_title_100", "customField.cf_title_100", "title_100", "sr_title_100", "Team", "team",
             "Planned Start Date", "  planned   start date ", "nope", "", "subCategory", "sub_category_id", "Status",
             "Needed by", "1. expected benefit", "Request for", "category"]
    out["find"] = {k: {n: (lambda f: None if f is None else f.api_name)(s.find(n)) for n in names}
                   for k, s in specs.items()}
    out["resolve_name"] = {k: {n: s.resolve_name(n) for n in names} for k, s in specs.items()}
    out["field"] = {k: {n: run(lambda: s.field(n).api_name) for n in ("Plan", "start", "zzz", "")}
                    for k, s in specs.items()}
    cs = specs["change"]
    cf = {}
    for vals in ({}, {"category": "Hardware"}, {"category": "hardware", "sub_category": "Laptop"},
                 {"category": "Software"}, {"category": "Nope"}, {"category": "", "sub_category": "Laptop"},
                 {"category": 2101, "sub_category": 2102}):
        key = json.dumps(vals, sort_keys=True)
        cf[key] = {n: [c.value for c in cs.choices_for(cs.by_name[n], vals)]
                   for n in ("category", "sub_category", "item_category", "team")}
    out["choices_for"] = cf
    chf = {}
    for sname, fname, raws in (
        ("change", "priority", ["low", " HIGH ", "3", 4, "urgent", "nope", ""]),
        ("change", "team", ["platform", "11", 1102, "13", "x"]),
        ("ops", "tier", ["gold", "9000003001", "9000002002", "9000003003", 9000003002]),
        ("change", "change_type", ["major", "MAJOR", "3"]),
    ):
        f = specs[sname].by_name[fname]
        chf[f"{sname}.{fname}"] = {json.dumps(r): run(lambda: (lambda c: None if c is None else c.value)(f.choice(r)))
                                   for r in raws}
    out["choice"] = chf
    dids = {}
    for sname, fname, raws in (("change", "team", [11, "12", "1101", "x"]),
                               ("ops", "tier", [9000003001, "9000003003", 9000002001]),
                               ("change", "category", [1, "5", 2])):
        f = specs[sname].by_name[fname]
        dids[f"{sname}.{fname}"] = {json.dumps(r): (lambda c: None if c is None else c.value)(f.choice_by_display_id(r))
                                    for r in raws}
    out["choice_by_display_id"] = dids
    item = specs["sr"].item
    subj = {}
    for vals in ({"title_100": "Dark mode"}, {"cf_title_100": "Direct"}, {"title_100": ""}, {},
                 {"title_100": 42}, {"cf_title_100": None, "title_100": "Fallback"}):
        subj[json.dumps(vals, sort_keys=True)] = item.render_subject(vals)
    tmpl = spec.ServiceItem(1, 2, "x", subject_template="  {{ custom_field.cf_a }} - {{custom_field.b}} {{custom_field.cf_}} ")
    subj["template2"] = tmpl.render_subject({"a": "A", "b": "B"})
    out["render_subject"] = subj
    sri = {}
    base_item = ACME["service_item"]
    variants = {
        "bare_item": {"id": 5, "display_id": 6, "name": None, "custom_fields": None, "configs": None},
        "unwrapped": base_item["service_item"],
        "unknown_child": {"custom_fields": [{"id": "p", "name": "p", "type": 1003, "field_options": {
            "section_info": [{"sectionFields": ["ghost"], "sectionChoices": ["A"]}]}, "choices": [{"value": "A"}]}]},
        "non_child": {"custom_fields": [
            {"id": "p", "name": "p", "type": 1003, "field_options": {
                "section_info": [{"sectionFields": ["q"], "sectionChoices": ["A"]}]}, "choices": [{"value": "A"}]},
            {"id": "q", "name": "q", "type": 1001}]},
        "orphan_child": {"attachment_mandatory": False, "configs": {"attachment_mandatory": True},
                         "allow_attachments": 0, "custom_fields": [
            {"id": "p", "name": "p", "type": 1003, "choices": [{"value": "A"}],
             "fields": [{"id": "k", "name": "k", "type": 1001}]}]},
        "cfg_mandatory": {"configs": {"attachment_mandatory": 1}, "custom_fields": []},
    }
    for name, data in variants.items():
        sri[name] = {"in": data, "out": run(lambda: SPEC(FormSpec.from_service_item(data)))}
    out["from_service_item"] = sri
    raw = ACME["ticket_fields"]["ticket_fields"]
    w = []
    flds = []
    for f in raw:
        flds.extend(spec._fields_from_form_field(f, w))
    s1 = FormSpec(module="ticket", fields=list(flds), warnings=list(w))
    spec._link_sections(s1, raw)
    s2 = FormSpec(module="ticket", fields=[Field(**{k: getattr(f, k) for k in f.__dataclass_fields__
                                                    if k != "_display_ids"}) for f in flds], warnings=list(w))
    spec._link_ticket_sections(s2, raw)
    out["link_sections"] = SPEC(s1)
    out["link_ticket_sections"] = SPEC(s2)
    return out


def main() -> None:
    import tempfile
    with tempfile.TemporaryDirectory() as tmp:
        golden = {
            "errors": gen_errors(),
            "fetch": gen_fetch(),
            "plan": gen_plan(Path(tmp)),
            "tz": gen_tz(),
            "when": gen_when(),
            "fuzzy": gen_fuzzy(),
            "spec": gen_spec(),
        }
        for mod in EXTRA:
            golden.update(mod())
    (HERE / "py_golden.json").write_text(json.dumps(stamped(golden), ensure_ascii=False, separators=(",", ":"), allow_nan=False) + "\n")


EXTRA: list = []


# ---------------------------------------------------------------------------
# rules


def UNV(j):
    """Inverse of V()."""
    if isinstance(j, dict):
        if set(j) == {"lookup"}:
            i, d, e = j["lookup"]
            return LookupValue(i, d, e)
        if set(j) == {"date"}:
            return dt.date.fromisoformat(j["date"])
        if set(j) == {"datetime"}:
            return dt.datetime.fromisoformat(j["datetime"])
        return j
    if isinstance(j, list):
        return [UNV(x) for x in j]
    return j


def UNVS(d):
    return {k: UNV(v) for k, v in d.items()}


def ST(st: _State):
    return {
        "vis": dict(st.vis), "req": dict(st.req), "values": [[k, V(v)] for k, v in st.values.items()],
        "dis": sorted(st.dis), "populated": [[k, v] for k, v in st.populated.items()],
        "messages": [[i, [M(m) for m in st.messages[i]]] for i in sorted(st.messages)],
        "last_match": [[i, st.last_match[i]] for i in sorted(st.last_match)],
        "why": [[n, k, D(d)] for (n, k), d in st.why.items()],
        "matched_decisions": [[n, k, [D(d) for d in ds]] for (n, k), ds in st.matched_decisions.items()],
        "reg": {p: [[k, X(x)] for k, x in m.items()] for p, m in st.reg.items()},
        "pseudo_req": dict(st.pseudo_req),
        "restricted": {k: CR(v) for k, v in st.restricted.items()},
        "restricted_by": {k: D(v) for k, v in st.restricted_by.items()},
        "refused": [[n, D(d)] for n, d in st.refused],
        "overridden": [[k, V(a), b] for k, (a, b) in st.overridden.items()],
    }


def GROUP(g):
    return RULE(rules.Rule(0, "", g, ()))["condition"]


PORTAL_SYSTEM = {"portal": 9000006001}

# name -> (spec factory, rules fixture or inline, engine kwargs)
ENGINES = {
    "change_legacy": ("change", "change_rules", dict(module="change", **LEGACY)),
    "change_default": ("change", "change_rules", dict(module="change")),
    "change_replay": ("change", "change_rules", dict(module="change", model="replay", populate="set")),
    "change_requester": ("change", "change_rules", dict(module="change", user_type="requester", **LEGACY)),
    "change_dedupe_none": ("change", "change_rules", dict(module="change", dedupe="none", **LEGACY)),
    "change_dedupe_first": ("change", "change_rules", dict(module="change", dedupe="first", **LEGACY)),
    "change_dedupe_last": ("change", "change_rules", dict(module="change", dedupe="last", **LEGACY)),
    "change_same_fires": ("change", "change_rules", dict(module="change", same_value_fires=True, **LEGACY)),
    "ops": ("ops", "ops_rules", dict(module="change", workspace_id=2, **LEGACY)),
    "ops_default": ("ops", "ops_rules", dict(module="change", workspace_id=2)),
    "ops_any_ws": ("ops", "ops_rules", dict(module="change", item_id=55, user_type="any", **LEGACY)),
    "restrict": ("change", "restrict_rules", dict(module="change")),
    "restrict_transition": ("change", "restrict_rules", dict(module="change", event_reverse="transition",
                                                             event_scope="referenced")),
    "ticket": ("ticket_internal", "ticket_rules", dict(module="ticket")),
    "ticket_portal": ("ticket_internal", "ticket_rules", dict(module="ticket", system=PORTAL_SYSTEM)),
    "ticket_portal_edit_ctx": ("ticket_internal", "ticket_rules", dict(
        context=RuleContext(module="ticket", form="edit", user_type="agent", requester={"id": 1}),
        system=PORTAL_SYSTEM)),
}

SPECS = {"change": change_spec, "ops": ops_spec, "ticket_internal": ticket_internal_spec, "ticket": ticket_spec,
         "sr": sr_spec}


def engine(name: str) -> RulesEngine:
    s, r, kw = ENGINES[name]
    return RulesEngine(SPECS[s](), ACME[r], **kw)


def gen_rules():
    out: dict = {}
    eng = {}
    for name in ENGINES:
        e = engine(name)
        info = ENGINE(e)
        info["rules_for"] = {m: [[r.id, r.index] for r in e.rules_for(m)] for m in ("new", "edit", "new_form", "x")}
        info["unavailable_dependencies"] = {m: e.unavailable_dependencies(m) for m in ("new", "edit")}
        names = [f.api_name for f in e.spec.fields]
        info["position_order"] = e.position_order(list(reversed(names)) + ["zzz_unknown", "aaa_unknown"])
        info["condition_fields"] = [sorted(rules._condition_fields(r.condition)) for r in e.rules]
        eng[name] = info
    out["engines"] = eng

    # -- RuleContext.workspaces / Rule.applies_to ---------------------------
    out["workspaces"] = [[w, (lambda s: None if s is None else sorted(s))(RuleContext(workspace_id=w).workspaces())]
                         for w in (None, 2, "2", [1, "2", 3], [], "")]
    app = []
    for ft in ("both", "new_form", "edit_form", "weird"):
        for ut in ("both", "agent", "requester"):
            r = rules.Rule(1, "r", rules.ConditionGroup("all"), (), form_type=ft, user_type=ut)
            for mode in ("new", "edit", "new_form", "edit_form", "weird"):
                for user in ("agent", "requester", "any"):
                    app.append([ft, ut, mode, user, r.applies_to(mode, user)])
    out["applies_to"] = app

    # -- small value types ---------------------------------------------------
    out["message_key"] = [[m, list(Message(*m).key)] for m in (["hi", 5, "R", True], ["x", None, "", False],
                                                                ["y", "id-1", "Q", False])]
    out["decision_describe"] = [[d, Decision(*d).describe()] for d in (
        [5, "Rule A", "show", True, None, 0], [6, "B's", "hide", False, "team", 3], [None, "", "populate", True, "", 1])]
    out["exclude_name"] = [[x, acc, run(lambda: ExcludeName(*x).name(acc))] for x, acc in (
        (["team", True], "77"), (["team", True], None), (["team", True], ""), (["group_id", False], None),
        (["group_id", False], "77"))]
    cr = ChoiceRestriction({"live": None, "cannot log in": ChoiceRestriction({"vpn": None}, 1, "x")}, 1, "x")
    out["choice_restriction"] = {
        "offers": [[v, cr.offers(v)] for v in ("Live", " LIVE ", "Staging", "Cannot  log in", "", 5)],
        "below": [[v, CR(cr.below(v))] for v in ("Live", "cannot log in", "nope")],
    }
    out["is_empty"] = [[v, rules.is_empty(UNV(v))] for v in (None, "", " ", [], [""], 0, False, {"date": "2026-01-01"})]
    same_cases = [[None, ""], [None, []], ["a", "A"], ["a", "b"], [1, "1"], [1, 1.0], [True, 1], [True, True],
                  [False, None], [["a", "B"], ["A", "b"]], [["a"], ["a", "b"]], [{"lookup": [1, "A", None]},
                  {"lookup": [1, "a", None]}], [{"date": "2026-01-01"}, {"date": "2026-01-01"}], [" x", "x"],
                  [1.5, "1.5"], [0, False]]
    out["same"] = [[a, b, rules._same(UNV(a), UNV(b))] for a, b in same_cases]

    # -- private loading -----------------------------------------------------
    e = engine("ops")
    raws = list(ACME["ops_rules"]["business_rules"]) + list(ACME["change_rules"]["business_rules"]) + [
        {"name": "ws list", "workspace_id": "3"}, {"name": "item", "item_id": 55}, {"name": "item blank", "item_id": ""},
        {"name": "rt", "rule_type": "server_side"}, {"name": "state", "state": None}, {"name": "del0", "deleted": 0},
        {"name": "mod none", "module_type": None}]
    filt = {}
    for ename in ("ops", "ops_any_ws", "change_legacy", "ticket"):
        e = engine(ename)
        filt[ename] = [e._filter(r) for r in raws]
    out["filter"] = {"raws": raws, "out": filt}

    base_rule = {"condition_data": {"all": None}, "action_data": {"action": []}}
    dd_raws = [dict(base_rule, id=1, name="one"), dict(base_rule, id=2, name="two"),
               dict(base_rule, id=1, name="one bis"), dict(base_rule, id=None, name="anon"),
               dict(base_rule, name="anon2"), dict(base_rule, id=2, name="two"), dict(base_rule, id=1, name="one")]
    dedupe = {}
    for policy in ("exact", "first", "last", "none"):
        e = RulesEngine(change_spec(), [], module="change", dedupe=policy)
        kept = e._dedupe(dd_raws)
        dedupe[policy] = {"kept": [r["name"] for r in kept], "duplicates": list(e.duplicates)}
    out["dedupe"] = {"raws": dd_raws, "out": dedupe}

    load_cases = {
        "list": ACME["ops_rules"]["business_rules"],
        "singular_key": {"business_rule": ACME["ops_rules"]["business_rules"][:3]},
        "null": None,
        "empty_obj": {},
        "listing": ACME["business_rules_listing"],
    }
    loads = {}
    for name, data in load_cases.items():
        e = engine("ops")
        e.skipped.clear()
        e.duplicates.clear()
        r = run(lambda: [RULE(x) for x in e._load(data)])
        loads[name] = {"out": r, "skipped": list(e.skipped), "duplicates": list(e.duplicates),
                       "warnings": list(e.warnings)}
    out["load"] = {"cases": load_cases, "out": loads}

    pr_cases = [
        {"id": 70, "name": "  spaced  ", "condition_data": {"any": [
            {"name": "customFields.team", "operator": "is", "evaluate_on": "change", "value": ["Labs", 5]},
            {"all": [{"name": "priority", "operator": "selected", "evaluate_on": None}]}]},
         "action_data": {"action": [{"name": "customFields.stage", "property": "show"}]},
         "is_reversible": 1, "form_type": None, "user_type": "", "is_validation": "yes"},
        {"id": 71, "condition_data": {"all": [{"name": "x", "operator": "matches"}]}, "action_data": {}},
        {"id": 72, "name": "bad action", "condition_data": None,
         "action_data": {"action": [{"name": ["subject"], "property": "explode"}]}},
        {"id": 73, "name": "bad eval", "condition_data": {"all": [{"name": "x", "operator": "is",
                                                                    "evaluate_on": "planet"}]}},
        {"id": 74, "name": "both modes", "condition_data": {"all": [], "any": []}},
        {"name": None, "id": None, "condition_data": {"all": [{"name": "dept", "operator": "is",
                                                               "evaluate_on": "requester", "value": None}]},
         "action_data": {"action": [{"name": ["Hello"], "property": "message"},
                                    {"name": "attachment_ids", "property": "mandate"},
                                    {"name": ["nope", "group_id"], "property": "populate", "value": 601}]}},
    ]
    prs = {}
    for strict in (True, False):
        rows = []
        for i, raw in enumerate(pr_cases):
            e = RulesEngine(change_spec(), [], module="change", strict=strict)
            r = run(lambda: (lambda x: None if x is None else RULE(x))(e._parse_rule(raw, i)))
            rows.append({"out": r, "unsupported": list(e.unsupported), "warnings": list(e.warnings)})
        prs[str(strict).lower()] = rows
    out["parse_rule"] = {"cases": pr_cases, "out": prs}

    pg_cases = [None, {}, [], {"all": None}, {"any": []}, {"all": [], "any": []}, {"none": []},
                {"all": [{"name": "customFields.team", "operator": "is", "value": "Labs"},
                         {"any": [{"name": "nope", "operator": "is_not", "value": 3}], "operator": None}]},
                {"all": [{"any": [{"name": "priority", "operator": "selected"}]}]},
                {"all": [{"name": "dept", "operator": "includes", "evaluate_on": "current_user", "value": [1, "x"]}]},
                {"all": [{"name": "a.b.portal", "operator": "is", "evaluate_on": "system", "value": 9}]},
                {"all": [{"name": "x", "operator": "is", "evaluate_on": ""}]},
                {"all": [{"name": "x", "operator": ""}]},
                {"all": [{"name": "x", "operator": "is", "evaluate_on": "moon"}]},
                {"any": [{"operator": "is", "value": "v"}]}]
    pgs = []
    for data in pg_cases:
        e = RulesEngine(change_spec(), [], module="change")
        try:
            r = {"ok": GROUP(e._parse_group(data, "R"))}
        except rules._Skip as ex:
            r = {"skip": str(ex)}
        pgs.append({"out": r, "warnings": list(e.warnings)})
    out["parse_group"] = {"cases": pg_cases, "out": pgs}

    pa_cases = [
        [],
        [{"name": "customFields.team", "property": "hide"}],
        [{"name": ["customFields.team", "department_id", "nope", "customFields.nope", "attachment_ids"],
          "property": "mandate"}],
        [{"name": ["msg one", 2], "property": "message", "values": [1]}],
        [{"name": ["customFields.stage"], "property": "populate", "values": [801, 802]},
         {"name": ["subject"], "property": "populate", "value": "x"},
         {"name": ["subject"], "property": "populate", "value": None},
         {"name": None, "property": "disable"}],
        [{"name": ["subject"], "property": "enable"}],
        [{"name": ["subject"]}],
    ]
    pas = []
    for data in pa_cases:
        e = RulesEngine(change_spec(), [], module="change")
        try:
            acts = e._parse_actions(data, "R")
            r = {"ok": RULE(rules.Rule(0, "", rules.ConditionGroup("all"), acts))["actions"]}
        except rules._Skip as ex:
            r = {"skip": str(ex)}
        pas.append({"out": r, "warnings": list(e.warnings)})
    out["parse_actions"] = {"cases": pa_cases, "out": pas}

    # -- evaluation ----------------------------------------------------------
    evals = []
    ev_cases = [
        ("change_legacy", {}, "new", {}),
        ("change_legacy", {"team": "Labs"}, "new", {}),
        ("change_legacy", {"team": "Platform", "cloud_only": "Yes"}, "new", {}),
        ("change_legacy", {"cloud_only": "No", "stage": "Live", "rollout_tool": "Manual"}, "new", {}),
        ("change_legacy", {"change_type": "Emergency", "priority": "Urgent"}, "new", {}),
        ("change_legacy", {"change_type": "Normal", "needs_review": True, "team": "Payments", "priority": "High"},
         "edit", {}),
        ("change_legacy", {"cloud_only": "Yes", "stage": "Staging"}, "new", {"model": "replay"}),
        ("change_legacy", {"stage": "Live", "cloud_only": "No"}, "new",
         {"order": ["stage", "cloud_only"]}),
        ("change_legacy", {}, "new", {"events": [["team", "Labs"], ["department", "Finance"], ["team", "Platform"],
                                                  ["team", "Platform"], ["stage", ""]]}),
        ("change_legacy", {"team": "Payments"}, "new", {"base": {"team": "Labs", "priority": "High", "subject": ""}}),
        ("change_default", {"team": "Labs", "downtime": "Yes"}, "new", {}),
        ("change_default", {"cloud_only": "Yes", "stage": "Staging"}, "new", {}),
        ("change_replay", {"cloud_only": "No"}, "new", {}),
        ("change_replay", {"cloud_only": "Yes", "stage": "Staging", "team": "Labs", "department": "Finance"},
         "new", {}),
        ("change_requester", {}, "new", {}),
        ("change_same_fires", {"team": "Labs"}, "new", {"base": {"team": "Labs"}}),
        ("change_dedupe_none", {"change_type": "Normal"}, "new", {}),
        ("ops", {"tier": "Gold"}, "new", {}),
        ("ops", {"tier": "Silver", "region": "EU", "tags": ["A", "C"], "vip": True, "priority": "Urgent"}, "new", {}),
        ("ops", {"vip": True, "meeting": "N/A"}, "new", {"events": [["vip", True], ["meeting", "N/A"]]}),
        ("ops", {"area": "Ops", "area_team": "Infra"}, "new", {}),
        ("ops", {"area": "Dev", "area_team": "Other", "area_unit": "Misc"}, "edit", {}),
        ("ops_default", {"tier": "Gold", "region": "US"}, "new", {}),
        ("restrict", {}, "new", {}),
        ("restrict", {"cloud_only": "Yes"}, "new", {}),
        ("restrict", {"cloud_only": "Yes", "stage": "Staging"}, "new", {}),
        ("restrict", {"team": "Platform", "category": "Software"}, "new", {}),
        ("restrict", {"team": "Platform", "category": "Hardware", "sub_category": "Laptop"}, "new", {}),
        ("restrict", {}, "new", {"events": [["cloud_only", "Yes"], ["cloud_only", "No"], ["cloud_only", ""]]}),
        ("restrict_transition", {}, "new", {"events": [["cloud_only", "Yes"], ["cloud_only", "No"]]}),
        ("ticket", {}, "new", {}),
        ("ticket_portal", {}, "new", {}),
        ("ticket_portal", {"symptom": "Slow", "symptom_system": "Laptop", "attachment_check": "Yes"}, "new", {}),
        ("ticket_portal", {"symptom": "Other", "request_item": "Delivery"}, "edit", {}),
        ("ticket_portal_edit_ctx", {"symptom": "Cannot log in", "symptom_system": "Mail"}, "new", {}),
    ]
    for ename, values, mode, kw in ev_cases:
        e = engine(ename)
        kw2 = dict(kw)
        if "base" in kw2:
            kw2["base"] = UNVS(kw2["base"])
        if "events" in kw2:
            kw2["events"] = [(n, UNV(v)) for n, v in kw2["events"]]
        vals = UNVS(values)
        res = run(lambda: e.evaluate(vals, mode, **kw2))
        row = {"engine": ename, "values": values, "mode": mode, "kw": kw, "warnings": None}
        if "err" in res:
            row["out"] = res
        else:
            ev = res["ok"]
            names = [f.api_name for f in e.spec.ordered()]
            row["out"] = {"ok": EV(ev)}
            row["explain"] = [EXPL(x) for x in ev.explain()]
            row["explain_some"] = run(lambda: [EXPL(x) for x in ev.explain(["Stage", "nope"])])
            row["choices"] = {n: [c.value for c in ev.choices(n)] for n in names
                              if e.spec.by_name[n].kind in spec.CHOICE_KINDS}
            row["restriction_for"] = {n: [CR(r), D(d)] for n in names for r, d in [ev.restriction_for(n)]}
            row["is_visible"] = {n: ev.is_visible(n) for n in names + ["nope"]}
            row["is_required"] = {n: ev.is_required(n) for n in names + ["nope"]}
            row["blocking_messages"] = [M(m) for m in ev.blocking_messages]
            row["decision"] = {n: {p: D(ev.decision(n, p)) for p in ("visible", "required", "disabled")}
                               for n in names}
            row["offered_choices"] = {n: [c.value for c in rules.offered_choices(e.spec, e.spec.by_name[n],
                                                                                   ev.values, ev.restricted)]
                                      for n in names if e.spec.by_name[n].kind in spec.CHOICE_KINDS}
        row["warnings"] = list(e.warnings)
        evals.append(row)
    out["evaluate"] = evals

    # -- private evaluation pieces -------------------------------------------
    eo = []
    e = engine("change_legacy")
    for values, order in (({"stage": 1, "team": 2, "subject": 3}, None),
                          ({"stage": 1, "team": 2, "subject": 3}, ["subject", "nope", "subject", "stage"]),
                          ({"zz": 1, "team": 2}, []),
                          ({}, ["team"])):
        eo.append([values, order, e._event_order(values, order)])
    out["event_order"] = eo

    rep = []
    for ename, user, user_only in (
        ("change_legacy", {"cloud_only": "Yes", "team": "Labs"}, ["team", "cloud_only"]),
        ("change_legacy", {"cloud_only": "No", "stage": "Live", "department": "Finance", "team": "Labs"},
         ["department"]),
        ("ops", {"tier": "Gold", "vip": True, "meeting": "08/12/2030"}, ["meeting", "tier"]),
        ("restrict", {"cloud_only": "Yes", "team": "Platform"}, []),
    ):
        e = engine(ename)
        st = e._evaluate_replay(e.rules_for("new"), UNVS(user), set(user_only))
        rep.append({"engine": ename, "user": user, "user_only": user_only, "state": ST(st),
                    "excludes": {k: [X(x) for x in v] for k, v in e._excludes(st).items()},
                    "finish": EV(e._finish(st, "replay")), "warnings": list(e.warnings)})
    out["evaluate_replay"] = rep

    evs = []
    for ename, base, events in (
        ("change_legacy", {}, [["team", "Labs"], ["department", "Finance"]]),
        ("change_legacy", {"team": "Labs"}, [["team", "Labs"], ["team", ""], ["cloud_only", "Yes"], ["stage", "Staging"]]),
        ("change_same_fires", {"team": "Labs"}, [["team", "Labs"]]),
        ("ops", {}, [["tier", "Gold"], ["region", "US"], ["tier", "Silver"], ["vip", True], ["meeting", "N/A"]]),
        ("ops", {"area": "Ops", "area_team": "Infra", "area_unit": "Racks"}, [["area", "Dev"]]),
        ("ops_default", {}, [["tier", "Gold"], ["tier", "Bronze"]]),
        ("restrict", {}, [["cloud_only", "Yes"], ["stage", "Staging"], ["cloud_only", "No"]]),
        ("ticket_portal", {"symptom": "Slow"}, [["symptom", "Other"], ["attachment_check", "No"]]),
    ):
        e = engine(ename)
        st = e._evaluate_events(e.rules_for("new"), UNVS(base), [(n, UNV(v)) for n, v in events])
        evs.append({"engine": ename, "base": base, "events": events, "state": ST(st),
                    "excludes": {k: [X(x) for x in v] for k, v in e._excludes(st).items()},
                    "finish": EV(e._finish(st, "event")), "warnings": list(e.warnings)})
    out["evaluate_events"] = evs

    clr = []
    for ename, values, name in (
        ("ticket", {"symptom": "Slow", "symptom_system": "Laptop", "priority": "Low"}, "symptom"),
        ("ticket", {"symptom": "Slow"}, "symptom"),
        ("ticket", {"request_item": "Delivery", "delivery_team": "Apps", "delivery_squad": "Web"}, "delivery_team"),
        ("ticket", {"request_item": "Delivery", "delivery_team": "Apps", "delivery_squad": "Web"}, "request_item"),
        ("ops", {"area": "Ops", "area_team": "Infra", "area_unit": "Racks"}, "area"),
        ("ops", {"area": "Ops", "area_unit": "Racks"}, "area"),
        ("ops", {"area": "Ops", "area_team": "Infra", "area_unit": "Racks"}, "area_team"),
        ("ops", {"area": "Ops"}, "nope"),
    ):
        e = engine(ename)
        st = e._fresh(UNVS(values))
        for k in list(st.values):
            st.populated[k] = 1
        cleared = e._clear_lower_levels(name, st)
        clr.append({"engine": ename, "values": values, "name": name, "cleared": sorted(cleared),
                    "values_after": [[k, V(v)] for k, v in st.values.items()], "populated": list(st.populated)})
    out["clear_lower_levels"] = clr

    apl = []
    for ename, values, steps in (
        ("change_legacy", {"team": "Labs", "cloud_only": "Yes"},
         [[0, True, None], [0, False, "team"], [3, True, None], [3, False, "cloud_only"], [13, True, None],
          [12, True, "priority"], [12, False, None], [7, True, None], [9, True, None], [9, False, "change_type"]]),
        ("change_legacy", {"stage": "Staging"},
         [[3, True, "cloud_only", {"fixed": ["stage"]}], [3, True, None, {"user_set": ["stage"]}],
          [4, True, "cloud_only", {"user_set": ["stage"]}]]),
        ("ops", {"vip": True, "meeting": "N/A"},
         [[5, True, "vip", {"user_set": ["meeting"]}], [7, True, None], [7, False, None], [0, True, None]]),
        ("restrict", {"cloud_only": "Yes"}, [[1, True, None], [1, False, "cloud_only"], [2, True, None],
                                             [4, True, "team"], [4, False, "team"]]),
    ):
        e = engine(ename)
        st = e._fresh(UNVS(values))
        rows = []
        for step in steps:
            idx, ok, event = step[:3]
            opts = step[3] if len(step) > 3 else {}
            rule = e.rules[idx]
            fixed = {k: st.values.get(k) for k in opts["fixed"]} if "fixed" in opts else None
            user_set = set(opts["user_set"]) if "user_set" in opts else None
            e._apply(rule, ok, st, event, fixed=fixed, user_set=user_set)
            rows.append(ST(st))
        apl.append({"engine": ename, "values": values, "steps": steps, "states": rows, "warnings": list(e.warnings)})
    out["apply"] = apl

    rst = []
    e = engine("restrict")
    rule_by_id = {r.id: r for r in e.rules}
    for fname, vals, rid in (
        ("stage", [801], 32), ("stage", [999, 802], 32), ("stage", [], 32), ("stage", [999], 32),
        ("stage", ["Live", "staging"], 33),
        ("category", [{"display_id": 1, "nested_options": [{"display_id": 2, "nested_options": None}]}], 35),
        ("category", [{"id": 2101, "nested_options": []}, {"value": "Software"}], 35),
        ("category", [{"display_id": 1, "nested_options": [{"display_id": 99}]}], 35),
        ("subject", [1], 35), ("services", ["Billing"], 35),
    ):
        e.warnings.clear()
        r = e._restriction(e.spec.by_name[fname], vals, rule_by_id[rid])
        rst.append({"field": fname, "values": vals, "rule": rid, "out": CR(r), "warnings": list(e.warnings)})
    out["restriction"] = rst
    rs = []
    e = engine("restrict")
    st = e._fresh({})
    for rid, ok, event in ((32, True, None), (35, True, "team"), (32, False, "cloud_only"), (33, True, None),
                           (35, False, None)):
        rule = rule_by_id[rid]
        e._restrict(rule, rule.actions[0], ok, st, event)
        rs.append({"rule": rid, "ok": ok, "event": event, "state": ST(st)})
    out["restrict"] = rs

    # -- matching ------------------------------------------------------------
    value_sets = [
        {}, {"team": "Labs"}, {"team": "labs"}, {"team": "Platform", "priority": "High"},
        {"team": "Payments", "priority": "High"}, {"team": 1103}, {"stage": "Live", "cloud_only": "Yes"},
        {"rollout_tool": "Script"}, {"rollout_tool": ["Manual", "Pipeline"]}, {"needs_review": True},
        {"needs_review": False}, {"needs_review": "yes"}, {"change_type": "Normal"}, {"change_type": 1},
        {"change_type": "1"}, {"priority": "Urgent"}, {"downtime": "No"}, {"team": ""}, {"team": []},
        {"tier": "Gold"}, {"tier": "silver"}, {"tags": ["B"]}, {"tags": []}, {"vip": True}, {"vip": "x"},
        {"area": "Ops", "area_team": "Infra"}, {"area": "Dev", "area_team": "Infra"},
        {"symptom": "Slow"}, {"symptom": "Cannot log in", "symptom_system": "VPN"},
    ]
    mg = {}
    for ename in ("change_legacy", "ops", "ticket", "ticket_portal"):
        e = engine(ename)
        rows = []
        for r in e.rules:
            row = []
            for vs in value_sets:
                vals = UNVS(vs)
                conds = [e._match(c, vals) for c in r.condition.items if isinstance(c, rules.Condition)]
                row.append([e._match_group(r.condition, vals), conds])
            rows.append(row)
        mg[ename] = {"rows": rows, "warnings": list(e.warnings)}
    out["match"] = {"value_sets": value_sets, "out": mg}

    vk = []
    e = engine("change_legacy")
    eo_engine = engine("ops")
    for ename, fname, v, eff in (
        ("change_legacy", "team", "Labs", {}), ("change_legacy", "team", 1103, {}), ("change_legacy", "team", "13", {}),
        ("change_legacy", "services", ["Billing", "Reports, Exports", "zzz"], {}),
        ("change_legacy", "needs_review", True, {}), ("change_legacy", "needs_review", False, {}),
        ("change_legacy", "requester", {"lookup": [9001, "Jane Doe", "jane.doe@acme.example"]}, {}),
        ("change_legacy", "requester", {"lookup": [9001, "", None]}, {}),
        ("change_legacy", "subject", "Hello  World", {}), ("change_legacy", "subject", "", {}),
        ("change_legacy", "subject", [], {}), ("change_legacy", None, "Free", {}),
        ("change_legacy", "estimated_hours", 2.5, {}),
        ("change_legacy", "staging_start", {"date": "2026-11-26"}, {}),
        ("change_legacy", "sub_category", "Laptop", {"category": "Hardware"}),
        ("change_legacy", "sub_category", "Laptop", {}),
        ("ops", "area_team", "Infra", {"area": "Ops"}), ("ops", "area_team", "Infra", {"area": "Dev"}),
        ("ops", "tier", 9000003001, {}), ("ops", "tags", ["A", "c"], {}),
    ):
        e = engine(ename)
        f = e.spec.by_name[fname] if fname else None
        keys = run(lambda: [sorted(k) for k in e._value_keys(f, UNV(v), UNVS(eff))])
        vk.append([ename, fname, v, eff, keys])
    out["value_keys"] = vk

    pv = []
    for ename, fname, vals in (
        ("change_legacy", "stage", [801]), ("change_legacy", "stage", ["Live"]), ("change_legacy", "stage", [999]),
        ("change_legacy", "stage", []), ("change_legacy", "stage", [801, 802]),
        ("change_legacy", "services", [1501, "Search"]), ("change_legacy", "services", ["Billing", "nope"]),
        ("change_legacy", "needs_review", ["true"]), ("change_legacy", "needs_review", ["No"]),
        ("change_legacy", "needs_review", [1]), ("change_legacy", "subject", ["x"]),
        ("change_legacy", "subject", ["x", "y"]), ("change_legacy", "department", ["Finance"]),
        ("change_legacy", "estimated_hours", [3]), ("change_legacy", "signoff_date", [700]),
        ("ops", "region", [9000003011]), ("ops", "meeting", [9000003031]), ("ops", "area", ["Dev"]),
        ("ops", "tier", ["9000003002"]),
    ):
        e = engine(ename)
        e.warnings.clear()
        r = e._populate_value(e.spec.by_name[fname], vals, e.rules[0])
        pv.append([ename, fname, vals, V(r), list(e.warnings)])
    out["populate_value"] = pv

    pers = []
    for ename in ("change_legacy", "ops", "ticket", "ticket_portal", "ticket_portal_edit_ctx"):
        e = engine(ename)
        e.warnings.clear()
        row = {}
        for src in ("user", "requester", "requested_for", "system", "user"):
            row[src] = e._person(src)
        pers.append([ename, row, list(e.warnings)])
    out["person"] = pers

    # -- free helpers --------------------------------------------------------
    nc = {}
    for sname in ("change", "ticket", "ticket_internal", "ops"):
        s = SPECS[sname]()
        nc[sname] = {f.api_name: [x.api_name for x in rules._nested_chain(s, f)] for f in s.fields}
    out["nested_chain"] = nc

    ci = []
    for sname, fname, raws in (
        ("ticket_internal", "symptom", [1, "4", 9000007106, "cannot log in", "SLOW", "nope", 2]),
        ("change", "stage", [801, "802", "live", 1301, "Staging", "x"]),
        ("ops", "tier", [9000003001, "9000002001", "gold", 9000003003]),
    ):
        s = SPECS[sname]()
        f = s.by_name[fname]
        ci.append([sname, fname, [[r, (lambda c: None if c is None else c.value)(rules._choice_in(f.choices, r))]
                                  for r in raws],
                   [[r, run(lambda: (lambda c: None if c is None else c.value)(f.choice(r, f.choices)))]
                    for r in raws]])
    out["choice_in"] = ci

    cba = []
    for sname, fname, raws in (
        ("change", "stage", [801, "802", 1301, "1302", "Live", "staging", "x", 9]),
        ("change", "priority", [1, "4", "Urgent", 5]),
        ("ops", "tier", [9000003001, 9000002002, "Gold", "9000003003"]),
        ("ticket_internal", "symptom", [4, 9000007101, "Other"]),
    ):
        s = SPECS[sname]()
        f = s.by_name[fname]
        cba.append([sname, fname, [[r, run(lambda: (lambda c: None if c is None else c.value)(
            rules._choice_by_any(f, r)))] for r in raws]])
    out["choice_by_any"] = cba

    touch = []
    st = _State(vis={}, req={}, values={})
    for prop, t in (("show", Target("team", "customFields.team", "team", True)),
                    ("hide", Target("team", "x", "other", False)),
                    ("mandate", Target(None, "attachment_ids", "attachment_ids", False)),
                    ("unmandate", Target("group", "group_id", "group_id", False)),
                    ("disable", Target("subject", "subject", "subject", False)),
                    ("enable", Target(None, "customFields.gone", "gone", True)),
                    ("enable", Target(None, "customFields.gone", "gone2", True))):
        st.touch(prop, t)
        touch.append([prop, [t.api, t.raw, t.base, t.custom],
                      {p: [[k, X(x)] for k, x in m.items()] for p, m in st.reg.items()}])
    out["touch"] = touch
    return {"rules": out}


EXTRA.append(gen_rules)


# ---------------------------------------------------------------------------
# state


def build_state(setup: dict) -> FormState:
    if "engine" in setup:
        e = engine(setup["engine"])
        s = e.spec
    else:
        e = None
        s = SPECS[setup["spec"]]()
    existing = setup.get("existing")
    return FormState(
        s, e, mode=setup.get("mode", "new"), values=UNVS(setup.get("values") or {}),
        resolver=FakeResolver() if setup.get("resolver", True) else None, tz=setup.get("tz", "UTC"),
        apply_defaults=setup.get("apply_defaults", True), existing=UNVS(existing) if existing else None,
        event_order=setup.get("event_order", "position"))


def SNAP(st: FormState, full: bool = False):
    names = [f.api_name for f in st.spec.ordered()]
    out = {
        "values": [[k, V(v)] for k, v in st.values.items()],
        "sources": [[k, v] for k, v in st.sources.items()],
        "base": [[k, V(v)] for k, v in st.base.items()],
        "tz": [tz.tz_name(st.tz), st.tz_source],
        "evaluated": EV(st.evaluated),
        "acknowledged": sorted([list(k) for k in st.acknowledged]),
        "unacknowledged": [M(m) for m in st.unacknowledged()],
        "visible_fields": [f.api_name for f in st.visible_fields()],
        "messages": [M(m) for m in st.messages],
        "missing_required": [f.api_name for f in st.missing_required()],
        "unsupported_errors": st.unsupported_errors(),
        "unavailable_errors": st.unavailable_errors(),
        "errors": st.errors(),
        "payload_values": [[k, V(v)] for k, v in st.payload_values().items()],
        "to_review": [list(r) for r in st.to_review()],
        "check_refused": run(lambda: st._check_refused()),
    }
    if full:
        out.update({
            "is_populated": {n: st.is_populated(n) for n in names},
            "dropped": [[f.api_name, V(v), D(d)] for f, v, d in st.dropped()],
            "explain": [EXPL(x) for x in st.explain()],
            "conflicts": [CONFLICT(c) for c in st.conflicts],
            "choices": {n: [c.value for c in st.choices(n)] for n in names
                        if st.spec.by_name[n].kind in spec.CHOICE_KINDS},
        })
    return out


def step(st: FormState, s: dict):
    op = s["op"]
    if op == "set":
        return run(lambda: DIFF(st.set(s["name"], UNV(s["raw"])), st.spec))
    if op == "unset":
        return run(lambda: DIFF(st.unset(s["name"]), st.spec))
    if op == "ack":
        return run(lambda: st.acknowledge(*s["args"]))
    if op == "ack_msg":
        return run(lambda: st.acknowledge(*[Message(*m) for m in s["args"]]))
    raise ValueError(op)


def S(op, name=None, raw=None, args=None):
    out = {"op": op}
    if name is not None:
        out["name"] = name
    if op == "set":
        out["raw"] = raw
    if args is not None:
        out["args"] = args
    return out


JANE = {"lookup": [9001, "Jane Doe", "jane.doe@acme.example"]}
SCRIPTS = {
    "change": ({"engine": "change_legacy"}, [
        S("set", "team", "Labs"), S("set", "department", "Finance"), S("set", "priority", "Urgent"),
        S("ack", args=["nope"]), S("ack", args=["13"]), S("set", "priority", "Low"), S("set", "priority", "Urgent"),
        S("ack_msg", args=[["Urgent changes need the duty manager's approval", 13, "Urgent priority", True]]),
        S("ack_msg", args=[["other", 13, "Urgent priority", True]]), S("ack", args=[]),
        S("set", "change_type", "Emergency"), S("set", "cloud_only", "Yes"), S("set", "stage", "Staging"),
        S("unset", "team"), S("set", "requester", "jane"), S("set", "requester", "jane.doe@acme.example"),
        S("set", "reviewers", "Sam, 9001"), S("set", "reviewers", [JANE, "9003"]), S("set", "reviewers", "9999"),
        S("set", "group", "Platform"), S("set", "group", "plat"), S("set", "agent", 701), S("set", "agent", "701"),
        S("set", "agent", True), S("set", "change_window", "weekend"), S("set", "change_window", ["a", "b"]),
        S("set", "staging_start", "2026-11-26"), S("set", "staging_end", "26/11/2026"),
        S("set", "planned_start_date", "2026-11-26 19:00"), S("set", "planned_end_date", "2026-11-26T19:00+07:00"),
        S("set", "planned_end_date", "2026-11-26"), S("set", "services", "Billing, Search"),
        S("set", "services", "Reports, Exports"), S("set", "services", ["Billing", "billing", "Search"]),
        S("set", "services", "Nope"), S("set", "needs_review", "yes"), S("set", "needs_review", "maybe"),
        S("set", "estimated_hours", "1,500"), S("set", "estimated_hours", "2.5"), S("set", "estimated_hours", "x"),
        S("set", "guide", "x"), S("set", "subject", "  hi "), S("set", "description", "<p>Long</p>"),
        S("set", "team", "Payments"), S("set", "priority", "High"), S("unset", "priority"),
        S("set", "nope", "x"), S("set", "downtime", "No"), S("set", "downtime_window", "Sat"),
        S("set", "signoff_date", "N/A"), S("set", "change_type", "Normal"),
    ]),
    "ops": ({"engine": "ops"}, [
        S("set", "owner", {"lookup": [9000006001, "Robin Ops", "robin.ops@example.com"]}),
        S("set", "owner", "42"), S("set", "owner", "jane"), S("set", "owner", "9002"),
        S("set", "tier", "Gold"), S("set", "region", "US"), S("set", "vip", True), S("set", "meeting", "N/A"),
        S("set", "area", "Ops"), S("set", "area_team", "Infra"), S("set", "area_unit", "Racks"),
        S("set", "area", "Dev"), S("set", "area_team", "Infra"), S("set", "area_team", "Other"),
        S("set", "tags", "A, B"), S("set", "when", "2026-01-02"), S("set", "code", "x"), S("set", "tier", "Silver"),
        S("set", "priority", "Urgent"), S("set", "notes", "n"), S("unset", "vip"),
    ]),
    "ops_no_resolver": ({"engine": "ops", "resolver": False}, [
        S("set", "owner", 9000006001), S("set", "owner", {"lookup": [5, "Five", None]}), S("set", "owner", "Robin"),
    ]),
    "restrict": ({"engine": "restrict"}, [
        S("set", "stage", "Staging"), S("set", "cloud_only", "Yes"), S("set", "stage", "Staging"),
        S("set", "stage", "Live"), S("set", "cloud_only", "No"), S("set", "team", "Platform"),
        S("set", "category", "Software"), S("set", "category", "Hardware"), S("set", "sub_category", "Laptop"),
        S("set", "team", "Labs"), S("unset", "cloud_only"),
    ]),
    "ticket_portal": ({"engine": "ticket_portal"}, [
        S("set", "symptom", "Slow"), S("set", "symptom", "Cannot log in"), S("set", "symptom_system", "Mail"),
        S("set", "symptom_system", "VPN"), S("set", "symptom", "Other"), S("set", "symptom_system", "VPN"),
        S("set", "attachment_check", "Yes"), S("set", "request_item", "Delivery"), S("set", "delivery_team", "Apps"),
        S("set", "delivery_squad", "Web"), S("set", "request_item", "Access"), S("set", "owner", "sam"),
        S("set", "owner", 9003), S("set", "priority", "Low"),
    ]),
    "ticket": ({"spec": "ticket"}, [
        S("set", "issue", "Login"), S("set", "issue_system", "VPN"), S("set", "issue_detail", "Timeout"),
        S("set", "issue", "Printer"), S("set", "issue_system", "VPN"), S("set", "ticket_type", "Major Incident"),
        S("set", "customers_hit", "12"), S("set", "mystery", "x"), S("set", "due_on", "2026-02-30"),
        S("set", "due_on", "2026-02-28"), S("set", "requester", "sam.lee@acme.example"),
    ]),
    "sr": ({"spec": "sr"}, [
        S("set", "title_100", "Dark mode"), S("set", "goal", "Lower cost"), S("set", "goal_2_kpi", "10%"),
        S("set", "goal", "Other"), S("set", "date_103", "2026-12-01"), S("set", "date_103", "01/12/2026"),
        S("set", "priority", "Critical - blocks customers"), S("set", "overview_deck", "deck.pptx"),
        S("set", "overview_deck", ["a.pptx", "b.pptx"]), S("set", "overview_deck", "x.pdf"),
        S("set", "notice", "x"), S("set", "requested_for", "jane.doe@acme.example"),
    ]),
    "refused_ctor": ({"engine": "change_legacy", "existing": {"team": "Labs"}, "values": {"department": "Finance"}},
                     []),
    "edit_input": ({"engine": "change_legacy", "mode": "edit", "existing": {"subject": "Old", "Team": "Labs"},
                    "event_order": "input"}, [
        S("set", "subject", "New"), S("set", "team", "Payments"), S("set", "priority", "High"),
        S("set", "department", "Finance"), S("unset", "team"),
    ]),
    "history": ({"engine": "change_legacy", "event_order": "history"}, [
        S("set", "cloud_only", "Yes"), S("set", "cloud_only", "No"), S("set", "stage", "Live"),
        S("set", "team", "Labs"), S("set", "team", "Platform"),
    ]),
    "default_engine": ({"engine": "change_default", "values": {"cloud_only": "Yes", "priority": "Urgent"},
                        "apply_defaults": False, "tz": "Asia/Bangkok"}, [
        S("set", "team", "Labs"), S("set", "downtime", "Yes"), S("set", "stage", "Staging"),
        S("set", "planned_start_date", "2026-11-26 19:00"),
    ]),
    "ny": ({"engine": "change_legacy", "tz": "America/New_York"}, [
        S("set", "planned_start_date", "2026-03-08 02:30"), S("set", "planned_start_date", "2026-11-01 01:30"),
        S("set", "planned_start_date", "2026-11-01 01:30-05:00"), S("set", "planned_start_date", "2026-07-04 09:00"),
    ]),
}


def run_script(setup, steps):
    try:
        st = build_state(setup)
    except Exception as ex:  # noqa: BLE001
        return {"ctor": run(lambda: (_ for _ in ()).throw(ex))}
    out = {"ctor": {"ok": None}, "initial": SNAP(st, True), "steps": []}
    for i, s in enumerate(steps):
        r = step(st, s)
        out["steps"].append({"step": s, "out": r, "snap": SNAP(st, i == len(steps) - 1)})
    return out, st


def gen_state():
    out: dict = {}
    scripts = {}
    states = {}
    for name, (setup, steps) in SCRIPTS.items():
        res = run_script(setup, steps)
        if isinstance(res, tuple):
            res, st = res
            states[name] = st
        scripts[name] = {"setup": setup, **res}
    out["scripts"] = scripts

    # direct parsing on fresh states
    parse_cases = {
        "change": ["team", "services", "needs_review", "estimated_hours", "staging_start", "planned_start_date",
                   "requester", "reviewers", "group", "agent", "change_window", "guide", "subject", "description",
                   "department", "change_type", "Stage", "nope"],
        "ticket": ["issue", "issue_system", "ticket_type", "customers_hit", "due_on", "mystery"],
        "sr": ["overview_deck", "date_103", "goal", "goal_1_kpi", "notice", "requested_for"],
    }
    raws = ["", "  ", None, [], "Labs", "labs", "1102", 1102, "Billing, Search", ["Billing", "Search"], "yes", "No",
            True, False, 0, 1, 2.5, "1,200", "1e3", "abc", "2026-11-26", "2026-11-26 19:00", "2026-11-26T19:00Z",
            {"date": "2026-11-26"}, {"datetime": "2026-11-26T19:00:00+07:00"}, "jane", "Jane Doe", "9001", 9001,
            JANE, ["Sam"], ["Sam", "Jane Doe"], "Platform", "x.pptx", ["a.pptx", "b.pptx"], "Normal", "4",
            "Login", "VPN", "Lower cost", "  spaced  ", "Reports, Exports", ["", "x"]]
    parse = {}
    for sname, fields in parse_cases.items():
        st = build_state({"spec": sname} if sname != "change" else {"engine": "change_legacy"})
        rows = []
        for fname in fields:
            for raw in raws:
                rows.append([fname, raw, run(lambda: V(st.parse(fname, UNV(raw))))])
        parse[sname] = rows
    out["parse"] = {"raws": raws, "out": parse}

    direct = {}
    st = build_state({"engine": "change_legacy"})
    sp = st.spec
    rows = []
    for fname in ("team", "stage", "department", "change_type", "sub_category", "signoff_date"):
        f = sp.by_name[fname]
        for raw in ("Labs", "1102", 1102, "Live", "Engineering", "Major", "3", "Laptop", "N/A", "zz", "13"):
            rows.append([fname, raw, run(lambda: st._parse_choice(f, UNV(raw)).value)])
    direct["parse_choice"] = rows
    rows = []
    for fname in ("services", "review_scope"):
        f = sp.by_name[fname]
        for raw in ("Billing, Search", "Reports, Exports", ["Billing", "Billing"], "Code,Infra", "Nope", ["Code"]):
            rows.append([fname, raw, run(lambda: [c.value for c in st._parse_multi(f, UNV(raw))])])
    direct["parse_multi"] = rows
    rows = []
    for fname in ("requester", "reviewers", "group", "agent", "change_window"):
        f = sp.by_name[fname]
        for raw in ("jane", "Jane Doe", "9001", 9001, "Sam, Jane Doe", ["Sam"], ["Sam", "Alex"], JANE, "plat",
                    "Platform", "701", "weekend", True, "nobody"):
            rows.append([fname, raw, run(lambda: V(st._parse_lookup(f, UNV(raw)))),
                         run(lambda: V(st._lookup_one(f, UNV(raw))))])
    direct["parse_lookup"] = rows
    clr = []
    for values, old in (({"area": "Dev", "area_team": "Other", "area_unit": "Misc"}, "Ops"),
                        ({"area": "Dev", "area_team": "Other", "area_unit": "Misc"}, "Dev"),
                        ({"area": "Dev", "area_unit": "Misc"}, None),
                        ({"area_team": "Other"}, "dev")):
        s2 = build_state({"engine": "ops"})
        for k, v in values.items():
            s2._store(s2.spec.by_name[k], v, "user")
        cleared = s2._clear_lower_levels(s2.spec.by_name["area"], old)
        clr.append([values, old, cleared, [[k, V(v)] for k, v in s2.values.items()], list(s2._seq)])
    s2 = build_state({"engine": "ops"})
    s2._store(s2.spec.by_name["tier"], "Gold", "user")
    clr.append([{"tier": "Gold"}, "Silver", s2._clear_lower_levels(s2.spec.by_name["tier"], "Silver"),
                [[k, V(v)] for k, v in s2.values.items()], list(s2._seq)])
    direct["clear_lower_levels"] = clr
    out["direct"] = direct

    # free helpers
    num = Field("n", "Count", "number")
    chk = Field("c", "Flag", "checkbox")
    day = Field("d", "Day", "date")
    when_f = Field("w", "When", "datetime")
    free = {}
    free["parse_number"] = [[r, run(lambda: V(state._parse_number(num, UNV(r))))] for r in (
        1, -2, 2.5, "1,234", " 12 ", "1.5e2", "-0.5", "abc", True, "", "+7", "1_000", "٣", "inf", "nan", "0x10")]
    free["parse_bool"] = [[r, run(lambda: state._parse_bool(chk, UNV(r)))] for r in (
        True, False, "yes", "Y", "TRUE", "on", "1", "0", "off", "n", "maybe", 1, 0, " No ", "")]
    free["parse_date"] = [[r, run(lambda: V(state._parse_date(day, UNV(r))))] for r in (
        "2026-11-26", " 2026-11-26 ", "20261126", "2026-W48-4", "2026-11-26T10:00", "26/11/2026", 5,
        {"date": "2026-01-02"}, {"datetime": "2026-01-02T23:00:00-05:00"})]
    free["same_value"] = [[a, b, state._same_value(UNV(a), UNV(b))] for a, b in (
        (None, ""), ("a", "A "), ("a", "b"), (None, "x"), ([], None), (1, "1"), (["a"], ["A"]), ("", 0), (0, None))]
    pdt = []
    for zname in ("UTC", "Asia/Bangkok", "America/New_York"):
        z = tz.zone(zname)
        for r in ("2026-11-26 19:00", "2026-11-26T19:00:30", "2026-11-26 19:00+05:30", "2026-11-26T19:00Z",
                  "2026-11-26", "26 Nov 2026 19:00", "2026-03-08 02:30", "2026-11-01 01:30", "", {"date": "2026-01-02"},
                  {"datetime": "2026-01-02T03:04:05+09:00"}, "2026-11-26 19:00:00.123456"):
            pdt.append([zname, r, run(lambda: V(state._parse_datetime(when_f, UNV(r), z)))])
    free["parse_datetime"] = pdt
    att = []
    f_plain = Field("a", "Files", "attachment")
    f_lim = Field("b", "Deck", "attachment", max_files=2, allowed_file_types=(".pptx", "PDF"))
    for f in (f_plain, f_lim):
        for r in ("x.pptx", ["a.pptx", "b.PDF"], ["a.pptx", "b.pdf", "c.pdf"], "x.doc", 42, [42, "y.pdf"], ["noext"]):
            att.append([f.api_name, r, run(lambda: V(state._parse_attachments(f, UNV(r))))])
    free["parse_attachments"] = att
    out["free"] = free
    return {"state": out}


EXTRA.append(gen_state)


# ---------------------------------------------------------------------------
# payload

FILLED_CHANGE = [
    ["requester", "jane.doe@acme.example"], ["department", "Engineering"], ["team", "Platform"], ["group", 602],
    ["agent", "alex"], ["services", ["Billing"]], ["cloud_only", "No"], ["rollout_tool", "Manual"],
    ["reviewers", [{"lookup": [9003, "Sam Lee", None]}]], ["staging_start", "2030-11-18 09:00"],
    ["staging_end", "2030-11-18 12:00"], ["subject", "Billing: rotate keys"], ["description", "<p>Rotate keys.</p>"],
    ["needs_review", True], ["review_scope", "Code"], ["planned_start_date", "2030-11-26 19:00"],
    ["planned_end_date", "2030-11-26T22:00:00+07:00"], ["downtime", "No"], ["signoff_date", "01/12/2030"],
    ["risk", "Medium"],
]
FILLED_TICKET = [
    ["subject", "VPN drops"], ["description", "It drops <every> hour.\nSince Monday."], ["department", "Engineering"],
    ["issue", "Login"], ["issue_system", "VPN"], ["ticket_type", "Major Incident"], ["customers_hit", "40"],
    ["due_on", "2030-02-01"], ["group", 77],
]
FILLED_SR = [
    ["Title", "Dark mode"], ["Details", "Please add dark mode."], ["Needed by", "2030-12-03"],
    ["goal", "More revenue (Total USD)"], ["goal_1_kpi", "+5%"], ["Priority", "low - cosmetic, reports"],
]
BKK = {"tz": "Asia/Bangkok"}
# name -> (setup, sets, acknowledge args)
PAYLOAD_STATES = {
    "change_empty": ({"engine": "change_legacy", **BKK}, [], []),
    "change_filled": ({"engine": "change_legacy", **BKK}, FILLED_CHANGE, []),
    "change_urgent": ({"engine": "change_legacy", **BKK}, FILLED_CHANGE + [["priority", "Urgent"]], []),
    "change_urgent_acked": ({"engine": "change_legacy", **BKK}, FILLED_CHANGE + [["priority", "Urgent"]], [13]),
    "change_emergency": ({"engine": "change_legacy", **BKK}, FILLED_CHANGE + [
        ["change_type", "Emergency"], ["emergency_reason", "Outage"], ["category", "Hardware"]], []),
    "change_cloud": ({"engine": "change_legacy", **BKK}, FILLED_CHANGE + [["cloud_only", "Yes"]], []),
    "change_default": ({"engine": "change_default", **BKK}, FILLED_CHANGE + [["team", "Labs"]], []),
    "ticket_empty": ({"spec": "ticket"}, [], []),
    "ticket_filled": ({"spec": "ticket"}, FILLED_TICKET, []),
    "ticket_requester": ({"spec": "ticket"}, FILLED_TICKET + [["requester", "sam.lee@acme.example"]], []),
    "ticket_printer": ({"spec": "ticket"}, FILLED_TICKET + [["issue", "Printer"], ["mystery", "x"]], []),
    "ticket_internal": ({"spec": "ticket_internal"}, [], []),
    "ticket_portal": ({"engine": "ticket_portal"}, [
        ["symptom", "Cannot log in"], ["symptom_system", "VPN"], ["attachment_check", "Yes"],
        ["request_item", "Delivery"], ["delivery_team", "Apps"], ["delivery_squad", "Web"], ["owner", 9003],
        ["priority", "Low"], ["subject", "Portal"], ["description", "a\r\n\r\nb"]], []),
    "ticket_portal_empty": ({"engine": "ticket_portal"}, [], []),
    "sr_empty": ({"spec": "sr"}, [], []),
    "sr_filled": ({"spec": "sr"}, FILLED_SR, []),
    "sr_other": ({"spec": "sr"}, FILLED_SR + [["goal", "Other"]], []),
    "sr_deck": ({"spec": "sr"}, FILLED_SR + [["goal", "Other"], ["overview_deck", "deck.pptx"]], []),
    "sr_requested_for": ({"spec": "sr"}, FILLED_SR + [["requested_for", "jane.doe@acme.example"]], []),
    "ops": ({"engine": "ops"}, [
        ["owner", "9002"], ["tier", "Gold"], ["region", "US"], ["vip", True], ["area", "Ops"], ["area_team", "Infra"],
        ["area_unit", "Racks"], ["tags", "A, B"], ["when", "2026-01-02"], ["priority", "Urgent"]], []),
}
ACCOUNTS = [None, "", "77", 123]
MODULE_OPTS = [
    {},
    {"requester_email": "sam.lee@acme.example", "requester_name": "Sam Lee"},
    {"requester_email": "jane.doe@acme.example", "allow_incomplete": True},
    {"requester_email": " jane.doe@acme.example ", "requester_name": "  ", "allow_incomplete": True},
    {"requester_email": "not-an-email", "allow_incomplete": True},
    {"allow_incomplete": True},
    {"requester_email": "jane.doe@acme.example", "status": "Open"},
    {"requester_email": "jane.doe@acme.example", "status": 2, "allow_incomplete": True},
    {"requester_email": "jane.doe@acme.example", "status": "Done", "allow_incomplete": True},
    {"requester_email": "jane.doe@acme.example", "account_id": "77", "allow_incomplete": True},
    {"requester_email": "jane.doe@acme.example", "account_id": 123, "allow_incomplete": True},
]
SR_OPTS = [
    {},
    {"field_attachments": {"Overview deck (.pptx)": [555]}, "attachments": [556]},
    {"field_attachments": {"overview_deck": [1]}, "attachments": [2]},
    {"field_attachments": {"overview_deck": [1, 2, 3]}, "attachments": [2]},
    {"field_attachments": {"overview_deck": [1], "Overview deck (.pptx)": [2]}, "attachments": [2]},
    {"field_attachments": {"title_100": [1], "nope": [2]}, "attachments": [2]},
    {"attachments": [2]},
    {"field_attachments": {"overview_deck": []}, "attachments": [2]},
    {"allow_incomplete": True},
    {"allow_incomplete": True, "attachments": [9]},
    {"field_attachments": {"overview_deck": [1]}, "attachments": [2], "account_id": "77"},
    {"field_attachments": {"overview_deck": [1]}, "attachments": [2], "item": {"allow_attachments": False}},
    {"field_attachments": {"overview_deck": [1]}, "item": {"attachment_mandatory": False}},
    {"allow_incomplete": True, "item": {"attachment_mandatory": True, "id": None}},
]
SR_EMAILS = ["jane.doe@acme.example", "Jane <jane@x>", "nobody"]
ENCODE_CANDIDATES = [
    "Platform", "Engineering", 602, True, False, 2.5, None, "", ["Billing", "Search"], "Billing",
    {"lookup": [9003, "Sam Lee", None]}, [{"lookup": [9001, "Jane Doe", "jane.doe@acme.example"]}, 5],
    {"date": "2030-01-02"}, {"datetime": "2030-01-02T09:00:30+07:00"}, "plain <b>\n\ntext\nmore", "  <p>x</p>",
    "Major Incident", "Low - cosmetic, reports",
]


def STORED(v):
    """Form state stores lists as tuples."""
    return tuple(v) if isinstance(v, list) else v


def payload_state(name: str) -> FormState:
    setup, sets, acks = PAYLOAD_STATES[name]
    st = build_state(setup)
    for k, raw in sets:
        run(lambda: st.set(k, UNV(raw)))
    if acks:
        st.acknowledge(*acks)
    return st


def ENC(x):
    """encode_value passes some values through untouched; those are not
    JSON (Python would fail when sending the body)."""
    try:
        json.dumps(x, allow_nan=False)
    except (TypeError, ValueError):
        return {"unencodable": type(x).__name__}
    return x


def BODY(b: dict):
    return {"keys": list(b), "custom_keys": list(b.get("custom_fields", {})), "body": b}


def gen_payload():
    import dataclasses
    out: dict = {"setups": {k: list(v) for k, v in PAYLOAD_STATES.items()}, "states": {}}
    for name in PAYLOAD_STATES:
        st = payload_state(name)
        spec_ = st.spec
        values = st.evaluated.values
        row: dict = {
            "payload_values": [[k, V(v)] for k, v in st.payload_values().items()],
            "excludes_lists": [[a, run(lambda: payload.excludes_lists(st.evaluated, a))] for a in ACCOUNTS],
            "account": [[a, run(lambda: payload._account(st.evaluated, a))] for a in ACCOUNTS],
            "problems": [[s, payload._problems(st, s)] for s in ([], ["requester"], ["overview_deck", "team"])],
            "module_body": [
                [mod, s, run(lambda: list(payload._module_body(st, keys, order, s)))]
                for mod, keys, order in (("change", payload.CHANGE_KEYS, payload._CHANGE_ORDER),
                                         ("ticket", payload.TICKET_KEYS, payload._TICKET_ORDER))
                for s in (None, "Open", 2, "Done")
            ],
            "encode_state": [[k, run(lambda: ENC(payload.encode_value(spec_, spec_.by_name[k], v, values)))]
                             for k, v in st.payload_values().items()],
        }
        if name.startswith(("change", "ticket", "ops")):
            row["ticket"] = [[o, run(lambda: BODY(payload.build_ticket_payload(st, **o)))] for o in MODULE_OPTS]
            row["change"] = [[o, run(lambda: BODY(payload.build_change_payload(st, **o)))] for o in MODULE_OPTS]
        sr = []
        for o in SR_OPTS if name.startswith("sr") else SR_OPTS[:1]:
            kw = {k: v for k, v in o.items() if k != "item"}
            item = st.spec.item
            if "item" in o and item is not None:
                item = dataclasses.replace(item, **o["item"])
            for email in SR_EMAILS if o == {} else SR_EMAILS[:1]:
                sr.append([o, email, run(lambda: BODY(payload.build_sr_payload(item, st, email, **kw)))])
        row["sr"] = sr
        out["states"][name] = row
    # encode_value over every field of each spec, with the filled state's values
    enc = {}
    for name in ("change_filled", "ticket_filled", "ticket_internal", "sr_filled", "ops"):
        st = payload_state(name)
        values = st.evaluated.values
        enc[name] = [[f.api_name, c, run(lambda: ENC(payload.encode_value(st.spec, f, STORED(UNV(c)), values)))]
                     for f in st.spec.ordered() for c in ENCODE_CANDIDATES]
    out["encode_value"] = enc
    lid = []
    f = Field("who", "Who", "lookup_user")
    for c in ({"lookup": [9003, "Sam Lee", None]}, {"lookup": ["77", "S", None]}, 5, True, "x", 2.5, None, [1]):
        u = UNV(c)
        lid.append([c, run(lambda: payload._lookup_id(f, tuple(u) if isinstance(u, list) else u))])
    out["lookup_id"] = lid
    out["name_list"] = [[n, payload._name_list(n)] for n in ([], ["a"], ["a", "b_1"], ["ü", 'q"t'])]
    return {"payload": out}


EXTRA.append(gen_payload)

if __name__ == "__main__":
    main()
