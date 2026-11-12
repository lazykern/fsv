//! Captured problem known-error mark; reversal is deliberately unsupported
//! (`problem_known_error.py`).

use serde_json::{Map, Value, json};

use fsv_core::error::Result;
use fsv_core::resource::{PROBLEMS, parse_id};
use fsv_py::py_str;

use crate::reviewed_write::{WritePlan, execute, refused};
use crate::service::{Api, Journal, obj, snapshot_hash};
use crate::watchers::positive_id;

/// Port of fsv.problem_known_error.record: the exact problem with a boolean
/// `known_error`.
pub fn record(c: &dyn Api, target: i64) -> Result<Map<String, Value>> {
    let raw = c.int_get(&format!("problems/{target}"), None)?;
    let row = match &raw {
        Value::Object(m) => m.get("problem").unwrap_or(&raw),
        _ => &Value::Null,
    };
    match row {
        Value::Object(r)
            if py_str(r.get("id").unwrap_or(&Value::Null)) == target.to_string()
                && r.get("known_error").is_some_and(Value::is_boolean) =>
        {
            Ok(r.clone())
        }
        _ => Err(refused(
            "exact problem identity/known-error state is unavailable",
        )),
    }
}

/// Port of fsv.problem_known_error.build_plan.
pub fn build_plan(c: &dyn Api, target: &str) -> Result<WritePlan> {
    let cid = positive_id(&json!(parse_id(target, &PROBLEMS)?))?;
    let row = record(c, cid)?;
    Ok(WritePlan {
        action: "problem_known_error_mark".into(),
        target: cid,
        tenant: c.domain(),
        method: "PUT".into(),
        path: format!("problems/{cid}"),
        payload_json: r#"{"known_error": true}"#.into(),
        context_hash: snapshot_hash(&Value::Object(row))?,
    })
}

/// Port of fsv.problem_known_error.send.
pub fn send(
    plan: &WritePlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let target = plan.target.to_string();
    let fresh = build_plan(c, &target)?;
    let Some(reviewed) = reviewed_hash else {
        return Ok(json!({"status": "cancelled", "verified": false}));
    };
    let hash = plan.hash();
    if reviewed != hash || fresh.hash() != hash || plan.tenant != c.domain() {
        return Err(refused("problem or review changed; review again"));
    }
    if record(c, plan.target)?.get("known_error") == Some(&Value::Bool(true)) {
        return Ok(json!({"status": "already_in_requested_state", "verified": true}));
    }
    let rebuild = || build_plan(c, &target);
    let verify = |_: &Value| -> Result<Map<String, Value>> {
        let marked = record(c, plan.target)?.get("known_error") == Some(&Value::Bool(true));
        Ok(obj(json!({"problem_id": plan.target, "verified": marked})))
    };
    execute(plan, c, journal, reviewed_hash, &rebuild, &verify)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::service::testing::*;
    use fsv_core::ApiError;

    struct World {
        row: Value,
        failure: Option<u16>,
    }

    fn client(w: &Arc<Mutex<World>>) -> FnApi {
        let w = w.clone();
        FnApi::new(move |call| {
            let mut w = w.lock().unwrap();
            if call.method == "GET" {
                return Ok(json!({"problem": w.row}));
            }
            if let Some(s) = w.failure {
                return Err(ApiError::new(s, json!("uncertain")).into());
            }
            if let (Some(row), Some(Value::Object(body))) = (w.row.as_object_mut(), &call.body) {
                row.extend(body.clone());
            }
            Ok(json!({}))
        })
    }

    fn world() -> Arc<Mutex<World>> {
        Arc::new(Mutex::new(World {
            row: json!({"id": 123, "known_error": false, "subject": "Acme problem"}),
            failure: None,
        }))
    }

    fn err<T: std::fmt::Debug>(r: Result<T>) -> String {
        r.unwrap_err().to_string()
    }

    /// `test_problem_known_error.py::test_exact_mark_and_noop`.
    #[test]
    fn exact_mark_and_noop() {
        let oracle: Value =
            serde_json::from_str(include_str!("testdata/reviewed_oracle.json")).unwrap();
        let w = world();
        let c = client(&w);
        let j = MemJournal::default();
        let p = build_plan(&c, "PRB-123").unwrap();
        assert_eq!(
            (p.payload(), p.method.as_str()),
            (json!({"known_error": true}), "PUT")
        );
        assert_eq!(p.hash(), oracle["known_error"]["hash"]);
        assert_eq!(
            send(&p, &c, &j, Some(&p.hash())).unwrap(),
            json!({"status": "submitted", "plan_hash": p.hash(), "verified": true, "problem_id": 123})
        );
        let q = build_plan(&c, "123").unwrap();
        assert_eq!(
            send(&q, &c, &j, Some(&q.hash())).unwrap(),
            json!({"status": "already_in_requested_state", "verified": true})
        );
        let w0 = c.writes();
        assert_eq!(w0.len(), 1);
        assert_eq!(w0[0].raw.as_deref(), Some(&br#"{"known_error": true}"#[..]));
    }

    /// `test_ambiguous_failure_and_stale_review`.
    #[test]
    fn ambiguous_failure_and_stale_review() {
        let w = world();
        let c = client(&w);
        let j = MemJournal::default();
        let p = build_plan(&c, "123").unwrap();
        w.lock().unwrap().row["subject"] = json!("Globex");
        assert_eq!(
            err(send(&p, &c, &j, Some(&p.hash()))),
            "problem or review changed; review again"
        );
        let q = build_plan(&c, "123").unwrap();
        w.lock().unwrap().failure = Some(403);
        for _ in 0..2 {
            assert!(err(send(&q, &c, &j, Some(&q.hash()))).contains("unknown"));
        }
        assert_eq!(c.writes().len(), 1);
        assert_eq!(j.events(), ["started"]);
    }

    #[test]
    fn record_requires_exact_identity_and_bool() {
        for row in [
            json!({"problem": {"id": 9, "known_error": false}}),
            json!({"problem": {"id": 123, "known_error": 0}}),
            json!([]),
        ] {
            let c = FnApi::new(move |_| Ok(row.clone()));
            assert_eq!(
                err(record(&c, 123)),
                "exact problem identity/known-error state is unavailable"
            );
        }
        let c = FnApi::new(|_| Ok(json!({"id": "123", "known_error": true})));
        assert_eq!(record(&c, 123).unwrap()["known_error"], true);
    }
}
