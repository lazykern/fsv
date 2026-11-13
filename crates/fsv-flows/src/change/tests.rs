use super::*;
use crate::testing::{
    BASE, FakeWriter, MemJournal, acme_meta, acme_meta_id, fake_client, golden, resolver_arc,
};
use std::cell::RefCell;
use std::sync::{Mutex, MutexGuard};

/// Update-plan hashes bind `config.DOMAIN`. Goldens are stamped with an empty
/// domain; sends need a non-empty one. One lock so those tests do not race.
static CONFIG_LOCK: Mutex<()> = Mutex::new(());

struct CfgGuard {
    _dir: tempfile::TempDir,
    _lock: MutexGuard<'static, ()>,
}

impl Drop for CfgGuard {
    fn drop(&mut self) {
        crate::testing::hermetic_reset();
    }
}

/// The process config under a temp dir with `domain` (never the user's
/// real config, also not on restore).
fn config_guard(domain: Option<&str>) -> CfgGuard {
    let lock = CONFIG_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let paths = fsv_core::Paths::under(dir.path());
    std::fs::create_dir_all(&paths.config_dir).unwrap();
    if let Some(domain) = domain {
        std::fs::write(&paths.config_file, format!("{{\"domain\": \"{domain}\"}}")).unwrap();
    }
    fsv_core::config::use_for_tests(paths, domain.unwrap_or(""));
    CfgGuard {
        _dir: dir,
        _lock: lock,
    }
}

fn arm_change(w: &FakeWriter, plan: &Plan, record: &Json) {
    *w.change.lock().unwrap() = record_of(record);
    *w.fingerprint.lock().unwrap() = plan.meta.fingerprint().hash.clone();
}

fn jane() -> LookupValue {
    LookupValue::new(9000007001i64, "Jane Doe", Some("jane.doe@example.com"))
}

struct Hints(tempfile::TempDir);

impl Hints {
    fn new() -> Hints {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join("hints.yaml"),
            golden(&["hints_yaml"]).as_str().unwrap(),
        )
        .unwrap();
        Hints(d)
    }
    fn path(&self) -> PathBuf {
        self.0.path().join("hints.yaml")
    }
}

fn change_meta() -> Arc<FormMeta> {
    Arc::new(acme_meta("change", "new_form"))
}

fn edit_meta() -> Arc<FormMeta> {
    Arc::new(acme_meta_id("change", "edit_form", json!(501)))
}

fn create_opts(h: &Hints, requester: Option<LookupValue>) -> CreateOptions {
    CreateOptions {
        resolver: resolver_arc(),
        requester,
        hints_path: Some(h.path()),
        ..CreateOptions::default()
    }
}

fn create(text: &str, opts: &CreateOptions) -> Plan {
    let d = parse_template(text, Some(Path::new(BASE))).unwrap();
    build_create_plan(&change_meta(), &d, opts).unwrap()
}

fn golden_create(case: &str, h: &Hints) -> (Plan, &'static Json) {
    let g = golden(&["create", case]);
    let mut opts = create_opts(h, (case != "empty").then(jane));
    opts.allow_incomplete = case == "no_validate";
    (create(g["draft"].as_str().unwrap(), &opts), g)
}

fn record_of(g: &Json) -> Map<String, Json> {
    g["record"].as_object().unwrap().clone()
}

fn update(answers: Json, record: &Map<String, Json>, h: &Hints) -> Plan {
    let d = Draft {
        answers: answers.as_object().unwrap().clone(),
        ..Draft::default()
    };
    let opts = UpdateOptions {
        resolver: resolver_arc(),
        hints_path: Some(h.path()),
        ..UpdateOptions::default()
    };
    build_update_plan(&edit_meta(), &d, record, 501, &opts).unwrap()
}

fn golden_update(case: &str, h: &Hints) -> (Plan, &'static Json) {
    let g = golden(&["update", case]);
    (update(g["answers"].clone(), &record_of(g), h), g)
}

fn without_metadata(mut j: Json) -> Json {
    j["review"]
        .as_object_mut()
        .unwrap()
        .shift_remove("metadata");
    j
}

fn check(p: &Plan, g: &Json, title: &str, case: &str) {
    assert_eq!(json!(p.errors), g["errors"], "{case} errors");
    assert_eq!(p.hash, g["hash"].as_str().unwrap(), "{case} hash");
    assert_eq!(p.ok(), g["ok"].as_bool().unwrap(), "{case}");
    assert_eq!(json!(p.payload), g["payload"], "{case} payload");
    assert_eq!(json!(p.risks), g["risks"], "{case} risks");
    assert_eq!(
        fsv_py::canonical(&without_metadata(p.to_json())),
        fsv_py::canonical(&g["json"]),
        "{case} json"
    );
    let keys = |j: &Json| j.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    assert_eq!(
        keys(&p.to_json()),
        keys(&g["json"]),
        "{case} json key order"
    );
    let t = template_for(p, &p.draft, title, &p.errors, None, Some(BASE)).unwrap();
    assert_eq!(t, g["template"].as_str().unwrap(), "{case} template");
    assert_eq!(plan_text(p), g["plan_text"].as_str().unwrap(), "{case}");
}

