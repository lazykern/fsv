//! Replays `gen_fsv_api_ticket_tasks.py` cases (Python c46d3b0, 6e598b2, 39e4853, 4626e12).

use std::sync::{Arc, Mutex};

use fsv_core::ApiError;

use super::*;
use crate::service::testing::*;

fn task_fields(n: i64) -> Value {
    json!([{"name": "title", "required_for_create": true},
           {"name": "status", "choices": [{"id": n, "value": "Open"}, {"id": n + 1, "value": "Completed"},
                                          {"id": n + 2, "value": "Canceled"}]},
           {"name": "notify_before", "choices": [{"id": 0, "value": "Never"}]},
           {"name": "tier", "label": "Tier", "field_type": "custom_dropdown", "required_for_create": true,
            "choices": [{"id": n + 3, "value": "Standard"}]}])
}

fn new_state(n: i64) -> Value {
    json!({"rows": [], "fail": null, "partial": false, "partial_after": false, "mismatch": false,
           "fields": task_fields(n), "fields_key": "ticket_task_fields", "response": null, "requests": [],
           "agents": []})
}

fn write(s: &mut Value, call: &Call) -> Result<Value> {
    let raw = String::from_utf8(call.raw.clone().unwrap()).unwrap();
    let body = call.body.clone().unwrap();
    s["requests"]
        .as_array_mut()
        .unwrap()
        .push(json!({"method": call.method, "path": call.path, "raw": raw}));
    match &s["fail"] {
        Value::String(_) => return Err(Error::other("disconnect")),
        Value::Number(n) => {
            let status = n.as_u64().unwrap() as u16;
            return Err(ApiError::new(status, json!({"error": "failure"})).into());
        }
        _ => {}
    }
    let mut result = if call.path.ends_with("/update") {
        s["rows"] = json!([]);
        json!({"tasks": []})
    } else if call.method == "POST" {
        let mut row = json!({"id": 701, "human_display_id": "TSK-701"});
        for (k, v) in body.as_object().unwrap() {
            row[k] = v.clone();
        }
        s["rows"] = json!([row.clone()]);
        json!({"task": row})
    } else {
        for (k, v) in body.as_object().unwrap() {
            s["rows"][0][k] = v.clone();
        }
        json!({"task": s["rows"][0]})
    };
    if s["mismatch"] == true && !s["rows"].as_array().unwrap().is_empty() {
        s["rows"][0]["status"] = json!(999);
    }
    if s["partial_after"] == true {
        s["partial"] = json!(true);
    }
    if !s["response"].is_null() {
        result = s["response"].clone();
    }
    Ok(result)
}

fn api(state: Arc<Mutex<Value>>) -> FnApi {
    let mut a = FnApi::new(move |call: &Call| {
        let mut s = state.lock().unwrap();
        if call.method != "GET" {
            return write(&mut s, call);
        }
        match call.path.as_str() {
            "ticket_task_fields" => {
                let key = s["fields_key"].as_str().unwrap().to_string();
                Ok(json!({key: s["fields"]}))
            }
            "autocomplete/agents" => Ok(s.get("agents").cloned().unwrap_or_else(|| json!([]))),
            "tickets/123/tasks" => {
                if s["partial"] == true {
                    return Err(ApiError::new(403, json!({"error": "denied"})).into());
                }
                let n = s["rows"].as_array().unwrap().len();
                Ok(json!({"tasks": s["rows"], "meta": {"count": n}}))
            }
            _ => Err(ApiError::new(404, json!({"error": "unexpected"})).into()),
        }
    });
    a.domain = "acme.example.com".into();
    a
}

fn args(op: &Value) -> TaskArgs {
    let mut a = TaskArgs {
        action: op["action"].as_str().unwrap_or_default().into(),
        task: op.get("task").cloned().unwrap_or(Value::Null),
        title: op.get("title").and_then(Value::as_str).map(str::to_string),
        values: op.get("values").and_then(Value::as_object).cloned(),
        ..TaskArgs::default()
    };
    if let Some(s) = op.get("status").and_then(Value::as_str) {
        a.status = s.into();
    }
    a
}

