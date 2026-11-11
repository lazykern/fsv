//! Form sends whose metadata changed after the review, and a change written
//! before a later step failed. Expected text is from
//! `port/golden/gen_fsv_app_send.py` (Python `FormSession.send`,
//! `change_cli._send`, `cli_sr._send`, `_report_partial`,
//! `tui.actions.failure_detail`) over the same tenant.

use std::sync::Mutex;

use super::*;
use crate::testkit::{Call, MemJournal, Tenant, ok, status, tenant};
use fsv_api::formmeta::MemStore;
use fsv_core::client::Response;

const NEW_HASH: &str = "a28516d9594757efde10c3259d1f7e4499474bd3af6cb68b26f06ad66726ca24";
const PROMOTED_HASH: &str = "e998a65c2b2b4827bf66e8932050bf4eb3bc5e4ec3daf89d51c4c7a6a462a830";
const ATTACH_HASH: &str = "cee237dd4ee2092a8a533de7767f0786d6001014037eaafd9ac4cd0851834dd8";
const SR_HASH: &str = "51d1b1aaf494f9d0c061f3f62387b65e14b266a9d2b15cccff5e633d713c50b0";
const SR_NEW_HASH: &str = "e7e6c32b84d83a84db147ebe17280d47e87ded7ad13d982359a9695d5da90cb4";
const BR: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltdIn0=";
const SR_NEW_BR: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyIxMSI6eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltcInVyZ2VuY3lfMVwiXSJ9fQ==";

const CHANGE_HEAD: &str = "form metadata changed since the review; the plan is now different:\n  payload cf_note changed\n  payload custom_fields changed\n";

/// What the tenant serves after the review.
#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Review,
    /// `cf_note` becomes a default field: the payload moves it out of
    /// `custom_fields`.
    Promoted,
    /// A new required field.
    Required,
    /// The item gets an `urgency` field and a rule that hides it.
    Item,
}

fn field(id: i64, name: &str, label: &str, ft: &str, required: bool, default: bool) -> Json {
    json!({"id": id, "name": name, "label": label, "field_type": ft, "required": required, "position": id,
           "default_field": default, "choices": [], "nested_fields": [], "workspace_id": 2})
}

fn change_fields(p: Phase) -> Json {
    let mut f = vec![
        field(1, "subject", "Subject", "default_subject", true, true),
        field(2, "requester", "Requester", "default_requester", true, true),
        field(
            3,
            "cf_note",
            "Note",
            "custom_text",
            false,
            p == Phase::Promoted,
        ),
    ];
    if p == Phase::Required {
        f.push(field(4, "cf_reason", "Reason", "custom_text", true, false));
    }
    json!({ "change_fields": f })
}

fn item(p: Phase) -> Json {
    let mut cf = vec![
        json!({"id": 21, "name": "reason", "label": "Reason", "type": 1001, "required": false, "account_id": 1}),
    ];
    if p == Phase::Item {
        cf.push(json!({"id": 22, "name": "urgency", "label": "Urgency", "type": 1001, "required": false, "account_id": 1}));
    }
    json!({"id": 11, "display_id": 5, "name": "Laptop", "allow_attachments": false,
           "configs": {"attachment_mandatory": false}, "custom_fields": cf})
}

fn sr_rule() -> Json {
    json!({"id": 1, "name": "hide", "workspace_id": 2, "module_type": "service_request", "item_id": 11,
           "deleted": false, "user_type": "both", "form_type": "both", "state": "active",
           "rule_type": "client_side", "description": "",
           "condition_data": {"all": [{"name": "customField.reason", "operator": "is", "value": ["broken é"],
                                       "evaluate_on": "change"}]},
           "action_data": {"action": [{"name": ["customField.urgency"], "property": "hide"}]},
           "is_reversible": true, "is_validation": false})
}

struct Fixture {
    t: Tenant,
    j: Arc<MemJournal>,
    phase: Arc<Mutex<Phase>>,
}

