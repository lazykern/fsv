//! Replays `gen_fsv_api_attachments.py` cases (Python 1c186e0, ba6aef9, 8f156c0).

use std::sync::{Arc, Mutex};

use base64::Engine;

use super::*;
use crate::formmeta::MemStore;
use crate::service::testing::*;
use fsv_core::client::Response;
use fsv_core::error::ApiError;

const FILES: [(&str, &str); 2] = [
    ("acme.txt", "Acme evidence 0\n"),
    ("globex.txt", "Acme evidence 1\n"),
];

fn new_state(base: i64) -> Value {
    json!({"uploads": {}, "notes": [], "conversations": [], "ticket": {"id": 123, "attachments": []},
           "change": null,
           "problem": {"id": 123, "attachments": []}, "limit": 4096, "meta": null, "fail": null,
           "fail_step": "upload", "mismatch": false, "missing_id": false, "partial": false, "base": base,
           "planning": [], "requests": [], "page": null, "page_status": 200, "page_html": null})
}

fn read(s: &mut Value, path: &str) -> Result<Value> {
    Ok(match path {
        "ticket_form_fields" | "problem_form_fields" => {
            if !s["meta"].is_null() {
                s["meta"].clone()
            } else if s["limit"].is_null() {
                json!({"ticket_fields": [{"name": "subject"}]})
            } else {
                json!({"attachment_settings": {"max_attachment_size_bytes": s["limit"]}})
            }
        }
        "tickets/123/conversations" => {
            let mut rows = s["conversations"].clone();
            if s["rotate"] == true {
                let reads = s["reads"].as_u64().unwrap_or(0) as u32 + 1;
                s["reads"] = json!(reads);
                rotate_files(&mut rows, reads);
            }
            if s["mismatch"] == true
                && let Some(first) = rows.get_mut(0)
            {
                first["attachments"] = json!([]);
            }
            json!({"conversations": rows, "has_more": s["partial"]})
        }
        "problems/123/notes" => json!({"notes": s["notes"], "has_more": s["partial"]}),
        "tickets/123" => json!({"ticket": s["ticket"]}),
        "changes/123" => {
            if s["change"].is_null() {
                return Err(ApiError::new(404, json!({"error": "unexpected"})).into());
            }
            json!({"change": s["change"]})
        }
        "problems/123" => json!({"problem": s["problem"]}),
        "problems/123/planning-fields" => json!({"problem_planning_fields": s["planning"]}),
        _ => return Err(ApiError::new(404, json!({"error": "unexpected"})).into()),
    })
}

