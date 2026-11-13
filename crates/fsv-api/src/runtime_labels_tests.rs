//! Display labels from runtime choices; static maps only as a fallback
//! (Python `936e767`: `tests/test_shifted_tenant.py`).

use serde_json::{Value, json};

use crate::schema::{display_label, humanize_key};
use crate::site::{priority_name, source_name};

pub(crate) fn shifted_schema() -> Value {
    json!({"fields": [
        {"name": "priority", "choices": [{"id": 1, "value": "P4"}, {"id": 2, "value": "P3"},
                                         {"id": 3, "value": "P2"}, {"id": 4, "value": "P1"}]},
        {"name": "impact", "choices": [{"id": 1, "value": "Gering"}, {"id": 3, "value": "Hoch"}]},
        {"name": "source", "label": "Source", "choices": [{"id": 2, "value": "portal_key"},
                                                          {"id": 20, "value": "Slack"},
                                                          {"id": 5, "value": "feedback_widget_key"}]},
        {"name": "status", "choices": [{"id": 2, "value": "Offen"}, {"id": 3, "value": "Wartend"},
                                       {"id": 12, "value": "Abgebrochen"}, {"id": 4, "value": "Gelöst"},
                                       {"id": 5, "value": "Erledigt"}, {"id": 13, "value": "In Arbeit"}]},
    ]})
}

#[test]
fn priority_impact_labels_come_from_runtime_choices() {
    let sch = shifted_schema();
    assert_eq!(
        display_label("priority", Some(&json!(4)), Some(&sch), "-"),
        "P1"
    );
    assert_eq!(
        display_label("impact", Some(&json!(3)), Some(&sch), "-"),
        "Hoch"
    );
    // never relabelled by the static map
    assert_eq!(
        display_label("impact", Some(&json!(2)), Some(&sch), "-"),
        "2"
    );
    // documented fallback
    assert_eq!(
        display_label(
            "priority",
            Some(&json!(4)),
            Some(&json!({"fields": []})),
            "-"
        ),
        "Urgent"
    );
    assert_eq!(priority_name(&json!({"priority": 4}), Some(&sch)), "P1");
}

#[test]
fn source_labels_translate_raw_i18n_keys_and_filter_by_them() {
    let sch = shifted_schema();
    assert_eq!(source_name(&json!({"source": 2}), Some(&sch)), "Portal");
    assert_eq!(
        source_name(&json!({"source": 5}), Some(&sch)),
        "Feedback Widget"
    );
    assert_eq!(source_name(&json!({"source": 20}), Some(&sch)), "Slack");
    // custom id: no static guess
    assert_eq!(source_name(&json!({"source": 11}), Some(&sch)), "11");
    let source = &sch["fields"][2];
    assert_eq!(
        crate::query::choice_value(source, "Portal", "default").unwrap(),
        json!(2)
    );
    assert_eq!(
        crate::query::choice_value(source, "portal_key", "default").unwrap(),
        json!(2)
    );
}

/// Edge cases checked against Python `936e767` (`schema.humanize_key`,
/// `schema.display_label`).
#[test]
fn humanize_key_and_display_label_edges() {
    for (text, want) in [
        ("portal_key", "Portal"),
        ("feedback_widget_key", "Feedback Widget"),
        ("a_key_key", "A Key"),
        ("aws_2fa_key", "Aws 2fa"),
        ("key", "key"),
        ("_key", "_key"),
        ("Portal_key", "Portal_key"),
        ("portal__key", "portal__key"),
        ("1x_key", "1x_key"),
        ("Slack", "Slack"),
    ] {
        assert_eq!(humanize_key(text), want, "{text}");
    }
    assert_eq!(display_label("priority", None, None, "-"), "-");
    assert_eq!(display_label("priority", Some(&json!("")), None, "-"), "-");
    assert_eq!(display_label("priority", Some(&json!(0)), None, "-"), "0");
    assert_eq!(
        display_label("priority", Some(&json!(true)), None, "-"),
        "Low"
    );
    assert_eq!(
        display_label("priority", Some(&json!(2.0)), None, "-"),
        "Medium"
    );
    assert_eq!(display_label("priority", Some(&json!("2")), None, "-"), "2");
    assert_eq!(
        display_label("risk", Some(&json!(4)), None, "-"),
        "Very High"
    );
    assert_eq!(display_label("urgency", Some(&json!(4)), None, "-"), "4");
    let sch = json!({"fields": [{"name": "priority", "choices": [{"id": "3", "name": "Hi"}]}]});
    assert_eq!(
        display_label("priority", Some(&json!(3)), Some(&sch), "-"),
        "Hi"
    );
}
