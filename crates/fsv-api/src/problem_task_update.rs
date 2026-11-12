//! Captured problem-task updates, completion and native delete
//! (`problem_task_update.py`). The exact task GET verifies every field.

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::pystr::{casefold, strip};
use fsv_core::resource::{PROBLEMS, parse_id};
use fsv_py::py_str;

use crate::attachments::owner_row;
use crate::change_task_update::{
    TaskUpdatePlan, assign_party, closure_missing, instant, picked, planned_date, planned_effort,
    reminder, value_error,
};
use crate::readback::{same_instant, same_text, submitted_value_matches};
use crate::reviewed_write::{WritePlan, execute, refused};
use crate::service::{
    Api, Journal, MAX_DETAIL_PAGES, collect_pages, obj, snapshot_hash, stable_snapshot,
};
use crate::task_status;
use crate::tasks;
use crate::watchers::positive_id;

fn dumps(v: &Value) -> String {
    fsv_py::Dumps::default().sort_keys(true).dump(v)
}

const ALLOWED: &[&str] = &[
    "title",
    "status",
    "owner",
    "group",
    "note",
    "planned_start",
    "planned_end",
    "planned_effort",
    "notify_before",
    "due_date",
];

const WARNING: &str = "Task completion may rewrite planned dates and stop OLA timers; inspect the exact readback. Never resend.";

fn row_id(row: &Value) -> Result<i64> {
    positive_id(row.get("id").unwrap_or(&Value::Null))
}

/// Port of `problem_task_update.inventory`: every task on the problem, in
/// page order, with unique positive ids.
pub fn inventory(c: &dyn Api, target: i64) -> Result<Vec<Value>> {
    let params = obj(json!({"page": 1, "per_page": 100}));
    let pages = collect_pages(
        |p, q| c.int_get(p, q),
        &format!("problems/{target}/tasks"),
        "tasks",
        Some(&params),
        "problem tasks",
        MAX_DETAIL_PAGES,
    )?;
    if pages.missing.is_some() || !pages.meta.get("complete").is_some_and(fsv_py::truthy) {
        return Err(refused("complete task inventory unavailable"));
    }
    let ids = pages.rows.iter().map(row_id).collect::<Result<Vec<_>>>()?;
    if ids.iter().collect::<HashSet<_>>().len() != ids.len() {
        return Err(refused("duplicate task identities"));
    }
    Ok(pages.rows)
}

/// Port of `problem_task_update.fields_from`.
pub fn fields_from(c: &dyn Api) -> Result<Vec<Value>> {
    let raw = c.int_get(
        "problem_task_fields",
        Some(obj(json!({"include_deleted_choices": "true"}))),
    )?;
    ["problem_task_fields", "task_fields", "fields"]
        .iter()
        .find_map(|k| raw.get(*k).and_then(Value::as_array).cloned())
        .ok_or_else(|| refused("runtime problem-task fields unavailable"))
}

/// Port of `problem_task_update.exact_task`: the native task GET, bound to
/// its problem.
pub fn exact_task(c: &dyn Api, target: i64, task: i64) -> Result<Value> {
    let raw = c.int_get(&format!("problems/{target}/tasks/{task}"), None)?;
    let row = match &raw {
        Value::Object(m) => m.get("task").cloned().unwrap_or(raw.clone()),
        _ => json!({}),
    };
    let text = |k: &str| row.get(k).map_or("None".into(), py_str);
    if row_id(&row)? != task
        || text("taskable_display_id") != target.to_string()
        || row.get("taskable_type") != Some(&json!("Itil::Problem"))
    {
        return Err(refused("exact problem task identity/parent unavailable"));
    }
    Ok(row)
}

fn completed_alone(payload: &Map<String, Value>, fields: &[Value]) -> Result<()> {
    let Some(status) = payload.get("status") else {
        return Ok(());
    };
    let (_, label) = task_status::resolve(&py_str(status), fields).map_err(value_error)?;
    if casefold(&label) == "completed" && (payload.len() != 1) {
        return Err(refused(
            "captured completion is a separate status-only write",
        ));
    }
    Ok(())
}

