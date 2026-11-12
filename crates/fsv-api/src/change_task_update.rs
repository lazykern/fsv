//! Reviewed change-task updates (`change_task_update.py`).
//!
//! The PUT carries only the fields the caller supplied. Readback checks the
//! task independently; the response body is not proof.

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::pystr::{casefold, strip};
use fsv_core::resource::{CHANGES, parse_id};
use fsv_forms::pydt::datetime_fromisoformat;
use fsv_py::{py_str, truthy};

use crate::readback::{same_instant, same_text, submitted_value_matches};
use crate::reviewed_write::{WritePlan, execute, refused};
use crate::service::{Api, Journal, MAX_DETAIL_PAGES, collect_pages, obj, snapshot_hash};
use crate::task_status;
use crate::tasks;
use crate::ticket_tasks;
use crate::watchers::positive_id;

/// Port of change_task_update.TaskUpdatePlan. The hash is the write's;
/// `task_id` and `inputs_json` are not bound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskUpdatePlan {
    pub write: WritePlan,
    pub task_id: i64,
    pub inputs_json: String,
}

impl TaskUpdatePlan {
    pub fn hash(&self) -> String {
        self.write.hash()
    }

    pub fn review(&self) -> Map<String, Value> {
        self.write.review()
    }
}

fn dumps(v: &Value) -> String {
    fsv_py::Dumps::default().sort_keys(true).dump(v)
}

fn value_error(e: crate::QueryError) -> Error {
    Error::value(e.0)
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
    "values",
];

/// Port of change_task_update.inventory: every task on the change, in page
/// order, with unique positive ids.
pub fn inventory(c: &dyn Api, target: i64) -> Result<Vec<Value>> {
    let params = obj(json!({"page": 1, "per_page": 100}));
    let pages = collect_pages(
        |p, q| c.int_get(p, q),
        &format!("changes/{target}/tasks"),
        "tasks",
        Some(&params),
        "change tasks",
        MAX_DETAIL_PAGES,
    )?;
    if pages.missing.is_some() || !pages.meta.get("complete").is_some_and(truthy) {
        return Err(refused("complete task inventory unavailable"));
    }
    let mut ids = Vec::with_capacity(pages.rows.len());
    for row in &pages.rows {
        ids.push(positive_id(row.get("id").unwrap_or(&Value::Null))?);
    }
    if ids.iter().collect::<HashSet<_>>().len() != ids.len() {
        return Err(refused("duplicate task identities"));
    }
    Ok(pages.rows)
}

fn fields_from(c: &dyn Api) -> Result<Vec<Value>> {
    let params = obj(json!({"include_deleted_choices": "true"}));
    let data = c.int_get("change_task_fields", Some(params))?;
    task_status::fields_from_payload(&data).map_err(value_error)
}

/// Port of change_task_update.reminder: one active runtime choice, by label
/// or by its seconds id.
pub fn reminder(value: &Value, fields: &[Value]) -> Result<i64> {
    let matches: Vec<&Value> = fields
        .iter()
        .filter(|f| {
            f.get("name") == Some(&json!("notify_before")) && !f.get("deleted").is_some_and(truthy)
        })
        .collect();
    if matches.len() != 1 {
        return Err(refused("runtime reminder metadata unavailable"));
    }
    let want = casefold(&py_str(value));
    let choices: Vec<&Map<String, Value>> = matches[0]
        .get("choices")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter(|c| {
            !["deleted", "deleted_at", "deactivated"]
                .iter()
                .any(|k| c.get(*k).is_some_and(truthy))
                && {
                    let label = c
                        .get("value")
                        .filter(|v| truthy(v))
                        .or_else(|| c.get("label"))
                        .map(py_str)
                        .unwrap_or_else(|| "None".into());
                    let id = c.get("id").map(py_str).unwrap_or_else(|| "None".into());
                    want == id || want == casefold(&label)
                }
        })
        .collect();
    let [choice] = choices[..] else {
        return Err(Error::value(
            "notify-before must match a runtime reminder label or seconds value",
        ));
    };
    let id = choice.get("id").unwrap_or(&Value::Null);
    if id.is_boolean() {
        return Err(Error::value(
            "notify-before must match a runtime reminder label or seconds value",
        ));
    }
    let text = py_str(id);
    if !text.is_ascii() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::value(
            "notify-before must match a runtime reminder label or seconds value",
        ));
    }
    text.parse::<i64>().map_err(|_| {
        Error::value("notify-before must match a runtime reminder label or seconds value")
    })
}