impl Fixture {
    fn new() -> Fixture {
        let phase = Arc::new(Mutex::new(Phase::Review));
        let ph = phase.clone();
        let t = tenant(move |c: &Call| route(*ph.lock().unwrap(), c));
        Fixture {
            t,
            j: Arc::new(MemJournal::default()),
            phase,
        }
    }

    fn set(&self, p: Phase) {
        *self.phase.lock().unwrap() = p;
    }

    fn deps(&self) -> FormDeps {
        let api: Arc<dyn Api + Send + Sync> = self.t.client.clone();
        let store: Arc<dyn MetaStore + Send + Sync> = Arc::new(MemStore::default());
        FormDeps {
            form_api: self.t.client.clone(),
            meta: Arc::new(LiveMeta::new(api.clone(), store.clone())),
            store,
            plan_journal: self.j.clone(),
            journal: self.j.clone(),
            writer: Arc::new(LiveWriter::new(api.clone())),
            resolver: Arc::new(ClientResolver::new(api.clone())),
            config: Map::new(),
            paths: Paths::new(Some(self.t.dir.path().to_path_buf()), self.t.dir.path()),
            attach_flow: Some("after-create".into()),
            api,
        }
    }

    /// The reviewed new-change form (subject, note; optional attachment).
    fn change(&self, attach: Option<&str>) -> Submitted {
        let o = ChangeForm {
            requester: Some(jane()),
            ..ChangeForm::new(FormMode::New)
        };
        let mut s = change_session(&self.deps(), o).unwrap();
        s.set("subject", Value::str("Deploy é"));
        s.set("cf_note", Value::str("hi"));
        if let Some(a) = attach {
            s.draft.attachments = vec![a.to_string()];
        }
        let plan = s.build_plan(None).unwrap();
        Submitted { plan, session: s }
    }

    fn sr(&self) -> Submitted {
        let mut s = sr_session(&self.deps(), "5", None, Some(jane()), false).unwrap();
        s.set("reason", Value::str("broken é"));
        let plan = s.build_plan(None).unwrap();
        assert_eq!(plan.hash, SR_HASH);
        Submitted { plan, session: s }
    }

    fn writes(&self) -> Vec<(String, String, Option<String>)> {
        self.t
            .calls()
            .into_iter()
            .filter(|c| c.method != "GET")
            .map(|c| (c.method, c.path, c.body))
            .collect()
    }
}

fn route(p: Phase, c: &Call) -> Option<Response> {
    let path = c.path.strip_prefix("/api/_/").unwrap_or(&c.path);
    match (c.method.as_str(), path) {
        ("GET", "change_form_fields") => ok(change_fields(p)),
        ("GET", r) if r.ends_with("business_rules") => {
            let rules = if p == Phase::Item {
                vec![sr_rule()]
            } else {
                vec![]
            };
            ok(json!({"business_rules": rules, "user": {"time_zone": "Tokyo"}}))
        }
        ("GET", "catalog/service-items/5") => ok(json!({ "service_item": item(p) })),
        ("GET", "bootstrap/agents_groups") => ok(json!({"groups": [{"id": 4, "name": "Ops"}]})),
        ("GET", _) => ok(json!({})),
        ("POST", "changes") => ok(json!({"change": {"id": 78, "display_id": 78}})),
        ("POST", "attachments") => {
            let body = c.json().unwrap_or(Json::Null);
            ok(
                json!({"id": 9000005001_i64, "content_file_name": body["content_file_name"], "content_file_size": body["content_file_size"]}),
            )
        }
        ("PUT", "changes/78") => status(500, json!({"description": "boom"})),
        ("POST", "catalog/service-items/5/place-request") => {
            ok(json!({"service_request": {"id": 91, "display_id": 91}}))
        }
        _ => None,
    }
}

fn jane() -> LookupValue {
    LookupValue::new(9, "Jane Doe", Some("jane@example.com"))
}

fn exit_of(e: &Error) -> (i32, String) {
    match e {
        Error::Exit { code, message } => (*code, message.clone()),
        other => (other.exit_code(), other.to_string()),
    }
}

