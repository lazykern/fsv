//! Captured list-filter wires: dates, numbers, multi-select, text refusal
//! (Python `406ca71`: `tests/test_filter_wires.py`). The form metadata names
//! workspace 7 and no workspace is configured.

use serde_json::{Value, json};

use super::*;

fn schema() -> Value {
    json!({"fields": [
        {"name": "status", "label": "Status", "field_type": "default_status", "default_field": true,
         "choices": [{"id": 2, "value": "Open"}]},
        {"name": "subject", "label": "Subject", "field_type": "default_subject", "default_field": true},
        {"name": "cf_notes", "label": "Notes", "field_type": "custom_paragraph"},
        {"name": "cf_score", "label": "Impact Score", "field_type": "custom_number", "workspace_id": 7},
        {"name": "msf_kind", "label": "Impact Kind", "field_type": "custom_multi_select", "workspace_id": 7,
         "choices": [{"id": 160, "value": "Outage"}, {"id": "161", "value": "Slow"}, {"id": "x", "value": "Bad"}]},
    ]})
}

fn build_in(resource: &str, sch: &Value, filters: &[&str]) -> Result<Value, QueryError> {
    let filters: Vec<String> = filters.iter().map(|s| s.to_string()).collect();
    let hash = build_from_schema(None, resource, sch, &filters, &|| Value::Null)?;
    Ok(serde_json::from_str(&hash.unwrap()).unwrap())
}

fn build(resource: &str, filters: &[&str]) -> Result<Value, QueryError> {
    build_in(resource, &schema(), filters)
}

fn refused(resource: &str, filters: &[&str]) -> String {
    match build(resource, filters) {
        Ok(v) => panic!("{filters:?} on {resource} was sent: {v}"),
        Err(e) => e.0,
    }
}

#[test]
fn day_and_between_use_inclusive_is_in_the_range() {
    for res in ["tickets", "changes", "problems"] {
        assert_eq!(
            build(res, &["created_at=2030-01-05"]).unwrap(),
            json!([{"value": {"from": "2030-01-05", "to": "2030-01-05"}, "condition": "created_at",
                    "operator": "is_in_the_range", "type": "default"}]),
            "{res}"
        );
        assert_eq!(
            build(res, &["created_at>=2030-01-01", "created_at<=2030-01-31"]).unwrap()[0]["value"],
            json!({"from": "2030-01-01", "to": "2030-01-31"}),
            "{res}"
        );
    }
}

#[test]
fn strict_bounds_shift_one_whole_day_and_explain_it() {
    let mut explain = Vec::new();
    let item = date_condition(
        "tickets",
        "created_at",
        ">",
        "2030-01-31",
        &mut explain,
        "created_at",
    )
    .unwrap();
    assert_eq!(item["value"], json!({"from": "2030-02-01"}));
    assert!(
        explain[0].value.contains("sent as from 2030-02-01"),
        "{}",
        explain[0].value
    );
    let item = date_condition(
        "tickets",
        "created_at",
        "<",
        "2030-03-01",
        &mut Vec::new(),
        "created_at",
    )
    .unwrap();
    assert_eq!(item["value"], json!({"to": "2030-02-28"}));
}

#[test]
fn one_sided_and_sub_day_dates_are_refused() {
    for (expr, msg) in [
        ("created_at>=2030-01-01", "needs both ends"),
        ("created_at<=2030-01-01", "needs both ends"),
        ("created_at=2030-01-01T10:00:00", "whole day"),
        ("created_at=2030-02-30", "invalid date"),
        ("created_at!=2030-01-01", "not supported"),
    ] {
        let err = refused("tickets", &[expr]);
        assert!(err.contains(msg), "{expr}: {err}");
    }
}

#[test]
fn relative_dates_go_to_advanced_with_workspace() {
    for (raw, operator, token) in [
        ("on:yesterday", "on", "yesterday"),
        ("last:one_week", "in_the_last", "one_week"),
        ("next:six months", "in_the_next", "six_months"),
    ] {
        assert_eq!(
            build("tickets", &[&format!("created_at={raw}")]).unwrap(),
            json!({"all": [
                {"value": token, "condition": "created_at", "operator": operator, "type": "default"},
                {"condition": "workspace_id", "operator": "is_in", "value": [{"id": 7}], "type": "default"}],
                "any": []}),
            "{raw}"
        );
    }
    let due = build("changes", &["due_by=on:week"]).unwrap();
    assert_eq!(due["all"][0]["condition"], "due_by_date");
    assert_eq!(
        due["all"][1],
        json!({"condition": "workspace_id", "operator": "is", "value": 7, "type": "default"})
    );
}

