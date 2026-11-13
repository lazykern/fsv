//! Exact ticket-type proof from native ticket reads (`ticket_kind.py`).

use std::collections::BTreeSet;

use serde_json::{Map, Value};

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

pub const INCIDENT: &str = "incident";
pub const SERVICE_REQUEST: &str = "service request";

/// `_display_prefix`: the prefix of the record's own human display id,
/// checked against its own display number (`IT-77` with display_id 77 ->
/// `IT`).
fn display_prefix(record: &Map<String, Value>) -> Option<String> {
    let Some(Value::String(hid)) = record.get("human_display_id") else {
        return None;
    };
    let did = match record.get("display_id") {
        Some(v) => v,
        None => record.get("id").unwrap_or(&Value::Null),
    };
    if matches!(did, Value::Bool(_) | Value::Null) {
        return None;
    }
    let (head, tail) = strip(hid).rsplit_once('-')?;
    if head.is_empty() || tail != fsv_py::py_str(did) {
        return None;
    }
    Some(head.to_uppercase())
}

/// [`kind_evidence`] with the display-id prefix table supplied by
/// `prefixes` (only asked for when the record has a usable display id).
pub fn kind_evidence_with(
    record: &Value,
    prefixes: impl FnOnce() -> Vec<(String, String)>,
) -> Vec<(String, String)> {
    let Some(record) = record.as_object() else {
        return Vec::new();
    };
    let mut out: Vec<(String, String)> = Vec::new();
    for key in ["type", "ticket_type"] {
        if let Some(Value::String(value)) = record.get(key) {
            let folded = casefold(&collapse_ws(&value.replace('_', " ")));
            if folded == INCIDENT || folded == SERVICE_REQUEST {
                out.push((key.to_string(), folded));
            }
        }
    }
    if let Some(prefix) = display_prefix(record) {
        let mapped = prefixes()
            .into_iter()
            .find(|(p, _)| *p == prefix)
            .map(|(_, kind)| kind);
        if let Some(kind) = mapped.filter(|k| k == INCIDENT || k == SERVICE_REQUEST) {
            out.push(("human_display_id".into(), kind));
        }
    }
    if record.get("has_requested_items") == Some(&Value::Bool(true)) {
        out.push(("has_requested_items".into(), SERVICE_REQUEST.into()));
    }
    out
}

/// Port of `kind_evidence`: each piece of evidence the record itself
/// carries about its module: `type` (only the platform's built-in type
/// names), the display-id prefix (built-in or configured, see
/// [`fsv_core::resource::kind_prefixes`]) and requested items (only service
/// requests carry them).
pub fn kind_evidence(record: &Value) -> Vec<(String, String)> {
    kind_evidence_with(record, fsv_core::resource::kind_prefixes)
}

fn single_kind(evidence: &[(String, String)]) -> Option<String> {
    let kinds: BTreeSet<&String> = evidence.iter().map(|(_, k)| k).collect();
    (kinds.len() == 1).then(|| kinds.into_iter().next().cloned().unwrap_or_default())
}

/// Port of `ticket_kind`: `incident` / `service request` when the record's
/// own fields agree; `None` when they carry no evidence or conflict.
pub fn ticket_kind(record: &Value) -> Option<String> {
    single_kind(&kind_evidence(record))
}

/// [`require_kind`] over a precomputed [`kind_evidence`].
fn require_kind_from(
    record: &Value,
    evidence: &[(String, String)],
    kind: &str,
    what: &str,
) -> Result<()> {
    if single_kind(evidence).as_deref() == Some(kind) {
        return Ok(());
    }
    let seen: Vec<String> = ["type", "human_display_id"]
        .iter()
        .filter_map(|k| {
            record
                .get(*k)
                .filter(|v| !v.is_null())
                .map(|v| format!("{k}={}", fsv_py::py_repr(v)))
        })
        .collect();
    let seen = if seen.is_empty() {
        "no type or display id".to_string()
    } else {
        seen.join(", ")
    };
    let setting = if kind == INCIDENT {
        "ids.incident_prefix"
    } else {
        "ids.service_request_prefix"
    };
    Err(refused(format!(
        "{what} requires a proven {kind}; the record's own fields do not prove it ({seen}). \
         A tenant with a custom display-id prefix can declare it: fsv config set {setting} PREFIX"
    )))
}

