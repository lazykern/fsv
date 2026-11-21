//! Single-attempt mutation primitive for captured internal contracts
//! (`reviewed_write.py`).
//!
//! Builders supply exact methods/paths and independent verification. This is
//! not an arbitrary API command and never retries a mutation.

use serde_json::{Map, Value, json};

use fsv_core::client::Body;
use fsv_core::error::{Error, Result};

use crate::service::{
    Api, Journal, definite_rejection, last_event, obj, plan_hash, replay_refusal,
};

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

/// Port of fsv.reviewed_write.StatefulWritePlan: a write that may
/// legitimately repeat the same payload. `context_hash` binds a record-state
/// token (`journal.repeat_token`), shown in the review; `prior_context` is the
/// context without it (pre-token journals).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatefulWritePlan {
    pub plan: WritePlan,
    pub state_json: String,
    pub prior_context: String,
}

impl std::ops::Deref for StatefulWritePlan {
    type Target = WritePlan;
    fn deref(&self) -> &WritePlan {
        &self.plan
    }
}

impl StatefulWritePlan {
    /// A stateful write from [`stateful_context`]'s parts.
    pub fn new(plan: WritePlan, state: (String, String, String)) -> StatefulWritePlan {
        let (_, state_json, prior_context) = state;
        StatefulWritePlan {
            plan,
            state_json,
            prior_context,
        }
    }

    /// Port of StatefulWritePlan.review.
    pub fn review(&self) -> Map<String, Value> {
        let mut out = self.plan.review();
        if !self.state_json.is_empty() {
            out.insert(
                "record_state".into(),
                serde_json::from_str(&self.state_json).unwrap_or(Value::Null),
            );
        }
        out
    }

    /// Port of `prior_guards`: the hash of this plan without its record-state
    /// token, so an unresolved send journaled before the token was bound
    /// still refuses.
    pub fn prior_guards(&self) -> Vec<String> {
        prior_guards(&self.plan, &self.prior_context)
    }
}

/// Port of `reviewed_write.stateful_context`: (context hash with the token,
/// token JSON, context hash without it).
pub fn stateful_context(
    context: &Value,
    token: &Map<String, Value>,
) -> Result<(String, String, String)> {
    let with = crate::service::snapshot_hash(&json!({"context": context, "state": token}))?;
    let state = fsv_py::Dumps::default()
        .sort_keys(true)
        .dump(&Value::Object(token.clone()));
    Ok((with, state, crate::service::snapshot_hash(context)?))
}

/// Port of `reviewed_write.prior_guards` for a plan and its `prior_context`.
pub fn prior_guards(plan: &WritePlan, prior_context: &str) -> Vec<String> {
    if prior_context.is_empty() || prior_context == plan.context_hash {
        return Vec::new();
    }
    let mut older = plan.clone();
    older.context_hash = prior_context.to_string();
    vec![older.hash()]
}

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
    execute_rejecting(plan, hash, c, journal, reviewed_hash, rebuild, verify, None)
}

/// Port of fsv.reviewed_write.execute with `rejection=`: [`execute_as`]
/// where `rejection` words the refusal for a definite (4xx) rejection.
#[allow(clippy::too_many_arguments)]
pub fn execute_rejecting(
    plan: &WritePlan,
    hash: &str,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
    rebuild: &dyn Fn() -> Result<String>,
    verify: &dyn Fn(&Value) -> Result<Map<String, Value>>,
    rejection: Option<&dyn Fn(&fsv_core::ApiError) -> String>,
) -> Result<Value> {
    execute_guarded(
        plan,
        hash,
        c,
        journal,
        reviewed_hash,
        rebuild,
        verify,
        rejection,
        &[],
    )
}

