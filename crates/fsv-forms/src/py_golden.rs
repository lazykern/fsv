//! Test-only access to `testdata/py_golden.json` (what the Python reference
//! returns; regenerate with `testdata/gen_py_golden.py`) and
//! `testdata/acme.json` (the Python suite's synthetic form metadata), plus
//! the projections the generator uses so Rust results compare as JSON.

use std::sync::{Arc, OnceLock};

use serde_json::{Map, Value as Json, json};

use crate::errors::FormError;
use crate::pytext::casefold;
use crate::rules::{
    ChoiceRestriction, Conflict, Decision, Dedupe, EXCLUDE_KEYS, EngineOptions, EvaluatedForm,
    EventReverse, EventScope, ExcludeName, Explanation, Message, Model, PopulateMode, RuleContext,
    RulesEngine,
};
use crate::spec::{Choice, Field, FormSpec};
use crate::state::{EventOrder, FormState, LookupResolver, ResolverError, StateOptions};
use crate::tz::TzEnv;
use crate::value::{LookupValue, Value, Values};

pub fn golden() -> &'static Json {
    static G: OnceLock<Json> = OnceLock::new();
    G.get_or_init(|| {
        serde_json::from_str(include_str!("../testdata/py_golden.json")).expect("py_golden.json")
    })
}

pub fn acme() -> &'static Json {
    static A: OnceLock<Json> = OnceLock::new();
    A.get_or_init(|| {
        serde_json::from_str(include_str!("../testdata/acme.json")).expect("acme.json")
    })
}

/// `golden()["a"]["b"]...`, panicking on a missing key.
pub fn g(path: &[&str]) -> &'static Json {
    let mut cur = golden();
    for p in path {
        cur = cur
            .get(*p)
            .unwrap_or_else(|| panic!("py_golden.json: no {p:?} in {path:?}"));
    }
    cur
}

pub fn obj(v: &Json) -> &Map<String, Json> {
    v.as_object().expect("object")
}

pub fn fixture(name: &str) -> &'static Json {
    acme()
        .get(name)
        .unwrap_or_else(|| panic!("acme.json: no {name}"))
}

/// `{"ok": v}` / `{"err": [PythonClass, str(e)]}` like the generator's `run`.
pub fn run<T>(r: crate::Result<T>, f: impl FnOnce(T) -> Json) -> Json {
    match r {
        Ok(v) => json!({ "ok": f(v) }),
        Err(e) => json!({ "err": [py_class(&e), e.to_string()] }),
    }
}

pub fn run_str<T>(r: Result<T, String>, f: impl FnOnce(T) -> Json) -> Json {
    match r {
        Ok(v) => json!({ "ok": f(v) }),
        Err(e) => json!({ "err": ["ValueError", e] }),
    }
}

pub fn py_class(e: &FormError) -> &'static str {
    match e {
        FormError::Other(_) => "FormError",
        FormError::UnknownField { .. } => "UnknownFieldError",
        FormError::ValueParse { .. } => "ValueParseError",
        FormError::AmbiguousLookup { .. } => "AmbiguousLookupError",
        FormError::FieldDisabled(_) => "FieldDisabledError",
        FormError::FormRule {
            unsupported: false, ..
        } => "FormRuleError",
        FormError::FormRule {
            unsupported: true, ..
        } => "UnsupportedRuleError",
        FormError::Payload { .. } => "PayloadError",
        FormError::Value(_) => "ValueError",
        FormError::Type(_) => "TypeError",
        FormError::Lookup(_) => "LookupError",
    }
}

/// Python floats like `2.0` print as JSON `2.0` and ints as `2`; Rust
/// `f64` fields serialize as `2.0` either way, so compare integral floats
/// as ints.
fn num_norm(j: &Json) -> Json {
    match j {
        Json::Number(n) => match n.as_f64() {
            Some(f) if n.is_f64() && f.fract() == 0.0 && f.abs() < 9.0e15 => json!(f as i64),
            _ => j.clone(),
        },
        Json::Array(a) => Json::Array(a.iter().map(num_norm).collect()),
        Json::Object(o) => Json::Object(o.iter().map(|(k, x)| (k.clone(), num_norm(x))).collect()),
        _ => j.clone(),
    }
}

