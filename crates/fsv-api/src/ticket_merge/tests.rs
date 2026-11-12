//! Replays `gen_fsv_api_ticket_merge.py` cases (Python f402e61).

use std::sync::{Arc, Mutex};

use fsv_core::ApiError;

use super::*;
use crate::formmeta::MemStore;
use crate::service::testing::*;

fn status_field(closed: bool) -> Value {
    let mut choices = vec![json!({"id": 31, "value": "Open"})];
    if closed {
        choices.push(json!({"id": 89, "value": "Closed"}));
    }
    form_field(
        9000007101,
        "status",
        "Status",
        "default_status",
        1,
        json!({"choices": choices}),
    )
}

struct State {
    records: Value,
    histories: Value,
    has_next: bool,
    closed_choice: bool,
    fail: Value,
    bad_readback: Value,
    requests: Vec<Value>,
}

fn new_state() -> State {
    State {
        records: json!({"123": {"id": 123, "status": 31, "subject": "Acme 123"},
                        "124": {"id": 124, "status": 31, "subject": "Acme 124"},
                        "125": {"id": 125, "status": 31, "subject": "Acme 125"}}),
        histories: json!({"123": [{"id": 10, "body": "earlier"}], "124": [{"id": 11, "body": "source A"}],
                          "125": [{"id": 12, "body": "source B"}]}),
        has_next: false,
        closed_choice: true,
        fail: Value::Null,
        bad_readback: Value::Null,
        requests: Vec::new(),
    }
}

fn post(s: &mut State, call: &Call) -> Result<Value> {
    let raw = String::from_utf8(call.raw.clone().unwrap()).unwrap();
    s.requests
        .push(json!({"method": "POST", "path": call.path, "raw": raw}));
    match &s.fail {
        Value::String(_) => return Err(Error::other("connection lost")),
        Value::Number(n) => {
            let status = n.as_u64().unwrap() as u16;
            return Err(ApiError::new(status, json!({"message": "synthetic failure"})).into());
        }
        _ => {}
    }
    let body = call.body.clone().unwrap();
    for i in body["source"]["ticket_ids"].as_array().unwrap() {
        let key = i.to_string();
        s.records[&key]["status"] = json!(89);
        let moved = s.histories[&key].as_array().unwrap().clone();
        s.histories["123"].as_array_mut().unwrap().extend(moved);
        s.histories[&key] = json!([{"id": 100 + i.as_i64().unwrap(), "body": "<div>Secondary note</div>",
                                    "private": body["source"]["is_private"]}]);
    }
    s.histories["123"].as_array_mut().unwrap().push(
        json!({"id": 20, "body": "<p>Primary note</p>", "private": body["target"]["is_private"]}),
    );
    match s.bad_readback.as_str() {
        Some("status") => {
            s.records["124"]
                .as_object_mut()
                .unwrap()
                .shift_remove("status");
        }
        Some("note") => {
            let rows = s.histories["123"].as_array_mut().unwrap();
            rows.last_mut()
                .unwrap()
                .as_object_mut()
                .unwrap()
                .shift_remove("body");
        }
        Some("moved") => {
            s.histories["123"]
                .as_array_mut()
                .unwrap()
                .retain(|r| r["id"] != 11);
        }
        Some("primary_status") => s.records["123"]["status"] = json!(89),
        _ => {}
    }
    Ok(json!({"count": 2, "target": body["target"], "source": body["source"]}))
}

fn api(state: Arc<Mutex<State>>, domain: &str) -> FnApi {
    let mut a = FnApi::new(move |call: &Call| {
        let mut s = state.lock().unwrap();
        if call.method == "POST" {
            return post(&mut s, call);
        }
        let path = call.path.as_str();
        if path == "ticket_form_fields" {
            return Ok(json!({"ticket_fields": [status_field(s.closed_choice)]}));
        }
        if path.ends_with("business_rules") {
            return Ok(json!({"business_rules": []}));
        }
        let parts: Vec<&str> = path.split('/').collect();
        match parts.as_slice() {
            ["tickets", id] => Ok(json!({"ticket": s.records[*id]})),
            ["tickets", id, "conversations"] => {
                Ok(json!({"conversations": s.histories[*id], "meta": {"has_next": s.has_next}}))
            }
            _ => Err(ApiError::new(404, json!({"error": "unexpected"})).into()),
        }
    });
    a.domain = domain.into();
    a
}

