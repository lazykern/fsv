//! The reviewed hooks (ticket replies, notes and property edits, problem
//! known-error marks, planning text, asset candidates) over a recording
//! client. Expected requests, plans, results and error
//! texts are from running the Python TUI helpers over the same tenant
//! (`port/golden/gen_fsv_app_tui_reviewed.py`).

use std::sync::LazyLock;

use super::*;
use crate::testkit::{Call, MemJournal, Tenant, ok, tenant};

static PY: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("testdata/reviewed_expected.json")).unwrap()
});

fn backend(t: &Tenant) -> LiveBackend {
    let schema: SchemaLoader = Arc::new(|_r: Res| Ok(json!({"fields": []})));
    LiveBackend::new(t.client.clone(), schema)
        .with_journal(Arc::new(MemJournal::default()))
        .with_store(Arc::new(fsv_api::formmeta::MemStore::default()))
        .with_config(Map::new())
}

/// The generator's tenant: ticket 123, its edit form, and conversations
/// that the reply/note POSTs add; problem 77; planning rows of problem 77
/// and change 5; one asset search page.
fn ticket_tenant() -> Tenant {
    ticket_tenant_with(json!({
        "problems/77": [{"name": "problem_cause", "label": "Root Cause", "description": "", "attachments": []}],
        "changes/5": [{"name": "rollout_plan", "label": "Rollout Plan", "description": "", "attachments": []}],
    }))
}

/// [`ticket_tenant`] with other planning rows (`"record path": [rows]`).
fn ticket_tenant_with(planning: Value) -> Tenant {
    let row = Arc::new(Mutex::new(PY["row"].clone()));
    let convs = Arc::new(Mutex::new(Vec::<Value>::new()));
    let problem = Arc::new(Mutex::new(
        json!({"id": 77, "subject": "Disk", "known_error": false}),
    ));
    let planning = Arc::new(Mutex::new(planning));
    tenant(move |c: &Call| {
        let p = c.path.strip_prefix("/api/_/").unwrap_or(&c.path);
        let body = c.json();
        match (c.method.as_str(), p) {
            ("GET", "/search/autocomplete/requesters") => {
                let q = &c.query.iter().find(|(k, _)| k == "q")?.1;
                ok(json!({"results": [{"id": 11, "value": "Acme User", "email": q}]}))
            }
            ("GET", "ticket_form_fields") => ok(json!({"ticket_fields": PY["fields"]})),
            ("GET", p) if p.ends_with("business_rules") => ok(json!({"business_rules": []})),
            ("GET", "tickets/123") => ok(json!({"ticket": *row.lock().unwrap()})),
            ("GET", "tickets/123/conversations") => {
                let convs = convs.lock().unwrap();
                ok(json!({"conversations": *convs, "meta": {"count": convs.len()}}))
            }
            ("POST", "tickets/123/reply") => {
                let b = body?;
                let list = |k: &str| b.get(k).cloned().unwrap_or(json!([]));
                convs.lock().unwrap().push(
                    json!({"id": 456, "body": b["body"], "cc_emails": list("cc_emails"),
                                                  "bcc_emails": list("bcc_emails")}),
                );
                if let Some(h) = b.get("helpdesk_ticket") {
                    row.lock().unwrap()["status"] = h["status"].clone();
                }
                ok(json!({"conversation": {"id": 456}}))
            }
            ("POST", "tickets/123/notes") => {
                let b = body?;
                convs.lock().unwrap().push(
                    json!({"id": 457, "body": b["body"], "private": b["private"],
                                                  "notify_emails": b["notify_emails"]}),
                );
                ok(json!({"note": {"id": 457}}))
            }
            ("PUT", "tickets/123") => {
                let b = body?;
                let mut r = row.lock().unwrap();
                for (k, v) in b.as_object()? {
                    if k == "custom_fields" {
                        for (ck, cv) in v.as_object()? {
                            r["custom_fields"][ck] = cv.clone();
                        }
                    } else if k != "br_validation_excludes" {
                        r[k] = v.clone();
                    }
                }
                ok(json!({"ticket": *r}))
            }
            ("GET", "problems/77") => ok(json!({"problem": *problem.lock().unwrap()})),
            ("PUT", "problems/77") => {
                let mut pr = problem.lock().unwrap();
                for (k, v) in body?.as_object()? {
                    pr[k] = v.clone();
                }
                ok(json!({"problem": *pr}))
            }
            (m, p) if p.contains("/planning-fields/") => {
                let (rec, key) = p.split_once("/planning-fields/")?;
                let mut all = planning.lock().unwrap();
                let rows = all.get_mut(rec)?.as_array_mut()?;
                if m == "PUT" {
                    for r in rows.iter_mut().filter(|r| r["name"] == json!(key)) {
                        r["description"] = body.as_ref()?["description"].clone();
                    }
                }
                if m == "DELETE" {
                    rows.retain(|r| r["name"] != json!(key));
                }
                ok(json!({"planning_field": {"id": 1}}))
            }
            (m, p) if p.ends_with("/planning-fields") => {
                let rec = p.trim_end_matches("/planning-fields");
                let mut all = planning.lock().unwrap();
                let rows = all.get_mut(rec)?;
                if m == "GET" {
                    let key = if rec.starts_with("changes") {
                        "change_planning_fields"
                    } else {
                        "problem_planning_fields"
                    };
                    return ok(json!({ key: *rows }));
                }
                let id = &c.query.iter().find(|(k, _)| k == "id")?.1;
                for r in rows.as_array_mut()? {
                    if r["name"] == json!(id) {
                        r["description"] = body.as_ref()?["description"].clone();
                    }
                }
                ok(json!({"planning_field": {"id": 1}}))
            }
            ("GET", p) if p.contains("/itam/core/") && p.ends_with("/api/") => ok(json!({
                "page": 1,
                "error": null,
                "total_rows": 2,
                "data": [
                    {"device_pk": 7, "name": {"id": 7, "label": "Acme DB"}, "type": "Computer"},
                    {"device_pk": 8, "name": {"id": 8, "label": "Globex"}, "type": "Computer"}
                ]
            })),
            _ => None,
        }
    })
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect()
}