/// [`execute_rejecting`] with `guards=`: older hashes of the same plan (e.g.
/// before a record-state token was bound); an unresolved (`started`) one
/// still refuses.
#[allow(clippy::too_many_arguments)]
pub fn execute_guarded(
    plan: &WritePlan,
    hash: &str,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
    rebuild: &dyn Fn() -> Result<String>,
    verify: &dyn Fn(&Value) -> Result<Map<String, Value>>,
    rejection: Option<&dyn Fn(&fsv_core::ApiError) -> String>,
    guards: &[String],
) -> Result<Value> {
    let Some(reviewed) = reviewed_hash else {
        return Ok(json!({"status": "cancelled", "verified": false}));
    };
    let hash = hash.to_string();
    if plan.tenant.is_empty() || plan.tenant != c.domain() || reviewed != hash {
        return Err(refused("tenant or full review hash changed; review again"));
    }
    if let Some(refusal) = replay_refusal(
        journal,
        &plan.action,
        &hash,
        "write was sent or outcome unknown; inspect target and journal before retry",
    ) {
        return Err(refused(refusal));
    }
    for guard in guards {
        if last_event(journal, &plan.action, guard).as_deref() == Some("started") {
            return Err(refused(fsv_core::journal::note_refusal(
                "write was sent or outcome unknown; inspect target and journal before retry",
            )));
        }
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
            let Some(api) = e.as_api() else {
                return Err(refused(format!(
                    "write outcome unknown ({}); inspect target and journal before retry",
                    fsv_core::error::exception_name(&e)
                )));
            };
            let (detail, reason) = fsv_core::error::http_detail(api);
            let status = Some(api.status);
            if definite_rejection(&e).is_some() {
                journal.record_plan("failed", &hash, &plan.action, Some(&target), None);
                let text = match rejection {
                    Some(word) => word(api),
                    None => {
                        let tail = if reason.is_empty() {
                            String::new()
                        } else {
                            format!(": {reason}")
                        };
                        format!("write rejected (HTTP {}){tail}; no retry", api.status)
                    }
                };
                return Err(fsv_core::error::reviewed_write_error(
                    text,
                    status,
                    Some(reason),
                ));
            }
            return Err(fsv_core::error::reviewed_write_error(
                format!(
                    "write outcome unknown ({detail}); inspect target and journal before retry"
                ),
                status,
                Some(reason),
            ));
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
    if result.get("verified") == Some(&Value::Bool(true)) {
        journal.record_verified(&hash, &plan.action, Some(&target));
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
        fsv_core::config::use_domain_on_thread("acme.freshservice.com");
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
            "this exact plan was already sent at an earlier run and the server answered; nothing was sent now. Plans are single-use and this one hashes the same because the target, payload and record state are unchanged; there is no override. Make a real change (e.g. edit a field of the record, or change the requested values), then review again for a new plan hash; do not replay it"
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
                format!(
                    "write outcome unknown (HTTP {status}: nope); inspect target and journal before retry"
                )
            );
            assert_eq!(j.events(), ["started"]);
        }
        let c = FakeApi::new().on("POST tickets/123/watch", Reply::Status(422));
        let j = MemJournal::default();
        assert_eq!(
            err(execute(&p, &c, &j, Some(&h), &same, &unverified)),
            "write rejected (HTTP 422): nope; no retry"
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

    /// `test_update_error_paths.test_execute_names_status_and_message`.
    #[test]
    fn execute_names_status_and_message() {
        let p = plan(r#"{"priority": 3}"#);
        let h = p.hash();
        let same = || Ok(p.clone());
        let verify =
            |_: &Value| -> Result<Map<String, Value>> { Ok(obj(json!({"verified": true}))) };
        let html = Value::String(
            "<html><head><style>p{}</style></head><body><h1>Service Unavailable</h1><p>Try later</p></body></html>"
                .into(),
        );
        let cases = [
            (
                422,
                json!({"description": "Validation failed", "errors": [{"field": "priority", "message": "Synthetic invalid"}]}),
                "rejected",
                "priority: Synthetic invalid",
            ),
            (
                500,
                json!({"message": "Synthetic server error"}),
                "outcome unknown",
                "Synthetic server error",
            ),
            (
                503,
                html,
                "outcome unknown",
                "Service Unavailable Try later",
            ),
            (
                429,
                json!({"message": "Synthetic rate limit"}),
                "outcome unknown",
                "Synthetic rate limit",
            ),
            (403, json!(""), "outcome unknown", ""),
        ];
        for (status, body, kind, message) in cases {
            let c = FakeApi::new().on("POST tickets/123/watch", Reply::Fail(status, body));
            let e = execute(&p, &c, &MemJournal::default(), Some(&h), &same, &verify).unwrap_err();
            let text = e.to_string();
            assert!(
                text.contains(kind) && text.contains(&format!("HTTP {status}")),
                "{text}"
            );
            assert_eq!(c.writes().len(), 1);
            assert!(!text.contains('<') && !text.contains("style"), "{text}");
            if message.is_empty() {
                assert!(text.contains("empty response body"), "{text}");
            } else {
                assert!(text.contains(message), "{text}");
            }
            let answer = fsv_core::error::write_answer(&e).expect("answer");
            assert_eq!(answer.http_status, Some(status));
            assert_eq!(answer.message, text);
            assert!(answer.server_message.as_deref().unwrap().contains(message));
        }
        let c = FakeApi::new().on("POST tickets/123/watch", Reply::Session);
        assert!(
            err(execute(
                &p,
                &c,
                &MemJournal::default(),
                Some(&h),
                &same,
                &verify
            ))
            .contains("outcome unknown (SessionError)")
        );
    }
}