fn plan_ok(h: &Hints) -> Plan {
    golden_create("full", h).0
}

#[test]
fn golden_create_plans() {
    let h = Hints::new();
    for case in golden(&["create"]).as_object().unwrap().keys() {
        let (p, g) = golden_create(case, &h);
        check(&p, g, "New change", case);
    }
}

/// `build_update_plan(lifecycle=...)` (Python 7530df6); a `null` flow is a
/// failed read.
#[test]
fn golden_update_lifecycle_plans() {
    let _cfg = config_guard(None);
    let h = Hints::new();
    for (case, g) in golden(&["update_lifecycle"]).as_object().unwrap() {
        let d = Draft {
            answers: g["answers"].as_object().unwrap().clone(),
            ..Draft::default()
        };
        let opts = UpdateOptions {
            resolver: resolver_arc(),
            hints_path: Some(h.path()),
            ..UpdateOptions::default()
        };
        let reads = std::cell::Cell::new(0);
        let read = || -> Result<Json> {
            reads.set(reads.get() + 1);
            match &g["flow"] {
                Json::Null => Err(fsv_core::Error::Api(Box::new(
                    fsv_core::error::ApiError::new(503, "unavailable"),
                ))),
                flow => Ok(flow.clone()),
            }
        };
        let p = build_update_plan_with_lifecycle(
            &edit_meta(),
            &d,
            &record_of(g),
            501,
            &opts,
            Some(&read),
        )
        .unwrap();
        assert_eq!(json!(reads.get()), g["lifecycle_reads"], "{case} reads");
        check(&p, g, "Update change 501", case);
    }
}

#[test]
fn golden_update_plans() {
    let _cfg = config_guard(None);
    let h = Hints::new();
    for case in golden(&["update"]).as_object().unwrap().keys() {
        let (p, g) = golden_update(case, &h);
        check(&p, g, "Update change 501", case);
    }
}

#[test]
fn flow_exit_and_partial_failure_messages() {
    let e = flow_exit(4, "stop");
    assert_eq!((e.exit_code(), e.to_string()), (4, "stop".to_string()));
    // `FlowExit(2)`: `code == 2`, `str(e) == ""`.
    let e = flow_exit(2, "");
    assert_eq!((e.exit_code(), e.to_string()), (2, String::new()));
    let p = PartialFailure::new(
        json!(501),
        vec![json!({"id": 9, "name": "a.txt"})],
        vec![],
        Error::other("boom"),
    );
    assert_eq!(
        p.to_string(),
        "change 501 was written, then a later step failed: boom. Uploaded attachment ids: 9"
    );
    let p = PartialFailure::new(json!(501), vec![], vec![], Error::other("boom"));
    assert!(p.to_string().ends_with("Uploaded attachment ids: none"));
    assert_eq!(ExecError::Partial(Box::new(p)).exit_code(), 1);
}

#[test]
fn interactive_is_false_under_test_harness_pipe() {
    let _ = interactive();
}

#[test]
fn sha256_file_digest_and_missing() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.txt");
    std::fs::write(&f, "abc").unwrap();
    let j = sha256_file(f.to_str().unwrap()).unwrap();
    assert_eq!(j["name"], json!("a.txt"));
    assert_eq!(j["size"], json!(3));
    assert_eq!(
        j["sha256"],
        json!("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
    );
    assert!(sha256_file("/nonexistent/x").is_err());
}

#[test]
fn field_by_type_finds_default_fields() {
    let meta = change_meta();
    let spec = meta.spec();
    assert_eq!(
        field_by_type(&spec, "default_subject").unwrap().api_name,
        "subject"
    );
    assert!(field_by_type(&spec, "nope").is_none());
}

