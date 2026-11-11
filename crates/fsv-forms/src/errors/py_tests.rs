//! Error messages against `fsv.forms.errors` (the Python `__init__`s build
//! the message; here the enum variants' `Display` does).

use serde_json::{Value as Json, json};

use super::FormError;
use crate::py_golden::{check, g};
use crate::value::{LookupValue, Value};

fn case(name: &str, e: &FormError) {
    let want = g(&["errors", name]);
    check(name, &json!(e.to_string()), &want["message"]);
}

#[test]
fn unknown_field_init_message() {
    case(
        "unknown_plain",
        &FormError::UnknownField {
            name: "Prio".into(),
            suggestions: vec![],
        },
    );
    case(
        "unknown_suggest",
        &FormError::UnknownField {
            name: "pri".into(),
            suggestions: vec!["Priority".into(), "Primary Contact".into()],
        },
    );
}

#[test]
fn value_parse_init_message_and_options() {
    let e = FormError::parse_opts(
        "Team",
        &Value::str("zz"),
        "'zz' is not a valid choice",
        vec!["Platform".into(), "Labs".into()],
    );
    case("parse", &e);
    check(
        "options",
        &json!(e.options()),
        &g(&["errors", "parse", "options"]).clone(),
    );
}

#[test]
fn ambiguous_lookup_init_message() {
    let mut people: Vec<LookupValue> = (1..12)
        .map(|i| LookupValue::new(i, format!("Person {i}"), None))
        .collect();
    people.push(LookupValue::new(99, "", None));
    case(
        "ambiguous",
        &FormError::AmbiguousLookup {
            field: "Owner".into(),
            query: "per".into(),
            candidates: people,
        },
    );
    case(
        "ambiguous_blank",
        &FormError::AmbiguousLookup {
            field: "Owner".into(),
            query: "x".into(),
            candidates: vec![
                LookupValue::new(7, "", None),
                LookupValue::new("a-1", "", None),
            ],
        },
    );
}

#[test]
fn form_rule_init_message() {
    case(
        "rule",
        &FormError::FormRule {
            message: "rule 'X' is odd".into(),
            rule_id: json!(5),
            rule_name: Some("X".into()),
            unsupported: false,
        },
    );
    case(
        "rule_bare",
        &FormError::FormRule {
            message: "odd".into(),
            rule_id: Json::Null,
            rule_name: None,
            unsupported: false,
        },
    );
}

#[test]
fn payload_init_message_and_problems() {
    case("payload_empty", &FormError::Payload { problems: vec![] });
    let e = FormError::Payload {
        problems: vec!["Team is required".into(), "Stage is required".into()],
    };
    case("payload_two", &e);
    check(
        "problems",
        &json!(e.problems()),
        g(&["errors", "payload_two", "problems"]),
    );
}
