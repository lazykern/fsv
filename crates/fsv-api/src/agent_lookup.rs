//! Exact active-agent identities from native autocomplete results.
//!
//! Port of `agent_lookup.exact_agent_id`. Native autocomplete uses an
//! email-valued `id`. A numeric `id` is never a user id.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use fsv_core::error::{Error, Result};
use fsv_core::pystr::casefold;
use fsv_py::truthy;

static EMAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[^\s@<>,;]+@[^\s@<>,;]+\.[^\s@<>,;]+$").expect("regex"));

/// Port of `agent_lookup.is_email`.
pub fn is_email(value: &str) -> bool {
    EMAIL.is_match(value)
}

/// One exact active row. `name` matches `value`/`name`; otherwise `id`,
/// `email`, and `details`, and every email on the row must be that address.
pub fn exact_agent_id(rows: &[Value], value: &str, name: bool) -> Result<i64> {
    if rows.iter().any(|row| !row.is_object()) {
        return Err(Error::value("agent lookup returned an unreadable result"));
    }
    let folded = casefold(value);
    let mut matches = Vec::new();
    for row in rows {
        let inactive = ["deleted", "deactivated", "is_deleted", "deleted_at"]
            .iter()
            .any(|key| row.get(*key).is_some_and(truthy))
            || row.get("active") == Some(&Value::Bool(false))
            || casefold(&text_or_empty(row.get("value"))).contains("(deactivated)");
        if inactive {
            continue;
        }
        let keys: &[&str] = if name {
            &["value", "name"]
        } else {
            &["id", "email", "details"]
        };
        let hit = keys.iter().any(|key| {
            row.get(*key)
                .and_then(Value::as_str)
                .is_some_and(|text| casefold(text) == folded)
        });
        if !hit {
            continue;
        }
        if !name {
            let disagree = ["id", "email", "details"].iter().any(|key| {
                row.get(*key)
                    .and_then(Value::as_str)
                    .is_some_and(|text| is_email(text) && casefold(text) != folded)
            });
            if disagree {
                continue;
            }
        }
        matches.push(row);
    }
    let ident = if matches.len() == 1 {
        matches[0].get("user_id")
    } else {
        None
    };
    match ident.and_then(user_id) {
        Some(id) if matches.len() == 1 => Ok(id),
        _ => Err(Error::value(format!(
            "agent must match exactly one active entry with a user ID; {} exact active candidates",
            matches.len()
        ))),
    }
}

fn text_or_empty(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(value) if !truthy(value) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => fsv_py::py_str(other),
    }
}

/// Python `str` that is ASCII digits and a positive int. Bools are rejected.
fn user_id(value: &Value) -> Option<i64> {
    if value.is_boolean() || value.is_null() {
        return None;
    }
    let text = match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    if !text.is_ascii() || text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse::<i64>().ok().filter(|n| *n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn native() -> Value {
        json!({"id": "agent@example.com", "value": "Alex Acme", "user_id": 81})
    }

    fn with(extra: Value) -> Value {
        let mut row = native();
        let obj = row.as_object_mut().unwrap();
        for (k, v) in extra.as_object().unwrap() {
            obj.insert(k.clone(), v.clone());
        }
        json!([row])
    }

    #[test]
    fn cases_match_python() {
        let oracle: Value =
            serde_json::from_str(include_str!("testdata/agent_lookup_oracle.json")).unwrap();
        let n = native();
        let cases = [
            ("email_id", json!([n]), "AGENT@example.com", false),
            (
                "email_key",
                json!([{"email": "agent@example.com", "user_id": "81", "value": "Alex Acme"}]),
                "AGENT@example.com",
                false,
            ),
            (
                "details",
                json!([{"details": "agent@example.com", "user_id": 81}]),
                "agent@example.com",
                false,
            ),
            ("empty", json!([]), "agent@example.com", false),
            (
                "duplicate",
                json!([native(), native()]),
                "agent@example.com",
                false,
            ),
            (
                "deleted",
                with(json!({"deleted": true})),
                "agent@example.com",
                false,
            ),
            (
                "inactive",
                with(json!({"active": false})),
                "agent@example.com",
                false,
            ),
            (
                "deactivated_label",
                with(json!({"value": "Alex Acme (deactivated)"})),
                "agent@example.com",
                false,
            ),
            (
                "null_user",
                with(json!({"user_id": null})),
                "agent@example.com",
                false,
            ),
            (
                "bool_user",
                with(json!({"user_id": true})),
                "agent@example.com",
                false,
            ),
            (
                "other_email",
                with(json!({"email": "other@example.com"})),
                "agent@example.com",
                false,
            ),
            (
                "bad_id",
                with(json!({"id": "agent@example.com.invalid"})),
                "agent@example.com",
                false,
            ),
            ("name", json!([native()]), "ALEX ACME", true),
            ("name_miss", json!([native()]), "Alex Globex", true),
            ("unreadable", json!(["x"]), "agent@example.com", false),
        ];
        assert!(is_email("agent@example.com"));
        assert!(!is_email("Alex Acme"));
        for (name, rows, value, by_name) in cases {
            let got = match exact_agent_id(rows.as_array().unwrap(), value, by_name) {
                Ok(id) => json!({"ok": id}),
                Err(err) => json!({"error": "ValueError", "msg": err.to_string()}),
            };
            assert_eq!(got, oracle["cases"][name], "{name}");
        }
    }
}
