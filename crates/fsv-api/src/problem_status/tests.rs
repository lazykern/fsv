//! Replays `gen_fsv_api_problem_status.py` cases (Python 5d7d777).

use std::sync::{Arc, Mutex};

use super::*;
use crate::formmeta::{ClientResolver, MemStore};
use crate::service::testing::*;

const LABELS: [&str; 4] = ["Open", "In Progress", "Resolve", "Closed"];

fn problem_fields(n: i64) -> Value {
    let choices: Vec<Value> = LABELS
        .iter()
        .enumerate()
        .map(|(i, l)| json!({"id": n + i as i64, "value": l}))
        .collect();
    json!([{"id": 501, "name": "subject", "label": "Subject", "field_type": "default_subject", "position": 1,
            "required_for_closure": true},
           {"id": 502, "name": "status", "label": "Status", "field_type": "default_status", "position": 2,
            "choices": choices}])
}

fn new_state(n: i64) -> Value {
    let analysis: Vec<Value> = PROBLEM_SECTIONS
        .iter()
        .enumerate()
        .map(|(i, (_, key, _))| json!({"id": i + 1, "name": key, "description": "<div>Acme evidence</div>"}))
        .collect();
    json!({"row": {"id": 123, "subject": "Acme problem", "status": n, "planned_start_date": null, "planned_end_date": null},
           "analysis": analysis, "solutions": [{"id": 701, "title": "Acme solution", "article_type": 1}], "fields": problem_fields(n),
           "rules": [], "fail": null, "mismatch": false, "partial": false, "drop_dates": false, "requests": []})
}

fn write(s: &mut Value, call: &Call) -> Result<Value> {
    let raw = String::from_utf8(call.raw.clone().unwrap()).unwrap();
    s["requests"]
        .as_array_mut()
        .unwrap()
        .push(json!({"method": call.method, "path": call.path, "raw": raw}));
    let body = call.body.clone().unwrap();
    match &s["fail"] {
        Value::String(_) => return Err(Error::other("disconnect")),
        Value::Object(f) => {
            let status = f["status"].as_u64().unwrap() as u16;
            return Err(ApiError::new(status, f["body"].clone()).into());
        }
        Value::Number(n) => {
            let status = n.as_u64().unwrap() as u16;
            return Err(ApiError::new(status, json!({"error": "failed"})).into());
        }
        _ => {}
    }
    if s["mismatch"] != true {
        for (k, v) in body.as_object().unwrap() {
            s["row"][k] = v.clone();
        }
    }
    s["row"]["planned_start_date"] = json!("2030-01-02T10:00:00Z");
    s["row"]["planned_end_date"] = json!("2030-01-02T11:00:00Z");
    if s["drop_dates"] == true {
        s["row"]
            .as_object_mut()
            .unwrap()
            .shift_remove("planned_end_date");
    }
    Ok(json!({"problem": {"id": 123}}))
}

fn api(state: Arc<Mutex<Value>>) -> FnApi {
    let mut a = FnApi::new(move |call: &Call| {
        let mut s = state.lock().unwrap();
        if call.method != "GET" {
            return write(&mut s, call);
        }
        let path = call.path.as_str();
        if path == "problem_form_fields" {
            return Ok(json!({"problem_fields": s["fields"]}));
        }
        if path.ends_with("business_rules") {
            return Ok(json!({"business_rules": s["rules"]}));
        }
        if path.ends_with("planning-fields") {
            return Ok(json!({"problem_planning_fields": s["analysis"]}));
        }
        if path.ends_with("solutions") {
            if let Some(status) = s
                .get("solution_error")
                .and_then(Value::as_u64)
                .filter(|n| *n > 0)
            {
                return Err(ApiError::new(status as u16, json!({"error": "unavailable"})).into());
            }
            if let Some(raw) = s.get("solution_raw").filter(|v| !v.is_null()) {
                return Ok(raw.clone());
            }
            if let Some(pages) = s.get("solution_pages").and_then(Value::as_array) {
                let page = call
                    .params
                    .as_ref()
                    .and_then(|p| p.get("page"))
                    .map(fsv_py::py_str)
                    .and_then(|text| text.parse::<usize>().ok())
                    .filter(|n| *n >= 1)
                    .unwrap_or(1);
                return Ok(pages.get(page - 1).cloned().unwrap_or(json!({})));
            }
            return Ok(json!({"articles": s["solutions"], "has_more": s["partial"]}));
        }
        if path == "problems/123" {
            return Ok(json!({"problem": s["row"]}));
        }
        Err(ApiError::new(404, json!({"error": "unexpected"})).into())
    });
    a.domain = "acme.example.com".into();
    a
}

#[test]
fn cases_match_python() {
    let oracle: Value =
        serde_json::from_str(include_str!("../testdata/problem_status_oracle.json")).unwrap();
    let cases = oracle.as_object().unwrap();
    assert_eq!(cases.len(), 38);
    for (name, case) in cases {
        if name == "_python" {
            continue;
        }
        let state = Arc::new(Mutex::new(new_state(case["n"].as_i64().unwrap())));
        let c = Arc::new(api(state.clone()));
        let resolver = || -> Option<Arc<dyn LookupResolver>> {
            Some(Arc::new(ClientResolver::new(c.clone())))
        };
        let store = MemStore {
            workspace: json!(17),
            ..MemStore::default()
        };
        let journal = MemJournal::default();
        let mut plan: Option<StatusPlan> = None;
        let ops = case["ops"].as_array().unwrap();
        for (i, (op, want)) in ops
            .iter()
            .zip(case["outputs"].as_array().unwrap())
            .enumerate()
        {
            let ctx = format!("{name} op {i}");
            let got: Option<Result<Value>> = match op["op"].as_str().unwrap() {
                "build" => {
                    let target = op.get("target").map_or("PRB-123".to_string(), py_str);
                    Some(
                        build_plan(&*c, &store, &target, op["to"].as_str().unwrap(), resolver())
                            .map(|p| {
                                let w = &p.write;
                                let out = json!({"plan": {"action": w.action, "target": w.target,
                                    "tenant": w.tenant, "method": w.method, "path": w.path,
                                    "payload_json": w.payload_json, "context_hash": w.context_hash,
                                    "label": p.label, "before_json": p.before_json},
                                    "hash": p.hash(), "review": p.review()});
                                plan = Some(p);
                                out
                            }),
                    )
                }
                "send" => {
                    let p = plan.as_ref().unwrap();
                    let full = p.hash();
                    let reviewed = match op.get("hash").and_then(Value::as_str).unwrap_or("full") {
                        "none" => None,
                        "prefix" => Some(&full[..16]),
                        _ => Some(full.as_str()),
                    };
                    Some(
                        send(p, &*c, &store, &journal, reviewed, resolver())
                            .map(|r| json!({"result": r})),
                    )
                }
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
                    for (k, v) in op
                        .get("row")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flatten()
                    {
                        s["row"][k] = v.clone();
                    }
                    for (k, v) in op
                        .get("analysis0")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flatten()
                    {
                        s["analysis"][0][k] = v.clone();
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
                        "ReviewedWriteError" => assert_eq!(e.exit_code(), 2, "{ctx}"),
                        _ => {}
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
        assert_eq!(json!(journal.events()), case["events"], "{name} events");
    }
}