fn writes(t: &Tenant, from: usize) -> Vec<String> {
    t.calls()[from..]
        .iter()
        .filter(|c| c.method == "POST" || c.method == "PUT")
        .map(|c| c.body.clone().unwrap_or_default())
        .collect()
}

fn emails(options: &Value, key: &str) -> Vec<String> {
    options
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .collect()
}

/// `_prepare_ticket_conversation`'s options dict as the TUI hands it over.
fn request(reply: bool, options: &Value) -> ConversationRequest {
    ConversationRequest {
        reply,
        public: false,
        cc: emails(options, "cc"),
        notify: emails(options, "notify"),
        bcc: emails(options, "bcc"),
        sender: options
            .get("sender")
            .and_then(Value::as_str)
            .map(str::to_string),
        set_status: options
            .get("pending")
            .is_some_and(fsv_py::truthy)
            .then(|| "Pending".to_string()),
    }
}

fn conversation_case(
    name: &str,
    body: &str,
    reply: bool,
    options: Value,
    confirm: bool,
    send_lines: Vec<String>,
) {
    let py = &PY[name];
    let t = ticket_tenant();
    let b = backend(&t);
    let plan = b
        .ticket_conversation_plan(&json!(123), body, &request(reply, &options))
        .unwrap();
    assert_eq!(
        t.lines(),
        strings(&py["plan_lines"]),
        "{name} plan requests"
    );
    assert_eq!(plan.hash, py["hash"].as_str().unwrap(), "{name} hash");
    let payload: Value = serde_json::from_str(py["payload_json"].as_str().unwrap()).unwrap();
    assert_eq!(plan.raw, payload, "{name} raw is the payload");
    assert_eq!(plan.review, py["review"], "{name} review");
    assert_eq!(plan.target_id, json!(123));
    assert_eq!(plan.public, reply);

    let n = t.calls().len();
    let result = b.send_ticket_conversation(&plan, confirm).unwrap();
    let lines = t.lines();
    assert_eq!(lines[n..], send_lines[..], "{name} send requests");
    assert_eq!(writes(&t, n), strings(&py["send_bodies"]));
    assert_eq!(result, py["result"]);
    assert_eq!(
        b.send_ticket_conversation(&plan, confirm)
            .unwrap_err()
            .to_string(),
        "conversation was sent or outcome unknown; inspect ticket and journal before retry"
    );
}