fn assign_party(c: &dyn Api, key: &str, value: &Value) -> Result<Value> {
    if value.as_str() == Some("none") {
        return Ok(Value::Null);
    }
    let text = match value {
        Value::String(s) => s.clone(),
        other => py_str(other),
    };
    let id = if key == "owner" {
        tasks::resolve_owner(c, Some(&text))
    } else {
        tasks::resolve_group(c, Some(&text))
    };
    let id = id.map_err(|err| match err {
        Error::Exit { message, .. } => Error::value(message),
        other => other,
    })?;
    Ok(match id {
        Some(n) => json!(n),
        None => Value::Null,
    })
}

fn planned_date(value: &Value) -> Result<Value> {
    let Some(text) = value.as_str() else {
        return Err(Error::value(
            "planned dates require ISO-8601 date/time with explicit offset, whole seconds (no host timezone inferred)",
        ));
    };
    Ok(match tasks::date(Some(text))? {
        Some(s) => Value::String(s),
        None => Value::Null,
    })
}

fn planned_effort(value: &Value) -> Result<Value> {
    let Some(text) = value.as_str() else {
        return Err(Error::value(
            "planned effort must be duration text (1h30m) or HH:MM (01:30)",
        ));
    };
    Ok(match tasks::effort(Some(text))? {
        Some(s) => Value::String(s),
        None => Value::Null,
    })
}

/// Aware instant, or `None` when the value is missing. A non-text or naive
/// value is the error `datetime.fromisoformat` / comparison would raise.
fn instant(value: &Value) -> Result<Option<jiff::Timestamp>> {
    if !truthy(value) {
        return Ok(None);
    }
    let Some(text) = value.as_str() else {
        return Err(Error::other("planned date is not text"));
    };
    let text = text.replace('Z', "+00:00");
    let Some((dt, off)) = datetime_fromisoformat(&text) else {
        return Err(Error::value(format!("Invalid isoformat string: {text:?}")));
    };
    let Some(off) = off else {
        return Err(Error::other(
            "can't compare offset-naive and offset-aware datetimes",
        ));
    };
    off.to_timestamp(dt)
        .map(Some)
        .map_err(|e| Error::value(e.to_string()))
}

fn picked(payload: &Map<String, Value>, before: &Value, key: &str) -> Value {
    payload
        .get(key)
        .cloned()
        .or_else(|| before.get(key).cloned())
        .unwrap_or(Value::Null)
}

fn object_map(value: Option<&Value>) -> Result<Map<String, Value>> {
    match value {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(Value::Object(m)) => Ok(m.clone()),
        Some(_) => Err(Error::other("custom_fields is not a dict")),
    }
}

fn required_for_closure(field: &Value) -> bool {
    if field.get("required_for_closure") == Some(&Value::Bool(true)) {
        return true;
    }
    match field
        .get("field_options")
        .filter(|v| truthy(v))
        .and_then(|o| o.get("required_for_closure"))
    {
        Some(Value::Bool(true)) => true,
        Some(Value::String(s)) if s == "true" => true,
        _ => false,
    }
}

fn closure_missing(
    payload: &Map<String, Value>,
    before: &Value,
    fields: &[Value],
) -> Result<Vec<String>> {
    if !payload.contains_key("status") {
        return Ok(Vec::new());
    }
    let mut custom = object_map(before.get("custom_fields"))?;
    custom.extend(object_map(payload.get("custom_fields"))?);
    let (_, label) =
        task_status::resolve(&py_str(&payload["status"]), fields).map_err(value_error)?;
    if casefold(&label) != "completed" {
        return Ok(Vec::new());
    }
    let mut missing = Vec::new();
    for field in fields {
        if !required_for_closure(field) {
            continue;
        }
        let name = match field.get("name") {
            Some(Value::String(s)) => s.clone(),
            Some(other) => py_str(other),
            None => String::new(),
        };
        let value = if let Some(v) = payload.get(&name) {
            v.clone()
        } else if let Some(v) = before.get(&name) {
            v.clone()
        } else {
            let short = name.strip_prefix("cf_").unwrap_or(&name);
            custom.get(short).cloned().unwrap_or(Value::Null)
        };
        if value.is_null() || value.as_str() == Some("") {
            let label = field
                .get("label")
                .filter(|v| truthy(v))
                .map(py_str)
                .unwrap_or(name);
            missing.push(label);
        }
    }
    Ok(missing)
}

