//! Captured self-owned change time CRUD and problem time create/delete
//! (`module_time.py`). Inventory reads go through the v2 API.

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::resource::{CHANGES, PROBLEMS, parse_id};
use fsv_py::{py_str, truthy};

use crate::attachments::owner_row;
use crate::readback::{duration_seconds, same_duration, same_instant};
use crate::reviewed_inventory::{Family, complete_inventory};
use crate::reviewed_write::{WritePlan, execute, refused};
use crate::service::{Api, Journal, obj, snapshot_hash};
use crate::ticket_time::duration;
use crate::watchers::{positive_id, self_id};

/// Port of `module_time.inventory`: every time entry of the change or
/// problem, sorted by id; an incomplete listing is refused.
pub fn inventory(c: &dyn Api, module: &str, target: i64) -> Result<Vec<Value>> {
    if !matches!(module, "changes" | "problems") {
        return Err(Error::value("unsupported time-entry module"));
    }
    let params = obj(json!({"page": 1, "per_page": 30}));
    let rows = complete_inventory(
        c,
        &format!("{module}/{target}/time_entries"),
        "time_entries",
        Some(&params),
        Family::V2,
    )?;
    let mut keyed = rows
        .into_iter()
        .map(|row| Ok((positive_id(row.get("id").unwrap_or(&Value::Null))?, row)))
        .collect::<Result<Vec<(i64, Value)>>>()?;
    keyed.sort_by_key(|(id, _)| *id);
    if keyed.windows(2).any(|w| w[0].0 == w[1].0) {
        return Err(refused(
            "time-entry inventory contains duplicate identities",
        ));
    }
    Ok(keyed.into_iter().map(|(_, row)| row).collect())
}

/// Keyword arguments of [`build_plan`].
#[derive(Debug, Clone, Copy, Default)]
pub struct TimeArgs<'a> {
    /// `create` | `update` | `delete`.
    pub action: &'a str,
    pub entry_id: Option<i64>,
    pub spent: Option<&'a str>,
    pub executed_at: Option<&'a str>,
    pub note: Option<&'a str>,
    pub billable: Option<bool>,
}

/// Port of `module_time.build_plan`.
pub fn build_plan(
    c: &dyn Api,
    module: &str,
    target: &str,
    args: &TimeArgs<'_>,
) -> Result<WritePlan> {
    let action = args.action;
    if !matches!(action, "create" | "update" | "delete") {
        return Err(Error::value("unsupported time-entry action"));
    }
    if !matches!(module, "changes" | "problems") {
        return Err(Error::value("unsupported time-entry module"));
    }
    if module == "problems" && action == "update" && args.billable.is_some() {
        // The captured problem update sends only time_spent and note.
        return Err(Error::value(
            "problem time update covers --spent and --note only; billable is not a captured update field",
        ));
    }
    let kind = if module == "changes" {
        &CHANGES
    } else {
        &PROBLEMS
    };
    let cid = positive_id(&json!(parse_id(target, kind)?))?;
    let parent = owner_row(c, module, cid)?;
    let user = self_id(c)?;
    let rows = inventory(c, module, cid)?;
    if args
        .note
        .is_some_and(|n| fsv_core::pystr::strip(n).is_empty())
    {
        return Err(Error::value("note must be nonempty text when provided"));
    }
    let (method, path, payload) = if action == "create" {
        let (None, Some(spent), Some(executed_at)) = (args.entry_id, args.spent, args.executed_at)
        else {
            return Err(Error::value(
                "create requires --spent and --executed-at, without an entry ID",
            ));
        };
        let Some(when) = crate::tasks::date(Some(executed_at))? else {
            return Err(Error::value("executed-at needs an explicit offset"));
        };
        let payload = json!({"executed_at": when, "timer_running": false,
                             "billable": args.billable.unwrap_or(true),
                             "time_spent": duration(spent)?, "note": args.note.unwrap_or(""), "agent_id": user});
        ("POST", format!("{module}/{cid}/time_entries"), payload)
    } else {
        let eid = positive_id(&json!(args.entry_id))?;
        let row = rows
            .iter()
            .find(|r| positive_id(&r["id"]).ok() == Some(eid))
            .filter(|r| {
                py_str(r.get("agent_id").unwrap_or(&Value::Null)) == user.to_string()
                    && r.get("billable").is_some_and(Value::is_boolean)
                    && r.get("timer_running") == Some(&Value::Bool(false))
            })
            .ok_or_else(|| refused("exact self-owned manual entry is unavailable"))?;
        if args.executed_at.is_some() {
            return Err(Error::value(
                "changing executed-at is not a captured update contract",
            ));
        }
        let path = format!("{module}/{cid}/time_entries/{eid}");
        if action == "delete" {
            if args.spent.is_some() || args.note.is_some() || args.billable.is_some() {
                return Err(Error::value("delete does not accept time or note changes"));
            }
            ("DELETE", path, Value::Null)
        } else {
            if args.spent.is_none() && args.note.is_none() && args.billable.is_none() {
                return Err(Error::value("update requires --spent or --note"));
            }
            let (Some(current), Some(old_note)) = (row.get("time_spent"), row.get("note")) else {
                return Err(refused("entry time/note is unavailable"));
            };
            // The server returns HH:MM:SS even for minute-only writes.
            let spent = match args.spent {
                Some(s) => s.to_string(),
                None => {
                    let seconds = duration_seconds(current)?;
                    if seconds % 60 != 0 {
                        return Err(refused(
                            "entry has sub-minute precision; provide --spent explicitly",
                        ));
                    }
                    format!("{:02}:{:02}", seconds / 3600, seconds / 60 % 60)
                }
            };
            let note = args.note.map_or_else(|| old_note.clone(), |n| json!(n));
            let mut payload = json!({"time_spent": duration(&spent)?, "note": note});
            if let Some(billable) = args.billable {
                payload["billable"] = json!(billable);
            }
            ("PUT", path, payload)
        }
    };
    Ok(WritePlan {
        action: format!("{module}_time_{action}"),
        target: cid,
        tenant: c.domain(),
        method: method.into(),
        path,
        payload_json: fsv_py::Dumps::default()
            .sort_keys(true)
            .ensure_ascii(false)
            .dump(&payload),
        context_hash: snapshot_hash(
            &json!({"self": user, "parent": Value::Object(parent), "entries": rows}),
        )?,
    })
}

