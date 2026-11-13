//! `tests/test_rule_write_gate.py`: the write gate for business rules fsv
//! cannot fully model (`RulesEngine::write_gate` through
//! `FormState::rule_gate`), on synthetic Acme rules.

use serde_json::{Value as Json, json};

use super::*;
use crate::testing::golden;
use fsv_forms::tz::TzEnv;

/// `test_ticket_create.meta`: the Acme incident form, its first three rules.
fn meta() -> FormMeta {
    let keep = [
        "requester",
        "subject",
        "description",
        "priority",
        "impact",
        "status",
        "cloud_only",
        "stage",
        "staging_start",
        "staging_end",
        "legacy_flag",
        "reviewer",
    ];
    let fields: Vec<Json> = golden(&["fields", "change_fields"])
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| keep.contains(&f["name"].as_str().unwrap()))
        .cloned()
        .collect();
    let mut rules = golden(&["rules"]).clone();
    let first: Vec<Json> = rules["business_rules"].as_array().unwrap()[..3]
        .iter()
        .cloned()
        .map(|mut r| {
            r["module_type"] = json!("ticket");
            r
        })
        .collect();
    rules["business_rules"] = json!(first);
    FormMeta::new(
        "ticket",
        "new_form",
        json!({"ticket_fields": fields}),
        rules,
        json!(2),
        0.0,
        "fixture",
        Json::Null,
    )
    .unwrap()
    .with_tz_env(TzEnv::empty())
}

fn who() -> LookupValue {
    LookupValue::new(9001i64, "Jane Doe", Some("jane.doe@example.com"))
}

fn draft() -> Draft {
    Draft {
        answers: json!({"Subject": "Service unavailable", "Description": "Please help", "Cloud Only": "Yes", "Stage": "Live"})
            .as_object()
            .unwrap()
            .clone(),
        ..Draft::default()
    }
}

fn plan_with(m: &FormMeta, d: Draft, devices: &[i64]) -> Plan {
    let opts = TicketOptions {
        requester: Some(who()),
        devices: devices.to_vec(),
        ..TicketOptions::default()
    };
    build_create_plan(&Arc::new(m.clone()), &d, &opts).unwrap()
}

fn plan(m: &FormMeta) -> Plan {
    plan_with(m, draft(), &[])
}

/// `acme_change_form._rule` with `module_type = "ticket"`, appended to the form.
fn add(m: &mut FormMeta, rid: i64, cond: Json, actions: Json) {
    m.rules["business_rules"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id": rid, "name": format!("Acme rule {rid}"), "workspace_id": 2, "module_type": "ticket",
                     "item_id": null, "deleted": false, "user_type": "both", "form_type": "both", "state": "active",
                     "rule_type": "client_side", "description": "", "condition_data": cond,
                     "action_data": {"action": actions}, "is_reversible": false, "is_validation": false,
                     "updated_at": "2030-01-03T00:00:00Z"}));
}

fn c(name: &str, source: &str, op: &str, value: Option<Json>) -> Json {
    let mut out = json!({"name": name, "evaluate_on": source, "operator": op});
    if let Some(v) = value {
        out["value"] = v;
    }
    out
}

fn a(prop: &str, names: &[&str]) -> Json {
    json!({"name": names, "property": prop})
}

fn has(lines: &[String], parts: &[&str]) -> bool {
    lines.iter().any(|l| parts.iter().all(|p| l.contains(p)))
}

/// Python tests/test_rule_write_gate.py::test_agent_form_models_portal_false
#[test]
fn agent_form_models_portal_false() {
    let mut m = meta();
    add(
        &mut m,
        9000006001,
        json!({"all": [c("portal", "system", "is", Some(json!("true")))]}),
        json!([a("hide", &["subject"])]),
    );
    let p = plan(&m);
    assert!(p.ok(), "{:?}", p.errors);
    assert_eq!(
        m.engine().unwrap().context.system,
        Some(json!({"portal": false}).as_object().unwrap().clone())
    );
}

/// Python tests/test_rule_write_gate.py::test_current_user_groups_come_from_runtime_profile
#[test]
fn current_user_groups_come_from_runtime_profile() {
    let mut m = meta();
    add(
        &mut m,
        9000006002,
        json!({"all": [c("group_ids", "current_user", "includes", Some(json!(["77"])))]}),
        json!([a("hide", &["priority"])]),
    );
    m.rules["user"] = json!({"group_ids": [77], "department_ids": []});
    let p = plan(&m);
    assert!(p.ok(), "{:?}", p.errors);
    assert!(p.state.evaluated.matched.contains(&json!(9000006002i64)));
    m.rules["user"].as_object_mut().unwrap().remove("group_ids");
    let p = plan(&m);
    assert!(!p.ok() && has(&p.errors, &["rule 9000006002", "group_ids"]));
}