#[test]
fn plain_reply_with_cc_and_pending_plans_and_sends_once() {
    let send = strings(&PY["reply"]["send_lines"]);
    conversation_case(
        "reply",
        "Acme update",
        true,
        json!({"cc": "person@example.com", "pending": true}),
        true,
        send,
    );
}

#[test]
fn private_note_with_notify_plans_and_sends_without_public_consent() {
    conversation_case(
        "note",
        "Acme note",
        false,
        json!({"notify": "b@example.com"}),
        false,
        strings(&PY["note"]["send_lines"]),
    );
}

#[test]
fn public_reply_needs_consent() {
    let t = ticket_tenant();
    let b = backend(&t);
    let plan = b
        .ticket_conversation_plan(&json!(123), "Acme", &request(true, &json!({})))
        .unwrap();
    assert_eq!(
        b.send_ticket_conversation(&plan, false)
            .unwrap_err()
            .to_string(),
        "public conversation requires explicit confirmation"
    );
    assert!(writes(&t, 0).is_empty());
}

#[test]
fn bcc_and_sender_reply_plans_and_sends_once() {
    conversation_case(
        "reply_bcc",
        "Acme bcc",
        true,
        json!({"bcc": "x@example.com", "sender": "help@acme.example.com"}),
        true,
        strings(&PY["reply_bcc"]["send_lines"]),
    );
}

#[test]
fn sender_and_bcc_plans_and_refusals_match_python() {
    for case in PY["refusals"].as_array().unwrap() {
        let t = ticket_tenant();
        let req = request(case["reply"] == json!(true), &case["options"]);
        let got = backend(&t).ticket_conversation_plan(&json!(123), "Acme", &req);
        match case["error"].as_str() {
            Some(want) => assert_eq!(got.unwrap_err().to_string(), want, "{}", case["options"]),
            None => {
                let plan = got.unwrap();
                assert_eq!(plan.raw, case["payload"], "{}", case["options"]);
                assert_eq!(
                    plan.hash,
                    case["hash"].as_str().unwrap(),
                    "{}",
                    case["options"]
                );
                assert_eq!(plan.review, case["review"], "{}", case["options"]);
            }
        }
        assert_eq!(t.lines(), strings(&case["lines"]), "{}", case["options"]);
        assert!(writes(&t, 0).is_empty());
    }
}

#[test]
fn a_conversation_plan_from_elsewhere_is_refused() {
    let t = ticket_tenant();
    let plan = ReviewPlan {
        review: json!({}),
        hash: "x".into(),
        public: true,
        target_id: json!(123),
        raw: json!({"private": true}),
    };
    assert_eq!(
        backend(&t)
            .send_ticket_conversation(&plan, true)
            .unwrap_err()
            .to_string(),
        "the conversation plan was not built by this session; review it again"
    );
    assert!(t.calls().is_empty());
}

#[test]
fn property_fields_follow_the_tui_filter_on_the_fresh_edit_form() {
    let t = ticket_tenant();
    let got = backend(&t).ticket_property_fields(&json!(123)).unwrap();
    let got: Vec<Value> = got
        .iter()
        .map(|f| {
            json!({"name": f.name, "label": f.label,
                   "choices": f.choices.as_ref().map(|c| c.iter().map(|(v, l)| json!([v, l])).collect::<Vec<_>>())})
        })
        .collect();
    assert_eq!(Value::Array(got), PY["property_fields"]["fields"]);
    assert_eq!(t.lines(), strings(&PY["property_fields"]["lines"]));
}