/// Compare with a readable diff on failure.
#[track_caller]
pub fn check(label: &str, actual: &Json, expected: &Json) {
    if num_norm(actual) != num_norm(expected) {
        panic!(
            "{label}: Rust differs from Python\n  rust:   {}\n  python: {}",
            actual, expected
        );
    }
}

/// [`check`] without the int/float leniency (wire payloads).
#[track_caller]
pub fn check_exact(label: &str, actual: &Json, expected: &Json) {
    if actual != expected {
        panic!(
            "{label}: Rust differs from Python\n  rust:   {}\n  python: {}",
            actual, expected
        );
    }
}

/// `DateTimeValue` keeps only the UTC offset, not Python's tzinfo object, so
/// `repr()` of an aware datetime cannot carry `tzinfo=...`.
pub fn strip_tzinfo(expected: &Json) -> Json {
    let Some(msg) = expected["err"][1].as_str() else {
        return expected.clone();
    };
    let Some(start) = msg.find(", tzinfo=") else {
        return expected.clone();
    };
    let mut depth = 0;
    let mut end = msg.len();
    for (i, c) in msg[start..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' if depth == 0 => {
                end = start + i;
                break;
            }
            ')' => depth -= 1,
            _ => {}
        }
    }
    json!({"err": [expected["err"][0], format!("{}{}", &msg[..start], &msg[end..])]})
}

// ---------------------------------------------------------------------------
// projections (mirror gen_py_golden.py)

pub fn v(value: &Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Bool(b) => json!(b),
        Value::Int(i) => json!(i),
        Value::Float(f) if !f.is_finite() => json!({ "float": fsv_py::float_repr(*f) }),
        Value::Float(f) => json!(f),
        Value::Str(s) => json!(s),
        Value::List(items) => Json::Array(items.iter().map(v).collect()),
        Value::Lookup(l) => lookup(l),
        Value::Date(d) => json!({ "date": crate::pydt::date_iso(*d) }),
        Value::DateTime(d) => json!({ "datetime": d.isoformat() }),
        Value::Json(j) => j.clone(),
    }
}

pub fn opt_v(value: Option<&Value>) -> Json {
    value.map(v).unwrap_or(Json::Null)
}

pub fn lookup(l: &LookupValue) -> Json {
    json!({ "lookup": [l.id, l.display, l.email] })
}

pub fn vs(values: &Values) -> Json {
    Json::Object(values.iter().map(|(k, x)| (k.clone(), v(x))).collect())
}

/// Python-encoded value (as `V()` emits it) -> [`Value`].
pub fn from_v(j: &Json) -> Value {
    match j {
        Json::Object(o) if o.len() == 1 && o.contains_key("lookup") => {
            let a = o["lookup"].as_array().expect("lookup triple");
            Value::Lookup(LookupValue {
                id: a[0].clone(),
                display: a[1].as_str().unwrap_or("").to_string(),
                email: a[2].as_str().map(str::to_string),
            })
        }
        Json::Object(o) if o.len() == 1 && o.contains_key("date") => Value::Date(
            crate::pydt::date_fromisoformat(o["date"].as_str().expect("date")).expect("iso date"),
        ),
        Json::Object(o) if o.len() == 1 && o.contains_key("datetime") => {
            let (local, off) =
                crate::pydt::datetime_fromisoformat(o["datetime"].as_str().expect("datetime"))
                    .expect("iso datetime");
            Value::DateTime(crate::pydt::DateTimeValue::new(
                local,
                off.expect("aware datetime"),
            ))
        }
        Json::Array(a) => Value::List(a.iter().map(from_v).collect()),
        other => Value::from_json(other),
    }
}

pub fn values_from_v(j: &Json) -> Values {
    obj(j).iter().map(|(k, x)| (k.clone(), from_v(x))).collect()
}

pub fn choice(c: &Choice) -> Json {
    json!({
        "value": c.value, "label": c.label, "id": c.id, "display_id": c.display_id,
        "description": c.description, "visible": c.visible,
        "children": c.children.iter().map(choice).collect::<Vec<_>>(),
    })
}