/// Port of `problem_task_update.build_plan`.
pub fn build_plan(
    c: &dyn Api,
    target: &Value,
    task: &Value,
    inputs: &Map<String, Value>,
) -> Result<TaskUpdatePlan> {
    let target = positive_id(&json!(parse_id(&py_str(target), &PROBLEMS)?))?;
    let task = positive_id(task)?;
    if inputs.is_empty() || inputs.keys().any(|k| !ALLOWED.contains(&k.as_str())) {
        return Err(Error::value(
            "provide captured task fields; unsupported fields cannot be sent",
        ));
    }
    if !inventory(c, target)?
        .iter()
        .any(|row| row_id(row).ok() == Some(task))
    {
        return Err(refused("exact task unavailable"));
    }
    let before = exact_task(c, target, task)?;
    let parent = owner_row(c, "problems", target)?;
    let fields = fields_from(c)?;
    let mut payload = Map::new();
    for (key, value) in inputs {
        match key.as_str() {
            "title" => {
                let Some(text) = value.as_str() else {
                    return Err(Error::value("nonempty task title required"));
                };
                if strip(text).is_empty() || text.chars().any(|ch| (ch as u32) < 32) {
                    return Err(Error::value("nonempty task title required"));
                }
                payload.insert(key.clone(), Value::String(text.to_string()));
            }
            "status" => {
                let (id, _) = task_status::resolve(&py_str(value), &fields).map_err(value_error)?;
                payload.insert(key.clone(), json!(id));
            }
            "owner" | "group" => {
                if value.as_str() == Some("none") {
                    return Err(refused("null owner/group update is not captured"));
                }
                payload.insert(format!("{key}_id"), assign_party(c, key, value)?);
            }
            "note" => {
                let Some(text) = value.as_str() else {
                    return Err(Error::value(
                        "note must be plain text without unsafe control characters",
                    ));
                };
                tasks::note(Some(text))?;
                let html = fsv_core::pystr::html_escape(text, true).replace('\n', "<br>");
                payload.insert("description".into(), json!(format!("<div>{html}</div>")));
            }
            "due_date" => {
                payload.insert(key.clone(), planned_date(value)?);
            }
            "planned_start" | "planned_end" => {
                payload.insert(format!("{key}_date"), planned_date(value)?);
            }
            "planned_effort" => {
                payload.insert(key.clone(), planned_effort(value)?);
            }
            "notify_before" => {
                payload.insert(key.clone(), json!(reminder(value, &fields)?));
            }
            _ => unreachable!("checked against ALLOWED"),
        }
    }
    if payload.values().any(Value::is_null) {
        return Err(refused("clearing task fields is not a captured update"));
    }
    let start = picked(&payload, &before, "planned_start_date");
    let end = picked(&payload, &before, "planned_end_date");
    if let (Some(start), Some(end)) = (instant(&start)?, instant(&end)?)
        && end < start
    {
        return Err(Error::value("planned end must not precede start"));
    }
    completed_alone(&payload, &fields)?;
    let missing = closure_missing(&payload, &before, &fields)?;
    if !missing.is_empty() {
        return Err(refused(format!(
            "required closure task fields missing: {}",
            missing.join(", ")
        )));
    }
    Ok(TaskUpdatePlan {
        write: WritePlan {
            action: "problem_task_update".into(),
            target,
            tenant: c.domain(),
            method: "PUT".into(),
            path: format!("problems/{target}/tasks/{task}"),
            payload_json: dumps(&Value::Object(payload)),
            context_hash: snapshot_hash(
                &json!({"task": before, "parent": Value::Object(parent), "fields": fields}),
            )?,
        },
        task_id: task,
        inputs_json: dumps(&Value::Object(inputs.clone())),
    })
}

fn verify_readback(
    c: &dyn Api,
    target: i64,
    task_id: i64,
    payload: &Value,
) -> Result<Map<String, Value>> {
    let exact = exact_task(c, target, task_id)?;
    let row = exact.as_object().cloned().unwrap_or_default();
    let mut results = Map::new();
    for (key, expected) in payload.as_object().into_iter().flatten() {
        let actual_key = if key == "owner_id" && !row.contains_key(key) {
            "agent_id"
        } else {
            key.as_str()
        };
        let Some(actual) = row.get(actual_key) else {
            results.insert(key.clone(), json!("unavailable"));
            continue;
        };
        let matched = if key.ends_with("_date") {
            same_instant(actual, expected)
        } else if key == "description" {
            same_text(actual, expected)
        } else if key == "planned_effort" {
            tasks::same_effort(actual, expected)
        } else {
            submitted_value_matches(actual, expected)
        };
        results.insert(
            key.clone(),
            json!(if matched { "matched" } else { "mismatched" }),
        );
    }
    let verified = !row.is_empty() && results.values().all(|v| v == "matched");
    Ok(obj(json!({
        "verified": verified,
        "fields_verified": verified,
        "fields": results,
        "readback": exact,
        "warning": WARNING,
    })))
}