#[test]
fn property_answers_plan_and_send_like_python() {
    for (name, py) in PY["property_answers"].as_object().unwrap() {
        let t = ticket_tenant();
        let b = backend(&t);
        let got = b.ticket_property_plan(&json!(123), name, &py["value"]);
        if let Some(want) = py["error"].as_str() {
            assert_eq!(got.unwrap_err().to_string(), want, "{name}");
            assert_eq!(t.lines(), strings(&py["lines"]), "{name}");
            continue;
        }
        let plan = got.unwrap();
        assert_eq!(
            t.lines(),
            strings(&py["plan_lines"]),
            "{name} plan requests"
        );
        assert_eq!(plan.hash, py["hash"].as_str().unwrap(), "{name} hash");
        assert_eq!(plan.review, py["review"], "{name} review");
        let n = t.calls().len();
        assert_eq!(
            b.send_ticket_property(&plan).unwrap(),
            py["result"],
            "{name} result"
        );
        assert_eq!(
            t.lines()[n..],
            strings(&py["send_lines"])[..],
            "{name} send requests"
        );
        assert_eq!(
            writes(&t, n),
            strings(&py["send_bodies"]),
            "{name} send bodies"
        );
    }
}

#[test]
fn property_send_rebuilds_puts_once_and_reads_back() {
    let py = &PY["property_send"];
    let t = ticket_tenant();
    let b = backend(&t);
    let tags = vec!["Acme".to_string(), "one".to_string()];
    let p =
        properties::build_plan(b.api(), b.store.as_ref(), "123", Some(&tags), None, None).unwrap();
    assert_eq!(t.lines(), strings(&py["plan_lines"]));
    assert_eq!(
        (&p.body_json, &p.before_json, &p.record_hash),
        (
            &py["body_json"].as_str().unwrap().to_string(),
            &py["before_json"].as_str().unwrap().to_string(),
            &py["record_hash"].as_str().unwrap().to_string()
        )
    );
    let plan = property_review(&p.into());
    assert_eq!(plan.hash, py["hash"].as_str().unwrap());
    assert_eq!(plan.review, py["review"]);

    let n = t.calls().len();
    assert_eq!(b.send_ticket_property(&plan).unwrap(), py["result"]);
    assert_eq!(t.lines()[n..], strings(&py["send_lines"])[..]);
    assert_eq!(writes(&t, n), strings(&py["send_bodies"]));
    assert_eq!(
        b.send_ticket_property(&plan).unwrap_err().to_string(),
        "update was sent or its outcome is unknown; inspect the ticket and journal before retry"
    );
    let other = ReviewPlan {
        raw: json!({"kind": "association"}),
        ..plan
    };
    assert_eq!(
        b.send_ticket_property(&other).unwrap_err().to_string(),
        "not a ticket property plan"
    );
}

/// A `WritePlan` hook pair: the plan, then the send with its reviewed hash.
fn write_case(
    name: &str,
    py: &Value,
    t: &Tenant,
    plan: ReviewPlan,
    send: impl Fn(&ReviewPlan) -> BResult<Value>,
) {
    assert_eq!(
        t.lines(),
        strings(&py["plan_lines"]),
        "{name} plan requests"
    );
    assert_eq!(plan.hash, py["hash"].as_str().unwrap(), "{name} hash");
    assert_eq!(plan.review, py["review"], "{name} review");
    assert_eq!(plan.target_id, py["target"]);
    for k in [
        "action",
        "target",
        "method",
        "path",
        "payload_json",
        "context_hash",
    ] {
        assert_eq!(plan.raw[k], py[k], "{name} {k}");
    }
    let n = t.calls().len();
    assert_eq!(send(&plan).unwrap(), py["result"], "{name} result");
    assert_eq!(
        t.lines()[n..],
        strings(&py["send_lines"])[..],
        "{name} send requests"
    );
    assert_eq!(
        writes(t, n),
        strings(&py["send_bodies"]),
        "{name} send bodies"
    );
    let foreign = ReviewPlan {
        raw: json!({"kind": "ticket_property"}),
        ..plan
    };
    assert_eq!(
        send(&foreign).unwrap_err().to_string(),
        "not a plan for this action"
    );
}

