//! Replays `gen_fsv_api_ticket_lifecycle.py` cases (Python 9ab0eaa).

use std::sync::{Arc, LazyLock, Mutex};

use fsv_core::ApiError;

use super::*;
use crate::formmeta::MemStore;
use crate::service::testing::*;

static ORACLE: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../testdata/ticket_lifecycle_oracle.json")).unwrap()
});

const FORM: &str = "/api/_/lookup?field_name=reason_900000777";

fn fields() -> Vec<Value> {
    vec![
        form_field(
            9000007101,
            "status",
            "Status",
            "default_status",
            1,
            json!({"choices": [{"id": 31, "value": "Open"}, {"id": 47, "value": "Pending"},
                                      {"id": 62, "value": "Resolved"}, {"id": 89, "value": "Closed"}]}),
        ),
        form_field(
            9000007102,
            "summary",
            "Resolution Summary",
            "custom_text",
            2,
            json!({"required_for_closure": true}),
        ),
        form_field(
            9000007103,
            "reason",
            "Pending Reason",
            "custom_text",
            3,
            json!({}),
        ),
    ]
}

fn rule(id: i64, name: &str, cond: Value, actions: Value) -> Value {
    json!({"id": id, "name": name, "workspace_id": 2, "module_type": "ticket", "item_id": null,
           "deleted": false, "user_type": "both", "form_type": "both", "state": "active",
           "rule_type": "client_side", "description": "", "condition_data": cond,
           "action_data": {"action": actions}, "is_reversible": false, "is_validation": false,
           "updated_at": "2030-01-03T00:00:00Z"})
}

/// `(fields, rules)`; `None` rules: the business-rules route 404s.
fn variant(name: &str) -> (Vec<Value>, Option<Value>) {
    let pending = rule(
        9000007107,
        "Pending needs reason",
        json!({"all": [{"name": "status", "operator": "is", "value": "47", "evaluate_on": "ticket"}]}),
        json!([{"name": ["customFields.reason"], "property": "mandate"}]),
    );
    let hide = rule(
        9000007108,
        "Hide summary",
        json!({"all": null}),
        json!([{"name": ["customFields.summary"], "property": "hide"}]),
    );
    let mut fs = fields();
    let mut rules = json!({"business_rules": []});
    match name {
        "label_changed" => fs[1]["label"] = json!("Changed requirement"),
        "pending_rule" => rules = json!({"business_rules": [pending]}),
        "pending_hide" => {
            rules = json!({"business_rules": [pending, hide]});
            fs[2]["lookup_config"] = json!({"link": FORM});
        }
        "unknown_required" => {
            fs.push(json!({"name": "unknown_requirement", "required_for_closure": true}))
        }
        "rules_unavailable" => return (fs, None),
        _ => {}
    }
    (fs, Some(rules))
}

struct World {
    row: Map<String, Value>,
    meta: (Vec<Value>, Option<Value>),
    flags: Value,
    puts: Vec<Value>,
}

fn api(world: Arc<Mutex<World>>, tenant: &str) -> FnApi {
    let mut a = FnApi::new(move |call: &Call| {
        let mut w = world.lock().unwrap();
        let missing = || Err(ApiError::new(404, json!("missing")).into());
        match (call.method.as_str(), call.path.as_str()) {
            ("GET", "tickets/123") => Ok(json!({"ticket": w.row})),
            ("GET", "ticket_form_fields") => Ok(json!({"ticket_fields": w.meta.0})),
            ("GET", p) if p.ends_with("business_rules") => {
                w.meta.1.clone().map_or_else(missing, Ok)
            }
            ("PUT", "tickets/123") => {
                let raw = String::from_utf8(call.raw.clone().unwrap()).unwrap();
                let body = call.body.clone().unwrap();
                w.puts.push(json!({"raw": raw, "body": body}));
                let flags = w.flags.clone();
                match &flags["put_error"] {
                    Value::String(_) => return Err(Error::other("offline disconnect")),
                    Value::Number(n) => {
                        return Err(
                            ApiError::new(n.as_u64().unwrap() as u16, json!("synthetic")).into(),
                        );
                    }
                    _ => {}
                }
                if flags["fail_at"].as_u64() == Some(w.puts.len() as u64) {
                    return Err(ApiError::new(403, json!("synthetic rejection")).into());
                }
                for (key, value) in body.as_object().unwrap() {
                    match key.as_str() {
                        "resolution_notes" => {
                            let html = if flags["bad_note"] == true {
                                json!("wrong")
                            } else {
                                value.clone()
                            };
                            w.row.insert(key.clone(), json!({"notes_html": html,
                                "user": {"name": "Acme Operator", "email": "operator@example.com"}}));
                            if flags["change_after_note"] == true {
                                w.row.insert("group_id".into(), json!(9000005678_i64));
                            }
                        }
                        "custom_fields" => {
                            let cf = w.row.get_mut(key).unwrap().as_object_mut().unwrap();
                            for (k, v) in value.as_object().unwrap() {
                                cf.insert(k.clone(), v.clone());
                            }
                        }
                        "br_validation_excludes" => {}
                        "status" if flags["bad_status"] == true => {}
                        _ => {
                            w.row.insert(key.clone(), value.clone());
                        }
                    }
                }
                Ok(json!({"ticket": w.row}))
            }
            _ => missing(),
        }
    });
    a.domain = tenant.into();
    a
}

