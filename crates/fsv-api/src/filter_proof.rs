//! Independent, deliberately narrow predicate proofs for server-filtered
//! rows (`filter_proof.py`).

use serde_json::{Map, Value};

use fsv_core::error::{Error, Result};
use fsv_py::{py_str, truthy};

use crate::QueryError;

/// `repr()` of a JSON value as Python prints a field name.
fn repr(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => "None".into(),
        Some(Value::String(s)) => fsv_py::py_repr_str(s),
        Some(other) => py_str(other),
    }
}

/// Port of `require_filterable_field`. Form metadata alone does not prove a
/// free-text filter operator exists. `default_subject` has none, and the
/// choice operator would silently broaden a list.
pub fn require_filterable_field(field: &Value) -> std::result::Result<(), QueryError> {
    let kind = field
        .get("field_type")
        .filter(|v| truthy(v))
        .map(py_str)
        .unwrap_or_default();
    let name = field.get("name").and_then(Value::as_str);
    let filterable = field.get("choices").is_some_and(truthy)
        || kind.contains("dropdown")
        || kind.ends_with("number")
        || matches!(
            kind.as_str(),
            "default_status"
                | "default_priority"
                | "default_requester"
                | "default_agent"
                | "default_group"
                | "default_department"
        )
        || matches!(name, Some("requester" | "agent" | "group" | "department"));
    if filterable {
        return Ok(());
    }
    Err(QueryError(format!(
        "filter operator unavailable in runtime metadata for {}; refusing an unproven filter (use explicit IDs)",
        repr(field.get("name"))
    )))
}

fn scalar(value: &Value) -> Result<String> {
    let value = match value {
        Value::Object(m) if m.contains_key("id") && m.keys().all(|k| k == "id" || k == "name") => {
            &m["id"]
        }
        other => other,
    };
    match value {
        Value::Null | Value::Object(_) | Value::Array(_) | Value::Bool(_) => Err(Error::value(
            "predicate value is not independently verifiable",
        )),
        Value::Number(n) if n.is_f64() => Err(Error::value(
            "predicate value is not independently verifiable",
        )),
        other => Ok(py_str(other)),
    }
}

/// Port of `filter_proof.matches`: whether the exact record satisfies every
/// `is_in` / `is_not_in` clause. Anything else cannot be proven.
pub fn matches(row: &Map<String, Value>, clauses: &[Value]) -> Result<bool> {
    if clauses.is_empty() {
        return Err(Error::value(
            "empty predicate cannot authorize a filtered write",
        ));
    }
    for clause in clauses {
        let name = clause.get("condition");
        let kind = clause.get("type").and_then(Value::as_str);
        let op = clause.get("operator").and_then(Value::as_str);
        let values = clause.get("value").and_then(Value::as_array);
        let (
            Some(Value::String(field)),
            Some("default" | "custom_field"),
            Some(op @ ("is_in" | "is_not_in")),
            Some(values),
        ) = (name, kind, op, values)
        else {
            return Err(Error::value(format!(
                "predicate cannot be independently verified for {}",
                repr(name)
            )));
        };
        if values.is_empty() {
            return Err(Error::value(format!(
                "predicate cannot be independently verified for {}",
                repr(name)
            )));
        }
        let source = if kind == Some("custom_field") {
            row.get("custom_fields").and_then(Value::as_object)
        } else {
            Some(row)
        };
        let Some(actual) = source.and_then(|s| s.get(field)) else {
            return Err(Error::value(format!(
                "predicate field unavailable in exact record: {}",
                repr(name)
            )));
        };
        let actual = scalar(actual)?;
        let expected = values.iter().map(scalar).collect::<Result<Vec<_>>>()?;
        if expected.contains(&actual) != (op == "is_in") {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Port of `filter_proof.contradictions`: ids of rows that provably fail
/// the predicate. Rows whose predicate cannot be proven are skipped.
pub fn contradictions(rows: &[Value], clauses: &[Value]) -> Vec<Value> {
    rows.iter()
        .filter_map(|row| {
            let map = row.as_object()?;
            match matches(map, clauses) {
                Ok(false) => Some(map.get("id").cloned().unwrap_or(Value::from("unknown"))),
                _ => None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::LazyLock;

    use serde_json::{Value, json};

    use super::*;

    static ORACLE: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(include_str!("testdata/filter_proof_oracle.json")).unwrap()
    });

    fn cases(key: &str) -> &'static [Value] {
        ORACLE[key].as_array().unwrap()
    }

    #[test]
    fn filterable_field_guard_matches_python() {
        assert_eq!(cases("filterable").len(), 14);
        for case in cases("filterable") {
            let got = match require_filterable_field(&case["field"]) {
                Ok(()) => json!({"ok": null}),
                Err(e) => json!({"err": e.0}),
            };
            let want: Value = case
                .as_object()
                .unwrap()
                .iter()
                .filter(|(k, _)| *k != "field")
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<Map<_, _>>()
                .into();
            assert_eq!(got, want, "{}", case["field"]);
        }
    }

    #[test]
    fn exact_row_predicate_proofs_match_python() {
        assert_eq!(cases("matches").len(), 26);
        for case in cases("matches") {
            let row = case["row"].as_object().unwrap();
            let clauses = case["clauses"].as_array().unwrap();
            let got = match matches(row, clauses) {
                Ok(b) => json!({"ok": b}),
                Err(e) => json!({"err": e.to_string()}),
            };
            let want = match case.get("ok") {
                Some(ok) => json!({"ok": ok}),
                None => json!({"err": case["err"]}),
            };
            assert_eq!(got, want, "{} {}", case["row"], case["clauses"]);
        }
    }

    #[test]
    fn list_contradictions_skip_unprovable_rows() {
        for case in cases("contradictions") {
            let got = contradictions(
                case["rows"].as_array().unwrap(),
                case["clauses"].as_array().unwrap(),
            );
            assert_eq!(Value::Array(got), case["ids"]);
        }
    }
}
