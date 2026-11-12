//! Pre-send change lifecycle checks from the runtime state flow
//! (`change_lifecycle.py`).
//!
//! The state flow (`change_management/<id>?include=state_info`) carries
//! `transitions` (`{"<from state>": [{"to_state", "entry_criteria"}]}`, a
//! criterion being `{name, operator, value}` or
//! `{"fields": {"<field id>": {name, operator, value}}}`) and
//! `state_properties.field_lock_config` (`{"<state>": [{"<field id>":
//! "status" | "customFields.<name>" | ...}]}`). The server rejects disallowed
//! edges, unmet criteria and locked fields with a bare HTTP 400, so a dry-run
//! checks them first. Only criteria fsv can evaluate refuse; the rest warn.

use std::collections::{BTreeSet, HashMap};

use serde_json::{Map, Value};

use fsv_py::{py_repr, py_str};

/// Display label of a criterion or lock field name.
pub type Label<'a> = &'a dyn Fn(&str) -> String;

fn plain(name: &str) -> &str {
    name.strip_prefix("customFields.").unwrap_or(name)
}

/// `_plain` as a [`Label`].
pub fn plain_label(name: &str) -> String {
    plain(name).to_string()
}

/// `state_names`: state id (as text) -> state name.
pub fn state_names(flow: &Value) -> HashMap<String, String> {
    flow.get("state_list")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .map(|s| {
            let text = |k: &str| s.get(k).map_or("None".to_string(), py_str);
            (text("id"), text("value"))
        })
        .collect()
}

/// `criteria`: both native entry-criteria shapes as `{name, operator, value}`.
pub fn criteria(transition: &Map<String, Value>) -> Vec<Map<String, Value>> {
    let mut out = Vec::new();
    for entry in transition
        .get("entry_criteria")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
    {
        if let Some(fields) = entry.get("fields").and_then(Value::as_object) {
            out.extend(
                fields
                    .values()
                    .filter_map(Value::as_object)
                    .filter(|c| c.get("name").is_some_and(fsv_py::truthy))
                    .cloned(),
            );
        } else if entry.get("name").is_some_and(fsv_py::truthy) {
            out.push(entry.clone());
        }
    }
    out
}

/// `locks`: the field lock names configured for `state`.
pub fn locks(flow: &Value, state: &Value) -> Vec<String> {
    flow.get("state_properties")
        .and_then(Value::as_object)
        .and_then(|p| p.get("field_lock_config"))
        .and_then(Value::as_object)
        .and_then(|config| config.get(&py_str(state)))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .flat_map(|row| row.values().map(py_str))
        .collect()
}

fn unset(value: Option<&Value>) -> bool {
    matches!(value, None | Some(Value::Null)) || value == Some(&Value::String(String::new()))
}

/// `describe`: one criterion as text.
pub fn describe(criterion: &Map<String, Value>, label: Label<'_>) -> String {
    let name = criterion.get("name").map_or("None".to_string(), py_str);
    let op = criterion.get("operator");
    let value = criterion.get("value");
    if op.and_then(Value::as_str) == Some("is_not") && unset(value) {
        return format!("{} must be set", label(&name));
    }
    format!(
        "{} {} {}",
        label(&name),
        op.map_or("None".to_string(), py_str).replace('_', " "),
        py_repr(value.unwrap_or(&Value::Null))
    )
}

/// `(known, value)` of a criterion field after the write.
fn field_value<'a>(
    name: &str,
    record: &'a Map<String, Value>,
    body: &'a Map<String, Value>,
) -> Option<&'a Value> {
    if let Some(key) = name.strip_prefix("customFields.") {
        return [body.get("custom_fields"), record.get("custom_fields")]
            .into_iter()
            .flatten()
            .filter_map(Value::as_object)
            .find_map(|source| source.get(key));
    }
    body.get(name).or_else(|| record.get(name))
}

fn met(
    criterion: &Map<String, Value>,
    record: &Map<String, Value>,
    body: &Map<String, Value>,
) -> Option<bool> {
    let name = criterion.get("name").map_or("None".to_string(), py_str);
    let actual = field_value(&name, record, body)?;
    let op = criterion.get("operator").and_then(Value::as_str);
    let value = criterion.get("value");
    let empty = actual.is_null()
        || actual == &Value::String(String::new())
        || actual.as_array().is_some_and(Vec::is_empty);
    let text = |v: Option<&Value>| v.map_or("None".to_string(), py_str);
    match op {
        Some("is_not") if unset(value) => Some(!empty),
        Some("is") => Some(!empty && py_str(actual) == text(value)),
        Some("is_not") => Some(py_str(actual) != text(value)),
        _ => None,
    }
}