/// Port of `problem_task_update.send`.
pub fn send(
    plan: &TaskUpdatePlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let inputs: Map<String, Value> =
        serde_json::from_str(&plan.inputs_json).map_err(|e| Error::value(e.to_string()))?;
    let target = plan.write.target;
    let task_id = plan.task_id;
    let payload = plan.write.payload();
    let rebuild = || build_plan(c, &json!(target), &json!(task_id), &inputs).map(|p| p.write);
    let verify = |_: &Value| verify_readback(c, target, task_id, &payload);
    execute(&plan.write, c, journal, reviewed_hash, &rebuild, &verify)
}

/// Port of `problem_task_update.delete_plan`.
pub fn delete_plan(c: &dyn Api, target: &Value, task: &Value) -> Result<WritePlan> {
    let target = positive_id(&json!(parse_id(&py_str(target), &PROBLEMS)?))?;
    let task = positive_id(task)?;
    let parent = owner_row(c, "problems", target)?;
    let rows = inventory(c, target)?;
    if !rows.iter().any(|row| row_id(row).ok() == Some(task)) {
        return Err(refused("exact problem task unavailable"));
    }
    let before = exact_task(c, target, task)?;
    Ok(WritePlan {
        action: "problem_task_delete".into(),
        target,
        tenant: c.domain(),
        method: "PUT".into(),
        path: format!("problems/{target}/tasks/update"),
        payload_json: dumps(&json!({"id": task.to_string(), "action": "delete"})),
        context_hash: dumps(&stable_snapshot(
            &json!({"parent": Value::Object(parent), "rows": rows, "task": before}),
        )?),
    })
}