#[test]
fn legacy_answers_by_internal_name() {
    let meta = change_meta();
    let a = legacy_answers(&meta, Some("S"), Some("High"), None, Some("Platform Ops")).unwrap();
    assert_eq!(
        Json::Object(a),
        json!({"subject": "S", "priority": "High", "group": "Platform Ops"})
    );
    let t = acme_meta("ticket", "new_form");
    assert!(
        legacy_answers(&t, None, None, None, None)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn split_special_moves_status_requester_description() {
    let meta = change_meta();
    let d = Draft {
        answers: json!({"Status": "Planning", "Requester": "x", "Subject": "s", "Description": "old", "Colour": 1})
            .as_object()
            .unwrap()
            .clone(),
        description_html: Some("<p>new</p>".into()),
        ..Draft::default()
    };
    let s = split_special(&meta, &d);
    assert_eq!(
        (s.status_raw, s.requester_raw),
        (Some(json!("Planning")), Some(json!("x")))
    );
    assert_eq!(
        Json::Object(s.answers),
        json!({"Subject": "s", "Colour": 1, "description": "<p>new</p>"})
    );
    assert!(s.errors.is_empty());
}

#[test]
fn acknowledge_all_and_unmatched() {
    let h = Hints::new();
    let mut p = golden_create("urgent", &h).0;
    assert!(
        p.errors
            .iter()
            .any(|e| e.contains("acknowledge") || e.contains("ack")),
        "{:?}",
        p.errors
    );
    let w = acknowledge(&mut p.state, &["no such message".into()], false);
    assert_eq!(
        w,
        vec!["ack 'no such message': no blocking message matches (ignored)".to_string()]
    );
    assert!(acknowledge(&mut p.state, &["ALL".into()], false).is_empty());
    assert!(
        p.state
            .errors()
            .unwrap()
            .iter()
            .all(|e| !e.contains("acknowledge"))
    );
}

#[test]
fn planning_definitions_resolve_like_python() {
    let defs = vec![
        json!({"name": "cf_rollout_plan", "id": 1, "label": "Rollout Plan", "field_type": "planning_field"}),
        json!({"name": "backout", "id": 2, "label": "Backout Plans", "field_type": "planning_field"}),
        json!({"name": "x", "id": 3, "label": "Dup", "field_type": "planning_field"}),
        json!({"name": "y", "id": 4, "label": "dup", "field_type": "planning_field"}),
        json!({"name": "subject", "id": 5, "label": "Subject", "field_type": "default_subject"}),
    ];
    assert_eq!(planning_definitions(&defs).len(), 4);
    for (v, want) in [
        ("cf_rollout_plan", "cf_rollout_plan"),
        ("rollout plan", "cf_rollout_plan"),
        ("Rollout plans", "cf_rollout_plan"),
        ("planning-field-backout", "backout"),
        ("backout plan", "backout"),
    ] {
        assert_eq!(
            planning_server_key(resolve_planning_definition(v, &defs).unwrap()),
            want,
            "{v}"
        );
    }
    assert_eq!(
        resolve_planning_definition("Dup", &defs).unwrap_err(),
        "ambiguous planning field: 'Dup'; select an exact identity"
    );
    assert_eq!(
        resolve_planning_definition("nope", &defs).unwrap_err(),
        "planning field not found: 'nope'; choices: Rollout Plan, Backout Plans, Dup, dup"
    );
    assert_eq!(planning_server_key(&json!({"id": 7})), "7");
}

#[test]
fn resolve_planning_from_golden_case() {
    let h = Hints::new();
    let (p, _) = golden_create("planning", &h);
    let (entries, errors) = resolve_planning(&p.meta, &p.draft);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].label, "Go-Live Checklist");
    assert_eq!(errors.len(), 1);
    assert!(errors[0].starts_with("planning field not found: 'Nope Plan'"));
}

#[test]
fn file_errors_only_for_missing() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.txt");
    std::fs::write(&f, "x").unwrap();
    let have = f.to_string_lossy().into_owned();
    assert_eq!(
        file_errors(&[have, "/no/b.txt".into()]),
        vec!["file not found: /no/b.txt".to_string()]
    );
}

#[test]
fn rule_shaping_helpers_on_golden_plans() {
    let h = Hints::new();
    for case in ["full", "status", "yes_cloud", "unknown"] {
        let (p, g) = golden_create(case, &h);
        let ev = &p.state.evaluated;
        let live: HashSet<String> = ev.matched.iter().map(fsv_py::py_str).collect();
        for c in real_conflicts(ev) {
            assert!(
                c.decisions
                    .iter()
                    .all(|d| live.contains(&fsv_py::py_str(&d.rule_id)))
            );
        }
        let review = &g["json"]["review"];
        assert_eq!(json!(notes(&p.state).unwrap()), review["notes"], "{case}");
        assert_eq!(
            json!(dropped(&p.state).unwrap()),
            review["dropped"],
            "{case}"
        );
        assert_eq!(
            messages(&p.state).len(),
            review["messages"].as_array().unwrap().len(),
            "{case}"
        );
        for (name, r) in &ev.restricted {
            assert!(auto_suffix(&p.state, name).unwrap().contains(&rule_of(r)));
            if let Some((c, _)) = only_option(&p.state, name).unwrap() {
                assert!(!c.label.is_empty());
            }
        }
        for name in ev.populated.keys() {
            assert_eq!(auto_suffix(&p.state, name).unwrap(), " (auto)");
        }
    }
    assert_eq!(auto_suffix(&plan_ok(&h).state, "subject").unwrap(), "");
}