/// Lock-config names of each sent key (`department_id` -> `department`).
fn lock_names(body: &Map<String, Value>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (key, value) in body {
        if key == "custom_fields"
            && let Some(custom) = value.as_object()
        {
            out.extend(custom.keys().map(|k| format!("customFields.{k}")));
        } else if key != "br_validation_excludes" {
            out.insert(key.strip_suffix("_id").unwrap_or(key).to_string());
            out.insert(key.clone());
        }
    }
    out
}

/// `precheck`: `(errors, warnings)` for sending `body` to a change in
/// `record`'s state.
pub fn precheck(
    flow: &Value,
    record: &Map<String, Value>,
    body: &Map<String, Value>,
    label: Label<'_>,
) -> (Vec<String>, Vec<String>) {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let names = state_names(flow);
    let current = record.get("status").cloned().unwrap_or(Value::Null);
    let current_text = py_str(&current);
    let name_of = |id: &str| names.get(id).cloned().unwrap_or_else(|| id.to_string());
    let here = name_of(&current_text);
    let locked: BTreeSet<String> = locks(flow, &current).into_iter().collect();
    let hit: BTreeSet<String> = lock_names(body)
        .iter()
        .filter(|n| locked.contains(*n))
        .map(|n| label(plain(n)))
        .collect();
    if !hit.is_empty() {
        errors.push(format!(
            "locked in lifecycle state {here}: {} (field lock); the server rejects this write",
            hit.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    let Some(target) = body.get("status").filter(|t| py_str(t) != current_text) else {
        return (errors, warnings);
    };
    let there = name_of(&py_str(target));
    let Some(edges) = flow
        .get("transitions")
        .and_then(Value::as_object)
        .and_then(|t| t.get(&current_text))
    else {
        warnings.push(format!(
            "lifecycle transitions from {here} unavailable; the edge {here} → {there} was not pre-checked"
        ));
        return (errors, warnings);
    };
    let edges: Vec<&Map<String, Value>> = edges
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .collect();
    let to_state = |t: &Map<String, Value>| t.get("to_state").map_or("None".to_string(), py_str);
    let Some(edge) = edges.iter().find(|t| to_state(t) == py_str(target)) else {
        let allowed = edges
            .iter()
            .map(|t| name_of(&to_state(t)))
            .collect::<Vec<_>>()
            .join(", ");
        let allowed = if allowed.is_empty() {
            "none".to_string()
        } else {
            allowed
        };
        errors.push(format!(
            "{here} → {there} is not an allowed lifecycle transition (allowed from {here}: {allowed})"
        ));
        return (errors, warnings);
    };
    for criterion in criteria(edge) {
        match met(&criterion, record, body) {
            Some(false) => {
                let hint = if criterion.get("operator").and_then(Value::as_str) == Some("is_not") {
                    "; supply it with --set"
                } else {
                    ""
                };
                errors.push(format!(
                    "entry criterion for {here} → {there} not met: {}{hint}",
                    describe(&criterion, label)
                ));
            }
            None => warnings.push(format!(
                "entry criterion for {here} → {there} not pre-checked: {}",
                describe(&criterion, label)
            )),
            Some(true) => {}
        }
    }
    (errors, warnings)
}

/// `outgoing`: `[(to-state name, [criterion descriptions])]` for display.
pub fn outgoing(flow: &Value, state: &Value, label: Label<'_>) -> Vec<(String, Vec<String>)> {
    let names = state_names(flow);
    flow.get("transitions")
        .and_then(Value::as_object)
        .and_then(|t| t.get(&py_str(state)))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .map(|t| {
            let to = t.get("to_state").map_or("None".to_string(), py_str);
            let name = names.get(&to).cloned().unwrap_or(to);
            (
                name,
                criteria(t).iter().map(|c| describe(c, label)).collect(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// `change_lifecycle.precheck` / `outgoing` (Python 7530df6).
    #[test]
    fn precheck_and_outgoing_match_python() {
        let oracle: Value =
            serde_json::from_str(include_str!("testdata/change_lifecycle_oracle.json")).unwrap();
        let flow = &oracle["flow"];
        let cases = oracle["precheck"].as_array().unwrap();
        assert!(cases.len() >= 10);
        for case in cases {
            let (errors, warnings) = precheck(
                flow,
                case["record"].as_object().unwrap(),
                case["body"].as_object().unwrap(),
                &plain_label,
            );
            assert_eq!(
                json!([errors, warnings]),
                case["expected"],
                "{}",
                case["name"]
            );
        }
        for case in oracle["outgoing"].as_array().unwrap() {
            let got: Vec<Value> = outgoing(flow, &case["state"], &plain_label)
                .into_iter()
                .map(|(n, c)| json!([n, c]))
                .collect();
            assert_eq!(Value::Array(got), case["expected"], "{}", case["state"]);
        }
        for case in oracle["locks"].as_array().unwrap() {
            assert_eq!(json!(locks(flow, &case["state"])), case["expected"]);
        }
    }
}
