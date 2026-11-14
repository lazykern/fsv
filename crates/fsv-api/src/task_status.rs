//! Runtime task-status choices (`task_status.py`).
//!
//! Ids and labels come from the tenant. Nothing here assumes Open=1.

use serde_json::Value;

use fsv_core::pystr;

use crate::QueryError;

/// One active choice: numeric id and the label Freshservice shows.
pub type Choice = (i64, String);

/// `active_choices`. Skips deleted rows. Fails closed on a missing, duplicate
/// or unreadable status field.
pub fn active_choices(fields: &[Value]) -> Result<Vec<Choice>, QueryError> {
    active_choices_of(fields, "task")
}

/// `active_choices(fields, kind)`: `kind` names the record in messages.
pub fn active_choices_of(fields: &[Value], kind: &str) -> Result<Vec<Choice>, QueryError> {
    let statuses: Vec<&Value> = fields
        .iter()
        .filter(|f| {
            f.as_object()
                .is_some_and(|o| o.get("name").and_then(Value::as_str) == Some("status"))
        })
        .collect();
    if statuses.len() != 1 {
        return Err(err(&format!(
            "exactly one runtime {kind}-status field is required"
        )));
    }
    let mut result = Vec::new();
    // Iterating a non-empty dict or string yields non-dict items.
    match statuses[0].get("choices") {
        Some(Value::Object(o)) if !o.is_empty() => return Err(err("invalid task-status choice")),
        Some(Value::String(s)) if !s.is_empty() => return Err(err("invalid task-status choice")),
        _ => {}
    }
    for choice in statuses[0]
        .get("choices")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(choice) = choice.as_object() else {
            return Err(err("invalid task-status choice"));
        };
        if ["deleted", "is_deleted", "deleted_at", "deactivated"]
            .iter()
            .any(|k| truthy(choice.get(*k)))
        {
            continue;
        }
        let ident = choice.get("id");
        let label = ["value", "label", "name"]
            .iter()
            .find_map(|k| choice.get(*k).filter(|v| truthy(Some(v))))
            .and_then(Value::as_str);
        let Some((id, label)) = ident
            .and_then(positive_id)
            .zip(label.map(pystr::strip).filter(|s| !s.is_empty()))
        else {
            return Err(err("invalid active task-status identity"));
        };
        result.push((id, label.to_string()));
    }
    let mut ids: Vec<i64> = result.iter().map(|(i, _)| *i).collect();
    let mut labels: Vec<String> = result.iter().map(|(_, l)| pystr::casefold(l)).collect();
    ids.sort_unstable();
    ids.dedup();
    labels.sort();
    labels.dedup();
    if ids.len() != result.len() || labels.len() != result.len() {
        return Err(err("ambiguous task-status choices"));
    }
    Ok(result)
}

/// `resolve`: the text must be one active id or one label (case-insensitive).
pub fn resolve(value: &str, fields: &[Value]) -> Result<Choice, QueryError> {
    resolve_of(value, fields, "task")
}

/// `resolve(value, fields, kind)`: `kind` names the record in messages
/// ("task", "problem").
pub fn resolve_of(value: &str, fields: &[Value], kind: &str) -> Result<Choice, QueryError> {
    let key = pystr::casefold(pystr::strip(value));
    let choices = active_choices_of(fields, kind)?;
    let matches: Vec<Choice> = choices
        .iter()
        .filter(|(id, label)| key == id.to_string() || key == pystr::casefold(label))
        .cloned()
        .collect();
    if matches.len() != 1 {
        let options: Vec<&str> = choices.iter().map(|(_, l)| l.as_str()).collect();
        return Err(err(&format!(
            "status must match one active runtime {kind}-status ID or label; {} is not one (options: {})",
            fsv_py::py_repr_str(value),
            options.join(", ")
        )));
    }
    Ok(matches[0].clone())
}

/// Pull the field list out of a `change_task_fields` response.
pub fn fields_from_payload(data: &Value) -> Result<Vec<Value>, QueryError> {
    for key in ["change_task_fields", "task_fields", "fields"] {
        if let Some(list) = data.get(key).and_then(Value::as_array) {
            return Ok(list.clone());
        }
    }
    Err(err("unreadable runtime task-status metadata"))
}

/// `display`. Unknown values render as `3 (unknown)` rather than a guessed label.
pub fn display(value: &Value, fields: Option<&[Value]>) -> String {
    if let Some(obj) = value.as_object() {
        // `name or label or value or id`: the first truthy value, else
        // `str` of `id` (`"None"` when that key is missing, `"0"` when it is 0).
        let mut result = Value::Null;
        for key in ["name", "label", "value", "id"] {
            let v = obj.get(key).cloned().unwrap_or(Value::Null);
            let stop = truthy(Some(&v));
            result = v;
            if stop {
                break;
            }
        }
        return scalar(&result);
    }
    match fields.and_then(|f| resolve(&scalar(value), f).ok()) {
        Some((_, label)) => label,
        None => format!("{} (unknown)", scalar(value)),
    }
}

fn positive_id(value: &Value) -> Option<i64> {
    if value.is_boolean() {
        return None;
    }
    let text = match value {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        _ => return None,
    };
    if !pystr::is_digit(&text) {
        return None;
    }
    pystr::parse_decimal(&text).filter(|id| *id > 0)
}

fn truthy(value: Option<&Value>) -> bool {
    value.is_some_and(fsv_py::truthy)
}

/// Python `str(value)`: `None`, `True` and floats keep Python's spelling.
fn scalar(value: &Value) -> String {
    fsv_py::py_str(value)
}

