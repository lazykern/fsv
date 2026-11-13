//! Replays `gen_fsv_api_ticket_assignment.py` cases (Python cc41912).

use std::sync::Mutex;

use fsv_core::ApiError;

use super::*;
use crate::formmeta::{ClientResolver, MemStore};
use crate::service::testing::*;

struct State {
    boot: Value,
    tickets: Value,
    me: Value,
    fail: Value,
    mismatch: bool,
    writes: Vec<Value>,
}

fn new_state() -> State {
    State {
        boot: json!({"agents": [{"id": 71, "name": "Acme Agent", "email": "agent@example.com", "group_ids": [81], "deleted": false},
                                {"id": 72, "name": "Globex Agent", "group_ids": [], "deleted": false}],
                     "groups": [{"id": 81, "name": "Acme Desk", "agent_ids": [71]},
                                {"id": 82, "name": "Globex Desk", "agent_ids": []}]}),
        tickets: json!({"123": {"id": 123, "responder_id": 71, "group_id": 81},
                        "124": {"id": 124, "responder_id": null, "group_id": 81}}),
        me: json!({"agent": {"id": 71}}),
        fail: Value::Null,
        mismatch: false,
        writes: Vec::new(),
    }
}

fn put(s: &mut State, call: &Call) -> Result<Value> {
    let raw = String::from_utf8(call.raw.clone().unwrap()).unwrap();
    s.writes
        .push(json!({"method": "PUT", "path": call.path, "raw": raw}));
    match &s.fail {
        Value::String(_) => return Err(Error::other("connection lost")),
        Value::Number(n) => {
            let status = n.as_u64().unwrap() as u16;
            return Err(ApiError::new(status, json!({"message": "synthetic failure"})).into());
        }
        _ => {}
    }
    let body = call.body.clone().unwrap();
    if call.path == "tickets/pick_up" {
        for cid in body["ids"].as_array().unwrap() {
            if !s.mismatch {
                s.tickets[cid.to_string()]["responder_id"] = json!(71);
            }
        }
        return Ok(Value::Null);
    }
    let cid = call.path.split('/').nth(1).unwrap().to_string();
    if !s.mismatch {
        for (k, v) in body.as_object().unwrap() {
            if k != "br_validation_excludes" {
                s.tickets[&cid][k] = v.clone();
            }
        }
    }
    Ok(json!({"ticket": s.tickets[&cid]}))
}

fn api(state: Arc<Mutex<State>>) -> Arc<FnApi> {
    let fields = json!([
        form_field(9000007100, "group", "Group", "default_group", 1, json!({})),
        form_field(9000007101, "agent", "Agent", "default_agent", 2, json!({}))
    ]);
    let mut a = FnApi::new(move |call: &Call| {
        let mut s = state.lock().unwrap();
        let path = call.path.as_str();
        match call.method.as_str() {
            "ME" => return Ok(s.me.get("agent").cloned().unwrap_or(json!({}))),
            "PUT" => return put(&mut s, call),
            _ => {}
        }
        if path == "bootstrap/agents_groups" {
            return Ok(s.boot.clone());
        }
        if path == "ticket_form_fields" {
            return Ok(json!({"ticket_fields": fields}));
        }
        if path.ends_with("business_rules") {
            return Ok(json!({"business_rules": []}));
        }
        if let Some(id) = path.strip_prefix("tickets/") {
            return Ok(json!({"ticket": s.tickets[id]}));
        }
        Err(ApiError::new(404, json!({"error": "unexpected"})).into())
    });
    a.domain = "acme.example.com".into();
    Arc::new(a)
}

