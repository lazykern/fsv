//! Exact ticket-type proof from native ticket reads (`ticket_kind.py`).

use std::collections::BTreeSet;

use serde_json::Value;

use fsv_core::error::Result;
use fsv_core::pystr::{casefold, collapse_ws, strip};

use crate::reviewed_write::refused;

/// Port of `exact_ticket_type`: the case-folded type of an exact native
/// ticket read. Native reads expose `type`, echoes and form adapters
/// `ticket_type`. Every present representation must be a nonempty string
/// and all of them must agree.
pub fn exact_ticket_type(record: &Value) -> Result<String> {
    let Some(record) = record.as_object() else {
        return Err(refused("exact ticket type unavailable"));
    };
    let values: Vec<&Value> = ["type", "ticket_type"]
        .iter()
        .filter_map(|key| record.get(*key).filter(|v| !v.is_null()))
        .collect();
    let mut kinds = BTreeSet::new();
    for value in &values {
        match value.as_str() {
            Some(text) if !strip(text).is_empty() => {
                kinds.insert(casefold(&collapse_ws(&text.replace('_', " "))));
            }
            _ => return Err(refused("exact ticket type unavailable")),
        }
    }
    if kinds.is_empty() {
        return Err(refused("exact ticket type unavailable"));
    }
    if kinds.len() != 1 {
        return Err(refused("conflicting exact ticket type representations"));
    }
    Ok(kinds.pop_first().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use std::sync::LazyLock;

    use serde_json::{Value, json};

    use super::*;

    static ORACLE: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(include_str!("testdata/ticket_kind_oracle.json")).unwrap()
    });

    #[test]
    fn exact_ticket_type_matches_python() {
        let cases = ORACLE["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 21);
        for case in cases {
            let got = match exact_ticket_type(&case["record"]) {
                Ok(kind) => json!({"ok": kind}),
                Err(e) => json!({"err": e.to_string()}),
            };
            let want = match case.get("ok") {
                Some(ok) => json!({"ok": ok}),
                None => json!({"err": case["err"]}),
            };
            assert_eq!(got, want, "{}", case["record"]);
        }
    }
}
