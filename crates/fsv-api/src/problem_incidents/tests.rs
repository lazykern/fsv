//! Replays `gen_fsv_api_problem_incidents.py` cases (Python 17ada1b).

use std::sync::{Arc, Mutex};

use fsv_core::ApiError;

use super::*;
use crate::service::testing::*;

fn new_state() -> Value {
    json!({"linked": false, "fail": null, "partial": false, "partial_after": false, "mismatch": false,
           "type": "Incident", "display": "INC-123", "problem": {"problem": {"id": 7, "subject": "Acme problem"}},
           "ticket_id": 123, "rows": null, "requests": []})
}

fn write(s: &mut Value, call: &Call) -> Result<Value> {
    let raw = call
        .raw
        .as_ref()
        .map(|b| String::from_utf8(b.clone()).unwrap())
        .unwrap_or_default();
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
    if s["partial_after"] == true {
        s["partial"] = json!(true);
    }
    if s["mismatch"] != true {
        s["linked"] = json!(call.method == "PUT");
    }
    Ok(if call.method == "PUT" {
        json!({"tickets": []})
    } else {
        Value::Null
    })
}

fn api(state: Arc<Mutex<Value>>, domain: &str) -> FnApi {
    let mut a = FnApi::new(move |call: &Call| {
        let mut s = state.lock().unwrap();
        if call.method != "GET" {
            return write(&mut s, call);
        }
        match call.path.as_str() {
            "problems/7" => Ok(s["problem"].clone()),
            "tickets/123" => {
                let mut t = json!({"id": s["ticket_id"], "type": s["type"]});
                if !s["display"].is_null() {
                    t["human_display_id"] = s["display"].clone();
                }
                Ok(json!({"ticket": t}))
            }
            "problems/7/tickets" => {
                if s["partial"] == true {
                    return Err(ApiError::new(403, json!({"error": "denied"})).into());
                }
                let rows = if !s["rows"].is_null() {
                    s["rows"].clone()
                } else if s["linked"] == true {
                    json!([{"id": 123, "human_display_id": "INC-123", "subject": "Acme incident"}])
                } else {
                    json!([])
                };
                let n = rows.as_array().unwrap().len();
                Ok(json!({"tickets": rows, "meta": {"count": n}}))
            }
            _ => Err(ApiError::new(404, json!({"error": "unexpected"})).into()),
        }
    });
    a.domain = domain.into();
    a
}

#[test]
fn cases_match_python() {
    let oracle: Value =
        serde_json::from_str(include_str!("../testdata/problem_incidents_oracle.json")).unwrap();
    let cases = oracle.as_object().unwrap();
    assert_eq!(cases.len(), 22);
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
        let journal = MemJournal::default();
        let mut plan: Option<WritePlan> = None;
        let ops = case["ops"].as_array().unwrap();
        for (i, (op, want)) in ops
            .iter()
            .zip(case["outputs"].as_array().unwrap())
            .enumerate()
        {
            let ctx = format!("{name} op {i}");
            let id = |k: &str, d: &str| op.get(k).map_or(d.to_string(), py_str);
            let got: Option<Result<Value>> = match op["op"].as_str().unwrap() {
                "build" => Some(
                    build_plan(
                        &c,
                        &id("problem", "PRB-7"),
                        &id("incident", "INC-123"),
                        op["unlink"] == true,
                    )
                    .map(|p| {
                        let out = json!({"plan": {"action": p.action, "target": p.target, "tenant": p.tenant,
                            "method": p.method, "path": p.path, "payload_json": p.payload_json,
                            "context_hash": p.context_hash}, "hash": p.hash(), "review": p.review()});
                        plan = Some(p);
                        out
                    }),
                ),
                "send" => {
                    let p = plan.as_ref().unwrap();
                    let full = p.hash();
                    let reviewed = match op.get("hash").and_then(Value::as_str).unwrap_or("full") {
                        "none" => None,
                        "prefix" => Some(&full[..16]),
                        _ => Some(full.as_str()),
                    };
                    Some(
                        send(p, &c, &journal, &id("incident", "INC-123"), reviewed)
                            .map(|r| json!({"result": r})),
                    )
                }
                "set" => {
                    let mut s = state.lock().unwrap();
                    for (k, v) in op["state"].as_object().unwrap() {
                        s[k] = v.clone();
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