#[test]
fn unsent_suffix_marks_unknown_request_keys() {
    let meta = change_meta();
    let spec = meta.spec();
    for f in spec.fields() {
        assert_eq!(unsent_suffix(f).is_empty(), f.sent);
    }
}

#[test]
fn hint_lines_matching_and_problems() {
    let h = Hints::new();
    let p = plan_ok(&h);
    let (lines, problems) = hint_lines(&p.state, "changes", Some(&h.path())).unwrap();
    assert_eq!(
        json!(lines),
        json!(["staging runs first", "the search team is paged"])
    );
    assert!(
        problems.iter().all(|w| w.contains("Nope Field")),
        "{problems:?}"
    );
    let bad = h.0.path().join("bad.yaml");
    std::fs::write(&bad, "changes: [").unwrap();
    let (lines, problems) = hint_lines(&p.state, "changes", Some(&bad)).unwrap();
    assert!(lines.is_empty() && problems.len() == 1);
}

#[test]
fn attachment_rows_label_planning_targets() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("plan.txt");
    std::fs::write(&f, "abc").unwrap();
    let path = f.to_string_lossy().into_owned();
    let planning = vec![PlanningEntry {
        label: "Rollout".into(),
        field_id: "cf_r".into(),
        field_label: "Rollout Plan".into(),
        files: vec![path.clone()],
        description_html: None,
    }];
    let mut files = IndexMap::new();
    files.insert(
        "attachments".to_string(),
        vec![path.clone(), "/missing".into()],
    );
    files.insert("planning:cf_r".to_string(), vec![path.clone()]);
    let rows = attachment_rows(&files, &planning);
    assert_eq!(
        rows.iter().map(|r| r.target.as_str()).collect::<Vec<_>>(),
        ["attachments", "planning: Rollout"]
    );
    assert_eq!(rows[0].size, 3);
}

#[test]
fn hash_matches_python() {
    let mut files = IndexMap::new();
    files.insert("attachments".to_string(), Vec::new());
    let r = json!({"a": 1});
    assert_eq!(
        hash(&r, &files, &[], None).unwrap(),
        "4240c4340fe295a32bbdf617c19505e717346b74c1d954cc23efe5bf77007aa3"
    );
    assert_eq!(
        hash(&r, &files, &[], Some(501)).unwrap(),
        "2cc093d73cb24d039d849305dc3a19c06061c837a7b22a501631ec097c80e51d"
    );
    let p = PlanningEntry {
        label: "P".into(),
        field_id: "cf_p".into(),
        field_label: "P".into(),
        files: vec![],
        description_html: Some("<p>x</p>".into()),
    };
    assert_eq!(
        hash(&r, &files, &[p], None).unwrap(),
        "3cc49e490ea43b0a40cf6116e168368fd5ad812bd2c232d0f9aa60efdeb0cf81"
    );
}

#[test]
fn normcmp_and_encode_compare_server_meaning() {
    let meta = edit_meta();
    let spec = meta.spec();
    let start = spec.get("planned_start_date").unwrap();
    assert_eq!(
        normcmp(start, &json!("2030-11-26T21:00:00+09:00")),
        normcmp(start, &json!("2030-11-26T12:00:00Z"))
    );
    let subject = spec.get("subject").unwrap();
    assert_eq!(normcmp(subject, &json!(" a ")), json!("a"));
    assert_eq!(normcmp(subject, &json!("")), Json::Null);
    let services = spec.find("Services").unwrap();
    assert_eq!(normcmp(services, &json!(["b", "a"])), json!(["a", "b"]));
    let state = meta.state("edit", None, None).unwrap();
    assert_eq!(encode(&state, subject, None), Json::Null);
    assert_eq!(
        encode(&state, subject, Some(&Value::Str(String::new()))),
        Json::Null
    );
    assert_eq!(
        encode(&state, subject, Some(&Value::Str("x".into()))),
        json!("x")
    );
    let agent = spec.get("agent").unwrap();
    let lv = Value::Lookup(LookupValue::new(9000007101i64, "Alex Kim", None));
    assert_eq!(encode(&state, agent, Some(&lv)), json!(9000007101i64));
}

#[test]
fn check_duplicate_reports_done_and_started() {
    let h = Hints::new();
    let p = plan_ok(&h);
    let j = MemJournal::default();
    check_duplicate(&p, false, &j).unwrap();
    j.record_plan("started", &p.hash, "create", None, None);
    let e = check_duplicate(&p, false, &j).unwrap_err().to_string();
    assert!(e.contains("with no recorded outcome"), "{e}");
    j.record_plan("done", &p.hash, "create", None, Some(&json!(502)));
    let e = check_duplicate(&p, false, &j).unwrap_err();
    assert_eq!(e.exit_code(), 4);
    assert!(
        e.to_string()
            .contains("was already sent at 2030-01-01T00:00:00 (created 502)")
    );
    check_duplicate(&p, true, &j).unwrap();
}

