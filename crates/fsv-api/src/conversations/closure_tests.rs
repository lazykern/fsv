//! Replays `gen_fsv_api_closure_conversation.py` (Python 41783e2): replies
//! and notes that set Resolved/Closed over the properties-http wire.

use std::sync::{Arc, Mutex};

use fsv_core::ApiError;

use super::*;
use crate::formmeta::MemStore;
use crate::service::testing::*;

/// `(domain, cid, opened, pending, fid)`.
fn tenant(name: &str) -> (&'static str, i64, i64, i64, i64) {
    match name {
        "globex" => ("globex.example.com", 742, 2, 3, 9000008201),
        _ => ("acme.example.com", 123, 2, 3, 9000008101),
    }
}

struct Wire {
    cid: i64,
    conv_id: i64,
    row: Value,
    fields: Value,
    conversation: Option<Map<String, Value>>,
    failure: Value,
    conv_readback: Value,
    writes: Vec<Value>,
    rules: Value,
}

fn wire(name: &str, label: Option<&str>) -> Wire {
    let (_, cid, opened, pending, fid) = tenant(name);
    let mut w = Wire {
        cid,
        conv_id: cid + 9000000000,
        row: json!({"id": cid, "subject": "Acme review", "status": opened, "tags": ["before"],
                    "planned_start_date": null, "planned_end_date": null}),
        fields: json!([{"id": fid, "name": "subject", "label": "Subject", "field_type": "default_subject", "position": 1},
                       {"id": fid + 1, "name": "status", "label": "Status", "field_type": "default_status", "position": 2,
                        "choices": [{"id": opened, "value": "Open"}, {"id": pending, "value": "Pending"},
                                    {"id": 4, "value": "Resolved"}, {"id": 5, "value": "Closed"}]}]),
        conversation: None,
        failure: Value::Null,
        conv_readback: Value::Null,
        writes: Vec::new(),
        rules: json!([]),
    };
    if label.is_some() {
        for (k, v) in obj(json!({"group_id": null, "responder_id": 9000001234_i64,
            "resolution_notes": {"notes_html": "<p>Acme recovery verified</p>", "user": {"name": "Acme Operator"}},
            "custom_fields": {"summary": "Existing fix"}}))
        {
            w.row[k] = v;
        }
        w.fields.as_array_mut().unwrap().push(json!({"id": 9000008910_i64, "name": "summary",
            "label": "Resolution Summary", "field_type": "custom_text", "position": 3, "required_for_closure": true}));
    }
    w
}

fn api(w: Arc<Mutex<Wire>>, domain: &str) -> FnApi {
    let mut a = FnApi::new(move |call: &Call| {
        let mut w = w.lock().unwrap();
        let cid = w.cid;
        if call.method == "POST" || call.method == "PUT" {
            let raw = String::from_utf8(call.raw.clone().unwrap_or_default()).unwrap();
            w.writes
                .push(json!({"method": call.method, "path": call.path, "raw": raw}));
            match &w.failure {
                Value::String(_) => return Err(Error::other("synthetic disconnect")),
                Value::Number(n) => {
                    let status = n.as_u64().unwrap() as u16;
                    return Err(ApiError::new(status, json!({"error": "synthetic denial"})).into());
                }
                _ => {}
            }
            let payload = call.body.clone().unwrap();
            let mut conv = obj(json!({"id": w.conv_id, "body": payload["body"]}));
            for key in [
                "cc_emails",
                "bcc_emails",
                "private",
                "notify_emails",
                "from_email",
            ] {
                if let Some(v) = payload.get(key) {
                    conv.insert(key.into(), v.clone());
                }
            }
            if let Some(h) = payload.get("helpdesk_ticket") {
                w.row["status"] = h["status"].clone();
                if payload.get("private").is_some() {
                    conv.insert(
                        "to_emails".into(),
                        payload.get("notify_emails").cloned().unwrap_or(json!([])),
                    );
                }
            }
            w.conversation = Some(conv);
            return Ok(json!({"conversation": {"id": w.conv_id}}));
        }
        let path = call.path.as_str();
        if path == "ticket_form_fields" {
            return Ok(json!({"ticket_fields": w.fields}));
        }
        if path.ends_with("business_rules") {
            return Ok(json!({"business_rules": w.rules}));
        }
        if path == format!("tickets/{cid}/conversations") {
            let mut c = w.conversation.clone();
            if let Some(c) = c.as_mut() {
                match w.conv_readback.as_str() {
                    Some("wrong-id") => {
                        let id = c["id"].as_i64().unwrap() + 1;
                        c.insert("id".into(), json!(id));
                    }
                    Some("contradict") => {
                        c.insert("body".into(), json!("different"));
                    }
                    Some(key) => {
                        c.shift_remove(key);
                    }
                    None => {}
                }
            }
            let rows: Vec<Value> = c.into_iter().map(Value::Object).collect();
            return Ok(json!({"conversations": rows, "meta": {"count": 1}}));
        }
        if path == format!("tickets/{cid}") {
            return Ok(json!({"ticket": w.row}));
        }
        Err(ApiError::new(404, json!({"error": "unexpected"})).into())
    });
    a.domain = domain.into();
    a
}

