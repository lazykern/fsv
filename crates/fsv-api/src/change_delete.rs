//! Reviewed soft deletion of one exact change (`change_delete.py`).
//!
//! Accepted evidence: `DELETE /api/_/changes/{id}` with no body returned 204,
//! and an independent exact GET then returned HTTP 200 with `change.deleted:
//! true`. A 200 detail after soft deletion is not a failed delete. Restoration
//! and bulk change deletion are not captured.

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::resource::{CHANGES, parse_id};

use crate::attachments::owner_row;
use crate::delete_state::state_token;
use crate::readback::bounded_readback;
use crate::reviewed_write::{WritePlan, execute, refused};
use crate::service::{Api, Journal, obj, snapshot_hash};
use crate::watchers::positive_id;

fn exact(c: &dyn Api, cid: i64) -> Result<Map<String, Value>> {
    match owner_row(c, "changes", cid) {
        Err(Error::Exit { code: 2, .. }) => Err(refused("exact change unavailable")),
        other => other,
    }
}

/// Port of change_delete.build_plan.
pub fn build_plan(c: &dyn Api, target: &str) -> Result<WritePlan> {
    let cid = positive_id(&json!(parse_id(target, &CHANGES)?))?;
    let row = exact(c, cid)?;
    if row.get("deleted") != Some(&Value::Bool(false)) {
        return Err(refused(
            "change must be readable with deleted:false before deletion",
        ));
    }
    if !row.get("subject").is_some_and(Value::is_string) {
        return Err(refused("exact change subject unavailable"));
    }
    // `delete_state_tokens([row], [cid], ())`: no change restore is
    // captured, so the count is always 0.
    let token = state_token(&row, 0);
    Ok(WritePlan {
        action: "change_delete".into(),
        target: cid,
        tenant: c.domain(),
        method: "DELETE".into(),
        path: format!("changes/{cid}"),
        payload_json: "null".into(),
        context_hash: snapshot_hash(&json!({"change": row, "state": token}))?,
    })
}

/// Port of change_delete.review.
pub fn review(plan: &WritePlan, c: &dyn Api) -> Result<Map<String, Value>> {
    let row = exact(c, plan.target)?;
    let mut out = plan.review();
    out.insert("change_id".into(), json!(plan.target));
    out.insert(
        "subject".into(),
        row.get("subject").cloned().unwrap_or(Value::Null),
    );
    out.insert(
        "warning".into(),
        json!(
            "Destructive soft delete of one change. Restoration is not a captured fsv contract; unknown outcomes must be inspected, never automatically resent."
        ),
    );
    Ok(out)
}