pub fn choices(cs: &[Choice]) -> Json {
    Json::Array(cs.iter().map(choice).collect())
}

pub fn field(f: &Field) -> Json {
    json!({
        "api_name": f.api_name, "label": f.label, "kind": f.kind.as_str(),
        "choices": choices(&f.choices),
        "required_base": f.required_base, "position": f.position, "helptext": f.helptext,
        "help": f.help(), "hint": f.hint, "placeholder": f.placeholder,
        "description": f.description, "default": opt_v(f.default.as_ref()),
        "is_custom": f.is_custom, "field_type": f.field_type, "id": f.id, "multi": f.multi,
        "visible_base": f.visible_base, "on_form": f.on_form, "content": f.content,
        "parent": f.parent, "level": f.level, "levels": f.levels,
        "sections": f.sections.iter().map(|s| json!({"choices": s.choices, "fields": s.fields})).collect::<Vec<_>>(),
        "date_only": f.date_only, "lookup_entity": f.lookup_entity, "lookup_link": f.lookup_link,
        "lookup_config": f.lookup_config, "allowed_file_types": f.allowed_file_types,
        "max_files": f.max_files, "sent": f.sent, "synthetic": f.synthetic,
        "is_input": f.is_input(), "is_nested_level": f.is_nested_level(),
    })
}

pub fn spec(s: &FormSpec) -> Json {
    json!({
        "module": s.module, "account_id": s.account_id, "warnings": s.warnings,
        "fields": s.fields().iter().map(field).collect::<Vec<_>>(),
        "item": s.item.as_ref().map(|i| json!({
            "id": i.id, "display_id": i.display_id, "name": i.name,
            "short_description": i.short_description, "category_name": i.category_name,
            "description_html": i.description_html, "allow_attachments": i.allow_attachments,
            "attachment_mandatory": i.attachment_mandatory, "allow_quantity": i.allow_quantity,
            "subject_template": i.subject_template,
        })),
    })
}

pub fn decision(d: &Decision) -> Json {
    json!([
        d.rule_id,
        d.rule_name,
        d.action,
        d.matched,
        d.event,
        d.rule_index
    ])
}

pub fn opt_decision(d: Option<&Decision>) -> Json {
    d.map(decision).unwrap_or(Json::Null)
}

pub fn message(m: &Message) -> Json {
    json!([m.text, m.rule_id, m.rule_name, m.blocking])
}

pub fn exclude(x: &ExcludeName) -> Json {
    json!([x.base, x.custom])
}

pub fn restriction(r: &ChoiceRestriction) -> Json {
    json!({
        "allowed": r.allowed.iter().map(|(k, c)| (k.clone(), c.as_ref().map(restriction).unwrap_or(Json::Null))).collect::<Map<_, _>>(),
        "rule_id": r.rule_id, "rule_name": r.rule_name,
    })
}

pub fn conflict(c: &Conflict) -> Json {
    json!({
        "field": c.field, "property": c.property,
        "decisions": c.decisions.iter().map(decision).collect::<Vec<_>>(),
        "winner": decision(&c.winner), "describe": c.describe(),
    })
}

pub fn explanation(e: &Explanation) -> Json {
    json!([
        e.field,
        e.label,
        e.property,
        e.value,
        e.source,
        opt_decision(e.decision.as_ref()),
        e.to_string()
    ])
}