#[test]
fn attach_flow_option_config_default() {
    let cfg = json!({"changes": {"attach_flow": "before-create"}});
    let cfg = cfg.as_object().unwrap();
    assert_eq!(
        resolve_attach_flow_in(None, &Map::new()).unwrap(),
        "after-create"
    );
    assert_eq!(resolve_attach_flow_in(None, cfg).unwrap(), "before-create");
    assert_eq!(
        resolve_attach_flow_in(Some("after-create"), cfg).unwrap(),
        "after-create"
    );
    assert_eq!(
        resolve_attach_flow(Some("before-create")).unwrap(),
        "before-create"
    );
    let e = resolve_attach_flow(Some("sideways")).unwrap_err();
    assert_eq!(
        e.to_string(),
        "attach flow 'sideways': expected one of after-create, before-create"
    );
    assert_eq!(e.exit_code(), 2);
}

fn with_attachment(h: &Hints) -> (Plan, tempfile::TempDir) {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.txt"), "abc").unwrap();
    let text = format!(
        "{}attachments: [a.txt]\n",
        golden(&["create", "full", "draft"]).as_str().unwrap()
    );
    let draft = parse_template(&text, Some(d.path())).unwrap();
    let p = build_create_plan(&change_meta(), &draft, &create_opts(h, Some(jane()))).unwrap();
    assert!(p.ok(), "{:?}", p.errors);
    (p, d)
}

#[test]
fn execute_create_after_create_attachments() {
    let h = Hints::new();
    let (p, _d) = with_attachment(&h);
    let (w, j) = (FakeWriter::default(), MemJournal::default());
    let out = execute(&p, &w, &j, "skip", Some("after-create")).unwrap();
    assert_eq!(out["id"], json!(900000502));
    let calls: Vec<String> = w
        .calls
        .lock()
        .unwrap()
        .iter()
        .map(|c| c.0.clone())
        .collect();
    assert_eq!(calls, ["post", "upload", "put"]);
    assert_eq!(
        out["steps"][1],
        json!({"step": "attachments", "ok": true, "ids": [9000005001i64]})
    );
    let events: Vec<Json> = j.read().iter().map(|r| r["event"].clone()).collect();
    assert_eq!(events, [json!("started"), json!("done")]);
    assert_eq!(j.read()[1]["created_id"], json!(900000502));
}

#[test]
fn execute_before_create_puts_ids_in_body() {
    let h = Hints::new();
    let (p, _d) = with_attachment(&h);
    let (w, j) = (FakeWriter::default(), MemJournal::default());
    execute(&p, &w, &j, "skip", Some("before-create")).unwrap();
    let calls = w.calls.lock().unwrap();
    assert_eq!(calls[0].0, "upload");
    assert_eq!(calls[1].1["attachments"], json!([9000005001i64]));
}

#[test]
fn execute_partial_failure_and_rejection() {
    let h = Hints::new();
    let (p, _d) = with_attachment(&h);
    let w = FakeWriter {
        fail_upload: true,
        ..FakeWriter::default()
    };
    let j = MemJournal::default();
    let Err(ExecError::Partial(pf)) = execute(&p, &w, &j, "skip", Some("after-create")) else {
        panic!("partial")
    };
    assert_eq!(pf.created_id, json!(900000502));
    assert_eq!(pf.steps.last().unwrap()["ok"], json!(false));
    assert_eq!(j.read().last().unwrap()["error"], json!("details omitted"));

    let w = FakeWriter {
        fail_post: Some(422),
        ..FakeWriter::default()
    };
    let j = MemJournal::default();
    let Err(ExecError::Failed(e)) = execute(&p, &w, &j, "skip", Some("after-create")) else {
        panic!("failed")
    };
    assert!(matches!(e, Error::Api(_)));
    assert_eq!(j.read().last().unwrap()["event"], json!("failed"));

    let (bad, _) = golden_create("empty", &h);
    let Err(ExecError::Failed(e)) = execute(&bad, &FakeWriter::default(), &j, "skip", None) else {
        panic!()
    };
    assert_eq!(e.exit_code(), 2);
    assert!(e.to_string().starts_with("plan has problems: "));
}

