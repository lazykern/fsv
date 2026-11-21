//! Plain ticket reply on the reviewed-write contract (`ticket_plain_reply.py`).
//!
//! `tickets reply ID BODY` without CC/BCC/sender/status keeps its wire (a POST
//! of `{"body": BODY}` to `tickets/ID/reply`), now with a plan hash bound to
//! the tenant, target, exact body and (empty) recipients, an optional
//! `--expect-plan`, a single-attempt journaled send and an independent
//! readback of the returned conversation by listing the ticket's
//! conversations.

use serde_json::{Value, json};

use fsv_core::client::Body;
use fsv_core::error::{Error, Result};

use crate::notes::{note_id, returned_note};
use crate::readback::same_text;
use crate::service::{
    Api, Journal, definite_rejection, dumps_bytes, last_event, load_conversations, obj, plan_hash,
    replay_refusal,
};

pub const ACTION: &str = "ticket_reply_plain";
const UNKNOWN: &str = "reply outcome unknown; inspect the ticket conversations and journal before any retry; do not resend";

/// `PlainReplyError`: a refusal that exits 2.
fn refused(msg: impl Into<String>) -> Error {
    Error::exit(2, msg)
}

/// Port of `PlainReplyPlan`; `sends` is the verified-send count of
/// `journal.repeat_token` (zero keeps the bare hash).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlainReplyPlan {
    pub ticket_id: i64,
    pub tenant: String,
    pub body: String,
    pub sends: i64,
}

impl PlainReplyPlan {
    pub fn path(&self) -> String {
        format!("tickets/{}/reply", self.ticket_id)
    }

    pub fn payload(&self) -> Value {
        json!({"body": self.body})
    }

    pub fn hash(&self) -> String {
        let mut identity = obj(json!({
            "action": ACTION, "tenant": self.tenant, "target": self.ticket_id,
            "method": "POST", "path": self.path(), "payload": self.payload(),
            "recipients": {"cc": [], "bcc": []},
        }));
        if self.sends != 0 {
            identity.insert("state".into(), json!({"sends": self.sends}));
        }
        plan_hash(&Value::Object(identity))
    }
}

/// Port of `ticket_plain_reply.build_plan`.
pub fn build_plan(
    journal: &dyn Journal,
    tenant: &str,
    ticket_id: i64,
    body: &str,
) -> Result<PlainReplyPlan> {
    if fsv_core::pystr::strip(body).is_empty() || body.contains('\0') {
        return Err(refused(
            "reply body must be nonempty text without NUL characters",
        ));
    }
    if tenant.is_empty() {
        return Err(refused("configure a tenant before replying"));
    }
    Ok(PlainReplyPlan {
        ticket_id,
        tenant: tenant.into(),
        body: body.into(),
        sends: journal.verified_sends(&[ACTION], &json!(ticket_id)),
    })
}

fn error_name(e: &Error) -> String {
    match e {
        Error::Http { kind, .. } => kind.clone(),
        Error::Session(_) => "SessionError".into(),
        _ => "Exception".into(),
    }
}