/// `send_cli` with stderr captured and every question answered `answer`.
fn cli(s: &Submitted, o: &SendOptions<'_>, answer: bool) -> (Result<Json>, String, Vec<String>) {
    let mut asked = Vec::new();
    let (r, text) = fsv_core::term::capture(|| {
        s.send_cli(o, &mut |q: &str| {
            asked.push(q.to_string());
            Ok(answer)
        })
    });
    (r, text, asked)
}

/// The review's metadata line carries the fetch time; Rich also wraps it.
fn without_meta_line(text: &str) -> String {
    text.lines()
        .filter(|l| !l.starts_with("form metadata as of ") && !l.starts_with("fingerprint "))
        .map(|l| format!("{}\n", l.trim_end()))
        .collect()
}

fn tty() -> SendOptions<'static> {
    SendOptions {
        tty: true,
        ..SendOptions::default()
    }
}

#[test]
fn session_send_refuses_a_changed_change_plan() {
    let f = Fixture::new();
    let s = f.change(None);
    assert_eq!(s.plan.hash, NEW_HASH);
    f.set(Phase::Promoted);
    let (code, msg) = exit_of(&s.session.send(&s.plan, false).unwrap_err());
    assert_eq!(
        (code, msg.as_str()),
        (
            4,
            "form metadata changed since the review: payload cf_note changed; payload custom_fields changed"
        )
    );
    assert!(f.writes().is_empty());
}

#[test]
fn send_checked_hands_back_the_changed_plan_and_send_confirmed_sends_it() {
    let f = Fixture::new();
    let s = f.change(None);
    f.set(Phase::Promoted);
    let Ok(SendOutcome::Changed(c)) = s.send_checked(false) else {
        panic!("not changed");
    };
    assert_eq!(
        c.diff,
        ["payload cf_note changed", "payload custom_fields changed"]
    );
    assert!(c.ok());
    assert_eq!(c.plan.hash, PROMOTED_HASH);
    assert_eq!(c.question(), "Send the updated plan?");
    assert_eq!(
        c.lines(),
        [
            "[yellow]form metadata changed since the review; the plan is now different:[/yellow]",
            "  payload cf_note changed",
            "  payload custom_fields changed"
        ]
    );
    assert!(f.writes().is_empty());
    let out = s.send_confirmed(&c, false).unwrap();
    assert_eq!(out["plan_hash"], json!(PROMOTED_HASH));
    let body = format!(
        r#"{{"subject":"Deploy é","cf_note":"hi","custom_fields":{{}},"email":"Jane Doe <jane@example.com>","br_validation_excludes":"{BR}"}}"#
    );
    assert_eq!(
        f.writes(),
        [("POST".into(), "/api/_/changes".into(), Some(body))]
    );
}

#[test]
fn cli_without_a_terminal_prints_the_diff_and_exits_4() {
    for o in [
        SendOptions::default(),
        SendOptions { yes: true, ..tty() },
        SendOptions {
            expect: Some("a28516d95947"),
            ..tty()
        },
    ] {
        let f = Fixture::new();
        let s = f.change(None);
        f.set(Phase::Promoted);
        let (r, text, asked) = cli(&s, &o, true);
        assert_eq!(
            exit_of(&r.unwrap_err()),
            (
                4,
                format!("plan changed to {PROMOTED_HASH}; review it again with --dry-run")
            )
        );
        assert_eq!(text, CHANGE_HEAD);
        assert!(asked.is_empty() && f.writes().is_empty());
    }
}