fn write(s: &mut Value, call: &Call) -> Result<Value> {
    let raw = String::from_utf8(call.raw.clone().unwrap_or_default()).unwrap();
    let path = call.path.as_str();
    s["requests"]
        .as_array_mut()
        .unwrap()
        .push(json!({"method": call.method, "path": path, "raw": raw}));
    let step = if path == "attachments" {
        "upload"
    } else {
        "owner"
    };
    if !s["fail"].is_null() && s["fail_step"] == step {
        return match s["fail"].as_u64() {
            Some(status) => Err(ApiError::new(status as u16, json!({"error": "failed"})).into()),
            None => Err(Error::other("disconnect")),
        };
    }
    if call.method == "DELETE" {
        let ident: i64 = path.rsplit_once('/').unwrap().1.parse().unwrap();
        let strip = |row: &mut Value| {
            if let Some(a) = row.get_mut("attachments").and_then(Value::as_array_mut) {
                a.retain(|x| x["id"] != json!(ident));
            }
        };
        strip(&mut s["ticket"]);
        strip(&mut s["problem"]);
        for key in ["notes", "conversations", "planning"] {
            for row in s[key].as_array_mut().unwrap() {
                strip(row);
            }
        }
        return Ok(Value::Null);
    }
    let body = call.body.clone().unwrap();
    if path == "attachments" {
        let ident = s["base"].as_i64().unwrap() + s["uploads"].as_object().unwrap().len() as i64;
        let content = body["content"].as_str().unwrap();
        let data = base64::engine::general_purpose::STANDARD
            .decode(content.split_once(',').unwrap().1)
            .unwrap();
        let row = json!({"id": ident, "name": body["content_file_name"], "size": data.len().to_string(),
                         "content_type": format!("{};charset=utf-8", body["content_content_type"].as_str().unwrap())});
        s["uploads"][ident.to_string()] = row.clone();
        return Ok(if s["missing_id"] == true {
            json!({})
        } else {
            row
        });
    }
    let attachments: Vec<Value> = body["attachments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| s["uploads"][i.to_string()].clone())
        .collect();
    if path == "tickets" || path == "changes" {
        let mut row = obj(json!({"id": 123}));
        row.extend(body.as_object().unwrap().clone());
        row.insert("attachments".into(), json!(attachments));
        let key = if path == "tickets" {
            "ticket"
        } else {
            "change"
        };
        s[key] = Value::Object(row);
        return Ok(json!({key: {"id": 123}}));
    }
    let mut row = obj(json!({"id": 901}));
    row.extend(body.as_object().unwrap().clone());
    row.insert("attachments".into(), json!(attachments));
    row.insert("body".into(), json!("<div>Acme evidence</div>"));
    if path == "problems/123/notes" {
        s["notes"].as_array_mut().unwrap().push(Value::Object(row));
        return Ok(json!({"note": {"id": 901}}));
    }
    if let Some(v) = row.shift_remove("notify_emails") {
        row.insert("to_emails".into(), v);
    }
    s["conversations"]
        .as_array_mut()
        .unwrap()
        .push(Value::Object(row));
    Ok(json!({"conversation": {"id": 901}}))
}

fn page(s: &Value, path: &str) -> Result<Response> {
    assert_eq!(path, PAGE_PATH);
    let html = match s["page_html"].as_str() {
        Some(h) => h.to_string(),
        None => {
            let script: String = s["page"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|v| format!("window.MAX_ATTACHMENT_LIMIT = {};\n", v.as_str().unwrap()))
                .collect();
            format!(
                r#"<html><head><meta name="csrf-token" content="synthetic"></head><script>{script}</script></html>"#
            )
        }
    };
    let status = s["page_status"].as_u64().unwrap() as u16;
    Ok(Response::new(status).with_body(html.into_bytes()))
}

fn api(state: Arc<Mutex<Value>>, domain: &str) -> FnApi {
    let pages = state.clone();
    let mut a = FnApi::new(move |call: &Call| {
        let mut s = state.lock().unwrap();
        if call.method == "GET" {
            read(&mut s, &call.path)
        } else {
            write(&mut s, call)
        }
    })
    .with_fetch(move |path| page(&pages.lock().unwrap(), path));
    a.domain = domain.into();
    a
}

fn build(
    c: &dyn Api,
    store: &dyn MetaStore,
    paths: &[String],
    kind: &str,
    notify: &[String],
    max_bytes: Option<i64>,
) -> Result<AttachedPlan> {
    match kind {
        "problem" => problem_note_plan(c, "PRB-123", "Acme evidence", paths, max_bytes),
        "create" | "change" => {
            let raw = metadata(c, "tickets")?;
            let base = WritePlan {
                action: "ticket_create_attachments".into(),
                target: 0,
                tenant: c.domain(),
                method: "POST".into(),
                path: if kind == "change" {
                    "changes".into()
                } else {
                    "tickets".into()
                },
                payload_json: fsv_py::Dumps::default().dump(&json!({"subject": "Acme incident"})),
                context_hash: plan_hash(&raw),
            };
            wrap(base, paths, &raw, c, max_bytes)
        }
        _ => conversation_plan(
            c,
            store,
            "INC-123",
            "Acme evidence",
            paths,
            &ConversationAttach {
                kind,
                sender: (kind == "reply").then_some("support@example.com"),
                notify,
                max_bytes,
                ..ConversationAttach::default()
            },
        ),
    }
}

fn plan_json(p: &WritePlan) -> Value {
    json!({"action": p.action, "target": p.target, "tenant": p.tenant, "method": p.method,
           "path": p.path, "payload_json": p.payload_json, "context_hash": p.context_hash})
}

fn strings(v: Option<&Value>) -> Option<Vec<String>> {
    v.and_then(Value::as_array)
        .map(|a| a.iter().map(|x| x.as_str().unwrap().to_string()).collect())
}

#[test]
fn cases_match_python() {
    let oracle: Value =
        serde_json::from_str(include_str!("../testdata/attachments_oracle.json")).unwrap();
    let cases = oracle.as_object().unwrap();
    assert_eq!(cases.len(), 67);
    for (name, case) in cases {
        if name == "_python" {
            continue;
        }
        let tenant = case
            .get("tenant")
            .and_then(Value::as_str)
            .unwrap_or("acme.example.com");
        let state = Arc::new(Mutex::new(new_state(
            case.get("base").and_then(Value::as_i64).unwrap_or(701),
        )));
        let c = api(state.clone(), tenant);
        let store = MemStore {
            workspace: json!(1),
            ..MemStore::default()
        };
        let journal = MemJournal::default();
        let dir = tempfile::tempdir().unwrap();
        for (file, text) in FILES {
            std::fs::write(dir.path().join(file), text).unwrap();
        }
        let path_of = |n: &str| dir.path().join(n).to_string_lossy().into_owned();
        let mut plan: Option<AttachedPlan> = None;
        let mut built: (Vec<String>, String, Vec<String>, Option<i64>) = Default::default();
        let mut deletion: Option<WritePlan> = None;
        let ops = case["ops"].as_array().unwrap();
        for (i, (op, want)) in ops
            .iter()
            .zip(case["outputs"].as_array().unwrap())
            .enumerate()
        {
            let ctx = format!("{name} op {i}");
            let child = op.get("child");
            let section = op.get("section").and_then(Value::as_str);
            let owner = op.get("owner").and_then(Value::as_str).unwrap_or("");
            let got: Option<Result<Value>> = match op["op"].as_str().unwrap() {
                "build" => {
                    let names = strings(op.get("files"))
                        .unwrap_or_else(|| FILES.iter().map(|(n, _)| n.to_string()).collect());
                    let paths: Vec<String> = names.iter().map(|n| path_of(n)).collect();
                    let kind = op["kind"].as_str().unwrap().to_string();
                    let notify = strings(op.get("notify")).unwrap_or_default();
                    built = (
                        paths,
                        kind,
                        notify,
                        op.get("max_bytes").and_then(Value::as_i64),
                    );
                    Some(build(&c, &store, &built.0, &built.1, &built.2, built.3).map(|p| {
                        let out = json!({"plan": {"base": plan_json(&p.base),
                            "files": p.files.iter().map(LocalFile::review).collect::<Vec<_>>(),
                            "limit": p.limit, "limit_source": p.limit_source,
                            "published_limit": p.published_limit, "hash": p.hash(), "review": p.review()}});
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
                    let rebuild = || build(&c, &store, &built.0, &built.1, &built.2, built.3);
                    Some(send(p, &c, &journal, reviewed, &rebuild).map(|r| json!({"result": r})))
                }
                "files" => {
                    let paths: Vec<String> = strings(op.get("files"))
                        .unwrap()
                        .iter()
                        .map(|n| path_of(n))
                        .collect();
                    Some(local_files(&paths, op["limit"].as_i64()).map(
                        |f| json!({"files": f.iter().map(LocalFile::review).collect::<Vec<_>>()}),
                    ))
                }
                "limit" => Some(size_limit(&op["metadata"]).map(|l| json!({"limit": l}))),
                "resolve" => Some(
                    resolve_limit(
                        &c,
                        &op["metadata"],
                        op.get("max_bytes").and_then(Value::as_i64),
                    )
                    .map(|l| {
                        json!({"resolved": {"max_bytes": l.max_bytes, "source": l.source,
                                                "published": l.published}})
                    }),
                ),
                "write" => {
                    std::fs::write(
                        path_of(op["name"].as_str().unwrap()),
                        op["text"].as_str().unwrap(),
                    )
                    .unwrap();
                    None
                }
                "delete_build" => Some(delete_plan(&c, &op["ident"], owner, child, section).map(
                    |p| {
                        let out =
                            json!({"plan": plan_json(&p), "hash": p.hash(), "review": p.review()});
                        deletion = Some(p);
                        out
                    },
                )),
                "delete_send" => {
                    let p = deletion.as_ref().unwrap();
                    let full = p.hash();
                    let reviewed = match op.get("hash").and_then(Value::as_str) {
                        Some("none") => None,
                        _ => Some(full.as_str()),
                    };
                    Some(
                        delete(p, &c, &journal, owner, reviewed, child, section)
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
                        .get("ticket")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flatten()
                    {
                        s["ticket"][k] = v.clone();
                    }
                    None
                }
                other => panic!("{other}"),
            };
            match got {
                Some(Ok(v)) => {
                    assert!(want.get("error").is_none(), "{ctx}: {:?}", want["error"]);
                    for (k, x) in v.as_object().unwrap() {
                        assert_eq!(&want[k], x, "{ctx} {k}");
                    }
                }
                Some(Err(e)) => match want["error"]["type"].as_str() {
                    Some("APIError") => assert!(matches!(e, Error::Api(_)), "{ctx}: {e}"),
                    kind => {
                        assert_eq!(
                            e.to_string(),
                            want["error"]["msg"].as_str().unwrap_or_default(),
                            "{ctx}"
                        );
                        match kind {
                            Some("ValueError") => assert!(matches!(e, Error::Value(_)), "{ctx}"),
                            Some("ReviewedWriteError") => assert_eq!(e.exit_code(), 2, "{ctx}"),
                            Some("SessionError") => {
                                assert!(matches!(e, Error::Session(_)), "{ctx}")
                            }
                            _ => panic!("{ctx}: {e}"),
                        }
                    }
                },
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
            .map(|r| json!([r["kind"], r["event"], r["created_id"]]))
            .collect();
        assert_eq!(json!(events), case["events"], "{name} events");
    }
}

mod hardening {
    use std::sync::LazyLock;

    use super::*;

    static ORACLE: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(include_str!("../testdata/attachment_hardening_oracle.json")).unwrap()
    });

    const PAYLOAD: &str = r#"{"cc_emails": ["person@example.com"], "email": "jane.doe@example.com", "planned_start_date": "2030-01-02T10:00:00-05:00", "subject": "Acme"}"#;

    fn plan(path: &str, guards: &[(&str, &str)], requester: Option<i64>) -> AttachedPlan {
        AttachedPlan {
            base: WritePlan {
                action: "ticket_create_attachments".into(),
                target: 0,
                tenant: "acme.freshservice.com".into(),
                method: "POST".into(),
                path: path.into(),
                payload_json: PAYLOAD.into(),
                context_hash: "synthetic".into(),
            },
            files: Vec::new(),
            limit: Some(4096),
            limit_source: METADATA_SOURCE.into(),
            published_limit: None,
            prior_guards: guards
                .iter()
                .map(|(a, h)| (a.to_string(), h.to_string()))
                .collect(),
            requester_id: requester,
        }
    }

    #[test]
    fn identities_match_python() {
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        let guards = [
            ("create", a.as_str()),
            ("ticket_create_attachments", b.as_str()),
        ];
        for want in ORACLE["identities"].as_array().unwrap() {
            let name = want["name"].as_str().unwrap();
            let p = match name {
                "plain" => plan("tickets", &[], None),
                "guards" => plan("tickets", &guards, None),
                "requester" => plan("tickets", &[], Some(81)),
                _ => plan("tickets", &guards, Some(81)),
            };
            assert_eq!(p.hash(), want["hash"].as_str().unwrap(), "{name}");
            assert_eq!(Value::Object(p.review()), want["review"], "{name}");
        }
    }

    #[test]
    fn readbacks_match_python() {
        for case in ORACLE["readbacks"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let path = case["path"].as_str().unwrap().to_string();
            let key = if path == "tickets" {
                "ticket"
            } else {
                "change"
            };
            let row = case["row"].clone();
            let route = format!("{path}/123");
            let c = FnApi::new(move |call: &Call| {
                assert_eq!(call.path, route);
                Ok(json!({key: row.clone()}))
            });
            let p = plan(&path, &[], case["requester"].as_i64());
            let got = match readback(&c, &p, &json!({key: {"id": 123}}), &[]) {
                Ok(r) => json!({"result": r}),
                Err(_) => json!({"error": "ReviewedWriteError"}),
            };
            assert_eq!(got, case["outcome"], "{name}");
        }
    }

    #[test]
    fn senders_match_python() {
        for case in ORACLE["senders"].as_array().unwrap() {
            let row = case["row"].as_object();
            assert_eq!(
                crate::conversations::sender_readback(row, &case["expected"]),
                case["state"].as_str().unwrap(),
                "{case}"
            );
        }
    }

    fn change_plan(
        c: &dyn Api,
        store: &dyn MetaStore,
        paths: &[String],
        guards: &[(String, String)],
    ) -> Result<AttachedPlan> {
        let mut p = build(c, store, paths, "change", &[], None)?;
        p.prior_guards = guards.to_vec();
        Ok(p)
    }

    /// `test_prior_create_guard_blocks_new_attachment_hash`.
    #[test]
    fn prior_create_guard_blocks_send() {
        for event in ["started", "done"] {
            let state = Arc::new(Mutex::new(new_state(701)));
            state.lock().unwrap()["change"] = json!({"id": 123, "attachments": []});
            let c = api(state.clone(), "acme.example.com");
            let store = MemStore::default();
            let journal = MemJournal::default();
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("acme.txt");
            std::fs::write(&path, "Acme evidence 0\n").unwrap();
            let paths = vec![path.to_string_lossy().into_owned()];
            let legacy = "a".repeat(64);
            journal.record_plan(event, &legacy, "create", Some(&json!(0)), None);
            let guards = vec![("create".to_string(), legacy.clone())];
            let p = change_plan(&c, &store, &paths, &guards).unwrap();
            let rebuild = || change_plan(&c, &store, &paths, &guards);
            let err = send(&p, &c, &journal, Some(&p.hash()), &rebuild).unwrap_err();
            assert!(err.to_string().contains("prior attachment/create"), "{err}");
            assert!(
                state.lock().unwrap()["requests"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
    }

    /// `test_new_transaction_also_persists_legacy_guard_before_upload`.
    #[test]
    fn prior_guard_persists_before_upload() {
        let state = Arc::new(Mutex::new(new_state(701)));
        state.lock().unwrap()["change"] = json!({"id": 123, "attachments": []});
        state.lock().unwrap()["fail"] = json!("disconnect");
        let c = api(state.clone(), "acme.example.com");
        let store = MemStore::default();
        let journal = MemJournal::default();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("acme.txt");
        std::fs::write(&path, "Acme evidence 0\n").unwrap();
        let paths = vec![path.to_string_lossy().into_owned()];
        let legacy = "b".repeat(64);
        let guards = vec![("create".to_string(), legacy.clone())];
        let p = change_plan(&c, &store, &paths, &guards).unwrap();
        let rebuild = || change_plan(&c, &store, &paths, &guards);
        let err = send(&p, &c, &journal, Some(&p.hash()), &rebuild).unwrap_err();
        assert!(err.to_string().contains("outcome unknown"), "{err}");
        assert_eq!(
            last_event(&journal, "create", &legacy).as_deref(),
            Some("started")
        );
        assert_eq!(
            state.lock().unwrap()["requests"].as_array().unwrap().len(),
            1
        );
    }
}

mod identities {
    use std::sync::LazyLock;

    use super::*;

    static ORACLE: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(include_str!(
            "../testdata/attachment_identities_oracle.json"
        ))
        .unwrap()
    });

    const PAYLOAD: &str = r#"{"custom_fields": {"count": 12, "requester_lookup": "991"}, "email": "jane.doe@example.com", "subject": "Acme"}"#;

    #[test]
    fn custom_fields_match_python() {
        for case in ORACLE["matches"].as_array().unwrap() {
            assert_eq!(
                crate::readback::custom_fields_match(&case["actual"], &case["expected"]),
                case["matched"].as_bool().unwrap(),
                "{case}"
            );
        }
    }

    #[test]
    fn readbacks_keep_uploaded_ids_and_match_python() {
        for case in ORACLE["readbacks"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let path = case["path"].as_str().unwrap().to_string();
            let key = if path == "tickets" {
                "ticket"
            } else {
                "change"
            };
            let row = case["row"].clone();
            let route = format!("{path}/123");
            let c = FnApi::new(move |call: &Call| {
                assert_eq!(call.path, route);
                Ok(json!({key: row.clone()}))
            });
            let p = AttachedPlan {
                base: WritePlan {
                    action: "ticket_create_attachments".into(),
                    target: 0,
                    tenant: "acme.freshservice.com".into(),
                    method: "POST".into(),
                    path: path.clone(),
                    payload_json: PAYLOAD.into(),
                    context_hash: "synthetic".into(),
                },
                files: Vec::new(),
                limit: Some(4096),
                limit_source: METADATA_SOURCE.into(),
                published_limit: None,
                prior_guards: Vec::new(),
                requester_id: case["requester"].as_i64(),
            };
            let got = match readback(&c, &p, &json!({key: {"id": 123}}), &[501, 502]) {
                Ok(r) => json!({"result": r}),
                Err(_) => json!({"error": "ReviewedWriteError"}),
            };
            assert_eq!(got, case["outcome"], "{name}");
        }
    }
}