/// Port of `require_kind`: refuse unless the record proves `kind` from its
/// own fields.
pub fn require_kind(record: &Value, kind: &str, what: &str) -> Result<()> {
    require_kind_from(record, &kind_evidence(record), kind, what)
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

    fn prefixes(custom: &[(&str, &str)]) -> Vec<(String, String)> {
        [
            ("INC", "incident"),
            ("SR", "service request"),
            ("CHN", "change"),
            ("PRB", "problem"),
        ]
        .iter()
        .chain(custom)
        .map(|(p, k)| (p.to_string(), k.to_string()))
        .collect()
    }

    fn kind_of(record: &Value, custom: &[(&str, &str)]) -> Option<String> {
        single_kind(&kind_evidence_with(record, || prefixes(custom)))
    }

    /// tests/test_shifted_tenant.py::test_ticket_kind_comes_from_the_records_own_prefix_and_type
    #[test]
    fn ticket_kind_comes_from_the_records_own_prefix_and_type() {
        let fault =
            json!({"id": 77, "display_id": 77, "type": "Fault", "human_display_id": "IT-77"});
        assert_eq!(kind_of(&fault, &[]), None);
        let evidence = kind_evidence_with(&fault, || prefixes(&[]));
        let err = require_kind_from(&fault, &evidence, INCIDENT, "caused relationship")
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            "caused relationship requires a proven incident; the record's own fields do not prove it \
             (type='Fault', human_display_id='IT-77'). A tenant with a custom display-id prefix can \
             declare it: fsv config set ids.incident_prefix PREFIX"
        );
        let custom = [("IT", "incident"), ("RQ", "service request")];
        assert_eq!(kind_of(&fault, &custom).as_deref(), Some(INCIDENT));
        assert_eq!(
            kind_of(
                &json!({"display_id": 78, "type": "Anfrage", "human_display_id": "RQ-78"}),
                &custom
            )
            .as_deref(),
            Some(SERVICE_REQUEST)
        );
        let none: &[(&str, &str)] = &[];
        assert_eq!(
            kind_of(
                &json!({"display_id": 5, "type": "Störung", "human_display_id": "INC-5"}),
                none
            )
            .as_deref(),
            Some(INCIDENT)
        );
        assert_eq!(
            kind_of(
                &json!({"display_id": 6, "type": "Anfrage", "has_requested_items": true}),
                none
            )
            .as_deref(),
            Some(SERVICE_REQUEST)
        );
        assert_eq!(
            kind_of(
                &json!({"display_id": 7, "type": "Service Request", "human_display_id": "INC-7"}),
                none
            ),
            None
        );
        assert_eq!(
            kind_of(
                &json!({"display_id": 8, "type": "Fault", "human_display_id": "IT-9"}),
                &custom
            ),
            None
        );
    }

    #[test]
    fn display_prefix_edges() {
        let none: &[(&str, &str)] = &[];
        // display_id present but null: no fallback to id
        assert_eq!(
            kind_of(
                &json!({"id": 5, "display_id": null, "human_display_id": "INC-5"}),
                none
            ),
            None
        );
        assert_eq!(
            kind_of(&json!({"id": 5, "human_display_id": " inc-5 "}), none).as_deref(),
            Some(INCIDENT)
        );
        assert_eq!(
            kind_of(
                &json!({"display_id": true, "human_display_id": "INC-True"}),
                none
            ),
            None
        );
        assert_eq!(
            kind_of(&json!({"id": 5, "human_display_id": "-5"}), none),
            None
        );
        assert_eq!(kind_of(&json!({"has_requested_items": 1}), none), None);
        assert_eq!(
            kind_of(&json!({"ticket_type": "service_request"}), none).as_deref(),
            Some(SERVICE_REQUEST)
        );
        let err = require_kind_from(&json!({}), &[], SERVICE_REQUEST, "exact parent").unwrap_err();
        assert_eq!(
            err.to_string(),
            "exact parent requires a proven service request; the record's own fields do not prove it \
             (no type or display id). A tenant with a custom display-id prefix can declare it: fsv \
             config set ids.service_request_prefix PREFIX"
        );
    }
}
