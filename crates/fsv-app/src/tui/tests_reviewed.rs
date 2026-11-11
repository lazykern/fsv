//! The reviewed ticket hooks (`r` / `n` replies and notes, `e` on a ticket)
//! over a recording client. Expected requests, plans, results and error
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
/// that the reply/note POSTs add.
fn ticket_tenant() -> Tenant {
    let row = Arc::new(Mutex::new(PY["row"].clone()));
    let convs = Arc::new(Mutex::new(Vec::<Value>::new()));
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
                convs.lock().unwrap().push(
                    json!({"id": 456, "body": b["body"], "cc_emails": b["cc_emails"]}),
                );
                if let Some(h) = b.get("helpdesk_ticket") {
                    row.lock().unwrap()["status"] = h["status"].clone();
                }
                ok(json!({"conversation": {"id": 456}}))
            }
            ("POST", "tickets/123/notes") => {
                let b = body?;
                convs.lock().unwrap().push(json!({"id": 457, "body": b["body"], "private": b["private"],
                                                  "notify_emails": b["notify_emails"]}));
                ok(json!({"note": {"id": 457}}))
            }
            ("PUT", "tickets/123") => {
                let b = body?;
                let mut r = row.lock().unwrap();
                for (k, v) in b.as_object()? {
                    if k != "br_validation_excludes" {
                        r[k] = v.clone();
                    }
                }
                ok(json!({"ticket": *r}))
            }
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

/// The review without its `warning`: fsv-api still has the warning text
/// from before Python added sender/forward wording (see PARITY.md).
fn review_sans_warning(v: &Value) -> Value {
    let mut m = v.as_object().unwrap().clone();
    m.remove("warning");
    Value::Object(m)
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
    assert_eq!(t.lines(), strings(&py["plan_lines"]), "{name} plan requests");
    assert_eq!(plan.hash, py["hash"].as_str().unwrap(), "{name} hash");
    let payload: Value = serde_json::from_str(py["payload_json"].as_str().unwrap()).unwrap();
    assert_eq!(plan.raw, payload, "{name} raw is the payload");
    assert_eq!(
        review_sans_warning(&plan.review),
        review_sans_warning(&py["review"])
    );
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
    // Python's send also re-resolves the CC before the POST; fsv-api's
    // `conversations::send` does not (see PARITY.md).
    let mut send = strings(&PY["reply"]["send_lines"]);
    send.retain(|l| !l.starts_with("GET /search/autocomplete/requesters"));
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
fn sender_and_bcc_use_python_refusals_and_never_write() {
    const NOT_BUILT: &str =
        "replies with a sender or BCC are not available in this build; clear Sender and BCC to send";
    for case in PY["refusals"].as_array().unwrap() {
        let t = ticket_tenant();
        let req = request(case["reply"] == json!(true), &case["options"]);
        let err = backend(&t)
            .ticket_conversation_plan(&json!(123), "Acme", &req)
            .unwrap_err()
            .to_string();
        let want = case["error"].as_str().unwrap_or(NOT_BUILT);
        assert_eq!(err, want, "{}", case["options"]);
        assert_eq!(t.lines(), strings(&case["lines"]), "{}", case["options"]);
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
fn property_plans_wait_on_answers_in_fsv_api() {
    for (name, why) in PY["answers_for_flag_fields"].as_object().unwrap() {
        assert!(why.get("error").is_some(), "Python refuses {name} too");
    }
    let t = ticket_tenant();
    assert_eq!(
        backend(&t)
            .ticket_property_plan(&json!(123), "subject", &json!("Globex"))
            .unwrap_err()
            .to_string(),
        "reviewed ticket property edits from the form are not available in this build; use the web form"
    );
    assert!(t.calls().is_empty());
}

#[test]
fn property_send_rebuilds_puts_once_and_reads_back() {
    let py = &PY["property_send"];
    let t = ticket_tenant();
    let b = backend(&t);
    let tags = vec!["Acme".to_string(), "one".to_string()];
    let p = properties::build_plan(
        b.api(),
        b.store.as_ref(),
        "123",
        Some(&tags),
        None,
        None,
    )
    .unwrap();
    assert_eq!(t.lines(), strings(&py["plan_lines"]));
    assert_eq!(
        (&p.body_json, &p.before_json, &p.record_hash),
        (
            &py["body_json"].as_str().unwrap().to_string(),
            &py["before_json"].as_str().unwrap().to_string(),
            &py["record_hash"].as_str().unwrap().to_string()
        )
    );
    let plan = property_review(&p);
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
