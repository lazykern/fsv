//! The Properties pane (`views.load_properties`, `load_edit_rules`,
//! `_prop_value`): form fields in web order, visible per the edit-form
//! business rules, with display values. The result is what
//! [`crate::load_detail`] takes as [`crate::Properties`].

use std::collections::HashSet;
use std::sync::Arc;

use jiff::tz::TimeZone;
use serde_json::{Map, Value, json};

use fsv_api::formmeta::{self, ClientResolver, MetaStore, RULES_UNAVAILABLE_KEY};
use fsv_api::service::Api;
use fsv_api::site::{self, Prop};
use fsv_core::error::Error;
use fsv_core::resource::Resource;
use fsv_forms::fetch::BusinessRulesOptions;
use fsv_forms::rules::EngineOptions;
use fsv_forms::{
    Field, FormSpec, FormState, Kind, LookupValue, RulesEngine, StateOptions, Value as FormValue,
};
use fsv_py::{py_str, truthy};

use crate::source::{SessionError, record_for_form};

const SKIP_PROPS: [&str; 2] = ["subject", "description"];

/// `MODULE_OF.get(name, name.rstrip("s"))`.
fn module_of(name: &str) -> String {
    match name {
        "changes" => "change".into(),
        "tickets" => "ticket".into(),
        "problems" => "problem".into(),
        _ => name.trim_end_matches('s').into(),
    }
}

/// `value in (None, "")`.
fn blank(v: &Value) -> bool {
    v.is_null() || v.as_str() == Some("")
}

/// `load_edit_rules`: edit-form business rules, cached per record (the server
/// filters them by `module_id`); `(None, note)` when the account has none or
/// will not show them.
pub fn load_edit_rules(
    c: &dyn Api,
    store: &dyn MetaStore,
    module: &str,
    record: &Value,
    raw_fields: &[Value],
) -> Result<(Option<Value>, Option<String>), SessionError> {
    let ws = record.get("workspace_id").cloned().unwrap_or(Value::Null);
    let fws = formmeta::workspace_from_fields(raw_fields);
    let mid = record
        .get("display_id")
        .filter(|v| truthy(v))
        .or_else(|| record.get("id"))
        .cloned()
        .unwrap_or(Value::Null);
    let configured = store.configured_workspace();
    let place = if truthy(&configured) {
        configured
    } else if !blank(&ws) {
        ws.clone()
    } else {
        fws.clone()
    };
    let place = if blank(&place) {
        "none".to_string()
    } else {
        py_str(&place)
    };
    let key = format!("{module}-rules-edit_form-ws{place}-{}", py_str(&mid));
    if let Some(cached @ Value::Object(_)) = store.cached(&key) {
        return Ok((Some(cached), None));
    }
    let query = BusinessRulesOptions {
        form_type: "edit".into(),
        module_id: mid,
        ..Default::default()
    };
    let rules = match formmeta::load_rules(c, store, module, &[ws, fws], &query) {
        Ok(rules) => rules,
        Err(Error::Session(msg)) => return Err(SessionError(msg)),
        Err(e) => {
            let note = match e.as_api() {
                Some(api) => format!(
                    "static field visibility (business rules: unavailable ({}))",
                    api.status
                ),
                None => "static field visibility (business rules unreadable)".into(),
            };
            return Ok((None, Some(note)));
        }
    };
    if rules.get(RULES_UNAVAILABLE_KEY).is_some_and(truthy) {
        let status = rules.get("unavailable_status").filter(|v| truthy(v));
        let tail = status.map_or(")".to_string(), |s| format!(" ({}))", py_str(s)));
        return Ok((
            None,
            Some(format!(
                "static field visibility (business rules: not available on this account{tail}"
            )),
        ));
    }
    if !rules.get("business_rules").is_some_and(truthy) {
        return Ok((None, None));
    }
    store.save(&key, "form_rules", &rules);
    Ok((Some(rules), None))
}

/// `v in (None, "", [], (), {})`.
fn empty_value(v: &FormValue) -> bool {
    match v {
        FormValue::Null | FormValue::Json(Value::Null) => true,
        FormValue::Str(s) | FormValue::Json(Value::String(s)) => s.is_empty(),
        FormValue::List(l) => l.is_empty(),
        FormValue::Json(Value::Array(a)) => a.is_empty(),
        FormValue::Json(Value::Object(o)) => o.is_empty(),
        _ => false,
    }
}

/// `str(x.display or x.id)`.
fn lookup_text(l: &LookupValue) -> String {
    if l.display.is_empty() {
        py_str(&l.id)
    } else {
        l.display.clone()
    }
}

