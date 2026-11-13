//! Single-ticket spam / not spam, captured from the ticket page and the Spam
//! view (`ticket_spam.py`).
//!
//! * spam: `PUT tickets/{id}/spam` with `{}` (204, no confirm in the UI);
//! * not spam: `PUT tickets/{id}/unspam` without a body (204).
//!
//! Readback: the exact ticket's `spam` flag. Bulk spam lives in `tickets bulk`.

use serde_json::{Map, Value, json};

use fsv_core::error::Result;
use fsv_core::resource::{TICKETS, parse_id};

use crate::properties::record;
use crate::reviewed_write::{WritePlan, execute, refused};
use crate::service::{Api, Journal, obj, snapshot_hash};
use crate::ticket_trash::exact;
use crate::watchers::positive_id;

/// Port of ticket_spam.build_plan.
pub fn build_plan(c: &dyn Api, ticket: &str, spam: bool) -> Result<WritePlan> {
    let tid = positive_id(&json!(parse_id(ticket, &TICKETS)?))?;
    let row = exact(c, tid)?;
    let Some(Value::Bool(flag)) = row.get("spam") else {
        return Err(refused(format!(
            "ticket {tid}: spam flag unavailable; nothing was sent"
        )));
    };
    if *flag == spam {
        let state = if spam { "already" } else { "not" };
        return Err(refused(format!(
            "ticket {tid} is {state} marked as spam; nothing was sent"
        )));
    }
    if row.get("deleted") == Some(&Value::Bool(true)) {
        return Err(refused(format!(
            "ticket {tid} is in the trash; restore it first; nothing was sent"
        )));
    }
    let (action, body, verb) = if spam {
        ("ticket_spam", "{}", "spam")
    } else {
        ("ticket_unspam", "null", "unspam")
    };
    Ok(WritePlan {
        action: action.into(),
        target: tid,
        tenant: c.domain(),
        method: "PUT".into(),
        path: format!("tickets/{tid}/{verb}"),
        payload_json: body.into(),
        context_hash: snapshot_hash(&json!({"ticket": row}))?,
    })
}

/// Port of ticket_spam.send: spam needs `confirm_destructive`.
pub fn send(
    plan: &WritePlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
    confirm_destructive: bool,
) -> Result<Value> {
    let spam = plan.action == "ticket_spam";
    if spam && !confirm_destructive && reviewed_hash.is_some() {
        return Err(refused(
            "spam hides the ticket from every view; pass --confirm-destructive after review",
        ));
    }
    let target = plan.target;
    let verify = |_: &Value| -> Result<Map<String, Value>> {
        let flag = record(c, target)?
            .get("spam")
            .cloned()
            .unwrap_or(Value::Null);
        let verified = flag == Value::Bool(spam);
        Ok(obj(
            json!({"ticket_id": target, "spam": flag, "verified": verified}),
        ))
    };
    let rebuild = || build_plan(c, &target.to_string(), spam);
    execute(plan, c, journal, reviewed_hash, &rebuild, &verify)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use fsv_core::ApiError;

    use super::*;
    use crate::service::testing::{Call, FnApi, MemJournal};

    struct World {
        spam: BTreeMap<i64, bool>,
        writes: Vec<(String, String, Option<Vec<u8>>)>,
        effect: bool,
    }

    fn tenant() -> (Arc<Mutex<World>>, FnApi) {
        let state = Arc::new(Mutex::new(World {
            spam: [(32, false), (33, true)].into_iter().collect(),
            writes: Vec::new(),
            effect: true,
        }));
        let s = Arc::clone(&state);
        let c = FnApi::new(move |call: &Call| {
            let mut s = s.lock().unwrap();
            let tid: i64 = call.path.split('/').nth(1).unwrap().parse().unwrap();
            if call.method == "GET" {
                return match s.spam.get(&tid) {
                    None => Err(ApiError::new(404, json!({})).into()),
                    Some(f) => Ok(json!({"ticket": {"id": tid, "spam": f, "deleted": false}})),
                };
            }
            s.writes
                .push((call.method.clone(), call.path.clone(), call.raw.clone()));
            if s.effect {
                s.spam.insert(tid, call.path.ends_with("/spam"));
            }
            Ok(Value::Null)
        });
        (state, c)
    }

    fn msg(r: Result<impl std::fmt::Debug>) -> String {
        r.unwrap_err().to_string()
    }

    #[test]
    fn spam_and_unspam_wires_and_flag_readback() {
        let (s, c) = tenant();
        let j = MemJournal::default();
        let p = build_plan(&c, "INC-32", true).unwrap();
        assert_eq!(
            (p.method.as_str(), p.path.as_str(), p.payload()),
            ("PUT", "tickets/32/spam", json!({}))
        );
        assert!(msg(send(&p, &c, &j, Some(&p.hash()), false)).contains("confirm-destructive"));
        assert_eq!(
            send(&p, &c, &j, Some(&p.hash()), true).unwrap()["verified"],
            json!(true)
        );
        let u = build_plan(&c, "32", false).unwrap();
        assert_eq!(
            (u.path.as_str(), u.payload()),
            ("tickets/32/unspam", Value::Null)
        );
        assert_eq!(
            send(&u, &c, &j, Some(&u.hash()), false).unwrap()["verified"],
            json!(true)
        );
        let bodies: Vec<Option<Vec<u8>>> = s
            .lock()
            .unwrap()
            .writes
            .iter()
            .map(|w| w.2.clone())
            .collect();
        assert_eq!(bodies, [Some(b"{}".to_vec()), None]);
    }

    #[test]
    fn state_guards_unverified_and_no_resend() {
        let (s, c) = tenant();
        let j = MemJournal::default();
        assert!(msg(build_plan(&c, "33", true)).contains("already"));
        assert!(msg(build_plan(&c, "32", false)).contains("not marked"));
        assert!(msg(build_plan(&c, "99", true)).contains("not found"));
        s.lock().unwrap().effect = false;
        let p = build_plan(&c, "33", false).unwrap();
        assert_eq!(
            send(&p, &c, &j, Some(&p.hash()), false).unwrap()["verified"],
            json!(false)
        );
        assert!(msg(send(&p, &c, &j, Some(&p.hash()), false)).contains("sent"));
        assert_eq!(s.lock().unwrap().writes.len(), 1);
    }
}