/// Port of `problem_task_update.delete`: removed, or still listed exactly
/// once with `deleted: true`, and every other task unchanged.
pub fn delete(
    plan: &WritePlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let task = positive_id(&plan.payload()["id"])?;
    let target = plan.target;
    let context: Value = serde_json::from_str(&plan.context_hash)?;
    let rebuild = || delete_plan(c, &json!(target), &json!(task));
    let verify = |_: &Value| -> Result<Map<String, Value>> {
        let rows = inventory(c, target)?;
        let exact: Vec<&Value> = rows
            .iter()
            .filter(|r| row_id(r).ok() == Some(task))
            .collect();
        let before: HashSet<i64> = context["rows"]
            .as_array()
            .into_iter()
            .flatten()
            .map(row_id)
            .collect::<Result<_>>()?;
        let after: HashSet<i64> = rows.iter().map(row_id).collect::<Result<_>>()?;
        let mut gone = before.clone();
        gone.remove(&task);
        let removed = (exact.is_empty() && after == gone)
            || (exact.len() == 1
                && exact[0].get("deleted") == Some(&Value::Bool(true))
                && after == before);
        Ok(obj(json!({"task_id": task, "verified": removed})))
    };
    execute(plan, c, journal, reviewed_hash, &rebuild, &verify)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use serde_json::{Value, json};

    use super::*;
    use crate::module_time::tests::{ORACLE, api, assert_plan, assert_result, refusal_kind, world};
    use crate::service::testing::MemJournal;

    fn full() -> Map<String, Value> {
        obj(json!({
            "title": "Acme updated", "note": "Acme task note", "planned_effort": "1h30m",
            "due_date": "2026-01-03T09:00:00Z", "planned_start": "2026-01-02T09:00:00Z",
            "planned_end": "2026-01-02T10:30:00Z", "owner": "901", "group": "81",
            "status": "In Progress", "notify_before": "15 Minutes",
        }))
    }

    #[test]
    fn updates_match_python() {
        for (key, inputs, omit) in [
            ("task-full", full(), None),
            ("task-complete", obj(json!({"status": "Completed"})), None),
            (
                "task-missing",
                obj(json!({"notify_before": "15 Minutes"})),
                Some("notify_before"),
            ),
        ] {
            let want = &ORACLE[key];
            let state = Arc::new(Mutex::new(world()));
            let c = api(Arc::clone(&state));
            let plan = build_plan(&c, &json!("PRB-123"), &json!(71), &inputs).unwrap();
            assert_plan(&plan.write, &want["plan"], key);
            assert_eq!(plan.inputs_json, want["inputs"].as_str().unwrap(), "{key}");
            state.lock().unwrap().omit = omit;
            let journal = MemJournal::default();
            let result = send(&plan, &c, &journal, Some(&plan.hash())).unwrap();
            assert_result(&result, &want["result"], key);
            let err = send(&plan, &c, &journal, Some(&plan.hash())).unwrap_err();
            assert!(err.to_string().contains("was sent"), "{err}");
            assert_eq!(json!(state.lock().unwrap().writes), want["writes"], "{key}");
        }
    }

    #[test]
    fn delete_matches_python() {
        let want = &ORACLE["task-delete"];
        let state = Arc::new(Mutex::new(world()));
        let c = api(Arc::clone(&state));
        let plan = delete_plan(&c, &json!(123), &json!(71)).unwrap();
        assert_plan(&plan, &want["plan"], "task-delete");
        let journal = MemJournal::default();
        let result = delete(&plan, &c, &journal, Some(&plan.hash())).unwrap();
        assert_result(&result, &want["result"], "task-delete");
        let err = delete(&plan, &c, &journal, Some(&plan.hash())).unwrap_err();
        assert!(err.to_string().contains("was sent"), "{err}");
        assert_eq!(json!(state.lock().unwrap().writes), want["writes"]);
    }

    #[test]
    fn disconnect_never_replays() {
        let state = Arc::new(Mutex::new(world()));
        let c = api(Arc::clone(&state));
        let inputs = obj(json!({"notify_before": "15 Minutes"}));
        let plan = build_plan(&c, &json!(123), &json!(71), &inputs).unwrap();
        state.lock().unwrap().failure = true;
        let journal = MemJournal::default();
        let err = send(&plan, &c, &journal, Some(&plan.hash())).unwrap_err();
        assert!(err.to_string().contains("unknown"), "{err}");
        let err = send(&plan, &c, &journal, Some(&plan.hash())).unwrap_err();
        assert!(err.to_string().contains("was sent"), "{err}");
        assert_eq!(state.lock().unwrap().writes.len(), 1);
    }

    #[test]
    fn refusals_match_python() {
        let cases = [
            (
                "combined_completion",
                json!({"title": "Acme", "status": "Completed"}),
                false,
            ),
            ("closure_missing", json!({"status": "Completed"}), true),
            ("null_owner", json!({"owner": "none"}), false),
            ("unsupported", json!({"values": {}}), false),
            (
                "end_before_start",
                json!({"planned_end": "2026-01-02T08:00:00Z"}),
                false,
            ),
        ];
        for (key, inputs, closure) in cases {
            let mut w = world();
            if closure {
                w.fields
                    .push(json!({"name": "cf_answer", "label": "Closure answer",
                    "field_options": {"required_for_closure": "true"}}));
            }
            let state = Arc::new(Mutex::new(w));
            let c = api(Arc::clone(&state));
            let err = build_plan(&c, &json!(123), &json!(71), &obj(inputs)).unwrap_err();
            let want = &ORACLE["refusals"][key];
            assert_eq!(
                refusal_kind(&err),
                want["kind"].as_str().unwrap(),
                "{key}: {err}"
            );
            assert_eq!(err.to_string(), want["message"].as_str().unwrap(), "{key}");
            assert!(state.lock().unwrap().writes.is_empty(), "{key}");
        }
        let c = api(Arc::new(Mutex::new(world())));
        let err = delete_plan(&c, &json!(123), &json!(72)).unwrap_err();
        assert_eq!(
            err.to_string(),
            ORACLE["refusals"]["delete_missing"]["message"]
                .as_str()
                .unwrap()
        );
    }
}