#[test]
fn cli_in_a_terminal_shows_the_new_review_and_asks_again() {
    let f = Fixture::new();
    let s = f.change(None);
    f.set(Phase::Promoted);
    let (r, text, asked) = cli(&s, &tty(), true);
    let out = r.unwrap();
    assert_eq!(asked, ["Send the updated plan?"]);
    let want = format!(
        "{CHANGE_HEAD}──────── Review: New Change: Deploy é ────────\nRequester  Jane Doe <jane@example.com>\nSubject    Deploy é\nNote       hi\nWill notify / create work for\n  Requester: Jane Doe <jane@example.com> (requester notifications)\nplan {PROMOTED_HASH}\n"
    );
    assert_eq!(without_meta_line(&text), want);
    assert_eq!(out["plan_hash"], json!(PROMOTED_HASH));
    assert_eq!(f.writes().len(), 1);
    assert_eq!(f.j.events().last().unwrap()["plan"], json!(PROMOTED_HASH));

    let f = Fixture::new();
    let s = f.change(None);
    f.set(Phase::Promoted);
    let (r, _, asked) = cli(&s, &tty(), false);
    assert_eq!(
        exit_of(&r.unwrap_err()),
        (1, "cancelled; nothing was sent".into())
    );
    assert_eq!(asked, ["Send the updated plan?"]);
    assert!(f.writes().is_empty());
}

#[test]
fn cli_refuses_an_updated_form_that_needs_more_answers() {
    let f = Fixture::new();
    let s = f.change(None);
    f.set(Phase::Required);
    let (r, text, asked) = cli(&s, &tty(), true);
    assert_eq!(
        exit_of(&r.unwrap_err()),
        (
            2,
            "the updated form needs more answers: Reason is required".into()
        )
    );
    let want = "form metadata changed since the review; the plan is now different:\n  problem: Reason is required\n──────── Review: New Change: Deploy é ────────\nRequester  Jane Doe <jane@example.com>\nSubject    Deploy é\nNote       hi\nWill notify / create work for\n  Requester: Jane Doe <jane@example.com> (requester notifications)\nProblems (must be fixed before sending)\n  Reason is required\n";
    assert_eq!(without_meta_line(&text), want);
    assert!(asked.is_empty() && f.writes().is_empty());
}

fn attachment(f: &Fixture) -> String {
    let p = f.t.dir.path().join("a.txt");
    std::fs::write(&p, "hello").unwrap();
    p.to_string_lossy().into_owned()
}

#[test]
fn cli_reports_a_partial_failure_like_report_partial() {
    let f = Fixture::new();
    let s = f.change(Some(&attachment(&f)));
    assert_eq!(s.plan.hash, ATTACH_HASH);
    let (r, text, _) = cli(&s, &SendOptions::default(), true);
    assert_eq!(
        exit_of(&r.unwrap_err()),
        (5, "partial failure; change id 78".into())
    );
    // Python's first line, `  uploading a.txt... ok (5 B)`, is written to
    // stderr directly by the upload and is not captured here.
    let want = "error: change 78 was written, but a later step failed: update failed: boom\nuploaded attachment ids: 9000005001 (a.txt)\n  {'step': 'create', 'ok': True, 'id': 78}\n  {'step': 'after', 'ok': False, 'error': 'update failed: boom'}\n";
    assert_eq!(text, want);
    let w: Vec<(String, String)> = f.writes().into_iter().map(|(m, p, _)| (m, p)).collect();
    assert_eq!(
        w,
        [
            ("POST".into(), "/api/_/changes".into()),
            ("POST".into(), "/api/_/attachments".into()),
            ("PUT".into(), "/api/_/changes/78".into())
        ]
    );
}

#[test]
fn a_tui_send_partial_failure_lists_the_outcome() {
    let f = Fixture::new();
    let s = f.change(Some(&attachment(&f)));
    let (r, _) = fsv_core::term::capture(|| crate::tui::forms::send_submitted(s));
    let failure = r.unwrap_err();
    assert_eq!(
        fsv_tui::actions::failure_detail(&failure),
        "change 78 was written, then a later step failed: update failed: boom. Uploaded attachment ids: 9000005001\ncreated_id: 78\nsteps: [{'step': 'create', 'ok': True, 'id': 78}, {'step': 'after', 'ok': False, 'error': 'update failed: boom'}]\nuploaded: [{'id': 9000005001, 'name': 'a.txt'}]\nerror: update failed: boom\nNo automatic retry. Check the exact record and journal before another send; outcome may be unknown."
    );
    assert_eq!(failure.error.exit_code(), 1);
}