#[test]
fn closure_conversations_match_python() {
    let oracle: Value =
        serde_json::from_str(include_str!("../testdata/closure_conversation_oracle.json")).unwrap();
    let cases = oracle.as_object().unwrap();
    assert_eq!(cases.len(), 21);
    for (name, case) in cases {
        if name == "_python" {
            continue;
        }
        let t = case.get("tenant").and_then(Value::as_str).unwrap_or("acme");
        let w = Arc::new(Mutex::new(wire(
            t,
            case.get("label").and_then(Value::as_str),
        )));
        let c = api(w.clone(), tenant(t).0);
        let store = MemStore {
            workspace: json!(1),
            ..MemStore::default()
        };
        let journal = MemJournal::default();
        let mut plan: Option<ConversationPlan> = None;
        let cid = w.lock().unwrap().cid.to_string();
        let ops = case["ops"].as_array().unwrap();
        for (i, (op, want)) in ops
            .iter()
            .zip(case["outputs"].as_array().unwrap())
            .enumerate()
        {
            let ctx = format!("{name} op {i}");
            let got: Option<Result<Value>> = match op["op"].as_str().unwrap() {
                "build" => {
                    let args = ConversationArgs {
                        kind: op["kind"].as_str().unwrap(),
                        set_status: op["status"].as_str(),
                        ..Default::default()
                    };
                    Some(
                        build_plan(&c, &store, &cid, op["body"].as_str().unwrap(), &args).map(|p| {
                            let out = json!({"payload_json": p.payload_json, "record_hash": p.record_hash,
                                "status_label": p.status_label, "hash": p.hash(), "review": p.review()});
                            plan = Some(p);
                            out
                        }),
                    )
                }
                "send" => {
                    let p = plan.as_ref().unwrap();
                    let public = op.get("public").and_then(Value::as_bool).unwrap_or(false);
                    Some(
                        send(p, &c, &journal, &store, Some(&p.hash()), public)
                            .map(|r| json!({"result": r})),
                    )
                }
                "set" => {
                    let mut w = w.lock().unwrap();
                    for (k, v) in op
                        .get("world")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flatten()
                    {
                        match k.as_str() {
                            "failure" => w.failure = v.clone(),
                            "conv_readback" => w.conv_readback = v.clone(),
                            "row" => w.row = v.clone(),
                            other => panic!("{other}"),
                        }
                    }
                    for (k, v) in op
                        .get("custom")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flatten()
                    {
                        w.row["custom_fields"][k] = v.clone();
                    }
                    if let Some(label) = op.get("summary_label") {
                        let last = w.fields.as_array().unwrap().len() - 1;
                        w.fields[last]["label"] = label.clone();
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
            let posts = w
                .lock()
                .unwrap()
                .writes
                .iter()
                .filter(|r| r["method"] == "POST")
                .count();
            assert_eq!(posts as u64, want["posts"].as_u64().unwrap(), "{ctx}");
        }
        assert_eq!(
            json!(w.lock().unwrap().writes),
            case["writes"],
            "{name} writes"
        );
        let hash = plan.as_ref().map(ConversationPlan::hash);
        let events: Vec<Value> = journal
            .read()
            .into_iter()
            .filter(|e| hash.as_deref().is_some_and(|h| e["plan"] == h))
            .map(|e| e["event"].clone())
            .collect();
        assert_eq!(json!(events), case["events"], "{name} events");
    }
}

/// `test_ticket_closure_conversation.py::test_cli_closure_blocking_rule_ack_bound_to_review`
/// and `test_cli_ack_requires_closing_set_status` (1c9dca7), at the library level.
#[test]
fn closure_blocking_rule_ack_bound_to_review() {
    for kind in ["reply", "note"] {
        let w = Arc::new(Mutex::new(wire("acme", Some("Closed"))));
        let closed = json!(5);
        w.lock().unwrap().rules = json!([{"id": 9000007120_i64, "name": "Only requester closes",
            "workspace_id": 1, "module_type": "ticket", "item_id": null, "deleted": false,
            "user_type": "both", "form_type": "both", "state": "active", "rule_type": "client_side",
            "description": "", "is_reversible": false, "is_validation": true,
            "updated_at": "2030-01-03T00:00:00Z",
            "condition_data": {"all": [{"name": "status", "operator": "is",
                "value": fsv_py::py_str(&closed), "evaluate_on": "ticket"}]},
            "action_data": {"action": [{"name": ["Only the requester closes Acme tickets"],
                "property": "message"}]}}]);
        let c = api(w.clone(), "acme.example.com");
        let store = MemStore {
            workspace: json!(1),
            ..MemStore::default()
        };
        let build = |ack: bool| {
            let args = ConversationArgs {
                kind,
                set_status: Some("Closed"),
                ack,
                ..Default::default()
            };
            build_plan(&c, &store, "123", "Acme closing", &args)
        };
        let refused = build(false).unwrap_err().to_string();
        assert!(refused.contains("pass --ack"), "{refused}");
        let p = build(true).unwrap();
        let review = p.review();
        assert_eq!(
            review["acknowledged_rule_messages"],
            json!(["Only the requester closes Acme tickets"])
        );
        assert!(
            review["warning"]
                .as_str()
                .unwrap()
                .contains("blocks submit")
        );
        let journal = MemJournal::default();
        let result = send(&p, &c, &journal, &store, Some(&p.hash()), true).unwrap();
        assert_eq!(result["verified"], json!(true), "{result}");
        let posts = w
            .lock()
            .unwrap()
            .writes
            .iter()
            .filter(|r| r["method"] == "POST")
            .count();
        assert_eq!(posts, 1);

        let args = ConversationArgs {
            kind,
            ack: true,
            ..Default::default()
        };
        let alone = build_plan(&c, &store, "123", "Acme", &args).unwrap_err();
        assert!(matches!(alone, Error::Value(_)));
        assert!(alone.to_string().contains("--set-status"));
        let args = ConversationArgs {
            kind,
            set_status: Some("Pending"),
            ack: true,
            ..Default::default()
        };
        let pending = build_plan(&c, &store, "123", "Acme", &args).unwrap_err();
        assert!(
            pending
                .to_string()
                .contains("only with --set-status Resolved or Closed")
        );
    }
}