/// A payload string field as a rebuild argument; `None` when absent/null.
fn text_arg(payload: &Value, key: &str) -> Result<Option<String>> {
    match payload.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(Error::value(format!("{key} must be text"))),
    }
}

/// Port of `module_time.send`: one write, then the exact entry read back
/// (durations and instants compared semantically, other fields by type and
/// value).
pub fn send(
    plan: &WritePlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let (module, action) = plan
        .action
        .split_once("_time_")
        .map(|(m, a)| (m.to_string(), a.to_string()))
        .unwrap_or_default();
    let payload = plan.payload();
    let eid = if action == "create" {
        None
    } else {
        let tail = plan.path.rsplit('/').next().unwrap_or_default();
        Some(positive_id(&json!(tail))?)
    };
    let spent = text_arg(&payload, "time_spent")?;
    let executed_at = text_arg(&payload, "executed_at")?;
    let billable = match payload.get("billable") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => return Err(Error::value("billable must be boolean")),
    };
    // `payload.get("note") or None`.
    let note = match payload.get("note") {
        Some(v) if truthy(v) => match v.as_str() {
            Some(s) => Some(s.to_string()),
            None => return Err(Error::value("note must be nonempty text when provided")),
        },
        _ => None,
    };
    let target = plan.target.to_string();
    let rebuild = || {
        build_plan(
            c,
            &module,
            &target,
            &TimeArgs {
                action: &action,
                entry_id: eid,
                spent: spent.as_deref(),
                executed_at: executed_at.as_deref(),
                note: note.as_deref(),
                billable,
            },
        )
    };
    let verify = |response: &Value| -> Result<Map<String, Value>> {
        let exact_id = match eid {
            Some(id) => id,
            None => {
                let raw = match response {
                    Value::Object(m) => m.get("time_entry").unwrap_or(response),
                    _ => &Value::Null,
                };
                positive_id(raw.get("id").unwrap_or(&Value::Null))?
            }
        };
        let rows = inventory(c, &module, plan.target)?;
        let want = exact_id.to_string();
        let row = rows
            .iter()
            .find(|r| py_str(r.get("id").unwrap_or(&Value::Null)) == want);
        if action == "delete" {
            return Ok(obj(
                json!({"entry_id": exact_id, "verified": row.is_none()}),
            ));
        }
        let mut fields = Map::new();
        for (key, value) in payload.as_object().into_iter().flatten() {
            let state = match row.and_then(|r| r.get(key)) {
                None => "unavailable",
                Some(actual) => {
                    let matched = match key.as_str() {
                        "time_spent" => same_duration(actual, value),
                        "executed_at" => same_instant(actual, value),
                        _ => actual == value,
                    };
                    if matched { "matched" } else { "mismatched" }
                }
            };
            fields.insert(key.clone(), json!(state));
        }
        let verified = row.is_some() && fields.values().all(|v| v == "matched");
        Ok(obj(
            json!({"entry_id": exact_id, "fields": fields, "verified": verified}),
        ))
    };
    execute(plan, c, journal, reviewed_hash, &rebuild, &verify)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::{Arc, LazyLock, Mutex};

    use serde_json::{Value, json};

    use super::*;
    use crate::service::testing::{Call, FnApi, MemJournal};

    pub(crate) static ORACLE: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(include_str!(
            "testdata/module_time_problem_tasks_oracle.json"
        ))
        .unwrap()
    });

    pub(crate) struct World {
        pub entries: Vec<Value>,
        pub task: Value,
        pub writes: Vec<Value>,
        pub omit: Option<&'static str>,
        pub fields: Vec<Value>,
        pub deleted: bool,
        pub failure: bool,
    }

    pub(crate) fn world() -> World {
        World {
            entries: Vec::new(),
            task: json!({"id": 71, "title": "Acme task", "status": 1, "taskable_display_id": 123,
                "taskable_type": "Itil::Problem", "owner_id": 901, "group_id": 81, "notify_before": 0,
                "planned_start_date": "2026-01-02T09:00:00+00:00",
                "planned_end_date": "2026-01-02T10:00:00+00:00", "custom_fields": {}}),
            writes: Vec::new(),
            omit: None,
            fields: vec![
                json!({"name": "status", "choices": [{"id": 1, "value": "Open"},
                    {"id": 2, "value": "In Progress"}, {"id": 3, "value": "Completed"}]}),
                json!({"name": "notify_before", "choices": [{"id": 0, "value": "Never"},
                    {"id": 900, "value": "15 Minutes"}]}),
            ],
            deleted: false,
            failure: false,
        }
    }

    pub(crate) fn api(state: Arc<Mutex<World>>) -> FnApi {
        FnApi::new(move |call: &Call| {
            let mut s = state.lock().unwrap();
            let path = call.path.as_str();
            match call.method.as_str() {
                "ME" => return Ok(json!({"id": 901, "email": "agent@example.com"})),
                "V2" => {
                    assert!(path.ends_with("/time_entries"), "{path}");
                    let mut rows = s.entries.clone();
                    for r in &mut rows {
                        let spent = r["time_spent"].as_str().unwrap().to_string();
                        if spent.len() == 5 {
                            r["time_spent"] = json!(format!("{spent}:00"));
                        }
                        if let Some(k) = s.omit
                            && !s.writes.is_empty()
                        {
                            r.as_object_mut().unwrap().remove(k);
                        }
                    }
                    return Ok(json!({"time_entries": rows, "has_more": false}));
                }
                "GET" => {
                    return Ok(match path {
                        "changes/123" => json!({"change": {"id": 123, "subject": "Acme work"}}),
                        "problems/123" => json!({"problem": {"id": 123, "subject": "Acme work"}}),
                        "problem_task_fields" => json!({"problem_task_fields": s.fields.clone()}),
                        "problems/123/tasks" => {
                            let mut row = s.task.clone();
                            let m = row.as_object_mut().unwrap();
                            let owner = m.remove("owner_id").unwrap();
                            m.insert("agent_id".into(), owner);
                            m.remove("notify_before");
                            let rows = if s.deleted { json!([]) } else { json!([row]) };
                            json!({"tasks": rows, "has_more": false})
                        }
                        "problems/123/tasks/71" => {
                            let mut row = s.task.clone();
                            if let Some(k) = s.omit
                                && !s.writes.is_empty()
                            {
                                row.as_object_mut().unwrap().remove(k);
                            }
                            json!({"task": row})
                        }
                        other => panic!("unexpected GET {other}"),
                    });
                }
                _ => {}
            }
            let body = call.body.clone().unwrap_or(Value::Null);
            s.writes.push(json!([call.method, path, body]));
            if s.failure {
                return Err(Error::session("disconnect"));
            }
            if path.contains("/time_entries") {
                match call.method.as_str() {
                    "POST" => {
                        let mut row = json!({"id": 456});
                        for (k, v) in body.as_object().unwrap() {
                            row[k] = v.clone();
                        }
                        s.entries.push(row);
                    }
                    "PUT" => {
                        for (k, v) in body.as_object().unwrap() {
                            s.entries[0][k] = v.clone();
                        }
                    }
                    _ => s.entries.clear(),
                }
                if call.method == "DELETE" {
                    return Ok(Value::Null);
                }
                return Ok(json!({"time_entry": {"id": 456}}));
            }
            if path.ends_with("/tasks/update") {
                s.deleted = true;
                return Ok(Value::Null);
            }
            for (k, v) in body.as_object().unwrap() {
                s.task[k] = v.clone();
            }
            if body == json!({"status": 3}) {
                s.task["planned_start_date"] = json!("2026-01-01T09:00:00Z");
                s.task["planned_end_date"] = json!("2026-01-01T09:15:00Z");
                s.task["closed_at"] = json!("2026-01-01T09:15:00Z");
            }
            Ok(json!({"task": {"id": 71}}))
        })
    }

    pub(crate) fn assert_plan(plan: &WritePlan, want: &Value, key: &str) {
        assert_eq!(plan.action, want["action"].as_str().unwrap(), "{key}");
        assert_eq!(plan.method, want["method"].as_str().unwrap(), "{key}");
        assert_eq!(plan.path, want["path"].as_str().unwrap(), "{key}");
        assert_eq!(
            plan.payload_json,
            want["payload"].as_str().unwrap(),
            "{key}"
        );
        assert_eq!(plan.tenant, want["tenant"].as_str().unwrap(), "{key}");
        assert_eq!(plan.target, want["target"].as_i64().unwrap(), "{key}");
        assert_eq!(
            plan.context_hash,
            want["context"].as_str().unwrap(),
            "{key}"
        );
        assert_eq!(plan.hash(), want["hash"].as_str().unwrap(), "{key}");
    }

    pub(crate) fn assert_result(result: &Value, want: &Value, key: &str) {
        for k in [
            "entry_id",
            "task_id",
            "fields",
            "verified",
            "fields_verified",
            "warning",
            "readback",
        ] {
            assert_eq!(result.get(k), want.get(k), "{key} {k}");
        }
    }

    pub(crate) fn refusal_kind(err: &Error) -> &'static str {
        if matches!(err, Error::Value(_)) {
            "ValueError"
        } else {
            "ReviewedWriteError"
        }
    }

    fn create<'a>() -> TimeArgs<'a> {
        TimeArgs {
            action: "create",
            spent: Some("00:01"),
            executed_at: Some("2026-01-01T09:00:00Z"),
            note: Some("Acme work"),
            billable: Some(true),
            ..TimeArgs::default()
        }
    }

    fn delete<'a>() -> TimeArgs<'a> {
        TimeArgs {
            action: "delete",
            entry_id: Some(456),
            ..TimeArgs::default()
        }
    }

    #[test]
    fn time_flows_match_python() {
        for module in ["changes", "problems"] {
            let key = format!("time-{module}");
            let want = &ORACLE[&key];
            let state = Arc::new(Mutex::new(world()));
            let c = api(Arc::clone(&state));
            let journal = MemJournal::default();
            let mut steps = vec![create()];
            if module == "changes" {
                steps.push(TimeArgs {
                    action: "update",
                    entry_id: Some(456),
                    spent: Some("00:02"),
                    note: Some("Updated"),
                    billable: Some(false),
                    ..TimeArgs::default()
                });
            }
            steps.push(delete());
            for (i, args) in steps.iter().enumerate() {
                let plan = build_plan(&c, module, "123", args).unwrap();
                let step = &want["steps"][i];
                assert_plan(&plan, &step["plan"], &key);
                let result = send(&plan, &c, &journal, Some(&plan.hash())).unwrap();
                assert_result(&result, &step["result"], &key);
                let err = send(&plan, &c, &journal, Some(&plan.hash())).unwrap_err();
                assert!(err.to_string().contains("was sent"), "{err}");
            }
            assert_eq!(json!(state.lock().unwrap().writes), want["writes"], "{key}");
        }
    }

    /// `test_self_time_crud_exact_wires[problems]`: the captured problem
    /// update sends only time_spent and note; billable is refused.
    #[test]
    fn problem_time_update_sends_spent_and_note_only() {
        let state = Arc::new(Mutex::new(world()));
        let c = api(Arc::clone(&state));
        let journal = MemJournal::default();
        let p = build_plan(&c, "problems", "123", &create()).unwrap();
        send(&p, &c, &journal, Some(&p.hash())).unwrap();
        let err = build_plan(
            &c,
            "problems",
            "123",
            &TimeArgs {
                action: "update",
                entry_id: Some(456),
                note: Some("x"),
                billable: Some(false),
                ..TimeArgs::default()
            },
        )
        .unwrap_err();
        assert!(matches!(err, Error::Value(_)) && err.to_string().contains("billable"));
        let q = build_plan(
            &c,
            "problems",
            "123",
            &TimeArgs {
                action: "update",
                entry_id: Some(456),
                spent: Some("00:30"),
                note: Some("Acme edited"),
                ..TimeArgs::default()
            },
        )
        .unwrap();
        assert_eq!(
            (q.method.as_str(), q.path.as_str(), q.payload()),
            (
                "PUT",
                "problems/123/time_entries/456",
                json!({"time_spent": "00:30", "note": "Acme edited"})
            )
        );
        assert_eq!(send(&q, &c, &journal, Some(&q.hash())).unwrap()["verified"], true);
        assert_eq!(state.lock().unwrap().entries[0]["time_spent"], "00:30");
    }

    #[test]
    fn time_missing_field_stays_unverified() {
        let want = &ORACLE["time-missing"];
        let state = Arc::new(Mutex::new(world()));
        let c = api(Arc::clone(&state));
        let plan = build_plan(&c, "changes", "123", &create()).unwrap();
        assert_plan(&plan, &want["plan"], "time-missing");
        state.lock().unwrap().omit = Some("time_spent");
        let journal = MemJournal::default();
        let result = send(&plan, &c, &journal, Some(&plan.hash())).unwrap();
        assert_result(&result, &want["result"], "time-missing");
        assert!(send(&plan, &c, &journal, Some(&plan.hash())).is_err());
        assert_eq!(json!(state.lock().unwrap().writes), want["writes"]);
    }

    #[test]
    fn time_disconnect_never_replays() {
        let state = Arc::new(Mutex::new(world()));
        let c = api(Arc::clone(&state));
        let plan = build_plan(&c, "changes", "123", &create()).unwrap();
        state.lock().unwrap().failure = true;
        let journal = MemJournal::default();
        let err = send(&plan, &c, &journal, Some(&plan.hash())).unwrap_err();
        assert!(err.to_string().contains("unknown"), "{err}");
        let err = send(&plan, &c, &journal, Some(&plan.hash())).unwrap_err();
        assert!(err.to_string().contains("was sent"), "{err}");
        assert_eq!(state.lock().unwrap().writes.len(), 1);
    }

    #[test]
    fn time_refusals_match_python() {
        let entry = |change: Value| {
            let mut row = json!({"id": 456, "agent_id": 901, "timer_running": false,
                "billable": true, "time_spent": "00:01"});
            for (k, v) in change.as_object().unwrap() {
                row[k] = v.clone();
            }
            vec![row]
        };
        let cases: Vec<(&str, Vec<Value>, &str, TimeArgs)> = vec![
            (
                "other_owner",
                entry(json!({"agent_id": 902})),
                "changes",
                delete(),
            ),
            (
                "running",
                entry(json!({"timer_running": true})),
                "changes",
                delete(),
            ),
            (
                "problem_update",
                entry(json!({})),
                "problems",
                TimeArgs {
                    action: "update",
                    entry_id: Some(456),
                    note: Some("No wire"),
                    ..TimeArgs::default()
                },
            ),
            (
                "bad_spent",
                Vec::new(),
                "changes",
                TimeArgs {
                    spent: Some("00:00"),
                    note: None,
                    billable: None,
                    ..create()
                },
            ),
            (
                "naive_when",
                Vec::new(),
                "changes",
                TimeArgs {
                    executed_at: Some("2026-01-01T09:00:00"),
                    note: None,
                    billable: None,
                    ..create()
                },
            ),
            (
                "delete_billable",
                entry(json!({})),
                "changes",
                TimeArgs {
                    billable: Some(false),
                    ..delete()
                },
            ),
        ];
        for (key, entries, module, args) in cases {
            let mut w = world();
            w.entries = entries;
            let state = Arc::new(Mutex::new(w));
            let c = api(Arc::clone(&state));
            let err = build_plan(&c, module, "123", &args).unwrap_err();
            let want = &ORACLE["refusals"][key];
            assert_eq!(
                refusal_kind(&err),
                want["kind"].as_str().unwrap(),
                "{key}: {err}"
            );
            assert_eq!(err.to_string(), want["message"].as_str().unwrap(), "{key}");
            assert!(state.lock().unwrap().writes.is_empty(), "{key}");
        }
    }
}