#[test]
fn cases_match_python() {
    let oracle: Value =
        serde_json::from_str(include_str!("../testdata/ticket_assignment_oracle.json")).unwrap();
    let cases = oracle.as_object().unwrap();
    assert_eq!(cases.len(), 18);
    for (name, case) in cases {
        if name == "_python" || name == "helpers" {
            continue;
        }
        let state = Arc::new(Mutex::new(new_state()));
        let arc = api(state.clone());
        let c: &dyn Api = &*arc;
        let resolver: Arc<dyn LookupResolver> = Arc::new(ClientResolver::new(arc.clone()));
        let store = MemStore {
            workspace: json!(1),
            ..MemStore::default()
        };
        let journal = MemJournal::default();
        let mut plan: Option<AssignmentPlan> = None;
        let ops = case["ops"].as_array().unwrap();
        for (i, (op, want)) in ops
            .iter()
            .zip(case["outputs"].as_array().unwrap())
            .enumerate()
        {
            let ctx = format!("{name} op {i}");
            let got: Option<Result<Value>> = match op["op"].as_str().unwrap() {
                "build" => {
                    let ids: Vec<String> =
                        op["ids"].as_array().unwrap().iter().map(py_str).collect();
                    let args = AssignmentArgs {
                        agent: op.get("agent").and_then(Value::as_str),
                        group: op.get("group").and_then(Value::as_str),
                        unassign: op.get("unassign") == Some(&json!(true)),
                        pickup: op.get("pickup") == Some(&json!(true)),
                    };
                    Some(build_plan(c, &store, &ids, &args, Some(resolver.clone())).map(|p| {
                        let w = &p.write;
                        let out = json!({"write": {"action": w.action, "target": w.target, "tenant": w.tenant,
                            "method": w.method, "path": w.path, "payload_json": w.payload_json,
                            "context_hash": w.context_hash}, "intent_json": p.intent_json,
                            "before_json": p.before_json, "hash": p.hash(), "review": p.review()});
                        plan = Some(p);
                        out
                    }))
                }
                "send" => {
                    let p = plan.as_ref().unwrap();
                    let full = p.hash();
                    let reviewed = match op.get("hash").and_then(Value::as_str) {
                        Some("prefix") => &full[..16],
                        _ => full.as_str(),
                    };
                    Some(
                        send(
                            p,
                            c,
                            &store,
                            &journal,
                            Some(reviewed),
                            Some(resolver.clone()),
                        )
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
                        match k.as_str() {
                            "fail" => s.fail = v.clone(),
                            "mismatch" => s.mismatch = v == true,
                            "me" => s.me = v.clone(),
                            other => panic!("{other}"),
                        }
                    }
                    for (k, v) in op
                        .get("boot")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flatten()
                    {
                        s.boot[k] = v.clone();
                    }
                    for (k, v) in op
                        .get("group0")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flatten()
                    {
                        let g = s.boot["groups"][0].as_object_mut().unwrap();
                        if v.is_null() {
                            g.shift_remove(k);
                        } else {
                            g.insert(k.clone(), v.clone());
                        }
                    }
                    for (k, v) in op
                        .get("ticket123")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flatten()
                    {
                        let t = s.tickets["123"].as_object_mut().unwrap();
                        if v == "<pop>" {
                            t.shift_remove(k);
                        } else {
                            t.insert(k.clone(), v.clone());
                        }
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
                state.lock().unwrap().writes.len() as u64,
                want["writes"].as_u64().unwrap(),
                "{ctx}"
            );
        }
        assert_eq!(
            json!(state.lock().unwrap().writes),
            case["writes"],
            "{name} writes"
        );
        let events: Vec<Value> = journal
            .read()
            .into_iter()
            .map(|r| json!([r["kind"], r["event"]]))
            .collect();
        assert_eq!(json!(events), case["events"], "{name} events");
    }
}

#[test]
fn helpers_match_python() {
    let oracle: Value =
        serde_json::from_str(include_str!("../testdata/ticket_assignment_oracle.json")).unwrap();
    let h = &oracle["helpers"];
    for case in h["positive"].as_array().unwrap() {
        assert_eq!(json!(positive(&case["value"])), case["expected"], "{case}");
    }
    assert_eq!(positive(&json!("१२")), Some(12));
    assert_eq!(positive(&json!("²")), None);
    for case in h["resolve"].as_array().unwrap() {
        match resolve(&case["rows"], case["value"].as_str().unwrap(), "agent") {
            Ok(row) => assert_eq!(Value::Object(row), case["ok"], "{case}"),
            Err(e) => assert_eq!(e.to_string(), case["error"].as_str().unwrap(), "{case}"),
        }
    }
    for case in h["member"].as_array().unwrap() {
        let (a, g) = (
            case["agent"].as_object().unwrap(),
            case["group"].as_object().unwrap(),
        );
        match member(a, g) {
            Ok(()) => assert!(case["ok"].is_null(), "{case}"),
            Err(e) => assert_eq!(e.to_string(), case["error"].as_str().unwrap(), "{case}"),
        }
    }
}
