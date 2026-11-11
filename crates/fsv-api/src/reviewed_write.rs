//! Single-attempt mutation primitive for captured internal contracts
//! (`reviewed_write.py`).
//!
//! Builders supply exact methods/paths and independent verification. This is
//! not an arbitrary API command and never retries a mutation.

use serde_json::{Map, Value, json};

use fsv_core::client::Body;
use fsv_core::error::{Error, Result};

use crate::service::{Api, Journal, definite_rejection, last_event, obj, plan_hash};

/// Port of fsv.reviewed_write.ReviewedWriteError: a safe user-facing refusal
/// (exit 2).
pub fn refused(msg: impl Into<String>) -> Error {
    Error::exit(2, msg)
}

/// Port of fsv.reviewed_write.WritePlan. `path` is relative to `/api/_/` and
/// may carry a query string, sent verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritePlan {
    pub action: String,
    pub target: i64,
    pub tenant: String,
    pub method: String,
    pub path: String,
    pub payload_json: String,
    pub context_hash: String,
}

impl WritePlan {
    /// Port of fsv.reviewed_write.WritePlan.payload (`null` for no body).
    pub fn payload(&self) -> Value {
        serde_json::from_str(&self.payload_json).unwrap_or(Value::Null)
    }

    /// Port of fsv.reviewed_write.WritePlan.hash.
    pub fn hash(&self) -> String {
        plan_hash(&json!({
            "action": self.action, "target": self.target, "tenant": self.tenant,
            "method": self.method, "path": self.path, "payload": self.payload(),
            "context": self.context_hash,
        }))
    }

    /// Port of fsv.reviewed_write.WritePlan.review.
    pub fn review(&self) -> Map<String, Value> {
        obj(json!({
            "action": self.action, "method": self.method, "path": format!("/api/_/{}", self.path),
            "payload": self.payload(), "plan_hash": self.hash(),
            "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
        }))
    }
}

const UNKNOWN: &str = "write outcome unknown; inspect target and journal before retry";

/// Port of fsv.reviewed_write.execute: guards, one request, then `verify` on
/// the response. `reviewed_hash` `None` cancels. `rebuild` re-derives the
/// plan from current server state; `verify` reads back independently and
/// its keys are merged into the result.
pub fn execute(
    plan: &WritePlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
    rebuild: &dyn Fn() -> Result<WritePlan>,
    verify: &dyn Fn(&Value) -> Result<Map<String, Value>>,
) -> Result<Value> {
    let rebuild_hash = || rebuild().map(|p| p.hash());
    execute_as(
        plan,
        &plan.hash(),
        c,
        journal,
        reviewed_hash,
        &rebuild_hash,
        verify,
    )
}

