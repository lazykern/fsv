//! Reviewed native change-task deletion (`change_task_delete.py`).
//!
//! Verification requires every other task id to remain. Target absence alone
//! is not enough.

use std::collections::HashSet;

use serde_json::{Value, json};

use fsv_core::error::Result;
use fsv_core::resource::{CHANGES, parse_id};
use fsv_py::py_str;

use crate::change_task_update::inventory;
use crate::reviewed_write::{WritePlan, execute, refused};
use crate::service::{Api, Journal, snapshot_hash};
use crate::watchers::positive_id;

fn dumps(v: &Value) -> String {
    fsv_py::Dumps::default().sort_keys(true).dump(v)
}

/// Port of change_task_delete.build_plan.
pub fn build_plan(c: &dyn Api, target: &Value, task: &Value) -> Result<WritePlan> {
    let target = positive_id(&json!(parse_id(&py_str(target), &CHANGES)?))?;
    let task = positive_id(task)?;
    let rows = inventory(c, target)?;
    if !rows
        .iter()
        .any(|row| positive_id(row.get("id").unwrap_or(&Value::Null)).ok() == Some(task))
    {
        return Err(refused("exact task unavailable"));
    }
    Ok(WritePlan {
        action: "change_task_delete".into(),
        target,
        tenant: c.domain(),
        method: "PUT".into(),
        path: format!("changes/{target}/tasks/update"),
        payload_json: dumps(&json!({"id": task.to_string(), "action": "delete"})),
        context_hash: snapshot_hash(&json!(rows))?,
    })
}

/// Port of change_task_delete.send.
pub fn send(
    plan: &WritePlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let task = positive_id(&plan.payload()["id"])?;
    let target = plan.target;
    let before: HashSet<i64> = inventory(c, target)?
        .iter()
        .map(|row| positive_id(row.get("id").unwrap_or(&Value::Null)))
        .collect::<Result<_>>()?;
    let verify = move |_: &Value| {
        let rows = inventory(c, target)?;
        let actual: HashSet<i64> = rows
            .iter()
            .map(|row| positive_id(row.get("id").unwrap_or(&Value::Null)))
            .collect::<Result<_>>()?;
        let mut expected = before.clone();
        expected.remove(&task);
        Ok(crate::service::obj(json!({
            "task_id": task,
            "verified": actual == expected,
            "readback": rows,
        })))
    };
    let rebuild = || build_plan(c, &json!(target), &json!(task));
    execute(plan, c, journal, reviewed_hash, &rebuild, &verify)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::service::testing::*;
    use fsv_core::error::Error;

    struct World {
        tasks: Vec<Value>,
        /// The fake clears every task. Target absence alone must not verify.
        clear_all: bool,
    }

    fn api(state: Arc<Mutex<World>>) -> FnApi {
        let mut api = FnApi::new(move |call: &Call| {
            let mut s = state.lock().unwrap();
            if call.method == "GET" {
                return Ok(json!({"tasks": s.tasks.clone(), "meta": {"has_next": false}}));
            }
            if s.clear_all {
                s.tasks.clear();
            } else if let Some(id) = call
                .body
                .as_ref()
                .and_then(|b| b.get("id"))
                .and_then(Value::as_str)
            {
                s.tasks.retain(|row| {
                    row.get("id").and_then(Value::as_i64).map(|n| n.to_string())
                        != Some(id.to_string())
                });
            }
            Ok(json!({"ok": true}))
        });
        api.domain = "acme.example.com".into();
        api
    }

    fn world(clear_all: bool) -> Arc<Mutex<World>> {
        Arc::new(Mutex::new(World {
            tasks: vec![
                json!({"id": 501, "title": "Acme target"}),
                json!({"id": 502, "title": "Globex retained"}),
            ],
            clear_all,
        }))
    }

    #[test]
    fn cleared_inventory_is_unverified_and_never_replayed() {
        let state = world(true);
        let c = api(state);
        let journal = MemJournal::default();
        let plan = build_plan(&c, &json!(123), &json!(501)).unwrap();
        assert_eq!(plan.action, "change_task_delete");
        assert_eq!(plan.path, "changes/123/tasks/update");
        assert_eq!(plan.payload(), json!({"action": "delete", "id": "501"}));
        let result = send(&plan, &c, &journal, Some(&plan.hash())).unwrap();
        assert_eq!(result["verified"], json!(false));
        assert_eq!(c.writes().len(), 1);
        let err = send(&plan, &c, &journal, Some(&plan.hash())).unwrap_err();
        assert!(err.to_string().contains("already sent at"), "{err}");
        assert!(matches!(err, Error::Exit { code: 2, .. }));
        assert_eq!(c.writes().len(), 1);
    }

    #[test]
    fn retained_sibling_verifies() {
        let state = world(false);
        let c = api(state);
        let journal = MemJournal::default();
        let plan = build_plan(&c, &json!("CHN-123"), &json!(501)).unwrap();
        let result = send(&plan, &c, &journal, Some(&plan.hash())).unwrap();
        assert_eq!(result["verified"], json!(true));
        assert_eq!(result["task_id"], json!(501));
        assert_eq!(c.writes().len(), 1);
    }

    #[test]
    fn missing_task_is_refused_before_a_write() {
        let state = world(false);
        let c = api(state);
        let err = build_plan(&c, &json!(123), &json!(999)).unwrap_err();
        assert_eq!(err.to_string(), "exact task unavailable");
        assert!(matches!(err, Error::Exit { code: 2, .. }));
        assert!(c.writes().is_empty());
    }
}