/// Port of change_task_update.build_plan.
pub fn build_plan(
    c: &dyn Api,
    target: &Value,
    task: &Value,
    inputs: &Map<String, Value>,
) -> Result<TaskUpdatePlan> {
    let target = positive_id(&json!(parse_id(&py_str(target), &CHANGES)?))?;
    let task = positive_id(task)?;
    if inputs.is_empty() || inputs.keys().any(|k| !ALLOWED.contains(&k.as_str())) {
        return Err(Error::value(
            "provide captured task fields; unsupported fields cannot be sent",
        ));
    }
    let before = inventory(c, target)?
        .into_iter()
        .find(|row| positive_id(row.get("id").unwrap_or(&Value::Null)).ok() == Some(task))
        .ok_or_else(|| refused("exact task unavailable"))?;
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
            "planned_start" | "planned_end" => {
                payload.insert(format!("{key}_date"), planned_date(value)?);
            }
            "planned_effort" => {
                payload.insert(key.clone(), planned_effort(value)?);
            }
            "notify_before" => {
                payload.insert(key.clone(), json!(reminder(value, &fields)?));
            }
            "values" => {
                let Some(map) = value.as_object() else {
                    return Err(Error::value("custom values must be an object"));
                };
                payload.insert(
                    "custom_fields".into(),
                    Value::Object(ticket_tasks::custom_values(&fields, map)?),
                );
            }
            _ => {
                return Err(Error::value(
                    "provide captured task fields; unsupported fields cannot be sent",
                ));
            }
        }
    }
    let start = picked(&payload, &before, "planned_start_date");
    let end = picked(&payload, &before, "planned_end_date");
    if let (Some(start), Some(end)) = (instant(&start)?, instant(&end)?)
        && end < start
    {
        return Err(Error::value("planned end must not precede start"));
    }
    let missing = closure_missing(&payload, &before, &fields)?;
    if !missing.is_empty() {
        return Err(refused(format!(
            "required closure task fields missing: {}",
            missing.join(", ")
        )));
    }
    Ok(TaskUpdatePlan {
        write: WritePlan {
            action: "change_task_update".into(),
            target,
            tenant: c.domain(),
            method: "PUT".into(),
            path: format!("changes/{target}/tasks/{task}"),
            payload_json: dumps(&Value::Object(payload)),
            context_hash: snapshot_hash(&json!({"task": before, "fields": fields}))?,
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
    let mut exact = json!({});
    for row in inventory(c, target)? {
        if positive_id(row.get("id").unwrap_or(&Value::Null))? == task_id {
            exact = row;
            break;
        }
    }
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
    let present = !row.is_empty();
    let fields_verified = present && results.values().all(|v| v == "matched");
    let verified = present
        && results
            .iter()
            .all(|(k, v)| v == "matched" || (k == "notify_before" && v == "unavailable"));
    let mut out = obj(json!({
        "verified": verified,
        "fields_verified": fields_verified,
        "fields": results,
        "readback": exact,
    }));
    if results.get("notify_before") == Some(&json!("unavailable")) {
        out.insert(
            "warning".into(),
            json!("Reminder absent from GET remains unverified; do not resend."),
        );
    }
    Ok(out)
}

/// Port of change_task_update.send.
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::service::testing::*;

    struct World {
        task: Value,
        fields: Value,
        persist: bool,
        after: Option<String>,
    }

    fn default_world() -> World {
        World {
            task: json!({
                "id": 17, "title": "Acme task", "owner_id": 81, "group_id": 91, "status": 1,
                "planned_start_date": "2026-12-01T10:00:00Z",
                "planned_end_date": "2026-12-01T11:00:00Z"
            }),
            fields: json!([
                {"name": "status", "choices": [{"id": "1", "value": "Open"}, {"id": "7", "value": "In Progress"}]},
                {"name": "notify_before", "choices": [{"id": "0", "value": "Never"}, {"id": "900", "value": "15 Minutes"}]}
            ]),
            persist: true,
            after: None,
        }
    }

    fn api(state: Arc<Mutex<World>>) -> FnApi {
        let mut api = FnApi::new(move |call: &Call| {
            let mut s = state.lock().unwrap();
            if call.method == "GET" {
                if call.path == "change_task_fields" {
                    return Ok(json!({"change_task_fields": s.fields.clone()}));
                }
                return Ok(json!({"tasks": [s.task.clone()], "meta": {"has_next": false}}));
            }
            let body = call.body.clone().unwrap_or(Value::Null);
            if s.persist
                && let (Some(task), Some(body)) = (s.task.as_object_mut(), body.as_object())
            {
                for (k, v) in body {
                    task.insert(k.clone(), v.clone());
                }
                if body.contains_key("owner_id") {
                    let owner = task.remove("owner_id").unwrap_or(Value::Null);
                    task.insert("agent_id".into(), owner);
                }
                if let Some(desc) = body.get("description").and_then(Value::as_str) {
                    task.insert(
                        "description".into(),
                        json!(format!("<div style='font:inherit'>{desc}</div>")),
                    );
                }
                if body.contains_key("planned_start_date") {
                    task.insert("planned_start_date".into(), json!("2026-12-01T10:15:00Z"));
                }
            }
            let after = s.after.clone();
            if let Some(after) = after.as_deref() {
                let effort = if after == "effort-mismatch" {
                    "2h"
                } else {
                    "1h"
                };
                s.task
                    .as_object_mut()
                    .unwrap()
                    .insert("planned_effort".into(), json!(effort));
                s.task.as_object_mut().unwrap().remove("notify_before");
                if after == "reminder-mismatch" {
                    s.task
                        .as_object_mut()
                        .unwrap()
                        .insert("notify_before".into(), json!(0));
                }
                if after == "title-missing" {
                    s.task.as_object_mut().unwrap().remove("title");
                }
            }
            Ok(json!({"task": body}))
        });
        api.domain = "acme.example.com".into();
        api
    }

    #[test]
    fn cases_match_python() {
        let oracle: Value =
            serde_json::from_str(include_str!("testdata/change_task_update_oracle.json")).unwrap();
        let cases = oracle["cases"].as_object().unwrap();
        assert_eq!(cases.len(), 14);
        for (name, case) in cases {
            let state = Arc::new(Mutex::new(default_world()));
            let c = api(state.clone());
            let journal = MemJournal::default();
            let mut plan: Option<TaskUpdatePlan> = None;
            for (i, (op, want)) in case["ops"]
                .as_array()
                .unwrap()
                .iter()
                .zip(case["outputs"].as_array().unwrap())
                .enumerate()
            {
                let ctx = format!("{name} op {i}");
                let got = match op["op"].as_str().unwrap() {
                    "set" => {
                        let mut s = state.lock().unwrap();
                        if let Some(v) = op.get("persist").and_then(Value::as_bool) {
                            s.persist = v;
                        }
                        if let Some(v) = op.get("task_title") {
                            s.task["title"] = v.clone();
                        }
                        if let Some(v) = op.get("fields") {
                            s.fields = v.clone();
                        }
                        if let Some(v) = op.get("after").and_then(Value::as_str) {
                            s.after = Some(v.to_string());
                        }
                        Ok(json!({}))
                    }
                    "build" => {
                        let inputs = op["inputs"].as_object().cloned().unwrap_or_default();
                        build_plan(
                            &c,
                            op.get("target").unwrap_or(&json!(23)),
                            op.get("task").unwrap_or(&json!(17)),
                            &inputs,
                        )
                        .map(|p| {
                            let out = json!({"plan": {
                                "action": p.write.action,
                                "target": p.write.target,
                                "tenant": p.write.tenant,
                                "method": p.write.method,
                                "path": p.write.path,
                                "payload": p.write.payload(),
                                "context_hash": p.write.context_hash,
                                "hash": p.hash(),
                                "review": p.review(),
                                "task_id": p.task_id,
                                "inputs": serde_json::from_str::<Value>(&p.inputs_json).unwrap(),
                            }});
                            plan = Some(p);
                            out
                        })
                    }
                    "send" => {
                        let p = plan.as_ref().unwrap();
                        let full = p.hash();
                        let reviewed = if op.get("hash").and_then(Value::as_str) == Some("prefix") {
                            Some(full[..12].to_string())
                        } else {
                            Some(full)
                        };
                        send(p, &c, &journal, reviewed.as_deref()).map(|r| json!({"result": r}))
                    }
                    other => panic!("{ctx} {other}"),
                };
                match got {
                    Ok(mut v) => {
                        v["writes"] = json!(c.writes().len());
                        assert_eq!(v, *want, "{ctx}");
                    }
                    Err(e) => {
                        assert_eq!(
                            e.to_string(),
                            want["error"]["msg"].as_str().unwrap(),
                            "{ctx}"
                        );
                        match want["error"]["type"].as_str().unwrap() {
                            "ValueError" => assert!(matches!(e, Error::Value(_)), "{ctx}"),
                            "ReviewedWriteError" => {
                                assert!(matches!(e, Error::Exit { code: 2, .. }), "{ctx} {e:?}")
                            }
                            other => panic!("{ctx} {other}"),
                        }
                        assert_eq!(
                            c.writes().len() as u64,
                            want["writes"].as_u64().unwrap(),
                            "{ctx}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn owner_lookup_failure_is_a_value_error() {
        let agents = json!([
            {"id": "agent@example.com", "value": "Alex Acme", "user_id": 81},
            {"id": "agent@example.com", "value": "Alex Acme", "user_id": 81}
        ]);
        let api = FakeApi::new()
            .ok(
                "GET changes/23/tasks",
                json!({"tasks": [{"id": 17, "title": "Acme"}], "meta": {"has_next": false}}),
            )
            .ok("GET change_task_fields", json!({"change_task_fields": []}))
            .ok("GET autocomplete/agents", agents);
        let inputs = obj(json!({"owner": "agent@example.com"}));
        let err = build_plan(&api, &json!("CHN-23"), &json!(17), &inputs).unwrap_err();
        assert!(matches!(err, Error::Value(_)), "{err:?}");
        assert!(err.to_string().contains("2 exact active candidates"));
    }
}
