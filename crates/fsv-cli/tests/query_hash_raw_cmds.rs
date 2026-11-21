//! Raw `--query-hash` shapes the list UI sends pass through unchanged, and a
//! scalar value with a list operator is a clean exit-2 refusal with nothing
//! sent (`tests/test_query_hash_raw_shapes.py`).

mod common;
use common::{Env, Run, fixture};
use serde_json::{Value, json};

fn sent_query_hashes(r: &Run, module: &str) -> Vec<Value> {
    r.requests
        .iter()
        .filter(|q| {
            q["method"] == "GET"
                && q["url"]
                    .as_str()
                    .is_some_and(|u| u.contains(&format!("/api/_/{module}?")))
        })
        .filter_map(|q| {
            let url = q["url"].as_str()?;
            let query = url.split_once('?')?.1;
            let encoded = query
                .split('&')
                .find_map(|p| p.strip_prefix("query_hash="))?;
            let decoded = url_decode(encoded);
            serde_json::from_str(&decoded).ok()
        })
        .collect()
}

fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap());
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap()
}

fn panel_shapes() -> Vec<Value> {
    vec![
        json!([{"value": ["2"], "condition": "status", "operator": "is_in", "type": "default"},
               {"value": "240", "condition": "created_at", "operator": "is_greater_than", "type": "default"}]),
        json!([{"value": {"from": "2030-01-01T00:00:00-04:00", "to": "2030-02-01T00:00:00-05:00"},
                "condition": "created_at", "operator": "is_greater_than", "type": "default"}]),
        json!([{"value": ["-1"], "condition": "responder_id", "operator": "is_in", "type": "default"}]),
        json!([{"condition": "status", "operator": "is_in", "type": "default", "value": [2]}]),
    ]
}

#[test]
fn panel_shapes_pass_through_unchanged() {
    for module in ["tickets", "changes"] {
        for shape in panel_shapes() {
            let env = Env::new(fixture("ls"));
            let r = env.run(&[module, "ls", "--json", "--query-hash", &shape.to_string()]);
            assert_eq!(r.code, 0, "{} {}", r.err, shape);
            assert!(!r.err.contains("Traceback"));
            assert_eq!(
                sent_query_hashes(&r, module).last(),
                Some(&shape),
                "{module}"
            );
            assert!(r.writes().is_empty());
        }
    }
}

#[test]
fn scalar_value_with_list_operator_is_a_clean_refusal() {
    for module in ["tickets", "changes"] {
        for value in [json!(2), json!("2"), json!({"id": 2})] {
            let env = Env::new(fixture("ls"));
            let raw = json!([{"condition": "status", "operator": "is_in", "type": "default", "value": value}]);
            let r = env.run(&[module, "ls", "--json", "--query-hash", &raw.to_string()]);
            assert_eq!(r.code, 2, "{} {}", r.out, r.err);
            let text = format!("{}{}", r.out, r.err);
            assert!(
                text.contains("takes a JSON list value") && text.contains("nothing was sent"),
                "{text}"
            );
            assert!(
                !text.contains("Traceback") && !text.contains("TypeError"),
                "{text}"
            );
            assert!(sent_query_hashes(&r, module).is_empty());
            assert!(r.writes().is_empty());
        }
    }
}

#[test]
fn repeated_condition_merges_lists_and_keeps_scalars() {
    let env = Env::new(fixture("ls"));
    let raw = json!([
        {"condition": "status", "operator": "is_in", "type": "default", "value": [2]},
        {"condition": "status", "operator": "is_in", "type": "default", "value": [3]},
        {"condition": "known_error", "operator": "is", "type": "default", "value": "1"},
    ]);
    let r = env.run(&["tickets", "ls", "--json", "--query-hash", &raw.to_string()]);
    assert_eq!(r.code, 0, "{}", r.err);
    let sent = sent_query_hashes(&r, "tickets").pop().unwrap();
    let sent = sent.as_array().unwrap();
    assert!(sent.contains(
        &json!({"condition": "known_error", "operator": "is", "type": "default", "value": "1"})
    ));
    let status: Vec<&Value> = sent.iter().filter(|i| i["condition"] == "status").collect();
    assert_eq!(status.len(), 1);
    let mut values: Vec<i64> = status[0]["value"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    values.sort();
    assert_eq!(values, [2, 3]);
}
