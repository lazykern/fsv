//! Runtime choice identities from native metadata (`choice_ids.py`).
//!
//! Freshservice metadata serializes some choice ids as JSON integers and
//! others as digit strings (for example `problem_task_fields`). Accepted
//! write wires send the integer form. Booleans, signs, whitespace,
//! non-ASCII digits and other shapes are never identities.

use serde_json::Value;

/// Port of `runtime_choice_id(value, minimum=)`.
pub fn runtime_choice_id(value: Option<&Value>, minimum: i64) -> Option<i64> {
    let ident = match value? {
        Value::Number(n) if n.is_i64() || n.is_u64() => n.as_i64()?,
        Value::String(s) if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) => {
            s.parse::<i64>().ok()?
        }
        _ => return None,
    };
    (ident >= minimum).then_some(ident)
}

#[cfg(test)]
mod tests {
    use std::sync::LazyLock;

    use serde_json::{Value, json};

    use super::*;

    static ORACLE: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(include_str!("testdata/choice_ids_oracle.json")).unwrap()
    });

    #[test]
    fn runtime_choice_ids_match_python() {
        let cases = ORACLE["ids"].as_array().unwrap();
        assert_eq!(cases.len(), 19);
        for case in cases {
            let got = runtime_choice_id(Some(&case["value"]), case["minimum"].as_i64().unwrap());
            assert_eq!(json!(got), case["expected"], "{case}");
        }
        assert_eq!(runtime_choice_id(None, 0), None);
    }
}
