//! Which approval model a tenant runs: parallel chains, or legacy flat approvals.
//!
//! `GET <module>/<id>/approval-chains` answering 400 or 404 proves the legacy
//! model. 403 and 5xx stay unknown and keep the chains path. Session errors
//! propagate.

use serde_json::json;

use fsv_core::error::{Error, Result};

use crate::reviewed_write::refused;
use crate::service::{Api, obj};

pub const CHAINS: &str = "chains";
pub const LEGACY: &str = "legacy";
pub const UNKNOWN: &str = "unknown";
pub const LEGACY_LABEL: &str = "legacy flat approvals";
pub const NOT_SUPPORTED: &str = "legacy flat approvals: not yet supported";

/// `describe`: one sentence naming the legacy model and the status that proved it.
pub fn describe(status: Option<u16>) -> String {
    let shown = status.map_or_else(|| "?".to_string(), |n| n.to_string());
    format!("this tenant uses {LEGACY_LABEL} (approval-chains answered HTTP {shown})")
}

/// An approval-chains error that means "no parallel approvals here".
pub fn is_absent(err: &Error) -> bool {
    matches!(err, Error::Api(e) if matches!(e.status, 400 | 404))
}

/// Chains-only actions on a proven legacy tenant.
pub fn refusal(action: &str, status: Option<u16>) -> Error {
    refused(format!(
        "{action} refused: {}; {NOT_SUPPORTED}",
        describe(status)
    ))
}

/// `--groups` on a proven legacy tenant, pointing at the flat list.
pub fn groups_unavailable(status: u16) -> Error {
    refused(format!(
        "approval groups unavailable: {}; {NOT_SUPPORTED} (list the flat records without --groups)",
        describe(Some(status))
    ))
}

/// The status inside [`groups_unavailable`], when `err` is that refusal.
pub fn groups_status(err: &Error) -> Option<u16> {
    let marker = "approval groups unavailable: this tenant uses legacy flat approvals (approval-chains answered HTTP ";
    let msg = err.to_string();
    let rest = msg.strip_prefix(marker)?;
    rest.split(')').next()?.parse().ok()
}

/// `(CHAINS, None)`, `(LEGACY, status)` or `(UNKNOWN, status)` from one read-only GET.
///
/// Only 400 and 404 prove the legacy model. Other API errors stay unknown.
/// Session errors propagate. `module` is `changes` or `tickets`.
pub fn detect(c: &dyn Api, module: &str, target: i64) -> Result<(String, Option<u16>)> {
    if module != "changes" && module != "tickets" {
        return Err(Error::value("unsupported approval module"));
    }
    match c.int_get(
        &format!("{module}/{target}/approval-chains"),
        Some(obj(json!({"page": 1, "per_page": 100}))),
    ) {
        Ok(_) => Ok((CHAINS.to_string(), None)),
        Err(e) if is_absent(&e) => Ok((LEGACY.to_string(), e.status())),
        Err(Error::Api(e)) => Ok((UNKNOWN.to_string(), Some(e.status))),
        Err(e) => Err(e),
    }
}

/// Refuse a chains/groups-only action on a proven legacy tenant.
pub fn require_chains(c: &dyn Api, module: &str, target: i64, action: &str) -> Result<()> {
    let (model, status) = detect(c, module, target)?;
    if model == LEGACY {
        return Err(refusal(action, status));
    }
    Ok(())
}

/// Keys every flat approval row carries in `tickets approvals --json` and
/// `sr approvals list --json` (added next to the native record's own keys).
pub const FLAT_KEYS: [&str; 8] = [
    "level",
    "approver",
    "approver_id",
    "status_name",
    "status_id",
    "type_name",
    "decided",
    "comment",
];