#[test]
fn problems_refuse_unproven_shapes() {
    for filters in [
        &["created_at=on:yesterday"][..],
        &["created_at=last:one_week"],
        &["Impact Score=3"],
        &["Impact Kind=Outage"],
        &["updated_at>=2030-01-01", "updated_at<=2030-02-01"],
    ] {
        let err = refused("problems", filters);
        assert!(err.contains("nothing was sent"), "{filters:?}: {err}");
    }
}

#[test]
fn unknown_relative_tokens_are_refused() {
    for bad in ["on:fortnight", "last:yesterday", "on:5"] {
        let err = refused("tickets", &[&format!("created_at={bad}")]);
        assert!(err.contains("unknown"), "{bad}: {err}");
    }
}

#[test]
fn number_operators_send_string_values() {
    for (op, operator) in [
        ("=", "is"),
        ("!=", "is_not"),
        (">=", "is_greater_than_or_equal"),
        ("<=", "is_less_than_or_equal"),
    ] {
        let payload = build("changes", &[&format!("Impact Score{op}3")]).unwrap();
        assert_eq!(
            payload["all"][0],
            json!({"value": "3", "condition": "cf_score", "operator": operator, "type": "custom_field"})
        );
    }
}

#[test]
fn number_strict_and_non_integer_are_refused() {
    for expr in [
        "Impact Score>3",
        "Impact Score<3",
        "Impact Score=3.5",
        "Impact Score~=3",
    ] {
        refused("tickets", &[expr]);
    }
}

#[test]
fn multi_select_sends_integer_choice_id() {
    assert_eq!(
        build("tickets", &["Impact Kind=Outage"]).unwrap()["all"][0]["value"],
        json!([160])
    );
    assert_eq!(
        build("tickets", &["Impact Kind!=Slow"]).unwrap()["all"][0],
        json!({"value": [161], "condition": "msf_kind", "operator": "is_not_in", "type": "custom_field"})
    );
    let err = refused("tickets", &["Impact Kind=Bad"]);
    assert!(err.contains("integer choice id"), "{err}");
}

