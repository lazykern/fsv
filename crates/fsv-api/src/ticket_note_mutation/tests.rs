//! Replays `gen_fsv_api_ticket_note_mutation.py` cases (Python 9910ddc).

use std::sync::{Arc, Mutex};

use fsv_core::ApiError;

use super::*;
use crate::service::testing::*;

struct State {
    rows: Value,
    fail: Value,
    partial: bool,
    partial_after: bool,
    mismatch: bool,
    drop_body: bool,
    requests: Vec<Value>,
}

fn new_state() -> State {
    State {
        rows: json!([{"id": 99001, "source": 2, "private": true, "body": "<div>Old</div>"}]),
        fail: Value::Null,
        partial: false,
        partial_after: false,
        mismatch: false,
        drop_body: false,
        requests: Vec::new(),
    }
}

fn write(s: &mut State, call: &Call) -> Result<Value> {
    let raw = call
        .raw
        .as_ref()
        .map(|b| String::from_utf8(b.clone()).unwrap())
        .unwrap_or_default();
    s.requests
        .push(json!({"method": call.method, "path": call.path, "raw": raw}));
    match &s.fail {
        Value::String(_) => return Err(Error::other("disconnect")),
        Value::Number(n) => {
            let status = n.as_u64().unwrap() as u16;
            return Err(ApiError::new(status, json!({"message": "failure"})).into());
        }
        _ => {}
    }
    if s.partial_after {
        s.partial = true;
    }
    let row = &mut s.rows[0];
    if call.method == "PUT" && s.drop_body {
        row.as_object_mut().unwrap().shift_remove("body");
        return Ok(json!({}));
    }
    if call.method == "PUT" {
        row["body"] = json!(if s.mismatch {
            "<div>Unexpected</div>"
        } else {
            "<div class=\"server\"><div>New &amp; text</div></div>"
        });
        return Ok(json!({"conversation": row.clone()}));
    }
    s.rows.as_array_mut().unwrap().clear();
    Ok(Value::Null)
}

fn api(state: Arc<Mutex<State>>, domain: &str) -> FnApi {
    let mut a = FnApi::new(move |call: &Call| {
        let mut s = state.lock().unwrap();
        if call.method != "GET" {
            return write(&mut s, call);
        }
        if call.path != "tickets/123/conversations" {
            return Err(ApiError::new(404, json!({"error": "unexpected"})).into());
        }
        if s.partial {
            return Err(ApiError::new(403, json!({"message": "denied"})).into());
        }
        let n = s.rows.as_array().unwrap().len();
        Ok(json!({"conversations": s.rows, "meta": {"count": n}}))
    });
    a.domain = domain.into();
    a
}

#[test]
fn cases_match_python() {
    let oracle: Value =
        serde_json::from_str(include_str!("../testdata/ticket_note_mutation_oracle.json")).unwrap();
    let cases = oracle.as_object().unwrap();
    assert_eq!(cases.len(), 29);
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
            let got: Option<Result<Value>> = match op["op"].as_str().unwrap() {
                "build" => {
                    let id = |k: &str, d: &str| op.get(k).map_or(d.to_string(), py_str);
                    Some(
                        build_plan(
                            &c,
                            &id("ticket", "INC-123"),
                            &id("note", "99001"),
                            op.get("body").and_then(Value::as_str),
                        )
                        .map(|p| {
                            let out = json!({"plan": {"action": p.action, "target": p.target, "tenant": p.tenant,
                                "method": p.method, "path": p.path, "payload_json": p.payload_json,
                                "context_hash": p.context_hash}, "hash": p.hash()});
                            plan = Some(p);
                            out
                        }),
                    )
                }
                "review" => Some(review(plan.as_ref().unwrap(), &c).map(|r| json!({"review": r}))),
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
                "set" => {
                    let mut s = state.lock().unwrap();
                    for (k, v) in op
                        .get("state")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flatten()
                    {
                        match k.as_str() {
                            "fail" => s.fail = v.clone(),
                            "partial" => s.partial = v == true,
                            "partial_after" => s.partial_after = v == true,
                            "mismatch" => s.mismatch = v == true,
                            "drop_body" => s.drop_body = v == true,
                            other => panic!("{other}"),
                        }
                    }
                    for (k, v) in op
                        .get("row")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flatten()
                    {
                        s.rows[0][k] = v.clone();
                    }
                    if let Some(rows) = op.get("rows") {
                        s.rows = rows.clone();
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
                state.lock().unwrap().requests.len() as u64,
                want["writes"].as_u64().unwrap(),
                "{ctx}"
            );
        }
        assert_eq!(
            json!(state.lock().unwrap().requests),
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