/// [`execute`] for a `WritePlan` subclass whose `hash` binds more than the
/// wire (`hash` is the subclass hash; `rebuild` returns the rebuilt one).
pub fn execute_as(
    plan: &WritePlan,
    hash: &str,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
    rebuild: &dyn Fn() -> Result<String>,
    verify: &dyn Fn(&Value) -> Result<Map<String, Value>>,
) -> Result<Value> {
    let Some(reviewed) = reviewed_hash else {
        return Ok(json!({"status": "cancelled", "verified": false}));
    };
    let hash = hash.to_string();
    if plan.tenant.is_empty() || plan.tenant != c.domain() || reviewed != hash {
        return Err(refused("tenant or full review hash changed; review again"));
    }
    if matches!(
        last_event(journal, &plan.action, &hash).as_deref(),
        Some("started" | "done")
    ) {
        return Err(refused(
            "write was sent or outcome unknown; inspect target and journal before retry",
        ));
    }
    if rebuild()? != hash {
        return Err(refused(
            "target or metadata changed since review; review again",
        ));
    }
    let Ok(csrf) = c.get_csrf() else {
        return Err(refused("could not prepare session; no write attempted"));
    };
    let target = json!(plan.target);
    journal.record_plan("started", &hash, &plan.action, Some(&target), None);
    if last_event(journal, &plan.action, &hash).as_deref() != Some("started") {
        return Err(refused(
            "could not persist ambiguity journal; no write attempted",
        ));
    }
    let body = if plan.payload().is_null() {
        Body::None
    } else {
        Body::Raw(plan.payload_json.clone().into_bytes())
    };
    let response = match c.send_once(&plan.method, &plan.path, body, None, &csrf) {
        Ok(r) => r,
        Err(e) => {
            if let Some(status) = definite_rejection(&e) {
                journal.record_plan("failed", &hash, &plan.action, Some(&target), None);
                return Err(refused(format!("write rejected (HTTP {status}); no retry")));
            }
            return Err(refused(UNKNOWN));
        }
    };
    journal.record_plan("done", &hash, &plan.action, Some(&target), None);
    let mut result = obj(json!({"status": "submitted", "plan_hash": hash, "verified": false}));
    match verify(&response) {
        Ok(extra) => result.extend(extra),
        Err(_) => {
            result.insert(
                "warning".into(),
                json!("Independent readback failed; do not resend."),
            );
        }
    }
    if !result.get("verified").is_some_and(fsv_py::truthy) && !result.contains_key("warning") {
        result.insert(
            "warning".into(),
            json!("Readback was not fully verified; do not resend."),
        );
    }
    Ok(Value::Object(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::testing::*;

    fn plan(payload: &str) -> WritePlan {
        WritePlan {
            action: "tickets_self_watch".into(),
            target: 123,
            tenant: "acme.freshservice.com".into(),
            method: "POST".into(),
            path: "tickets/123/watch".into(),
            payload_json: payload.into(),
            context_hash: "ctx".into(),
        }
    }

    fn err<T: std::fmt::Debug>(r: Result<T>) -> String {
        r.unwrap_err().to_string()
    }

    /// Hash and review from `uv run python` on `reviewed_write.WritePlan`.
    #[test]
    fn hash_and_review_match_python() {
        let p = plan(r#"{"user_id": 901}"#);
        let oracle: Value =
            serde_json::from_str(include_str!("testdata/reviewed_oracle.json")).unwrap();
        assert_eq!(p.hash(), oracle["write_plan"]["hash"]);
        assert_eq!(Value::Object(p.review()), oracle["write_plan"]["review"]);
        assert_eq!(plan("null").payload(), Value::Null);
    }

    #[test]
    fn execute_guards_and_single_attempt() {
        let p = plan(r#"{"user_id": 901}"#);
        let h = p.hash();
        let ok = |_: &Value| -> Result<Map<String, Value>> { Ok(obj(json!({"verified": true}))) };
        let same = || Ok(p.clone());
        let c = FakeApi::new().ok("POST tickets/123/watch", json!({}));
        let j = MemJournal::default();
        assert_eq!(
            execute(&p, &c, &j, None, &same, &ok).unwrap(),
            json!({"status": "cancelled", "verified": false})
        );
        assert_eq!(
            err(execute(&p, &c, &j, Some("x"), &same, &ok)),
            "tenant or full review hash changed; review again"
        );
        let other = || Ok(plan("{}"));
        assert_eq!(
            err(execute(&p, &c, &j, Some(&h), &other, &ok)),
            "target or metadata changed since review; review again"
        );
        assert_eq!(
            err(execute(&p, &c, &DeafJournal, Some(&h), &same, &ok)),
            "could not persist ambiguity journal; no write attempted"
        );
        assert!(c.writes().is_empty());
        assert_eq!(
            execute(&p, &c, &j, Some(&h), &same, &ok).unwrap(),
            json!({"status": "submitted", "plan_hash": h, "verified": true})
        );
        assert_eq!(
            err(execute(&p, &c, &j, Some(&h), &same, &ok)),
            "write was sent or outcome unknown; inspect target and journal before retry"
        );
        assert_eq!(c.writes().len(), 1);
        assert_eq!(
            c.writes()[0].raw.as_deref(),
            Some(&b"{\"user_id\": 901}"[..])
        );
    }

    #[test]
    fn execute_outcomes() {
        let p = plan(r#"{"user_id": 901}"#);
        let h = p.hash();
        let same = || Ok(p.clone());
        let unverified =
            |_: &Value| -> Result<Map<String, Value>> { Ok(obj(json!({"verified": false}))) };
        let broken = |_: &Value| -> Result<Map<String, Value>> { Err(Error::other("x")) };
        let c = FakeApi::new().ok("POST tickets/123/watch", json!({}));
        assert_eq!(
            execute(&p, &c, &MemJournal::default(), Some(&h), &same, &unverified).unwrap(),
            json!({"status": "submitted", "plan_hash": h, "verified": false,
                   "warning": "Readback was not fully verified; do not resend."})
        );
        assert_eq!(
            execute(&p, &c, &MemJournal::default(), Some(&h), &same, &broken).unwrap(),
            json!({"status": "submitted", "plan_hash": h, "verified": false,
                   "warning": "Independent readback failed; do not resend."})
        );
        for status in [403, 429, 500] {
            let c = FakeApi::new().on("POST tickets/123/watch", Reply::Status(status));
            let j = MemJournal::default();
            assert_eq!(
                err(execute(&p, &c, &j, Some(&h), &same, &unverified)),
                UNKNOWN
            );
            assert_eq!(j.events(), ["started"]);
        }
        let c = FakeApi::new().on("POST tickets/123/watch", Reply::Status(422));
        let j = MemJournal::default();
        assert_eq!(
            err(execute(&p, &c, &j, Some(&h), &same, &unverified)),
            "write rejected (HTTP 422); no retry"
        );
        assert_eq!(j.events(), ["started", "failed"]);
        let d = WritePlan {
            method: "DELETE".into(),
            path: "changes/1/unwatch?user_id=9".into(),
            ..plan("null")
        };
        let c = FakeApi::new().ok("DELETE changes/1/unwatch?user_id=9", json!({}));
        let dh = d.hash();
        let same = || Ok(d.clone());
        execute(
            &d,
            &c,
            &MemJournal::default(),
            Some(&dh),
            &same,
            &unverified,
        )
        .unwrap();
        let w = c.writes();
        assert_eq!(
            (w[0].path.as_str(), &w[0].body, &w[0].raw),
            ("changes/1/unwatch?user_id=9", &None, &None)
        );
    }
}