#[test]
fn known_error_marks_once_and_reads_back() {
    let t = ticket_tenant();
    let b = backend(&t);
    let plan = b.known_error_plan(&json!(77)).unwrap();
    write_case("known_error", &PY["known_error"], &t, plan, |p| {
        b.send_known_error(p)
    });
    let again = b.known_error_plan(&json!(77)).unwrap();
    assert_ne!(again.hash, PY["known_error"]["hash"].as_str().unwrap());
}

fn planning_backend(t: &Tenant) -> LiveBackend {
    backend(t).with_change_schema(|_| Ok(json!({"fields": PY["change_fields"]})))
}

/// `test_tui_native_existing_content_preloads_and_reviews_edit`: stored
/// section text is reviewed as an `edit` and sent once as a PUT.
#[test]
fn planning_text_edits_stored_text_with_a_put() {
    let t = ticket_tenant_with(json!({
        "changes/5": [{"name": "rollout_plan", "label": "Rollout Plan", "description": "<p>v1</p>", "attachments": []}],
    }));
    let b = planning_backend(&t);
    let plan = b
        .planning_text_plan("changes", &json!(5), "rollout_plan", "<p>v2</p>")
        .unwrap();
    assert_eq!(plan.review["operation"], "edit");
    assert_eq!(plan.review["before"], "<p>v1</p>");
    write_case(
        "changes_edit",
        &PY["planning"]["changes_edit"],
        &t,
        plan,
        |p| b.send_planning_text(p),
    );
}

#[test]
fn planning_text_reads_plans_and_sends_like_python() {
    for (module, id, section) in [
        ("problems", 77, "problem_cause"),
        ("changes", 5, "rollout_plan"),
    ] {
        let py = &PY["planning"][module];
        let t = ticket_tenant();
        let b = planning_backend(&t);
        assert_eq!(
            Value::Array(b.planning_definitions(module).unwrap()),
            py["definitions"]
        );
        assert_eq!(
            b.planning_row(module, &json!(id), section).unwrap(),
            Some(py["row"].clone())
        );
        assert_eq!(b.planning_row(module, &json!(id), "nope").unwrap(), None);
        assert_eq!(
            t.lines(),
            [strings(&py["read_lines"]), strings(&py["read_lines"])].concat(),
            "{module} reads"
        );

        let t = ticket_tenant();
        let b = planning_backend(&t);
        let payload: Value = serde_json::from_str(py["payload_json"].as_str().unwrap()).unwrap();
        let plan = b
            .planning_text_plan(
                module,
                &json!(id),
                section,
                payload["description"].as_str().unwrap(),
            )
            .unwrap();
        write_case(module, py, &t, plan, |p| b.send_planning_text(p));
    }
    let t = ticket_tenant();
    assert_eq!(
        planning_backend(&t)
            .planning_definitions("tickets")
            .unwrap_err()
            .to_string(),
        PY["planning"]["tickets_error"].as_str().unwrap()
    );
}

#[test]
fn asset_candidates_read_one_search_page() {
    let t = ticket_tenant();
    let got = backend(&t)
        .asset_candidates(&AssetCandidatesRequest {
            change: json!(5),
            query: "acme".into(),
            page: 1,
        })
        .unwrap();
    assert_eq!(
        json!({"change_id": got.change_id, "page": got.page, "scope": got.scope, "assets": got.assets,
               "mutation": got.mutation, "warning": got.warning}),
        PY["assets"]["result"]
    );
    assert_eq!(t.lines(), strings(&PY["assets"]["lines"]));
    assert!(writes(&t, 0).is_empty());
}