/// Port of `ticket_plain_reply.send`: one POST, journaled; readback by
/// listing the ticket's conversations.
pub fn send(plan: &PlainReplyPlan, c: &dyn Api, journal: &dyn Journal) -> Result<Value> {
    if plan.tenant != c.domain() {
        return Err(refused("tenant changed since the review; review again"));
    }
    let hash = plan.hash();
    if let Some(refusal) = replay_refusal(journal, ACTION, &hash, UNKNOWN) {
        return Err(refused(refusal));
    }
    let Ok(csrf) = c.get_csrf() else {
        return Err(refused("could not prepare the session; no reply was sent"));
    };
    let target = json!(plan.ticket_id);
    journal.record_plan("started", &hash, ACTION, Some(&target), None);
    if last_event(journal, ACTION, &hash).as_deref() != Some("started") {
        return Err(refused(
            "could not persist the reply journal; no reply was sent",
        ));
    }
    let response = match c.send_once(
        "POST",
        &plan.path(),
        Body::Raw(dumps_bytes(&plan.payload(), false)),
        None,
        &csrf,
    ) {
        Ok(r) => r,
        Err(e) => {
            if let Some(api) = e.as_api() {
                if definite_rejection(&e).is_some() {
                    journal.record_plan("failed", &hash, ACTION, Some(&target), None);
                    let reason = fsv_core::error::server_message(&api.body, 240);
                    let reason = if reason.is_empty() {
                        reason
                    } else {
                        format!(": {reason}")
                    };
                    return Err(refused(format!(
                        "reply rejected (HTTP {}){reason}; no retry",
                        api.status
                    )));
                }
                return Err(refused(format!("{UNKNOWN} (HTTP {})", api.status)));
            }
            return Err(refused(format!("{UNKNOWN} ({})", error_name(&e))));
        }
    };
    let Some(conv_id) = note_id(returned_note(&response)) else {
        return Err(refused(format!(
            "reply response has no exact conversation ID; {UNKNOWN}"
        )));
    };
    journal.record_plan("done", &hash, ACTION, Some(&target), Some(&conv_id));
    let mut result = obj(json!({
        "status": "submitted", "ticket_id": plan.ticket_id, "conversation_id": conv_id,
        "plan_hash": hash, "verified": false, "readback": null,
    }));
    if let Ok(pages) = load_conversations(c, plan.ticket_id) {
        let want = fsv_py::py_str(&conv_id);
        let exact = pages
            .rows
            .iter()
            .find(|r| r.is_object() && fsv_py::py_str(r.get("id").unwrap_or(&Value::Null)) == want)
            .cloned();
        result.insert("readback".into(), exact.clone().unwrap_or(Value::Null));
        let mut matched = false;
        if let Some(exact) = &exact {
            let actual = if exact.get("body").is_some() {
                exact.get("body")
            } else {
                exact.get("body_text")
            };
            let state = match actual {
                None | Some(Value::Null) => "unavailable",
                Some(v) if same_text(v, &json!(plan.body)) => "matched",
                Some(_) => "mismatched",
            };
            result.insert("fields".into(), json!({"body": state}));
            matched = state == "matched";
        }
        let partial = pages.meta.get("partial").is_some_and(fsv_py::truthy);
        result.insert(
            "verified".into(),
            json!(exact.is_some() && pages.missing.is_none() && !partial && matched),
        );
    }
    if result.get("verified") == Some(&Value::Bool(true)) {
        journal.record_verified(&hash, ACTION, Some(&target));
    } else {
        result.insert(
            "warning".into(),
            json!("Reply sent but conversation read-back did not verify it; do not resend."),
        );
    }
    Ok(Value::Object(result))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::service::testing::*;
    use fsv_core::ApiError;

    const T: &str = "acme.freshservice.com";
    const BODY: &str = "Acme is working on it";

    #[derive(Clone, Copy)]
    enum Mode {
        Confirm,
        Unverified,
        Server500,
        Reject400,
    }

    fn world(mode: Arc<Mutex<Mode>>) -> FnApi {
        let rows: Arc<Mutex<Vec<Value>>> = Arc::default();
        FnApi::new(move |call| {
            if call.method == "GET" {
                let rows = rows.lock().unwrap().clone();
                return Ok(json!({"conversations": rows, "meta": {"count": rows.len()}}));
            }
            match *mode.lock().unwrap() {
                Mode::Server500 => Err(ApiError::new(500, json!("boom")).into()),
                Mode::Reject400 => {
                    Err(ApiError::new(400, json!({"description": "Validation failed"})).into())
                }
                m => {
                    let mut rows = rows.lock().unwrap();
                    let id = 501 + rows.len() as i64;
                    let text = if matches!(m, Mode::Unverified) {
                        "<div>other</div>".to_string()
                    } else {
                        format!("<div>{BODY}</div>")
                    };
                    rows.push(json!({"id": id, "body": text}));
                    Ok(json!({"conversation": {"id": id, "body": BODY}}))
                }
            }
        })
    }

    fn plan(j: &MemJournal) -> PlainReplyPlan {
        build_plan(j, T, 123, BODY).unwrap()
    }

    #[test]
    fn dry_run_hash_is_stable_and_binds_tenant_and_body() {
        let j = MemJournal::default();
        let p = plan(&j);
        assert_eq!(p.hash(), plan(&j).hash());
        assert_eq!(p.sends, 0);
        let other = PlainReplyPlan {
            tenant: "globex.freshservice.com".into(),
            ..p.clone()
        };
        let edited = PlainReplyPlan {
            body: "Globex text".into(),
            ..p.clone()
        };
        assert_ne!(p.hash(), other.hash());
        assert_ne!(p.hash(), edited.hash());
        assert!(build_plan(&j, T, 123, "  ").is_err());
        assert!(build_plan(&j, "", 123, BODY).is_err());
    }

    #[test]
    fn send_verifies_never_replays_and_a_repeat_is_a_fresh_plan() {
        let c = world(Arc::new(Mutex::new(Mode::Confirm)));
        let j = MemJournal::default();
        let first = plan(&j);
        let r = send(&first, &c, &j).unwrap();
        assert_eq!(r["verified"], true);
        assert_eq!(r["conversation_id"], 501);
        assert_eq!(r["fields"], json!({"body": "matched"}));
        assert_eq!(c.writes()[0].path, "tickets/123/reply");
        assert_eq!(c.writes()[0].body, Some(json!({"body": BODY})));
        let again = send(&first, &c, &j).unwrap_err().to_string();
        assert!(again.contains("already sent"), "{again}");
        assert_eq!(c.writes().len(), 1);
        let fresh = plan(&j);
        assert_eq!(fresh.sends, 1);
        assert_ne!(fresh.hash(), first.hash());
        assert_eq!(send(&fresh, &c, &j).unwrap()["verified"], true);
    }

    #[test]
    fn unverified_and_ambiguous_sends_are_not_resent() {
        for mode in [Mode::Unverified, Mode::Server500] {
            let state = Arc::new(Mutex::new(mode));
            let c = world(state.clone());
            let j = MemJournal::default();
            let first = plan(&j);
            let r = send(&first, &c, &j);
            if matches!(mode, Mode::Unverified) {
                let r = r.unwrap();
                assert_eq!(r["verified"], false);
                assert!(r["warning"].as_str().unwrap().contains("do not resend"));
            } else {
                assert!(r.unwrap_err().to_string().contains("outcome unknown"));
            }
            *state.lock().unwrap() = Mode::Confirm;
            let second = plan(&j);
            assert_eq!(second.hash(), first.hash());
            assert!(send(&second, &c, &j).is_err());
            assert_eq!(c.writes().len(), 1);
        }
    }

    #[test]
    fn rejection_names_the_server_answer() {
        let c = world(Arc::new(Mutex::new(Mode::Reject400)));
        let j = MemJournal::default();
        let e = send(&plan(&j), &c, &j).unwrap_err().to_string();
        assert!(
            e.contains("reply rejected (HTTP 400)") && e.contains("no retry"),
            "{e}"
        );
        assert_eq!(j.events(), ["started", "failed"]);
    }

    #[test]
    fn tenant_change_after_review_refuses() {
        let c = world(Arc::new(Mutex::new(Mode::Confirm)));
        let j = MemJournal::default();
        let p = PlainReplyPlan {
            tenant: "globex.freshservice.com".into(),
            ..plan(&j)
        };
        assert!(
            send(&p, &c, &j)
                .unwrap_err()
                .to_string()
                .contains("tenant changed")
        );
        assert!(c.writes().is_empty());
    }
}