#[test]
fn text_fields_fail_closed() {
    let mut sch = schema();
    sch["fields"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name": "description", "field_type": "default_description"}));
    for expr in ["Subject=Acme", "Notes=Globex", "description=x"] {
        let err = build_in("tickets", &sch, &[expr]).unwrap_err().0;
        assert!(err.contains("no list filter exists"), "{expr}: {err}");
    }
}

#[test]
fn or_grouping_keeps_workspace_in_all() {
    let status =
        json!({"condition": "status", "operator": "is_in", "value": ["2"], "type": "default"});
    let (payload, advanced, _) =
        serialize("tickets", std::slice::from_ref(&status), true, &json!(7)).unwrap();
    assert!(advanced);
    assert_eq!(
        serde_json::from_str::<Value>(&payload.unwrap()).unwrap(),
        json!({"any": [status],
               "all": [{"condition": "workspace_id", "operator": "is_in", "value": [{"id": 7}], "type": "default"}]})
    );
    let err = serialize(
        "problems",
        &[json!({"condition": "status"})],
        true,
        &json!(7),
    )
    .unwrap_err();
    assert!(err.0.contains("no advanced filter"), "{}", err.0);
}

#[test]
fn configured_workspace_wins_over_form_metadata() {
    let sch = schema();
    assert_eq!(runtime_workspace(json!(9), Some(&sch)), json!(9));
    assert_eq!(runtime_workspace(json!(""), Some(&sch)), json!(7));
    assert_eq!(runtime_workspace(Value::Null, None), Value::Null);
    let hash = build_from_schema(None, "tickets", &sch, &["Impact Score=3".into()], &|| {
        json!(9)
    })
    .unwrap()
    .unwrap();
    assert!(
        hash.contains(
            r#"{"condition":"workspace_id","operator":"is_in","value":[{"id":9}],"type":"default"}"#
        ),
        "{hash}"
    );
}

#[test]
fn number_predicates_are_verified_client_side() {
    use crate::filter_proof::{contradictions, matches};
    let clause = [
        json!({"condition": "cf_score", "operator": "is_greater_than_or_equal", "value": "3", "type": "custom_field"}),
    ];
    let row = |v: Value| json!({"custom_fields": {"cf_score": v}});
    assert!(matches(row(json!(4)).as_object().unwrap(), &clause).unwrap());
    assert!(!matches(row(json!("2")).as_object().unwrap(), &clause).unwrap());
    assert_eq!(
        contradictions(
            &[json!({"id": 9, "custom_fields": {"cf_score": 1}})],
            &clause
        ),
        [json!(9)]
    );
    assert!(matches(row(Value::Null).as_object().unwrap(), &clause).is_err());
}

// Python `3ad6041`: dates without a basic range wire use the advanced builder.

fn ws_tickets() -> Value {
    json!({"condition": "workspace_id", "operator": "is_in", "value": [{"id": 7}], "type": "default"})
}

#[test]
fn date_ranges_without_a_basic_wire_use_the_advanced_condition_name() {
    for (field, adv) in [
        ("due_by", "due_by_date"),
        ("fr_due_by", "fr_due_by_date"),
        ("closed_at", "closed_date"),
        ("resolved_at", "resolved_date"),
    ] {
        assert_eq!(
            build(
                "tickets",
                &[
                    &format!("{field}>=2030-01-01"),
                    &format!("{field}<=2030-01-31")
                ]
            )
            .unwrap(),
            json!({"all": [
                {"value": {"from": "2030-01-01", "to": "2030-01-31"}, "condition": adv,
                 "operator": "is_in_the_range", "type": "default"}, ws_tickets()], "any": []}),
            "{field}"
        );
    }
}

#[test]
fn date_ranges_with_a_basic_name_stay_in_query_hash() {
    for field in [
        "created_at",
        "updated_at",
        "planned_start_date",
        "planned_end_date",
    ] {
        assert_eq!(
            build("tickets", &[&format!("{field}=2030-01-05")]).unwrap(),
            json!([{"value": {"from": "2030-01-05", "to": "2030-01-05"}, "condition": field,
                    "operator": "is_in_the_range", "type": "default"}]),
            "{field}"
        );
    }
}

#[test]
fn due_range_on_changes_is_advanced_and_ticket_only_dates_refuse_elsewhere() {
    let due = build("changes", &["due_by=2030-01-05"]).unwrap();
    assert_eq!(due["all"][0]["condition"], "due_by_date");
    for res in ["changes", "problems"] {
        let err = refused(res, &["closed_at=2030-01-05"]);
        assert!(err.contains("nothing was sent"), "{res}: {err}");
    }
    let err = refused("changes", &["resolved_at=on:today"]);
    assert!(err.contains("nothing was sent"), "{err}");
}

#[test]
fn updated_relative_period_uses_advanced_updated_at() {
    let payload = build("tickets", &["updated_at=last:one_week"]).unwrap();
    assert_eq!(
        payload["all"][0],
        json!({"value": "one_week", "condition": "updated_at", "operator": "in_the_last", "type": "default"})
    );
}

/// `test_query_matrix.test_due_range_is_merged_in_the_advanced_builder_without_erasing_other_fields`.
#[test]
fn due_range_is_merged_in_the_advanced_builder_without_erasing_other_fields() {
    let sch = json!({"fields": [{"name": "status", "label": "State", "default_field": true,
        "field_type": "default_status", "choices": [{"id": 47, "value": "Awaiting review"}]}]});
    let result = build_in(
        "changes",
        &sch,
        &["due_by>=2026-01-01", "status=47", "due_by<=2026-12-31"],
    )
    .unwrap();
    assert_eq!(result["any"], json!([]));
    assert_eq!(
        result["all"].as_array().unwrap()[..2],
        [
            json!({"condition": "due_by_date", "type": "default", "operator": "is_in_the_range",
                   "value": {"from": "2026-01-01", "to": "2026-12-31"}}),
            json!({"condition": "status", "type": "default", "operator": "is_in", "value": [47]}),
        ]
    );
}