/// Python tests/test_rule_write_gate.py::test_tags_and_approval_state_are_empty_on_create
#[test]
fn tags_and_approval_state_are_empty_on_create() {
    let mut m = meta();
    add(
        &mut m,
        9000006003,
        json!({"any": [c("tags", "ticket", "selected", None), c("approval_status", "ticket", "selected", None)]}),
        json!([a("hide", &["subject"])]),
    );
    let p = plan(&m);
    assert!(p.ok(), "{:?}", p.errors);
    assert!(!p.state.evaluated.matched.contains(&json!(9000006003i64)));
}

/// Python tests/test_rule_write_gate.py::test_widget_hide_warns_with_rule_id
#[test]
fn widget_hide_warns_with_rule_id() {
    for widget in [
        "tags",
        "approval_request",
        "add_child_ticket",
        "edit_service_item",
    ] {
        let mut m = meta();
        add(
            &mut m,
            9000006004,
            json!({"all": null}),
            json!([a("hide", &[widget])]),
        );
        let p = plan(&m);
        assert!(p.ok(), "{widget}: {:?}", p.errors);
        assert!(
            has(&p.review.warnings, &["rule 9000006004", widget]),
            "{widget}: {:?}",
            p.review.warnings
        );
    }
}

/// Python tests/test_rule_write_gate.py::test_widget_hide_blocks_when_the_write_sends_it
#[test]
fn widget_hide_blocks_when_the_write_sends_it() {
    let mut m = meta();
    add(
        &mut m,
        9000006005,
        json!({"all": null}),
        json!([a("hide", &["config_item_ids"])]),
    );
    assert!(plan(&m).ok());
    let p = plan_with(&m, draft(), &[9000000101]);
    assert!(!p.ok() && has(&p.errors, &["rule 9000006005", "config_item_ids"]));
}

/// Python tests/test_rule_write_gate.py::test_widget_mandate_blocks
#[test]
fn widget_mandate_blocks() {
    let mut m = meta();
    add(
        &mut m,
        9000006006,
        json!({"all": null}),
        json!([a("mandate", &["tags"])]),
    );
    let p = plan(&m);
    assert!(!p.ok() && has(&p.errors, &["rule 9000006006", "could require"]));
}

/// Python tests/test_rule_write_gate.py::test_unknown_context_rule_blocks_only_when_it_touches_the_write
#[test]
fn unknown_context_rule_blocks_only_when_it_touches_the_write() {
    let mut m = meta();
    add(
        &mut m,
        9000006007,
        json!({"all": [c("channel", "system", "is", Some(json!("email")))]}),
        json!([a("hide", &["customFields.legacy_flag"])]),
    );
    let p = plan(&m);
    assert!(p.ok(), "{:?}", p.errors);
    assert!(has(&p.review.warnings, &["rule 9000006007", "channel"]));
    let last = m.rules["business_rules"].as_array_mut().unwrap().last_mut();
    last.unwrap()["action_data"]["action"] = json!([a("hide", &["subject"])]);
    let p = plan(&m);
    assert!(!p.ok() && has(&p.errors, &["rule 9000006007", "Subject"]));
}

/// Python tests/test_rule_write_gate.py::test_unknown_context_rule_could_require_unfilled_field_blocks
#[test]
fn unknown_context_rule_could_require_unfilled_field_blocks() {
    let mut m = meta();
    add(
        &mut m,
        9000006008,
        json!({"all": [c("channel", "system", "is", Some(json!("email")))]}),
        json!([a("mandate", &["customFields.legacy_flag"])]),
    );
    let p = plan(&m);
    assert!(!p.ok() && has(&p.errors, &["rule 9000006008"]));
}

/// Python tests/test_rule_write_gate.py::test_hidden_answer_is_refused_with_rule_id
#[test]
fn hidden_answer_is_refused_with_rule_id() {
    let m = meta();
    let mut d = draft();
    d.answers.insert("Legacy Flag".into(), json!(true));
    let p = plan_with(&m, d, &[]);
    assert!(!p.ok());
    assert!(
        p.errors
            .iter()
            .any(|e| e.starts_with("business rules: Legacy Flag: hidden by rule")),
        "{:?}",
        p.errors
    );
}