fn named(value: Option<&serde_json::Value>) -> (serde_json::Value, String) {
    use fsv_py::{py_str, truthy};
    match value {
        Some(serde_json::Value::Object(m)) => {
            let name = [m.get("name"), m.get("label")]
                .into_iter()
                .flatten()
                .find(|v| truthy(v))
                .map(py_str)
                .unwrap_or_default();
            (
                m.get("id").cloned().unwrap_or(serde_json::Value::Null),
                name,
            )
        }
        other => (
            other.cloned().unwrap_or(serde_json::Value::Null),
            String::new(),
        ),
    }
}

/// `flat_row`: display fields of one flat (legacy) approval record. Status
/// and type names come from the record's own status/type objects; a record
/// without a name shows its id rather than a guessed label.
pub fn flat_row(record: &serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    use fsv_py::{py_str, truthy};
    use serde_json::Value;
    let empty = serde_json::Map::new();
    let a = record.as_object().unwrap_or(&empty);
    let remark = match a.get("remark") {
        Some(Value::Array(rs)) => rs.first().and_then(Value::as_object).unwrap_or(&empty),
        _ => &empty,
    };
    let member = a.get("member").and_then(Value::as_object).unwrap_or(&empty);
    let (status_id, status_name) = named(a.get("status"));
    let (type_id, type_name) = named(a.get("type"));
    let or_id = |name: String, id: &Value| {
        if !name.is_empty() {
            name
        } else if id.is_null() {
            String::new()
        } else {
            py_str(id)
        }
    };
    let member_id = a.get("member_id");
    let approver = member
        .get("name")
        .filter(|v| truthy(v))
        .map(py_str)
        .unwrap_or_else(|| {
            member_id
                .filter(|v| !v.is_null())
                .map(py_str)
                .unwrap_or_default()
        });
    let approver_id = match member_id {
        Some(v) => v.clone(),
        None => member.get("id").cloned().unwrap_or(Value::Null),
    };
    let decided = remark
        .get("updated_at")
        .filter(|v| truthy(v))
        .or_else(|| a.get("updated_at"))
        .filter(|v| truthy(v))
        .map(|v| py_str(v).chars().take(19).collect::<String>())
        .unwrap_or_default();
    let comment = remark
        .get("data")
        .filter(|v| truthy(v))
        .map(py_str)
        .unwrap_or_default();
    let mut out = serde_json::Map::new();
    out.insert(
        "level".into(),
        a.get("level_id").cloned().unwrap_or(Value::Null),
    );
    out.insert("approver".into(), json!(approver));
    out.insert("approver_id".into(), approver_id);
    out.insert("status_name".into(), json!(or_id(status_name, &status_id)));
    out.insert("status_id".into(), status_id);
    out.insert("type_name".into(), json!(or_id(type_name, &type_id)));
    out.insert("decided".into(), json!(decided));
    out.insert("comment".into(), json!(comment));
    out
}

/// `with_flat`: native records with the [`FLAT_KEYS`] added (native keys kept).
pub fn with_flat(records: &[serde_json::Value]) -> Vec<serde_json::Value> {
    records
        .iter()
        .filter_map(|r| {
            let mut m = r.as_object()?.clone();
            m.extend(flat_row(r));
            Some(serde_json::Value::Object(m))
        })
        .collect()
}