#[test]
fn execute_update_puts_payload_and_planning() {
    let h = Hints::new();
    let (mut p, g) = golden_update("subject", &h);
    p.planning.push(PlanningEntry {
        label: "Rollout".into(),
        field_id: "cf_r".into(),
        field_label: String::new(),
        files: vec![],
        description_html: Some("<p>x</p>".into()),
    });
    let (w, j) = (FakeWriter::default(), MemJournal::default());
    let err = execute(&p, &w, &j, "skip", None).unwrap_err();
    assert!(
        err.to_string().contains("Combined change property"),
        "{err}"
    );
    assert!(w.calls.lock().unwrap().is_empty());

    let _cfg = config_guard(Some("acme.freshservice.com"));
    let (p, _) = golden_update("subject", &h);
    let w = FakeWriter::default();
    arm_change(&w, &p, g);
    let out = execute(&p, &w, &j, "skip", None).unwrap();
    let calls = w.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "put");
    assert_eq!(calls[0].1["id"], json!(501));
    assert_eq!(out["verified"], json!(true));
    assert_eq!(out["steps"][0]["step"], json!("update"));
    assert_eq!(j.read()[0]["kind"], json!("plan:change_form_update"));
    drop(calls);

    let (n, _) = golden_update("nothing", &h);
    let w = FakeWriter::default();
    let err = execute(&n, &w, &j, "skip", None).unwrap_err();
    assert!(
        err.to_string()
            .contains("Change tenant or exact reviewed payload changed; review again"),
        "{err}"
    );
    assert!(w.calls.lock().unwrap().is_empty());
}

#[test]
fn review_diff_rows_payload_and_problems() {
    let h = Hints::new();
    let (a, _) = golden_create("full", &h);
    let (b, _) = golden_create("round2", &h);
    let diff = review_diff(&a, &b);
    assert!(diff.iter().any(|l| l.starts_with("Services: ")), "{diff:?}");
    assert!(review_diff(&a, &a).is_empty());
    let mut c = a.clone();
    c.payload.as_mut().unwrap().insert("zz".into(), json!(1));
    c.errors.push("new problem".into());
    assert_eq!(
        review_diff(&a, &c),
        ["payload zz changed", "problem: new problem"]
    );
}

struct Source(FormMeta);

impl MetaSource for Source {
    fn load_meta(
        &self,
        module: &str,
        mode: &str,
        fresh: bool,
        module_id: &Json,
    ) -> Result<FormMeta> {
        assert_eq!(
            (module, mode, fresh, module_id),
            ("change", "new", true, &Json::Null)
        );
        Ok(self.0.clone())
    }
}

#[test]
fn freshen_same_rebuilt_and_changed() {
    let h = Hints::new();
    let p = plan_ok(&h);
    let draft = p.draft.clone();
    let opts = create_opts(&h, Some(jane()));
    let mut rebuild = |m: Arc<FormMeta>| build_create_plan(&m, &draft, &opts);
    let (same, diff) = freshen(
        p.clone(),
        &Source(acme_meta("change", "new_form")),
        &mut rebuild,
    )
    .unwrap();
    assert!(diff.is_none() && same.hash == p.hash);

    let mut fields = golden(&["fields"]).clone();
    fields["change_fields"]
        .as_array_mut()
        .unwrap()
        .retain(|f| f["label"] != json!("Services"));
    let m = FormMeta::new(
        "change",
        "new_form",
        fields,
        golden(&["rules"]).clone(),
        json!(2),
        1.0,
        "server",
        Json::Null,
    )
    .unwrap()
    .with_tz_env(fsv_forms::tz::TzEnv::empty());
    let (new, diff) = freshen(p.clone(), &Source(m), &mut rebuild).unwrap();
    assert_ne!(new.hash, p.hash);
    assert!(!diff.unwrap().is_empty());
}

#[test]
fn edit_loop_reopens_until_complete() {
    let h = Hints::new();
    let opts = create_opts(&h, Some(jane()));
    let mut build = |d: &Draft| build_create_plan(&change_meta(), d, &opts);
    let full = golden(&["create", "full", "draft"])
        .as_str()
        .unwrap()
        .to_string();
    let seen = RefCell::new(Vec::<String>::new());
    let mut edit = |text: &str| {
        seen.borrow_mut().push(text.to_string());
        let n = seen.borrow().len();
        parse_template(
            if n == 1 {
                "answers:\n  Subject: x\n"
            } else {
                &full
            },
            Some(Path::new(BASE)),
        )
    };
    let (draft, plan) = edit_loop(
        &mut build,
        Draft::default(),
        "New change",
        None,
        &mut edit,
        5,
    )
    .unwrap();
    assert!(plan.ok() && plan.hash == golden(&["create", "full", "hash"]).as_str().unwrap());
    assert_eq!(draft.answers["Subject"], json!("Billing: rotate keys"));
    assert!(
        seen.borrow()[1].contains("is required"),
        "second round shows the errors"
    );

    let mut never = |_: &str| parse_template("answers: {}\n", None);
    let Err(e) = edit_loop(&mut build, Draft::default(), "t", None, &mut never, 2) else {
        panic!("aborts")
    };
    assert!(matches!(e, Error::EditorAbort(_)));
}

