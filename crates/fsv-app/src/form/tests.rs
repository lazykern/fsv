//! Expected plans, requests and results are from `formui/session.py` run
//! over a recording client with the same fixtures (`change_session`,
//! `ticket_session`, `sr_session`, then `build_plan` and `send`).

use super::*;
use crate::testkit::{Call, MemJournal, Tenant, ok, pairs, status, tenant};
use fsv_api::formmeta::MemStore;
use fsv_core::client::Response;

const BR: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltdIn0=";
const SR_BR: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyIxMSI6eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltdIn19";
const NEW_HASH: &str = "a28516d9594757efde10c3259d1f7e4499474bd3af6cb68b26f06ad66726ca24";
const EDIT_HASH: &str = "de282eb78ab489672f6fba88eafaccc5a4585ecfe6770832c57a85370d262869";
const TICKET_HASH: &str = "0f2785c0cb49ffd3b55b9a38696dc271157e7225aeb601050bd5bd65afa02d9a";
const SR_HASH: &str = "51d1b1aaf494f9d0c061f3f62387b65e14b266a9d2b15cccff5e633d713c50b0";

fn field(id: i64, name: &str, label: &str, ft: &str, required: bool, default: bool) -> Json {
    json!({"id": id, "name": name, "label": label, "field_type": ft, "required": required, "position": id,
           "default_field": default, "choices": [], "nested_fields": [], "workspace_id": 2})
}

fn fields(key: &str) -> Json {
    json!({key: [
        field(1, "subject", "Subject", "default_subject", true, true),
        field(2, "requester", "Requester", "default_requester", true, true),
        field(3, "cf_note", "Note", "custom_text", false, false),
    ]})
}

fn item() -> Json {
    json!({"id": 11, "display_id": 5, "name": "Laptop", "allow_attachments": false, "configs": {"attachment_mandatory": false},
           "custom_fields": [{"id": 21, "name": "reason", "label": "Reason", "type": 1001, "required": false, "account_id": 1}]})
}

/// The recording client of the Python run: `{}` for any other GET.
fn route(c: &Call) -> Option<Response> {
    let p = c.path.strip_prefix("/api/_/").unwrap_or(&c.path);
    match (c.method.as_str(), p) {
        ("GET", "change_form_fields") => ok(fields("change_fields")),
        ("GET", "ticket_form_fields") => ok(fields("ticket_fields")),
        ("GET", p) if p.ends_with("business_rules") => {
            ok(json!({"business_rules": [], "user": {"time_zone": "Tokyo"}}))
        }
        ("GET", "changes/77") => ok(
            json!({"change": {"id": 77, "display_id": 77, "human_display_id": "CHN-77", "subject": "Old",
                                                    "requester_id": 5, "custom_fields": {"cf_note": "n"}}}),
        ),
        ("GET", "catalog/service-items/5") => ok(json!({"service_item": item()})),
        ("GET", "bootstrap/agents_groups") => ok(json!({"groups": [{"id": 4, "name": "Ops"}]})),
        ("GET", "groups/403") => status(403, json!({})),
        ("GET", _) => ok(json!({})),
        ("POST", "changes") => ok(json!({"change": {"id": 78, "display_id": 78}})),
        ("PUT", "changes/77") => ok(json!({"change": {"id": 77}})),
        ("POST", "tickets") => ok(json!({"ticket": {"id": 31}})),
        ("POST", "catalog/service-items/5/place-request") => {
            ok(json!({"service_request": {"id": 91, "display_id": 91}}))
        }
        _ => None,
    }
}

fn deps(t: &Tenant, j: &Arc<MemJournal>) -> FormDeps {
    let api: Arc<dyn Api + Send + Sync> = t.client.clone();
    let store: Arc<dyn MetaStore + Send + Sync> = Arc::new(MemStore::default());
    FormDeps {
        form_api: t.client.clone(),
        meta: Arc::new(LiveMeta::new(api.clone(), store.clone())),
        store,
        plan_journal: j.clone(),
        journal: j.clone(),
        writer: Arc::new(LiveWriter::new(api.clone())),
        resolver: Arc::new(ClientResolver::new(api.clone())),
        config: Map::new(),
        paths: Paths::new(Some(t.dir.path().to_path_buf()), t.dir.path()),
        attach_flow: Some("after-create".into()),
        api,
    }
}

fn jane() -> LookupValue {
    LookupValue::new(9, "Jane Doe", Some("jane@example.com"))
}