pub fn evaluated(ev: &EvaluatedForm) -> Json {
    let mut required: Vec<&String> = ev.required.iter().collect();
    required.sort();
    let mut disabled: Vec<&String> = ev.disabled.iter().collect();
    disabled.sort();
    json!({
        "visible": ev.visible, "required": required, "disabled": disabled,
        "values": vs(&ev.values), "populated": ev.populated,
        "messages": ev.messages.iter().map(message).collect::<Vec<_>>(),
        "matched": ev.matched, "model": ev.model.as_str(),
        "excludes": EXCLUDE_KEYS.iter().map(|k| (k.to_string(), Json::Array(ev.excludes.get(k).unwrap_or_default().iter().map(exclude).collect()))).collect::<Map<_, _>>(),
        "decisions": ev.decisions.iter().map(|((n, p), d)| json!([n, p, decision(d)])).collect::<Vec<_>>(),
        "conflicts": ev.conflicts.iter().map(conflict).collect::<Vec<_>>(),
        "refused": ev.refused.iter().map(|(n, d)| json!([n, opt_decision(d.as_ref())])).collect::<Vec<_>>(),
        "overridden": ev.overridden.iter().map(|(k, (a, b))| (k.clone(), json!([v(a), b]))).collect::<Map<_, _>>(),
        "attachments_required": ev.attachments_required, "account_id": ev.account_id,
        "restricted": ev.restricted.iter().map(|(k, r)| (k.clone(), restriction(r))).collect::<Map<_, _>>(),
        "restricted_by": ev.restricted_by.iter().map(|(k, d)| (k.clone(), decision(d))).collect::<Map<_, _>>(),
        "unavailable": ev.unavailable.iter().map(|(k, (a, d))| (k.clone(), json!([v(a), opt_decision(d.as_ref())]))).collect::<Map<_, _>>(),
        "populate_mode": ev.populate_mode.as_str(),
    })
}

// ---------------------------------------------------------------------------
// fixtures (tests/forms/conftest.py)

pub fn change_spec() -> Arc<FormSpec> {
    Arc::new(FormSpec::from_form_fields(fixture("change_fields"), "change").expect("change spec"))
}

pub fn ops_spec() -> Arc<FormSpec> {
    Arc::new(FormSpec::from_form_fields(fixture("ops_fields"), "change").expect("ops spec"))
}

pub fn sr_spec() -> Arc<FormSpec> {
    Arc::new(FormSpec::from_service_item(fixture("service_item")).expect("sr spec"))
}

pub fn ticket_spec() -> Arc<FormSpec> {
    Arc::new(FormSpec::from_form_fields(fixture("ticket_fields"), "ticket").expect("ticket spec"))
}

pub fn ticket_internal_spec() -> Arc<FormSpec> {
    Arc::new(
        FormSpec::from_form_fields(fixture("ticket_internal_fields"), "ticket")
            .expect("ticket internal spec"),
    )
}

pub fn legacy(module: &str) -> EngineOptions {
    EngineOptions {
        module: Some(module.to_string()),
        ..EngineOptions::legacy()
    }
}

// ---------------------------------------------------------------------------
// engines (gen_py_golden.py ENGINES)

pub fn spec_of(name: &str) -> Arc<FormSpec> {
    match name {
        "change" => change_spec(),
        "ops" => ops_spec(),
        "ticket_internal" => ticket_internal_spec(),
        "ticket" => ticket_spec(),
        "sr" => sr_spec(),
        other => panic!("no spec {other}"),
    }
}

pub fn portal() -> Option<Map<String, Json>> {
    json!({"portal": 9_000_006_001_i64}).as_object().cloned()
}

pub fn engine_parts(name: &str) -> (&'static str, &'static str, EngineOptions) {
    let legacy = || legacy("change");
    let default = |m: &str| EngineOptions::default().module(m);
    match name {
        "change_legacy" => ("change", "change_rules", legacy()),
        "change_default" => ("change", "change_rules", default("change")),
        "change_replay" => (
            "change",
            "change_rules",
            EngineOptions {
                model: Model::Replay,
                populate: PopulateMode::Set,
                ..default("change")
            },
        ),
        "change_requester" => (
            "change",
            "change_rules",
            EngineOptions {
                user_type: "requester".into(),
                ..legacy()
            },
        ),
        "change_dedupe_none" | "change_dedupe_first" | "change_dedupe_last" => (
            "change",
            "change_rules",
            EngineOptions {
                dedupe: Dedupe::parse(name.rsplit('_').next().unwrap()).unwrap(),
                ..legacy()
            },
        ),
        "change_same_fires" => (
            "change",
            "change_rules",
            EngineOptions {
                same_value_fires: true,
                ..legacy()
            },
        ),
        "ops" => (
            "ops",
            "ops_rules",
            EngineOptions {
                workspace_id: Some(json!(2)),
                ..legacy()
            },
        ),
        "ops_default" => (
            "ops",
            "ops_rules",
            EngineOptions {
                workspace_id: Some(json!(2)),
                ..default("change")
            },
        ),
        "ops_any_ws" => (
            "ops",
            "ops_rules",
            EngineOptions {
                item_id: json!(55),
                user_type: "any".into(),
                ..legacy()
            },
        ),
        "restrict" => ("change", "restrict_rules", default("change")),
        "restrict_transition" => (
            "change",
            "restrict_rules",
            EngineOptions {
                event_reverse: EventReverse::Transition,
                event_scope: EventScope::Referenced,
                ..default("change")
            },
        ),
        "ticket" => ("ticket_internal", "ticket_rules", default("ticket")),
        "ticket_portal" => (
            "ticket_internal",
            "ticket_rules",
            EngineOptions {
                system: portal(),
                ..default("ticket")
            },
        ),
        "ticket_portal_edit_ctx" => (
            "ticket_internal",
            "ticket_rules",
            EngineOptions {
                context: Some(RuleContext {
                    module: Some("ticket".into()),
                    form: "edit".into(),
                    user_type: "agent".into(),
                    requester: json!({"id": 1}).as_object().cloned(),
                    ..RuleContext::default()
                }),
                system: portal(),
                ..EngineOptions::default()
            },
        ),
        other => panic!("no engine {other}"),
    }
}