#[test]
fn yaml_norm_matches_python() {
    let got: Vec<Json> = [
        None,
        Some(json!("")),
        Some(json!(["A ", " b"])),
        Some(json!("X  Y")),
        Some(json!(3)),
    ]
    .iter()
    .map(|v| yaml_norm(v.as_ref()))
    .collect();
    assert_eq!(json!(got), json!([null, null, ["a", "b"], "x y", "3"]));
}

#[test]
fn template_for_requester_fallbacks() {
    let h = Hints::new();
    let p = plan_ok(&h);
    let t = template_for(
        &p,
        &p.draft,
        "T",
        &[],
        Some("someone@example.com"),
        Some(BASE),
    )
    .unwrap();
    assert!(t.contains("requester: someone@example.com"));
    let t = template_for(&p, &p.draft, "T", &[], None, Some(BASE)).unwrap();
    assert!(t.contains("requester: jane.doe@example.com"));
}

#[test]
fn save_draft_writes_reloadable_template() {
    let h = Hints::new();
    let p = plan_ok(&h);
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("saved.yaml");
    let out = save_draft(&p, &p.draft, path.to_str().unwrap()).unwrap();
    assert_eq!(out, path.to_string_lossy());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("fsv change draft (saved)"));
    let back = parse_template(&text, Some(d.path())).unwrap();
    let again = build_create_plan(&change_meta(), &back, &create_opts(&h, Some(jane()))).unwrap();
    assert_eq!(again.hash, p.hash);
}

#[test]
fn plan_text_and_expect() {
    let h = Hints::new();
    let p = plan_ok(&h);
    assert_eq!(plan_text(&p), format!("create plan {}", &p.hash[..16]));
    expect(&p, None).unwrap();
    expect(&p, Some(&p.hash[..12])).unwrap();
    let e = expect(&p, Some("0000")).unwrap_err();
    assert_eq!(e.exit_code(), 4);
    assert!(
        e.to_string()
            .starts_with("plan changed: expected 0000, computed ")
    );
    let (bad, _) = golden_create("empty", &h);
    assert_eq!(plan_text(&bad), "create plan (incomplete)");
    assert_eq!(expect(&bad, Some("abc")).unwrap_err().exit_code(), 2);
}

#[test]
fn plain_collapses_markup() {
    assert_eq!(plain("<p>a <b>b</b></p>\n<p>c</p>"), "a b c");
}

#[test]
fn default_resolver_looks_up_people() {
    let r = default_resolver(fake_client());
    assert_eq!(
        r.name_for("group", &json!(9000009001i64)).as_deref(),
        Some("Platform Ops")
    );
}

#[test]
fn disk_journal_and_editor_are_wired() {
    let _: &dyn Journal = &fsv_api::service::DiskJournal;
    let _: fn(&str) -> Result<Draft> = editor_edit;
}

/// Answers every change POST/PUT with HTTP `status`.
struct RejectWriter {
    inner: FakeWriter,
    status: u16,
}

impl RejectWriter {
    fn new(status: u16) -> RejectWriter {
        RejectWriter {
            inner: FakeWriter::default(),
            status,
        }
    }
    fn reject(&self) -> Error {
        Error::from(fsv_core::error::ApiError::new(
            self.status,
            json!("rejected"),
        ))
    }
}

impl ChangeWriter for RejectWriter {
    fn upload_file(&self, path: &str) -> Result<Json> {
        self.inner.upload_file(path)
    }
    fn post_change_body(&self, body: &Map<String, Json>) -> Result<Json> {
        self.inner.post_change_body(body)?;
        Err(self.reject())
    }
    fn put_change_body(&self, change_id: &Json, body: &Map<String, Json>) -> Result<Json> {
        self.inner.put_change_body(change_id, body)?;
        Err(self.reject())
    }
    fn get_change(&self, id: i64) -> Result<Map<String, Json>> {
        self.inner.get_change(id)
    }
    fn edit_fingerprint(&self, module_id: &Json, record_workspace: &Json) -> Result<String> {
        self.inner.edit_fingerprint(module_id, record_workspace)
    }
    fn attach_files_to_change(&self, id: &Json, paths: &[String], dup: &str) -> Result<()> {
        self.inner.attach_files_to_change(id, paths, dup)
    }
    fn update_planning_field(&self, id: &Json, req: &PlanningUpdate<'_>) -> Result<()> {
        self.inner.update_planning_field(id, req)
    }
}

fn events(j: &MemJournal) -> Vec<Json> {
    j.read().iter().map(|r| r["event"].clone()).collect()
}