fn rules_query(module: &str, form_type: &str, module_id: &str) -> Vec<(String, String)> {
    pairs(&[
        ("ember_api", "true"),
        ("module_type", module),
        ("item_id", "null"),
        ("requester", ""),
        ("requested_for", ""),
        ("include_workspace", "false"),
        ("filter_by_default_form_action", "true"),
        ("form_type", form_type),
        ("user_type", "null"),
        ("module_id", module_id),
        ("cf_name_prefix", "customFields."),
    ])
}

fn sr_rules_query() -> Vec<(String, String)> {
    pairs(&[
        ("ember_api", "true"),
        ("module_type", "service_request"),
        ("item_id", "11"),
        ("requester", ""),
        ("requested_for", ""),
        ("include_workspace", "false"),
        ("form_type", "new_form"),
        ("user_type", "null"),
        ("module_id", ""),
        ("cf_name_prefix", "customField."),
    ])
}

fn exit_of(e: &Error) -> (i32, String) {
    match e {
        Error::Exit { code, message } => (*code, message.clone()),
        other => (other.exit_code(), other.to_string()),
    }
}

fn set(s: &mut FormSession, key: &str, v: &str) {
    s.set(key, Value::str(v));
}

fn new_change(t: &Tenant, j: &Arc<MemJournal>) -> FormSession {
    let o = ChangeForm {
        requester: Some(jane()),
        ..ChangeForm::new(FormMode::New)
    };
    let mut s = change_session(&deps(t, j), o).unwrap();
    set(&mut s, "subject", "Deploy é");
    set(&mut s, "cf_note", "hi");
    s
}

#[test]
fn new_change_plans_and_sends_like_the_python_session() {
    let (t, j) = (tenant(route), Arc::new(MemJournal::default()));
    let mut s = new_change(&t, &j);
    assert_eq!(s.title, "New Change");
    let plan = s.build_plan(None).unwrap();
    assert_eq!(plan.errors, Vec::<String>::new());
    assert_eq!(plan.hash, NEW_HASH);
    let payload = json!({"subject": "Deploy é", "custom_fields": {"cf_note": "hi"}, "email": "Jane Doe <jane@example.com>", "br_validation_excludes": BR});
    assert_eq!(plan.payload, Some(payload.clone()));
    let rows: Vec<(&str, &str)> = plan
        .review
        .rows
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    assert_eq!(
        rows,
        [
            ("Requester", "Jane Doe <jane@example.com>"),
            ("Subject", "Deploy é"),
            ("Note", "hi")
        ]
    );
    let n = t.calls().len();
    let out = s.send(&plan, false).unwrap();
    let calls = t.calls().split_off(n);
    assert_eq!(
        calls.iter().map(Call::line).collect::<Vec<_>>()[..1],
        ["GET /api/_/change_form_fields"]
    );
    assert_eq!(
        (calls[1].path.as_str(), calls[1].query.clone()),
        (
            "/api/_/ws/2/business_rules",
            rules_query("change", "new_form", "")
        )
    );
    assert_eq!(calls[2].line(), "POST /api/_/changes");
    assert_eq!(
        calls[2].body.as_deref(),
        Some(fsv_py::compact(&payload).as_str())
    );
    assert_eq!(calls.len(), 3);
    assert_eq!(
        out,
        json!({"id": 78, "result": {"id": 78, "display_id": 78}, "steps": [{"step": "create", "ok": true, "id": 78}], "uploaded": [], "plan_hash": NEW_HASH})
    );
    assert_eq!(
        j.events(),
        [
            json!({"kind": "plan:create", "event": "started", "plan": NEW_HASH, "created_id": null}),
            json!({"kind": "plan:create", "event": "done", "plan": NEW_HASH, "created_id": 78}),
        ]
    );
    let (code, msg) = exit_of(&s.send(&plan, false).unwrap_err());
    assert_eq!(code, 4);
    assert!(
        msg.starts_with("this exact plan was already sent at "),
        "{msg}"
    );
}

