//! Runtime task-status choices (`task_status.py`).
//!
//! Ids and labels come from the tenant. Nothing here assumes Open=1.

use serde_json::Value;

use crate::QueryError;

/// One active choice: numeric id and the label Freshservice shows.
pub type Choice = (i64, String);

/// `active_choices`. Skips deleted rows. Fails closed on a missing, duplicate
/// or unreadable status field.
pub fn active_choices(fields: &[Value]) -> Result<Vec<Choice>, QueryError> {
    let statuses: Vec<&Value> = fields
        .iter()
        .filter(|f| {
            f.as_object()
                .is_some_and(|o| o.get("name").and_then(Value::as_str) == Some("status"))
        })
        .collect();
    if statuses.len() != 1 {
        return Err(err("exactly one runtime task-status field is required"));
    }
    let mut result = Vec::new();
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
            .find_map(|k| choice.get(*k).and_then(Value::as_str));
        let Some((id, label)) = ident
            .and_then(positive_id)
            .zip(label.map(str::trim).filter(|s| !s.is_empty()))
        else {
            return Err(err("invalid active task-status identity"));
        };
        result.push((id, label.to_string()));
    }
    let mut ids: Vec<i64> = result.iter().map(|(i, _)| *i).collect();
    let mut labels: Vec<String> = result.iter().map(|(_, l)| l.to_lowercase()).collect();
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
    let key = value.trim().to_lowercase();
    let matches: Vec<Choice> = active_choices(fields)?
        .into_iter()
        .filter(|(id, label)| key == id.to_string() || key == label.to_lowercase())
        .collect();
    if matches.len() != 1 {
        return Err(err(
            "status must match one active runtime task-status ID or label",
        ));
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
        let picked = ["name", "label", "value", "id"]
            .iter()
            .find_map(|k| obj.get(*k).filter(|v| !v.is_null()));
        return match picked {
            Some(Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
            None => "None".to_string(),
        };
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
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let id: i64 = text.parse().ok()?;
    (id > 0).then_some(id)
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null | Value::Bool(false)) => false,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Number(n)) => n.as_i64().unwrap_or(1) != 0,
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(Value::Bool(true)) => true,
    }
}

fn scalar(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
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

    #[test]
    fn no_default_id_inference() {
        assert!(fields_from_payload(&json!({})).is_err());
        assert_eq!(display(&json!(3), None), "3 (unknown)");
    }
}