/// `_prop_value`: the display text of one field's effective value.
fn prop_value(state: &FormState, f: &Field, tz: Option<&TimeZone>) -> String {
    let Some(v) = state
        .evaluated
        .values
        .get(&f.api_name)
        .filter(|v| !empty_value(v))
    else {
        return String::new();
    };
    if matches!(f.kind, Kind::Date | Kind::Datetime)
        && let Some(s) = v.as_str()
    {
        if f.kind == Kind::Datetime {
            return site::fmt_when(&json!(s), tz, true);
        }
        let head: String = s.chars().take(10).collect();
        let shown = if site::is_date_only(&json!(head)) {
            head
        } else {
            s.to_string()
        };
        return site::fmt_when(&json!(shown), tz, true);
    }
    if let Some(items) = v.as_list()
        && !items.is_empty()
        && items.iter().all(|x| matches!(x, FormValue::Lookup(_)))
    {
        let shown: Vec<String> = items
            .iter()
            .filter_map(|x| match x {
                FormValue::Lookup(l) => Some(lookup_text(l)),
                _ => None,
            })
            .collect();
        return shown.join(", ");
    }
    if let FormValue::Lookup(l) = v {
        return lookup_text(l);
    }
    match state.display(&f.api_name, None) {
        Ok(text) if matches!(f.kind, Kind::Textarea | Kind::Richtext) => {
            site::one_line(&json!(text))
        }
        Ok(text) => text,
        Err(_) => v.py_str(),
    }
}

/// `load_properties`: form fields in web order, visible per the edit-form
/// rules, with display values (names, not ids), and a note when the rules
/// could not be used. Falls back to [`site::flat_properties`] when the form
/// cannot be evaluated.
pub fn load_properties(
    c: Arc<dyn Api + Send + Sync>,
    store: &dyn MetaStore,
    res: &Resource,
    record: &Value,
    schema: Option<&Value>,
    tz: Option<&TimeZone>,
    user: Option<&Value>,
) -> Result<(Vec<Prop>, Option<String>), SessionError> {
    let raw_fields: Vec<Value> = schema
        .and_then(|s| s.get("fields"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if raw_fields.is_empty() {
        return Ok((Vec::new(), Some("no form fields".into())));
    }
    let module = module_of(res.name);
    let data = json!({ res.form_fields_key: raw_fields });
    let Ok(spec) = FormSpec::from_form_fields(&data, &module).map(Arc::new) else {
        return Ok((
            site::flat_properties(record, schema),
            Some("form metadata unreadable; fields shown as stored".into()),
        ));
    };
    let (rules, note) = load_edit_rules(c.as_ref(), store, &module, record, &raw_fields)?;
    let rules = rules.unwrap_or_else(|| json!({"business_rules": []}));
    let empty = Map::new();
    let rec = record_for_form(res, record.as_object().unwrap_or(&empty));
    let engine = || {
        RulesEngine::new(
            spec.clone(),
            Some(&rules),
            EngineOptions {
                module: Some(module.clone()),
                workspace_id: record.get("workspace_id").filter(|v| !v.is_null()).cloned(),
                ..Default::default()
            },
        )
    };
    let existing = formmeta::existing_from_record(&spec, &rec);
    let existing =
        formmeta::name_existing(existing, &spec, &rec, Some(&ClientResolver::new(c.clone())));
    let tzname = user
        .and_then(|u| u.get("time_zone"))
        .filter(|v| truthy(v))
        .map(py_str);
    let state = |tz: Option<String>| {
        let opts = StateOptions {
            tz,
            existing: Some(existing.clone()),
            ..Default::default()
        };
        FormState::new(spec.clone(), Some(engine()?), "edit", None, opts)
    };
    let state = match state(tzname) {
        Err(e) if e.is_value_error() => state(None),
        other => other,
    };
    let Ok(state) = state else {
        return Ok((
            site::flat_properties(record, schema),
            Some("form rules could not be applied; fields shown as stored".into()),
        ));
    };
    let visible: HashSet<&str> = state
        .visible_fields()
        .into_iter()
        .map(|f| f.api_name.as_str())
        .collect();
    let mut out = Vec::new();
    for f in spec.ordered() {
        if !visible.contains(f.api_name.as_str())
            || SKIP_PROPS.contains(&f.api_name.as_str())
            || matches!(f.kind, Kind::Notice | Kind::Attachment)
        {
            continue;
        }
        let value = prop_value(&state, f, tz);
        if f.synthetic && value.is_empty() {
            continue;
        }
        out.push(Prop {
            name: f.api_name.clone(),
            label: f.label.clone(),
            value,
            required: state.evaluated.required.contains(&f.api_name),
        });
    }
    Ok((out, note))
}

#[cfg(test)]
mod golden;