fn error_matches(e: &Error, want: &Value, ctx: &str) {
    assert_eq!(e.to_string(), want["msg"].as_str().unwrap(), "{ctx}");
    match want["type"].as_str().unwrap() {
        "ReviewedWriteError" => assert_eq!(e.exit_code(), 2, "{ctx}"),
        "ValueError" => assert!(matches!(e, Error::Value(_)), "{ctx}"),
        _ => {}
    }
}

fn opt_str(v: &Value) -> Option<&str> {
    v.as_str()
}

#[test]
fn cases_match_python() {
    let cases = ORACLE["cases"].as_object().unwrap();
    assert_eq!(cases.len(), 28);
    for (name, case) in cases {
        let mut row = obj(
            json!({"id": 123, "status": 31, "group_id": null, "responder_id": 9000001234_i64,
            "resolution_notes": null, "custom_fields": {"summary": "Acme fix", "reason": "", "untouched": "keep"}}),
        );
        for (k, v) in case
            .get("row")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            row.insert(k.clone(), v.clone());
        }
        for (k, v) in case
            .get("custom")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            row["custom_fields"][k] = v.clone();
        }
        let world = Arc::new(Mutex::new(World {
            row,
            meta: variant(case.get("meta").and_then(Value::as_str).unwrap_or("base")),
            flags: case.get("flags").cloned().unwrap_or(json!({})),
            puts: Vec::new(),
        }));
        let tenant = case
            .get("tenant")
            .and_then(Value::as_str)
            .unwrap_or("acme.freshservice.com");
        let c = api(world.clone(), tenant);
        let store = MemStore::default();
        let mem = MemJournal::default();
        let deaf = DeafJournal;
        let mut journal: &dyn Journal = &mem;
        let mut plan: Option<LifecyclePlan> = None;
        let ops = case["ops"].as_array().unwrap();
        for (i, (op, want)) in ops
            .iter()
            .zip(case["outputs"].as_array().unwrap())
            .enumerate()
        {
            let ctx = format!("{name} op {i}");
            let got: Option<Result<Value>> = match op["op"].as_str().unwrap() {
                "build" => {
                    let target = match &op["target"] {
                        Value::Null => "123".to_string(),
                        v => fsv_py::py_str(v),
                    };
                    Some(
                        build_plan(&c, &store, &target, op["status"].as_str().unwrap(),
                                   opt_str(&op["note"]), op["answers"].as_object(), None)
                        .map(|p| {
                            assert_eq!(Value::Object(p.payload()), serde_json::from_str::<Value>(&p.payload_json).unwrap());
                            let empty = Map::new();
                            let (payload, fingerprint, label) = context(&c, &store, p.target, op["status"].as_str().unwrap(),
                                op["answers"].as_object().unwrap_or(&empty), p.note_html.as_deref(), None).unwrap();
                            assert_eq!((Value::Object(payload), &fingerprint, &label),
                                       (Value::Object(p.payload()), &p.context_hash, &p.status), "{ctx}");
                            let out = json!({"plan": {"target": p.target, "tenant": p.tenant, "status": p.status,
                                "answers_json": p.answers_json, "payload_json": p.payload_json,
                                "context_hash": p.context_hash, "note_html": p.note_html},
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
                        "prefix" => Some(&full[..12]),
                        _ => Some(full.as_str()),
                    };
                    Some(send(p, &c, &store, journal, reviewed, None).map(|r| json!({"result": r})))
                }
                "row" => {
                    let mut w = world.lock().unwrap();
                    for (k, v) in op
                        .get("set")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flatten()
                    {
                        w.row.insert(k.clone(), v.clone());
                    }
                    for k in op
                        .get("pop")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        w.row.remove(k.as_str().unwrap());
                    }
                    None
                }
                "meta" => {
                    world.lock().unwrap().meta = variant(op["variant"].as_str().unwrap());
                    None
                }
                "deaf" => {
                    journal = &deaf;
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
                Some(Err(e)) => error_matches(&e, &want["error"], &ctx),
                None => {}
            }
            assert_eq!(
                world.lock().unwrap().puts.len() as u64,
                want["writes"].as_u64().unwrap(),
                "{ctx}"
            );
        }
        let w = world.lock().unwrap();
        assert_eq!(json!(w.puts), case["puts"], "{name} puts");
        assert_eq!(
            Value::Object(w.row.clone()),
            case["final_row"],
            "{name} row"
        );
        let events: Vec<Value> = mem.events().into_iter().map(Value::from).collect();
        assert_eq!(json!(events), case["events"], "{name} events");
    }
}

#[test]
fn matches_and_resolution_text_match_python() {
    for case in ORACLE["matches"].as_array().unwrap() {
        assert_eq!(
            matches(
                case["row"].as_object().unwrap(),
                case["payload"].as_object().unwrap()
            ),
            case["expected"].as_bool().unwrap(),
            "{case}"
        );
    }
    for case in ORACLE["resolution_text"].as_array().unwrap() {
        assert_eq!(
            json!(resolution_text(&case["record"])),
            case["expected"],
            "{case}"
        );
    }
}