fn strings(v: Option<&Value>, default: &[&str]) -> Vec<String> {
    match v.and_then(Value::as_array) {
        Some(a) => a.iter().map(py_str).collect(),
        None => default.iter().map(|s| s.to_string()).collect(),
    }
}

#[test]
fn cases_match_python() {
    let oracle: Value =
        serde_json::from_str(include_str!("../testdata/ticket_merge_oracle.json")).unwrap();
    let cases = oracle.as_object().unwrap();
    assert_eq!(cases.len(), 17);
    for (name, case) in cases {
        if name == "_python" {
            continue;
        }
        let state = Arc::new(Mutex::new(new_state()));
        let domain = case
            .get("tenant")
            .and_then(Value::as_str)
            .unwrap_or("acme.example.com");
        let c = api(state.clone(), domain);
        let store = MemStore {
            workspace: json!(1),
            ..MemStore::default()
        };
        let journal = MemJournal::default();
        let mut plan: Option<WritePlan> = None;
        let ops = case["ops"].as_array().unwrap();
        for (i, (op, want)) in ops
            .iter()
            .zip(case["outputs"].as_array().unwrap())
            .enumerate()
        {
            let ctx = format!("{name} op {i}");
            let got: Option<Result<Value>> = match op["op"].as_str().unwrap() {
                "build" => {
                    let text = |k: &str, d: &'static str| {
                        op.get(k).and_then(Value::as_str).unwrap_or(d).to_string()
                    };
                    let flag = |k: &str| op.get(k).and_then(Value::as_bool).unwrap_or(true);
                    Some(
                        build_plan(
                            &c,
                            &store,
                            &text("primary", "INC-123"),
                            &strings(op.get("secondaries"), &["INC-124", "INC-125"]),
                            &text("np", "Primary note"),
                            &text("ns", "Secondary note"),
                            flag("pp"),
                            flag("ps"),
                        )
                        .map(|p| {
                            let out = json!({"plan": {"action": p.action, "target": p.target, "tenant": p.tenant,
                                "method": p.method, "path": p.path, "payload_json": p.payload_json,
                                "context_hash": p.context_hash}, "hash": p.hash(), "review": review(&p)});
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
                    Some(send(p, &c, &store, &journal, reviewed).map(|r| json!({"result": r})))
                }
                "set" => {
                    let mut s = state.lock().unwrap();
                    for (k, v) in op
                        .get("state")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flatten()
                    {
                        match k.as_str() {
                            "has_next" => s.has_next = v == true,
                            "closed_choice" => s.closed_choice = v == true,
                            "fail" => s.fail = v.clone(),
                            "bad_readback" => s.bad_readback = v.clone(),
                            other => panic!("{other}"),
                        }
                    }
                    for (k, v) in op
                        .get("subject")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flatten()
                    {
                        s.records[k]["subject"] = v.clone();
                    }
                    for (k, v) in op
                        .get("histories")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flatten()
                    {
                        s.histories[k] = v.clone();
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
                None => {}
            }
            assert_eq!(
                state.lock().unwrap().requests.len() as u64,
                want["posts"].as_u64().unwrap(),
                "{ctx}"
            );
        }
        assert_eq!(
            json!(state.lock().unwrap().requests),
            case["writes"],
            "{name} writes"
        );
        assert_eq!(json!(journal.events()), case["events"], "{name} events");
    }
}

#[test]
fn closed_status_matches_the_python_context() {
    let oracle: Value =
        serde_json::from_str(include_str!("../testdata/ticket_merge_oracle.json")).unwrap();
    let plan = &oracle["exact_wire"]["outputs"][0]["plan"];
    let context: Value = serde_json::from_str(plan["context_hash"].as_str().unwrap()).unwrap();
    let state = Arc::new(Mutex::new(new_state()));
    let c = api(state, "acme.example.com");
    let store = MemStore {
        workspace: json!(1),
        ..MemStore::default()
    };
    for id in [124, 125] {
        let (status, hash) = closed(&c, &store, id).unwrap();
        assert_eq!(json!([status, hash]), context["closed"][id.to_string()]);
        assert_eq!(
            json!(history(&c, id).unwrap()),
            context["histories"][id.to_string()]
        );
    }
}