/// `flat_table` cells: Level, Approver, Status, Decided, Type, Comment, with
/// `-` for an empty value (markup not yet escaped).
pub fn flat_cells(record: &serde_json::Value) -> [String; 6] {
    let row = flat_row(record);
    [
        "level",
        "approver",
        "status_name",
        "decided",
        "type_name",
        "comment",
    ]
    .map(|k| match &row[k] {
        serde_json::Value::Null => "-".to_string(),
        serde_json::Value::String(s) if s.is_empty() => "-".to_string(),
        v => fsv_py::py_str(v),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    use crate::service::testing::*;

    fn oracle() -> Value {
        serde_json::from_str(include_str!("testdata/approval_model_oracle.json")).unwrap()
    }

    #[test]
    fn is_absent_only_for_direct_400_and_404() {
        use fsv_core::error::ApiError;
        assert!(is_absent(&ApiError::new(400, "x").into()));
        assert!(is_absent(&ApiError::new(404, "x").into()));
        for status in [403, 409, 422, 500] {
            assert!(!is_absent(&ApiError::new(status, "x").into()));
        }
        assert!(!is_absent(
            &Error::from(ApiError::new(404, "x")).context("outer")
        ));
        assert!(!is_absent(&Error::session("s")));
        assert!(!is_absent(&Error::value("v")));
    }

    #[test]
    fn legacy_unknown_and_listing_match_python() {
        let o = oracle();
        assert_eq!(describe(Some(400)), o["describe"]["400"].as_str().unwrap());
        assert_eq!(describe(Some(404)), o["describe"]["404"].as_str().unwrap());
        assert_eq!(describe(None), o["describe"]["none"].as_str().unwrap());
        assert_eq!(NOT_SUPPORTED, o["not_supported"].as_str().unwrap());
        assert_eq!(
            groups_unavailable(400).to_string(),
            o["groups_400"].as_str().unwrap()
        );
        assert_eq!(
            refusal("approval-request", Some(400)).to_string(),
            o["refusals"]["approval-request"].as_str().unwrap()
        );
        assert_eq!(
            refusal("approval-cancel", Some(404)).to_string(),
            o["refusals"]["approval-cancel"].as_str().unwrap()
        );
        assert_eq!(
            refusal("approval-respond --approve", Some(400)).to_string(),
            o["refusals"]["approve"].as_str().unwrap()
        );
        assert_eq!(
            refusal("approval-respond --reject", Some(404)).to_string(),
            o["refusals"]["reject"].as_str().unwrap()
        );

        let absent = FakeApi::new().on("GET changes/5/approval-chains", Reply::Status(400));
        assert_eq!(
            detect(&absent, "changes", 5).unwrap(),
            (LEGACY.to_string(), Some(400))
        );
        assert_eq!(groups_status(&groups_unavailable(400)), Some(400));
        let missing = FakeApi::new();
        assert_eq!(
            require_chains(&missing, "tickets", 123, "sr approvals cancel")
                .unwrap_err()
                .to_string(),
            o["refusals"]["sr_cancel"].as_str().unwrap()
        );
        let denied = FakeApi::new().on("GET changes/5/approval-chains", Reply::Status(403));
        assert_eq!(
            detect(&denied, "changes", 5).unwrap(),
            (UNKNOWN.to_string(), Some(403))
        );
        require_chains(&denied, "changes", 5, "x").unwrap();
        let down = FakeApi::new().on("GET tickets/5/approval-chains", Reply::Status(500));
        assert_eq!(
            detect(&down, "tickets", 5).unwrap(),
            (UNKNOWN.to_string(), Some(500))
        );
        assert!(matches!(
            detect(&absent, "problems", 5).unwrap_err(),
            Error::Value(_)
        ));
        assert!(matches!(
            detect(
                &FakeApi::new().on("GET changes/5/approval-chains", Reply::Session),
                "changes",
                5
            )
            .unwrap_err(),
            Error::Session(_)
        ));
        let chains = FakeApi::new().ok(
            "GET changes/5/approval-chains",
            json!({"approval_chains": []}),
        );
        assert_eq!(
            detect(&chains, "changes", 5).unwrap(),
            (CHAINS.to_string(), None)
        );

        let legacy = FakeApi::new()
            .on("GET tickets/9/approval-chains", Reply::Status(404))
            .ok("GET tickets/9/approvals", json!({"approvals": [{"id": 7}]}));
        let listed = crate::sr_approvals::listing(&legacy, 9).unwrap();
        assert_eq!(
            listed,
            json!({
                "model": "legacy",
                "note": o["describe"]["404"],
                "approvals": [{"id": 7}],
            })
        );
        let present = FakeApi::new().ok(
            "GET tickets/9/approval-chains",
            json!({"approval_chains": []}),
        );
        assert_eq!(
            crate::sr_approvals::listing(&present, 9).unwrap(),
            json!({"approval_groups": []})
        );
    }
}