pub fn engine(name: &str) -> RulesEngine {
    let (s, r, opts) = engine_parts(name);
    RulesEngine::new(spec_of(s), Some(fixture(r)), opts).expect("engine")
}

/// conftest.py `FakeResolver`.
pub struct FakeResolver;

impl LookupResolver for FakeResolver {
    fn lookup(
        &self,
        entity: &str,
        query: &str,
        _field: &Field,
    ) -> Result<Vec<LookupValue>, ResolverError> {
        let records: Vec<LookupValue> = match entity {
            "user" => vec![
                LookupValue::new(9001, "Jane Doe", Some("jane.doe@acme.example")),
                LookupValue::new(9002, "Jane Doerr", Some("jane.doerr@acme.example")),
                LookupValue::new(9003, "Sam Lee", Some("sam.lee@acme.example")),
            ],
            "group" => vec![
                LookupValue::new(601, "Platform Ops", None),
                LookupValue::new(602, "Platform", None),
            ],
            "agent" => vec![LookupValue::new(
                701,
                "Alex Kim",
                Some("alex.kim@acme.example"),
            )],
            "change_window" => vec![LookupValue::new(801, "Weekend window", None)],
            _ => vec![],
        };
        let q = casefold(query);
        Ok(records
            .into_iter()
            .filter(|r| q == fsv_py::py_str(&r.id) || casefold(&r.to_string()).contains(&q))
            .collect())
    }
}

/// `build_state(setup)` of the generator.
pub fn build_state(setup: &Json) -> crate::Result<FormState> {
    let (spec, rules) = match setup.get("engine").and_then(Json::as_str) {
        Some(name) => {
            let e = engine(name);
            (e.spec.clone(), Some(e))
        }
        None => (spec_of(setup["spec"].as_str().expect("spec")), None),
    };
    let values = setup.get("values").map(values_from_v).unwrap_or_default();
    let opts = StateOptions {
        resolver: if setup
            .get("resolver")
            .and_then(Json::as_bool)
            .unwrap_or(true)
        {
            Some(Arc::new(FakeResolver) as Arc<dyn LookupResolver>)
        } else {
            None
        },
        tz: Some(
            setup
                .get("tz")
                .and_then(Json::as_str)
                .unwrap_or("UTC")
                .to_string(),
        ),
        apply_defaults: setup
            .get("apply_defaults")
            .and_then(Json::as_bool)
            .unwrap_or(true),
        existing: setup.get("existing").map(values_from_v),
        event_order: EventOrder::parse(
            setup
                .get("event_order")
                .and_then(Json::as_str)
                .unwrap_or("position"),
        )
        .expect("event order"),
        tz_env: Some(TzEnv::empty()),
    };
    let mode = setup.get("mode").and_then(Json::as_str).unwrap_or("new");
    FormState::new(spec, rules, mode, Some(&values), opts)
}