#[test]
fn cases_match_python() {
    let oracle: Value =
        serde_json::from_str(include_str!("../testdata/ticket_tasks_oracle.json")).unwrap();
    let cases = oracle.as_object().unwrap();
    assert_eq!(cases.len(), 46);
    for (name, case) in cases {
        if name == "_python" {
            continue;
        }
        let state = Arc::new(Mutex::new(new_state(case["n"].as_i64().unwrap())));
        let c = api(state.clone());
        let journal = MemJournal::default();
        let mut plan: Option<TaskPlan> = None;
        let ops = case["ops"].as_array().unwrap();
        for (i, (op, want)) in ops
            .iter()
            .zip(case["outputs"].as_array().unwrap())
            .enumerate()
        {
            let ctx = format!("{name} op {i}");
            let got: Option<Result<Value>> = match op["op"].as_str().unwrap() {
                "build" => {
                    let target = op.get("target").map_or("INC-123".to_string(), py_str);
                    let built = args(op);
                    let status = match op.get("status") {
                        None => Some(built.status.clone()),
                        Some(Value::Null) => None,
                        Some(Value::String(s)) => Some(s.clone()),
                        Some(other) => Some(py_str(other)),
                    };
                    let owner = op.get("owner").and_then(Value::as_str);
                    let group = op.get("group").and_then(Value::as_str);
                    let planned = if owner.is_none() && group.is_none() && status.is_some() {
                        build_plan(&c, &target, &built)
                    } else {
                        build_plan_assigned(&c, &target, &built, owner, group, status.as_deref())
                    };
                    Some(planned.map(|p| {
                        let w = &p.write;
                        let out = json!({"plan": {"action": w.action, "target": w.target, "tenant": w.tenant,
                            "method": w.method, "path": w.path, "payload_json": w.payload_json,
                            "context_hash": w.context_hash, "inputs_json": p.inputs_json},
                            "hash": p.hash(), "review": p.review()});
                        plan = Some(p);
                        out
                    }))
                }
                "send" => {
                    let p = plan.as_ref().unwrap();
                    let full = p.hash();
                    let reviewed = match op.get("hash").and_then(Value::as_str).unwrap_or("full") {
                        "none" => None,
                        "prefix" => Some(&full[..16]),
                        _ => Some(full.as_str()),
                    };
                    Some(send(p, &c, &journal, reviewed).map(|r| json!({"result": r})))
                }
                "custom" => Some(
                    custom_values(
                        op["fields"].as_array().unwrap(),
                        op["values"].as_object().unwrap(),
                    )
                    .map(|m| json!({"custom": m})),
                ),
                "set" => {
                    let mut s = state.lock().unwrap();
                    for (k, v) in op
                        .get("state")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flatten()
                    {
                        s[k] = v.clone();
                    }
                    if let Some(b) = op.get("bump_open").and_then(Value::as_i64) {
                        let id = s["fields"][1]["choices"][0]["id"].as_i64().unwrap();
                        s["fields"][1]["choices"][0]["id"] = json!(id + b);
                    }
                    None
                }
                other => panic!("{other}"),
            };
            match got {
                Some(Ok(v)) => {
                    for (k, x) in v.as_object().unwrap() {
                        assert_eq!(&want[k], x, "{ctx} {k}");
                    }
                }
                Some(Err(e)) => {
                    assert_eq!(
                        e.to_string(),
                        want["error"]["msg"].as_str().unwrap(),
                        "{ctx}"
                    );
                    match want["error"]["type"].as_str().unwrap() {
                        "ValueError" => assert!(matches!(e, Error::Value(_)), "{ctx}"),
                        _ => assert_eq!(e.exit_code(), 2, "{ctx}"),
                    }
                }
                None => assert!(want.get("error").is_none(), "{ctx}"),
            }
            assert_eq!(
                state.lock().unwrap()["requests"].as_array().unwrap().len() as u64,
                want["writes"].as_u64().unwrap(),
                "{ctx}"
            );
        }
        assert_eq!(
            state.lock().unwrap()["requests"],
            case["writes"],
            "{name} writes"
        );
        let events: Vec<Value> = journal
            .read()
            .iter()
            .map(|r| json!([r["kind"], r["event"]]))
            .collect();
        assert_eq!(json!(events), case["events"], "{name} events");
    }
}