#[test]
fn edit_change_fetches_the_record_and_puts_the_diff() {
    let (t, j) = (tenant(route), Arc::new(MemJournal::default()));
    let o = ChangeForm {
        target: Some(json!(77)),
        ..ChangeForm::new(FormMode::Edit)
    };
    let mut s = change_session(&deps(&t, &j), o).unwrap();
    assert_eq!(s.title, "Edit change CHN-77");
    let lines = t.lines();
    assert_eq!(
        lines[..2],
        ["GET /api/_/changes/77", "GET /api/_/change_form_fields"]
    );
    assert_eq!(t.calls()[2].query, rules_query("change", "edit_form", "77"));
    set(&mut s, "subject", "New subject");
    let plan = s.build_plan(None).unwrap();
    assert_eq!(
        (plan.hash.as_str(), plan.payload.clone()),
        (EDIT_HASH, Some(json!({"subject": "New subject"})))
    );
    assert!(plan.review.rows.is_empty());
    let n = t.calls().len();
    let out = s.send(&plan, false).unwrap();
    let calls = t.calls().split_off(n);
    assert_eq!(
        calls
            .iter()
            .map(|c| format!("{} {}", c.method, c.path))
            .collect::<Vec<_>>(),
        [
            "GET /api/_/change_form_fields",
            "GET /api/_/ws/2/business_rules",
            "GET /api/_/change_form_fields",
            "GET /api/_/ws/2/business_rules",
            "GET /api/_/changes/77",
            "PUT /api/_/changes/77",
            "GET /api/_/changes/77"
        ]
    );
    assert_eq!(calls[1].query, rules_query("change", "edit_form", "77"));
    assert_eq!(
        calls[5].body.as_deref(),
        Some(r#"{"subject":"New subject"}"#)
    );
    assert_eq!(out["plan_hash"], json!(EDIT_HASH));
    assert_eq!(out["verified"], json!(false));
    assert_eq!(out["fields"]["subject"], json!("mismatched"));
    assert_eq!(out["steps"][0]["step"], json!("update"));
}

#[test]
fn ticket_form_posts_the_reviewed_bytes_once() {
    let (t, j) = (tenant(route), Arc::new(MemJournal::default()));
    let mut s = ticket_session(&deps(&t, &j), None, Some(jane()), false).unwrap();
    assert_eq!(s.title, "New Incident");
    set(&mut s, "subject", "Printer");
    set(&mut s, "cf_note", "x");
    let plan = s.build_plan(None).unwrap();
    assert_eq!(plan.hash, TICKET_HASH);
    assert_eq!(
        plan.payload,
        Some(
            json!({"subject": "Printer", "custom_fields": {"cf_note": "x"}, "email": "Jane Doe <jane@example.com>", "br_validation_excludes": BR})
        )
    );
    let n = t.calls().len();
    let out = s.send(&plan, false).unwrap();
    let calls = t.calls().split_off(n);
    assert_eq!(
        calls
            .iter()
            .map(|c| format!("{} {}", c.method, c.path))
            .collect::<Vec<_>>(),
        [
            "GET /api/_/ticket_form_fields",
            "GET /api/_/ws/2/business_rules",
            "POST /api/_/tickets"
        ]
    );
    assert_eq!(calls[1].query, rules_query("ticket", "new_form", ""));
    let body = format!(
        r#"{{"subject": "Printer", "custom_fields": {{"cf_note": "x"}}, "email": "Jane Doe <jane@example.com>", "br_validation_excludes": "{BR}"}}"#
    );
    assert_eq!(calls[2].body.as_deref(), Some(body.as_str()));
    let post = t
        .mock
        .requests()
        .into_iter()
        .rfind(|r| r.method == "POST")
        .unwrap();
    assert_eq!(post.header("X-CSRF-Token"), Some("tok"));
    assert_eq!(
        out,
        json!({"id": 31, "result": {"id": 31}, "plan_hash": TICKET_HASH, "steps": [{"step": "create", "ok": true}]})
    );
    let kinds: Vec<Json> = j
        .events()
        .iter()
        .map(|e| json!([e["kind"], e["event"], e["created_id"]]))
        .collect();
    assert_eq!(
        kinds,
        [
            json!(["plan:ticket_create", "started", null]),
            json!(["plan:ticket_create", "done", 31])
        ]
    );
}

#[test]
fn sr_form_places_the_request() {
    let (t, j) = (tenant(route), Arc::new(MemJournal::default()));
    let mut s = sr_session(&deps(&t, &j), "5", None, Some(jane()), false).unwrap();
    assert_eq!(s.title, "New service request: Laptop");
    let load = [
        "GET /api/_/catalog/service-items/5",
        "GET /api/_/bootstrap/me",
        "GET /api/_/workspaces",
    ];
    assert_eq!(t.lines()[..3], load);
    assert_eq!(t.calls()[3].query, sr_rules_query());
    set(&mut s, "reason", "broken é");
    let plan = s.build_plan(None).unwrap();
    assert_eq!(plan.hash, SR_HASH);
    let payload = json!({"email": "jane@example.com", "custom_fields": {"reason": "broken é"}, "br_validation_excludes": SR_BR, "attachments": []});
    assert_eq!(plan.payload, Some(payload.clone()));
    let n = t.calls().len();
    let out = s.send(&plan, false).unwrap();
    let calls = t.calls().split_off(n);
    assert_eq!(
        calls.iter().map(Call::line).take(3).collect::<Vec<_>>(),
        load
    );
    assert_eq!(
        (calls[3].path.as_str(), calls[3].query.clone()),
        ("/api/_/business_rules", sr_rules_query())
    );
    assert_eq!(
        calls[4].line(),
        "POST /api/_/catalog/service-items/5/place-request"
    );
    assert_eq!(
        calls[4].body.as_deref(),
        Some(fsv_py::compact(&payload).as_str())
    );
    assert_eq!(calls.len(), 5);
    assert_eq!(
        out,
        json!({"created": {"id": 91, "display": "SR-91"}, "result": {"service_request": {"id": 91, "display_id": 91}}, "uploaded": [],
               "body": payload, "plan_hash": SR_HASH,
               "steps": [{"step": "upload", "ok": true, "ids": []}, {"step": "place-request", "ok": true, "id": 91}],
               "verification": {"verified": false, "fields": {}}})
    );
    let rows = j.rows.lock().unwrap().clone();
    assert_eq!(
        rows.iter()
            .map(|r| (r["event"].clone(), r["target"].clone()))
            .collect::<Vec<_>>(),
        [
            (json!("started"), json!("catalog-item/5")),
            (json!("done"), json!("catalog-item/5"))
        ]
    );
}

#[test]
fn save_draft_writes_the_change_template() {
    let (t, j) = (tenant(route), Arc::new(MemJournal::default()));
    let mut s = new_change(&t, &j);
    let plan = s.build_plan(None).unwrap();
    std::fs::create_dir(t.dir.path().join("drafts")).unwrap();
    let path = t.dir.path().join("drafts/change-draft.yaml");
    let saved = s.save_draft(&path.to_string_lossy(), Some(&plan)).unwrap();
    let text = std::fs::read_to_string(saved).unwrap();
    let want = "# fsv change draft (saved)\n# Fill in values after the colons. Lines starting with # are ignored.\n# Fields appear and disappear with your answers, like the web form; after\n# saving, new required fields are added and the file reopens if needed.\n# Full choice lists: fsv form fields changes --choices LABEL\n# Save and close to continue; close without changes to abort.\n\n# REQUIRED · email or exact name of the requester\nrequester: jane@example.com\nanswers:\n  # REQUIRED · text\n  Subject: Deploy é\n  # optional · lookup (name or email) · The user this change is for, if not the requester (shown in fsv only: not sent yet).\n  Request for:\n  # optional · text\n  Note: hi\n\n# Files to attach after the change is created (uploaded only after you confirm)\nattachments: []\n# planning:            # planning field label -> files\n#   Rollout Plan: [rollout.xlsx]\n";
    assert_eq!(text, want);
}

fn lookup_route(c: &Call) -> Option<Response> {
    let p = c.path.strip_prefix("/api/_/").unwrap_or(&c.path);
    match (c.method.as_str(), p) {
        ("GET", "change_form_fields") => ok(json!({"change_fields": [
            field(1, "subject", "Subject", "default_subject", true, true),
            field(2, "requester", "Requester", "default_requester", true, true),
            field(3, "group_id", "Group", "default_group", false, true),
            field(4, "agent_id", "Agent", "default_agent", false, true),
        ]})),
        _ => route(c),
    }
}

/// Picked lookups keep their names into the review rows and the saved
/// draft (`gen_fsv_app_form_lookup.py`).
#[test]
fn picked_lookups_show_their_names() {
    let want: Json = serde_json::from_str(include_str!("testdata/form_lookup.json")).unwrap();
    let (t, j) = (tenant(lookup_route), Arc::new(MemJournal::default()));
    let o = ChangeForm {
        requester: Some(jane()),
        ..ChangeForm::new(FormMode::New)
    };
    let mut s = change_session(&deps(&t, &j), o).unwrap();
    set(&mut s, "subject", "Deploy");
    s.set("group_id", Value::Lookup(LookupValue::new(4, "Ops", None)));
    s.set(
        "agent_id",
        Value::Lookup(LookupValue::new(7, "Ann Agent", Some("ann@example.com"))),
    );
    let plan = s.build_plan(None).unwrap();
    assert_eq!(json!(plan.errors), want["errors"]);
    let rows: Vec<Json> = plan
        .review
        .rows
        .iter()
        .map(|(a, b)| json!([a, b]))
        .collect();
    assert_eq!(Json::Array(rows), want["rows"]);
    assert_eq!(plan.payload.as_ref(), Some(&want["payload"]));
    assert_eq!(json!(plan.hash), want["hash"]);
    let path = t.dir.path().join("change-draft.yaml");
    let saved = s.save_draft(&path.to_string_lossy(), Some(&plan)).unwrap();
    assert_eq!(
        json!(std::fs::read_to_string(saved).unwrap()),
        want["draft"]
    );
    let ann = LookupValue::new(7, "Ann Agent", Some("ann@example.com"));
    let ops = LookupValue::new(4, "Ops", None);
    let kv = [
        ("both", Value::lookup_answer(&ann)),
        ("display", Value::lookup_answer(&ops)),
        (
            "email",
            Value::lookup_answer(&LookupValue::new(Json::Null, "", Some("x@y.io"))),
        ),
        ("id", Value::lookup_answer(&LookupValue::id(12))),
        (
            "colon",
            Value::lookup_answer(&LookupValue::new(5, "Team: A", None)),
        ),
        (
            "list",
            json!([
                Value::lookup_answer(&ops),
                Value::lookup_answer(&ann),
                3,
                "a, b"
            ]),
        ),
    ];
    for (k, v) in kv {
        assert_eq!(
            json!(fsv_flows::drafts::answer_kv(k, &v)),
            want["kv"][k],
            "{k}"
        );
    }
}

/// `FormSession.save_draft` for a service request (`render_sr_template`
/// with paths relative to the file's folder), captured from Python.
#[test]
fn save_draft_writes_the_sr_template() {
    let (t, j) = (tenant(route), Arc::new(MemJournal::default()));
    let mut s = sr_session(&deps(&t, &j), "5", None, Some(jane()), false).unwrap();
    set(&mut s, "reason", "broken é");
    let plan = s.build_plan(None).unwrap();
    assert_eq!(plan.hash, SR_HASH);
    std::fs::create_dir(t.dir.path().join("drafts")).unwrap();
    let path = t.dir.path().join("drafts/sr-draft.yaml");
    let saved = s.save_draft(&path.to_string_lossy(), Some(&plan)).unwrap();
    let want = "# New service request: Laptop (catalog item 5)\n# Fill in values after the colons. Lines starting with # are ignored.\n# Fields appear with your answers, like the web form; after saving, new\n# required fields are added and the file reopens if needed.\n# Full item form: fsv sr fields 5\n# Save and close to continue; close without changes to abort.\n# This sends a real service request to the catalog item's fulfilment team.\n# Separate requested-for is available via --requested-for; requester remains unchanged.\n\n# REQUIRED · email or exact name of the requester (approvals route via their manager)\nrequester: jane@example.com\n# Optional · exact email or name of the person receiving this request\nrequested_for:\nanswers:\n  # optional · text\n  Reason: broken é\n\n# This item does not accept general attachments.\n";
    assert_eq!(std::fs::read_to_string(saved).unwrap(), want);
}

#[test]
fn pending_input_errors_block_the_plan_and_the_send() {
    let (t, j) = (tenant(route), Arc::new(MemJournal::default()));
    let mut s = new_change(&t, &j);
    s.mark_invalid("cf_note", "not a note");
    let plan = s.build_plan(None).unwrap();
    assert!(plan.hash.is_empty() && !plan.errors.is_empty());
    let n = t.calls().len();
    let (code, msg) = exit_of(&s.send(&plan, false).unwrap_err());
    assert_eq!(code, 2);
    assert!(msg.starts_with("plan has problems: "), "{msg}");
    assert!(t.calls()[n..].iter().all(|c| c.method == "GET"));
}

/// `FormSession.send(plan)` sends the reviewed plan object: an answer
/// changed after the review is not sent until the plan is rebuilt.
#[test]
fn send_uses_the_reviewed_plan() {
    let (t, j) = (tenant(route), Arc::new(MemJournal::default()));
    let mut s = new_change(&t, &j);
    let plan = s.build_plan(None).unwrap();
    assert_eq!(plan.hash, NEW_HASH);
    set(&mut s, "subject", "Changed later");
    let n = t.calls().len();
    let out = s.send(&plan, false).unwrap();
    let post = t
        .calls()
        .split_off(n)
        .into_iter()
        .find(|c| c.method == "POST")
        .unwrap();
    let payload = json!({"subject": "Deploy é", "custom_fields": {"cf_note": "hi"}, "email": "Jane Doe <jane@example.com>", "br_validation_excludes": BR});
    assert_eq!(
        post.body.as_deref(),
        Some(fsv_py::compact(&payload).as_str())
    );
    assert_eq!(out["plan_hash"], json!(NEW_HASH));

    let foreign = fsv_formui::Plan {
        detail: None,
        ..plan
    };
    let n = t.calls().len();
    let (code, msg) = exit_of(&s.send(&foreign, false).unwrap_err());
    assert_eq!(
        (code, msg.as_str()),
        (2, "the plan was not built by this form; review it again")
    );
    assert_eq!(t.calls().len(), n);
}

#[test]
fn live_form_backend_builds_directly() {
    let (t, j) = (tenant(route), Arc::new(MemJournal::default()));
    let d = deps(&t, &j);
    let meta = Arc::new(
        d.meta
            .load_meta("change", "new", false, &Json::Null)
            .unwrap(),
    );
    let form = LiveForm::new(d, Meta::Form(meta));
    let mut draft = fsv_formui::Draft::default();
    draft
        .answers
        .insert("subject".into(), Value::str("Deploy é"));
    draft.answers.insert("cf_note".into(), Value::str("hi"));
    draft.requester = Some("jane@example.com".into());
    let who = jane();
    let pending = vec!["Note: bad".to_string()];
    let req = PlanRequest {
        kind: FormKind::Change,
        mode: FormMode::New,
        draft: &draft,
        record: &json!({}),
        target: None,
        requester: Some(&who),
        ack_all: false,
        pending_errors: &[],
    };
    assert_eq!(form.build_plan(&req).unwrap().hash, NEW_HASH);
    let blocked = form
        .build_plan(&PlanRequest {
            pending_errors: &pending,
            ..req
        })
        .unwrap();
    assert_eq!(
        (blocked.hash.as_str(), blocked.errors.clone()),
        ("", pending.clone())
    );
    assert_eq!(blocked.review.errors, pending);
    let sr = PlanRequest {
        kind: FormKind::Sr,
        ..req
    };
    assert!(
        form.build_plan(&sr)
            .unwrap_err()
            .contains("catalog item metadata")
    );
}

#[test]
fn answers_become_draft_json() {
    let jane_json =
        json!({"__fsv_lookup__": {"id": 9, "display": "Jane Doe", "email": "jane@example.com"}});
    assert_eq!(answer_json(&Value::Lookup(jane())), jane_json);
    assert_eq!(
        answer_json(&Value::Lookup(LookupValue::new(Json::Null, "Jane", None))),
        json!({"__fsv_lookup__": {"id": null, "display": "Jane", "email": null}})
    );
    assert_eq!(
        answer_json(&Value::list([Value::Lookup(jane()), Value::Int(3)])),
        json!([jane_json, 3])
    );
    let back = Value::from_answer(&json!([jane_json, 3]));
    assert_eq!(back.as_list().unwrap()[0], Value::Lookup(jane()));
    assert_eq!(back.as_list().unwrap()[1], Value::Int(3));
    assert_eq!(
        answer_json(&Value::Date(jiff::civil::date(2026, 11, 12))),
        json!("2026-11-12")
    );
    assert_eq!(answer_json(&Value::Null), Json::Null);
}

#[test]
fn drafts_map_field_by_field() {
    let mut d = fsv_formui::Draft {
        requester: Some("j@x.io".into()),
        description_html: Some("<p>d</p>".into()),
        attachments: vec!["/a".into()],
        ack: vec!["r".into()],
        ..Default::default()
    };
    d.answers.insert(
        "group".into(),
        Value::Lookup(LookupValue::new(4, "Ops", None)),
    );
    d.planning.insert(
        "Rollout plan".into(),
        fsv_formui::PlanningFiles {
            files: vec!["/p".into()],
            description_html: None,
        },
    );
    let ops = Value::lookup_answer(&LookupValue::new(4, "Ops", None));
    let f = flows_draft(&d);
    assert_eq!(
        (
            f.answers.get("group"),
            f.requester.as_deref(),
            f.attachments.clone(),
            f.ack.clone()
        ),
        (
            Some(&ops),
            Some("j@x.io"),
            vec!["/a".to_string()],
            vec!["r".to_string()]
        )
    );
    assert_eq!(
        f.planning["Rollout plan"],
        PlanningSpec {
            files: vec!["/p".into()],
            description_html: None
        }
    );
    let s = sr_draft(&d);
    assert_eq!(
        (s.answers.get("group"), s.description_html.as_deref()),
        (Some(&ops), Some("<p>d</p>"))
    );
    assert_eq!(
        s.planning["Rollout plan"],
        json!({"files": ["/p"], "description_html": null})
    );
}

#[test]
fn config_requester_resolves_through_autocomplete() {
    let t = tenant(|c| {
        (c.path == "/search/autocomplete/requesters").then(|| ok(json!({"results": [{"id": 9, "value": "Jane Doe", "email": "jane@example.com"}]}))).flatten()
    });
    let j = Arc::new(MemJournal::default());
    let mut d = deps(&t, &j);
    d.config = crate::testkit::obj(json!({"defaults": {"requester": " jane@example.com "}}));
    let (who, note) = config_requester(&d, "requester");
    assert_eq!(
        (who, note.as_str()),
        (
            Some(jane()),
            "Requester jane@example.com from config defaults.requester"
        )
    );
    assert_eq!(
        t.calls()[0].query,
        pairs(&[
            ("q", "jane@example.com"),
            ("qf", "value"),
            ("all_users", "true")
        ])
    );
    assert_eq!(config_requester(&d, "sr_requester"), (None, String::new()));
}

#[test]
fn form_resolver_shares_the_client_resolver() {
    let t = tenant(route);
    let api: Arc<dyn Api + Send + Sync> = t.client.clone();
    let r = FormResolver(Arc::new(ClientResolver::new(api)));
    let f = Field::new("group", "Group", fsv_forms::Kind::Lookup);
    let got = fsv_formui::Resolver::lookup(&r, "group", "op", &f).unwrap();
    assert_eq!(got, [LookupValue::new(4, "Ops", None)]);
    assert_eq!(t.lines(), ["GET /api/_/bootstrap/agents_groups"]);
}

#[test]
fn finish_maps_the_form_outcome_like_run_form() {
    let (t, j) = (tenant(route), Arc::new(MemJournal::default()));
    let s = new_change(&t, &j);
    let saved = FormResult {
        action: Action::SaveDraft,
        plan: None,
        saved_path: Some("/tmp/d.yaml".into()),
        session: s,
    };
    assert_eq!(
        exit_of(&finish(saved).err().unwrap()),
        (
            0,
            "saved /tmp/d.yaml; continue later with --from /tmp/d.yaml (nothing was sent)".into()
        )
    );
    let s = new_change(&t, &j);
    let cancel = FormResult {
        action: Action::Cancel,
        plan: None,
        saved_path: None,
        session: s,
    };
    assert_eq!(
        exit_of(&finish(cancel).err().unwrap()),
        (1, "cancelled; nothing was sent".into())
    );
    let mut s = new_change(&t, &j);
    let plan = s.build_plan(None).unwrap();
    let no_plan = FormResult {
        action: Action::Submit,
        plan: None,
        saved_path: None,
        session: new_change(&t, &j),
    };
    assert_eq!(exit_of(&finish(no_plan).err().unwrap()).0, 1);
    let sent = finish(FormResult {
        action: Action::Submit,
        plan: Some(plan),
        saved_path: None,
        session: s,
    })
    .ok()
    .unwrap();
    assert_eq!(sent.plan.hash, NEW_HASH);
}

#[test]
fn partial_failures_keep_their_message_and_exit_code() {
    let p = change::PartialFailure::new(
        json!(78),
        vec![json!({"id": 501, "name": "a.txt"})],
        vec![],
        Error::other("boom"),
    );
    let (code, msg) = exit_of(&SendError::from(ExecError::Partial(Box::new(p))).into_error());
    assert_eq!(
        (code, msg.as_str()),
        (
            1,
            "change 78 was written, then a later step failed: boom. Uploaded attachment ids: 501"
        )
    );
    assert_eq!(
        exit_of(&SendError::from(ExecError::Failed(Error::exit(5, "x"))).into_error()),
        (5, "x".into())
    );
}

#[test]
fn live_deps_use_the_profile_and_working_directory() {
    let t = tenant(route);
    let d = FormDeps::live(t.client.clone()).unwrap();
    assert_eq!(d.paths.cwd, std::env::current_dir().unwrap());
    assert_eq!(d.attach_flow, None);
    assert_eq!(d.api.domain(), crate::testkit::HOST);
}

mod tui_forms {
    //! `N` / `e` through [`crate::LiveBackend`]: the session `_build_form`
    //! opens, driven as an embedded `FormScreen` on its own thread.

    use std::time::Duration;

    use fsv_tui::backend::{Buffer, FormExit, FormRequest, FormView, Rect};
    use fsv_tui::keys::parse;
    use fsv_tui::{App, Backend, BasicPages};

    use super::*;

    fn backend(t: &Tenant, j: &Arc<MemJournal>) -> crate::LiveBackend {
        let (t2, j2) = (t.client.clone(), j.clone());
        let dir = t.dir.path().to_path_buf();
        let factory: crate::tui::forms::DepsFactory = Arc::new(move || {
            let api: Arc<dyn Api + Send + Sync> = t2.clone();
            let store: Arc<dyn MetaStore + Send + Sync> = Arc::new(MemStore::default());
            Ok(FormDeps {
                form_api: t2.clone(),
                meta: Arc::new(LiveMeta::new(api.clone(), store.clone())),
                store,
                plan_journal: j2.clone(),
                journal: j2.clone(),
                writer: Arc::new(LiveWriter::new(api.clone())),
                resolver: Arc::new(ClientResolver::new(api.clone())),
                config: Map::new(),
                paths: Paths::new(Some(dir.clone()), &dir),
                attach_flow: Some("after-create".into()),
                api,
            })
        });
        crate::LiveBackend::new(t.client.clone(), Arc::new(|_| Ok(json!({"fields": []}))))
            .with_form_deps(factory)
    }

    fn text(view: &mut dyn FormView) -> String {
        let area = Rect::new(0, 0, 100, 30);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn press(view: &mut dyn FormView, keys: &[&str]) {
        for k in keys {
            view.handle_key(&parse(k));
        }
    }

    /// The edit of `edit_change_fetches_the_record_and_puts_the_diff`, typed
    /// into the form: same reviewed hash, same PUT, same send result.
    #[test]
    fn e_opens_the_change_edit_form_and_sends_the_reviewed_plan() {
        let (t, j) = (tenant(route), Arc::new(MemJournal::default()));
        let b = backend(&t, &j);
        assert!(b.forms_available());
        let req = FormRequest {
            mode: "edit".into(),
            item: Some(json!({"id": 77, "display_id": 77})),
            record: Some(json!({"id": 77, "display_id": 77, "subject": "Old"})),
        };
        let mut view = b.open_form(&req).unwrap();
        assert_eq!(
            t.lines()[..2],
            ["GET /api/_/changes/77", "GET /api/_/change_form_fields"]
        );
        assert!(text(view.as_mut()).contains("Edit change CHN-77"));
        press(view.as_mut(), &["backspace"; 3]);
        for c in "New subject".chars() {
            view.handle_key(&parse(&c.to_string()));
        }
        press(view.as_mut(), &["ctrl+s", "ctrl+s"]);
        assert!(view.done());
        let FormExit::Submit(send) = view.finish() else {
            panic!("not submitted");
        };
        let n = t.calls().len();
        let out = send.send().unwrap();
        let calls = t.calls().split_off(n);
        let put = calls.iter().find(|c| c.method == "PUT").unwrap();
        assert_eq!(put.path, "/api/_/changes/77");
        assert_eq!(put.body.as_deref(), Some(r#"{"subject":"New subject"}"#));
        assert_eq!(out["plan_hash"], json!(EDIT_HASH));
        assert_eq!(out["verified"], json!(false));
        assert_eq!(out["fields"]["subject"], json!("mismatched"));
        assert_eq!(
            fsv_tui::actions::form_result("edit", &out),
            ("Change".to_string(), "CHN-77".to_string())
        );
    }

    #[test]
    fn a_session_that_cannot_open_is_an_error() {
        let (t, j) = (tenant(route), Arc::new(MemJournal::default()));
        let b = backend(&t, &j);
        let req = FormRequest {
            mode: "sr".into(),
            item: Some(json!({"id": 1})),
            record: None,
        };
        assert_eq!(b.open_form(&req).err().unwrap().to_string(), "'display_id'");
    }

    /// `N` -> Change: the new-change form fills the screen; Escape on an
    /// untouched form cancels with nothing sent and no toast.
    #[test]
    fn shift_n_change_opens_the_new_change_form_in_the_tui() {
        let (t, j) = (tenant(route), Arc::new(MemJournal::default()));
        let mut a = App::new_inline(Arc::new(backend(&t, &j)), Arc::new(BasicPages), false);
        a.mount();
        a.settle();
        a.press("N");
        a.press("enter");
        a.settle();
        assert!(a.form_open());
        assert!(fsv_tui::view::screen_text(&a, 100, 30).contains("New Change"));
        let toasts = a.toasts.len();
        a.press("escape");
        a.advance(Duration::from_millis(10));
        assert!(!a.form_open());
        assert_eq!(a.toasts.len(), toasts);
        assert!(!t.lines().iter().any(|l| l.starts_with("POST")));
    }
}

/// `change_cli._report_partial(PartialFailure(77, [{"id": 5, "name":
/// "a.txt"}], [{"step": "attach"}], RuntimeError("boom")), "change")`: the
/// stderr lines and the `FlowExit` from running Python.
#[test]
fn report_partial_matches_python() {
    let e = change::PartialFailure {
        created_id: json!(77),
        uploaded: vec![json!({"id": 5, "name": "a.txt"})],
        steps: vec![json!({"step": "attach"})],
        error: Error::other("boom"),
    };
    assert_eq!(
        partial_lines(&e, "change"),
        [
            "[red]error[/red]: change 77 was written, but a later step failed: boom",
            "uploaded attachment ids: 5 (a.txt)",
            "  {'step': 'attach'}",
        ]
    );
    let exit = report_partial(&e, "change");
    assert_eq!(exit.exit_code(), 5);
    assert_eq!(exit.to_string(), "partial failure; change id 77");
}