fn err(msg: &str) -> QueryError {
    QueryError(msg.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fields(ids: &[i64], labels: &[&str]) -> Vec<Value> {
        let choices: Vec<Value> = ids
            .iter()
            .zip(labels)
            .map(|(i, v)| json!({"id": i, "value": v}))
            .collect();
        vec![json!({"name": "status", "choices": choices})]
    }

    #[test]
    fn resolves_id_or_label_per_tenant() {
        let cases = [
            (
                vec![71, 82, 93, 104],
                ["Open", "In Progress", "Completed", "Blocked"],
            ),
            (
                vec![11, 22, 33, 44],
                ["Neu", "In Arbeit", "Fertig", "Gesperrt"],
            ),
        ];
        for (ids, labels) in cases {
            let defs = fields(&ids, &labels);
            for (id, label) in ids.iter().zip(labels) {
                assert_eq!(resolve(label, &defs).unwrap(), (*id, (*label).to_string()));
                assert_eq!(resolve(&id.to_string(), &defs).unwrap().0, *id);
                assert_eq!(display(&json!(id), Some(&defs)), *label);
            }
            assert!(resolve("done", &defs).is_err());
            assert!(resolve("999", &defs).is_err());
        }
    }

    #[test]
    fn bad_choices_fail_closed() {
        let samples = [
            json!([]),
            json!([{"id": 1, "value": "Open", "deleted": true}]),
            json!([{"id": 1, "value": "Open"}, {"id": 2, "value": "open"}]),
            json!([{"id": 1, "value": "Open"}, {"id": 1, "value": "Other"}]),
        ];
        for choices in samples {
            let defs = vec![json!({"name": "status", "choices": choices})];
            assert!(resolve("Open", &defs).is_err(), "{defs:?}");
        }
    }

    /// Python 31a9103 `active_choices` on float flags and non-list choices.
    #[test]
    fn float_flags_and_non_list_choices() {
        let run = |choices: Value| {
            active_choices(&[json!({"name": "status", "choices": choices})]).map_err(|e| e.0)
        };
        assert_eq!(
            run(json!([{"id": 1, "value": "Open", "deleted": 0.0},
                       {"id": 2, "value": "Done", "deleted": 0.5}])),
            Ok(vec![(1, "Open".to_string())])
        );
        for bad in [json!({"a": 1}), json!("ab")] {
            assert_eq!(run(bad), Err("invalid task-status choice".to_string()));
        }
        for empty in [json!({}), json!("")] {
            assert_eq!(run(empty), Ok(vec![]));
        }
    }

    #[test]
    fn active_choices_match_python() {
        let oracle: Value =
            serde_json::from_str(include_str!("testdata/coverage_oracle.json")).unwrap();
        let ac = &oracle["misc"]["active_choices"];
        for (name, fields) in ac["cases"].as_object().unwrap() {
            let got = match active_choices(fields.as_array().unwrap()) {
                Ok(rows) => json!({"ok": rows}),
                Err(e) => json!({"err": e.0}),
            };
            assert_eq!(got, ac["want"][name], "{name}");
        }
    }

    #[test]
    fn no_default_id_inference() {
        assert!(fields_from_payload(&json!({})).is_err());
        assert_eq!(display(&json!(3), None), "3 (unknown)");
    }

    #[test]
    fn unknown_scalars_use_python_str() {
        // Python: f"{value} (unknown)" over str(value) / task_status_display(None).
        assert_eq!(display(&Value::Null, None), "None (unknown)");
        assert_eq!(display(&json!(true), None), "True (unknown)");
        assert_eq!(display(&json!(false), None), "False (unknown)");
        assert_eq!(display(&json!(1.5), None), "1.5 (unknown)");
        assert_eq!(display(&json!("Open"), None), "Open (unknown)");
    }

    #[test]
    fn display_dict_uses_python_or_chain() {
        assert_eq!(display(&json!({"name": "", "label": "Open"}), None), "Open");
        assert_eq!(display(&json!({"name": ""}), None), "None");
        assert_eq!(display(&json!({"id": 0}), None), "0");
        assert_eq!(display(&json!({"id": ""}), None), "");
        assert_eq!(display(&json!({"name": false, "id": 5}), None), "5");
        assert_eq!(display(&json!({"name": true}), None), "True");
        assert_eq!(display(&json!({"id": true}), None), "True");
        assert_eq!(display(&json!({}), None), "None");
        assert_eq!(display(&json!({"value": 0, "id": "7"}), None), "7");
        assert_eq!(display(&json!({"label": "  x  "}), None), "  x  ");
    }

    /// `test_sweep1_messages.py::test_problem_status_label_error_names_problem_status_and_options` (c692ce2).
    #[test]
    fn problem_status_label_error_names_problem_status_and_options() {
        let fields = vec![
            json!({"name": "status", "choices": [{"id": 1, "value": "Open"},
            {"id": 2, "value": "Change Requested"}, {"id": 3, "value": "Closed"}]}),
        ];
        let text = resolve_of("Resolved", &fields, "problem").unwrap_err().0;
        assert_eq!(
            text,
            "status must match one active runtime problem-status ID or label; 'Resolved' is not one (options: Open, Change Requested, Closed)"
        );
        assert!(
            resolve("Resolved", &fields)
                .unwrap_err()
                .0
                .contains("task-status ID or label")
        );
        assert_eq!(
            resolve_of("closed", &fields, "problem").unwrap(),
            (3, "Closed".to_string())
        );
        assert_eq!(
            active_choices_of(&[], "problem").unwrap_err().0,
            "exactly one runtime problem-status field is required"
        );
    }
}