#[test]
fn session_send_refuses_a_changed_service_request() {
    let f = Fixture::new();
    let s = f.sr();
    f.set(Phase::Item);
    let (code, msg) = exit_of(&s.session.send(&s.plan, false).unwrap_err());
    assert_eq!(
        (code, msg.as_str()),
        (
            4,
            "the item form changed since the review: payload br_validation_excludes changed"
        )
    );
    assert!(f.writes().is_empty());
}

const SR_HEAD: &str = "the catalog item or its rules changed since the review; the plan is now \ndifferent:\n  payload br_validation_excludes changed\n";

#[test]
fn sr_cli_prints_the_diff_and_exits_4_without_a_terminal() {
    let f = Fixture::new();
    let s = f.sr();
    f.set(Phase::Item);
    let (r, text, asked) = cli(&s, &SendOptions::default(), true);
    assert_eq!(
        exit_of(&r.unwrap_err()),
        (
            4,
            format!("plan changed to {SR_NEW_HASH}; review it again with --dry-run")
        )
    );
    assert_eq!(text, SR_HEAD);
    assert!(asked.is_empty() && f.writes().is_empty());
}

#[test]
fn sr_cli_in_a_terminal_places_the_confirmed_request() {
    let f = Fixture::new();
    let s = f.sr();
    f.set(Phase::Item);
    let (r, text, asked) = cli(&s, &tty(), true);
    let out = r.unwrap();
    assert_eq!(asked, ["Place the updated request?"]);
    let want = format!(
        "{}──────── Review: New service request: Laptop ────────\nCatalog item  Laptop (#5)\nRequester     Jane Doe <jane@example.com>\nReason        broken é\nWill notify / create work for\n  This sends a real service request to the catalog item's fulfilment team.\n  Requester: Jane Doe (approvals route via Jane Doe’s manager)\nWarnings\n  Separate requested-for is available via --requested-for; requester remains\nunchanged.\nplan {SR_NEW_HASH}\n",
        without_meta_line(SR_HEAD)
    );
    assert_eq!(without_meta_line(&text), want);
    assert_eq!(out["plan_hash"], json!(SR_NEW_HASH));
    let body = format!(
        r#"{{"email":"jane@example.com","custom_fields":{{"reason":"broken é"}},"br_validation_excludes":"{SR_NEW_BR}","attachments":[]}}"#
    );
    assert_eq!(
        f.writes(),
        [(
            "POST".into(),
            "/api/_/catalog/service-items/5/place-request".into(),
            Some(body)
        )]
    );

    let f = Fixture::new();
    let s = f.sr();
    f.set(Phase::Item);
    let (r, _, _) = cli(&s, &tty(), false);
    assert_eq!(
        exit_of(&r.unwrap_err()),
        (1, "cancelled; nothing was uploaded or sent".into())
    );
    assert!(f.writes().is_empty());
}

#[test]
fn unchanged_metadata_sends_through_the_cli_path() {
    let f = Fixture::new();
    let s = f.change(None);
    let (r, text, asked) = cli(
        &s,
        &SendOptions {
            expect: Some(&NEW_HASH[..12]),
            ..SendOptions::default()
        },
        true,
    );
    assert_eq!(r.unwrap()["plan_hash"], json!(NEW_HASH));
    assert!(text.is_empty() && asked.is_empty());
    let (r, _, _) = cli(&s, &SendOptions::default(), true);
    let (code, msg) = exit_of(&r.unwrap_err());
    assert_eq!(code, 4);
    assert!(
        msg.starts_with("this exact plan was already sent at "),
        "{msg}"
    );
}

#[test]
fn dry_run_and_foreign_plans_send_nothing() {
    let f = Fixture::new();
    let mut s = f.change(None);
    s.session.dry_run = true;
    assert_eq!(
        exit_of(&s.send_checked(false).err().unwrap().into_error()),
        (2, "dry run: nothing is sent".into())
    );
    s.session.dry_run = false;
    s.plan.detail = None;
    assert_eq!(
        exit_of(&cli(&s, &SendOptions::default(), true).0.unwrap_err()),
        (
            2,
            "the plan was not built by this form; review it again".into()
        )
    );
    assert!(f.writes().is_empty());
}