/// Port of change_delete.send. `reviewed_hash` `None` cancels without the
/// destructive acknowledgement; a real send requires it.
pub fn send(
    plan: &WritePlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
    confirm_destructive: bool,
) -> Result<Value> {
    if reviewed_hash.is_some() && !confirm_destructive {
        return Err(refused(
            "change delete requires --confirm-destructive after inspecting the review",
        ));
    }
    let target = plan.target;
    let verify = move |_: &Value| {
        let row = exact(c, target)?;
        let state = match row.get("deleted") {
            None => "unavailable",
            Some(Value::Bool(true)) => "matched",
            Some(_) => "mismatched",
        };
        Ok(obj(json!({
            "change_id": target,
            "fields": {"deleted": state},
            "verified": state == "matched",
        })))
    };
    let bounded = move |r: &Value| Ok(bounded_readback(|| verify(r)));
    let rebuild = || build_plan(c, &target.to_string());
    execute(plan, c, journal, reviewed_hash, &rebuild, &bounded)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use fsv_core::ApiError;

    use super::*;
    use crate::readback::with_readback_sleep;
    use crate::service::testing::{Call, FnApi, MemJournal};

    static ORACLE: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| {
        serde_json::from_str(include_str!("testdata/change_delete_oracle.json")).unwrap()
    });

    struct World {
        row: Map<String, Value>,
        lag: usize,
        fail: Option<Value>,
        wrote: bool,
        reads_after: usize,
    }

    fn world(row: Value) -> World {
        World {
            row: row.as_object().unwrap().clone(),
            lag: 0,
            fail: None,
            wrote: false,
            reads_after: 0,
        }
    }

    fn api(state: Arc<Mutex<World>>) -> FnApi {
        FnApi::new(move |call: &Call| {
            let mut s = state.lock().unwrap();
            if call.method == "GET" {
                let mut row = s.row.clone();
                if s.wrote {
                    s.reads_after += 1;
                    if s.lag > 0 {
                        s.lag -= 1;
                        row.insert("deleted".into(), json!(false));
                    }
                }
                return Ok(json!({"change": row}));
            }
            if let Some(fail) = s.fail.clone() {
                if let Some(status) = fail.as_u64() {
                    return Err(ApiError::new(status as u16, json!({"errors": "synthetic"})).into());
                }
                return Err(Error::other("disconnect"));
            }
            s.row.insert("deleted".into(), json!(true));
            s.wrote = true;
            Ok(Value::Null)
        })
    }

    fn case(name: &str) -> &'static Value {
        ORACLE["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap()
    }

    fn deletes(c: &FnApi) -> Vec<Call> {
        c.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| call.method == "DELETE")
            .cloned()
            .collect()
    }

    #[test]
    fn bodyless_delete_matches_python() {
        for name in ["ok", "lag2", "lag100"] {
            let want = case(name);
            let state = Arc::new(Mutex::new(world(json!({
                "id": 123, "subject": "Acme rollout", "deleted": false
            }))));
            let lag = match name {
                "lag2" => 2,
                "lag100" => 100,
                _ => 0,
            };
            state.lock().unwrap().lag = lag;
            let c = api(Arc::clone(&state));
            let plan = build_plan(&c, "CHN-123").unwrap();
            assert_eq!(plan.hash(), want["hash"].as_str().unwrap(), "{name}");
            assert_eq!(
                Value::Object(review(&plan, &c).unwrap()),
                want["review"],
                "{name}"
            );
            let journal = MemJournal::default();
            let sleeps = Arc::new(Mutex::new(Vec::new()));
            let seen = Arc::clone(&sleeps);
            let sent = with_readback_sleep(
                move |s| seen.lock().unwrap().push(s),
                || send(&plan, &c, &journal, Some(&plan.hash()), true),
            )
            .unwrap();
            assert_eq!(sent, want["result"], "{name}");
            assert_eq!(json!(*sleeps.lock().unwrap()), want["sleeps"], "{name}");
            assert_eq!(
                json!(state.lock().unwrap().reads_after),
                want["reads_after"],
                "{name}"
            );
            let wrote = deletes(&c);
            assert_eq!(wrote.len(), 1, "{name}");
            assert_eq!(wrote[0].path, "changes/123");
            assert!(wrote[0].body.is_none() && wrote[0].raw.is_none(), "{name}");
            let err = send(&plan, &c, &journal, Some(&plan.hash()), true).unwrap_err();
            assert_eq!(err.to_string(), want["replay"].as_str().unwrap(), "{name}");
            assert_eq!(deletes(&c).len(), 1, "{name}");
        }
    }

    #[test]
    fn confirmation_drift_and_unproven_targets_match_python() {
        let state = Arc::new(Mutex::new(world(json!({
            "id": 123, "subject": "Acme rollout", "deleted": false
        }))));
        let c = api(Arc::clone(&state));
        let plan = build_plan(&c, "CHN-123").unwrap();
        let err = send(&plan, &c, &MemJournal::default(), Some(&plan.hash()), false).unwrap_err();
        assert_eq!(
            err.to_string(),
            case("no_confirm")["send_err"].as_str().unwrap()
        );
        assert!(deletes(&c).is_empty());

        let state = Arc::new(Mutex::new(world(json!({
            "id": 123, "subject": "Acme rollout", "deleted": false
        }))));
        let c = api(Arc::clone(&state));
        let plan = build_plan(&c, "CHN-123").unwrap();
        state
            .lock()
            .unwrap()
            .row
            .insert("subject".into(), json!("Globex changed"));
        let err = send(&plan, &c, &MemJournal::default(), Some(&plan.hash()), true).unwrap_err();
        assert_eq!(err.to_string(), case("drift")["err"].as_str().unwrap());
        assert!(deletes(&c).is_empty());

        for (name, row) in [
            (
                "deleted_true",
                json!({"id": 123, "subject": "Acme rollout", "deleted": true}),
            ),
            (
                "deleted_missing",
                json!({"id": 123, "subject": "Acme rollout"}),
            ),
            (
                "deleted_str",
                json!({"id": 123, "subject": "Acme rollout", "deleted": "false"}),
            ),
            (
                "subject_null",
                json!({"id": 123, "subject": null, "deleted": false}),
            ),
        ] {
            let c = api(Arc::new(Mutex::new(world(row))));
            let err = build_plan(&c, "123").unwrap_err();
            assert_eq!(
                err.to_string(),
                case(name)["build_err"].as_str().unwrap(),
                "{name}"
            );
            assert!(deletes(&c).is_empty());
        }
    }

    #[test]
    fn failures_are_never_replayed() {
        for name in ["403", "429", "422", "disconnect"] {
            let want = case(name);
            let state = Arc::new(Mutex::new(world(json!({
                "id": 123, "subject": "Acme rollout", "deleted": false
            }))));
            state.lock().unwrap().fail = Some(match name {
                "disconnect" => json!("disconnect"),
                other => json!(other.parse::<u64>().unwrap()),
            });
            let c = api(Arc::clone(&state));
            let plan = build_plan(&c, "CHN-123").unwrap();
            let journal = MemJournal::default();
            let err = send(&plan, &c, &journal, Some(&plan.hash()), true).unwrap_err();
            assert_eq!(err.to_string(), want["err"].as_str().unwrap(), "{name}");
            if let Some(replay) = want["replay"].as_str() {
                let err = send(&plan, &c, &journal, Some(&plan.hash()), true).unwrap_err();
                assert_eq!(err.to_string(), replay, "{name}");
            }
            assert_eq!(json!(deletes(&c).len()), want["writes"], "{name}");
        }
    }

    #[test]
    fn delete_after_web_restore_is_fresh_but_unchanged_record_is_a_replay() {
        let state = Arc::new(Mutex::new(world(json!({
            "id": 123, "subject": "Acme rollout", "deleted": false
        }))));
        let c = api(Arc::clone(&state));
        let journal = MemJournal::default();
        let first = build_plan(&c, "123").unwrap();
        let sent = with_readback_sleep(
            |_| {},
            || send(&first, &c, &journal, Some(&first.hash()), true),
        )
        .unwrap();
        assert_eq!(sent["verified"], json!(true));
        state
            .lock()
            .unwrap()
            .row
            .insert("deleted".into(), json!(false));
        let same = build_plan(&c, "123").unwrap();
        assert_eq!(same.hash(), first.hash());
        let err = send(&same, &c, &journal, Some(&same.hash()), true).unwrap_err();
        assert!(err.to_string().contains("was sent"), "{err}");
        // the server bumps updated_at on restore
        state
            .lock()
            .unwrap()
            .row
            .insert("updated_at".into(), json!("2030-01-01T00:00:09Z"));
        let again = build_plan(&c, "123").unwrap();
        assert_ne!(again.hash(), first.hash());
        let sent = with_readback_sleep(
            |_| {},
            || send(&again, &c, &journal, Some(&again.hash()), true),
        )
        .unwrap();
        assert_eq!(sent["verified"], json!(true));
        assert_eq!(deletes(&c).len(), 2);
    }
}