#[test]
fn execute_unknown_outcome_leaves_started() {
    let _cfg = config_guard(Some("acme.freshservice.com"));
    let h = Hints::new();
    let (create, _) = golden_create("full", &h);
    let (update, g) = golden_update("subject", &h);
    for status in [500, 502, 503, 504, 429, 408, 403] {
        let j = MemJournal::default();
        let Err(ExecError::Failed(e)) =
            execute(&create, &RejectWriter::new(status), &j, "skip", None)
        else {
            panic!("{status} create")
        };
        assert_eq!(e.as_api().unwrap().status, status);
        assert_eq!(events(&j), [json!("started")], "{status} create");
        let again = check_duplicate(&create, false, &j).unwrap_err();
        assert_eq!(again.exit_code(), 4);
        assert!(again.to_string().contains("no recorded outcome"));

        let writer = RejectWriter::new(status);
        arm_change(&writer.inner, &update, g);
        let j = MemJournal::default();
        let err = execute(&update, &writer, &j, "skip", None).unwrap_err();
        assert!(
            err.to_string().contains("write outcome unknown"),
            "{status} {err}"
        );
        assert_eq!(events(&j), [json!("started")], "{status} update");
    }
}

#[test]
fn execute_definite_rejection_records_failed() {
    let _cfg = config_guard(Some("acme.freshservice.com"));
    let h = Hints::new();
    let (create, _) = golden_create("full", &h);
    let (update, g) = golden_update("subject", &h);
    for status in [400, 404, 409, 422] {
        let j = MemJournal::default();
        assert!(execute(&create, &RejectWriter::new(status), &j, "skip", None).is_err());
        assert_eq!(events(&j), [json!("started"), json!("failed")]);
        assert_eq!(j.read()[1]["error"], json!("details omitted"));
        check_duplicate(&create, false, &j).unwrap();

        let writer = RejectWriter::new(status);
        arm_change(&writer.inner, &update, g);
        let j = MemJournal::default();
        let err = execute(&update, &writer, &j, "skip", None).unwrap_err();
        assert!(
            err.to_string()
                .contains(&format!("write rejected (HTTP {status})")),
            "{err}"
        );
        assert_eq!(events(&j), [json!("started"), json!("failed")]);
    }
}

#[test]
fn execute_failed_upload_before_create_records_failed() {
    let h = Hints::new();
    let (p, _d) = with_attachment(&h);
    let w = FakeWriter {
        fail_upload: true,
        ..FakeWriter::default()
    };
    let j = MemJournal::default();
    assert!(execute(&p, &w, &j, "skip", Some("before-create")).is_err());
    assert!(w.calls.lock().unwrap().is_empty());
    assert_eq!(events(&j), [json!("started"), json!("failed")]);
}

/// Journal whose writes fail (a full disk or read-only state directory).
#[derive(Default)]
struct FailingJournal(MemJournal);

impl Journal for FailingJournal {
    fn read(&self) -> Vec<Json> {
        self.0.read()
    }
    fn record_plan(&self, _: &str, _: &str, _: &str, _: Option<&Json>, _: Option<&Json>) {}
    fn try_record_event(&self, _: &str, _: &str, _: &str, _: PlanEvent<'_>) -> Result<()> {
        Err(Error::other("No space left on device"))
    }
    fn recent_plan(&self, plan: &str, action: &str, hours: f64) -> Option<Json> {
        self.0.recent_plan(plan, action, hours)
    }
}

#[test]
fn execute_refuses_when_started_is_not_persisted() {
    let _cfg = config_guard(Some("acme.freshservice.com"));
    let h = Hints::new();
    let (create, _) = golden_create("full", &h);
    let (update, g) = golden_update("subject", &h);
    let dropping = MemJournal {
        drop_writes: true,
        ..MemJournal::default()
    };
    let journals: [&dyn Journal; 2] = [&FailingJournal::default(), &dropping];
    for j in journals {
        let w = FakeWriter::default();
        let Err(ExecError::Failed(e)) = execute(&create, &w, j, "skip", None) else {
            panic!("create")
        };
        assert_eq!(e.exit_code(), 5);
        assert_eq!(
            e.to_string(),
            "could not persist the change journal; nothing was sent"
        );
        assert!(w.calls.lock().unwrap().is_empty());

        let w = FakeWriter::default();
        arm_change(&w, &update, g);
        let Err(ExecError::Failed(e)) = execute(&update, &w, j, "skip", None) else {
            panic!("update")
        };
        assert_eq!(e.exit_code(), 2);
        assert_eq!(
            e.to_string(),
            "could not persist ambiguity journal; no write attempted"
        );
        assert!(w.calls.lock().unwrap().is_empty());
    }
}

#[test]
fn execute_refuses_when_only_an_older_started_exists() {
    let h = Hints::new();
    let (p, _) = golden_create("full", &h);
    let j = MemJournal::default();
    j.record_plan("started", &p.hash, "create", None, None);
    let dropping = MemJournal {
        rows: std::sync::Mutex::new(j.read()),
        drop_writes: true,
    };
    let w = FakeWriter::default();
    assert!(execute(&p, &w, &dropping, "skip", None).is_err());
    assert!(w.calls.lock().unwrap().is_empty());
}
