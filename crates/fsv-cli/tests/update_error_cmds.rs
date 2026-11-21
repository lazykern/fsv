//! A failed `changes update` send names the server's HTTP status and its
//! sanitized message (`tests/test_update_error_paths.py`), with `--json`
//! printing an error object. Synthetic routes only.

mod common;
use common::{DOMAIN, Env, fixture};
use serde_json::{Value, json};

const HTML_503: &str = "<html><head><style>p{}</style></head><body><h1>Service Unavailable</h1><p>Try later</p></body></html>";

fn relaxed(mut route: Value) -> Value {
    match route["path"].as_str() {
        Some("/api/_/business_rules") => route["json"] = json!({"business_rules": []}),
        Some("/api/_/change_form_fields") => {
            for field in route["json"]["change_fields"].as_array_mut().unwrap() {
                field["required"] = json!(false);
                field["required_for_closure"] = json!(false);
            }
        }
        _ => {}
    }
    route
}

fn routes(put: Value) -> Vec<Value> {
    let mut routes: Vec<Value> = fixture("change_resend")
        .into_iter()
        .filter(|r| r["once"].is_null() && r["method"] != "PUT" && r["method"] != "POST")
        .map(relaxed)
        .collect();
    let mut put = put;
    put["method"] = json!("PUT");
    put["path"] = json!("/api/_/changes/12");
    routes.push(put);
    routes
}

fn run(put: Value, json_flag: bool) -> common::Run {
    let env = Env::new(routes(put));
    let mut args = vec![
        "changes",
        "update",
        "12",
        "--set",
        "Subject=Patched",
        "--yes",
    ];
    if json_flag {
        args.push("--json");
    }
    env.run(&args)
}

#[test]
fn changes_update_failure_prints_status_and_body() {
    for json_flag in [true, false] {
        let r = run(json!({"status": 503, "text": HTML_503}), json_flag);
        assert_eq!(r.code, 5, "{} / {}", r.out, r.err);
        assert_eq!(r.write_paths(), ["PUT /api/_/changes/12"]);
        let flat = format!("{} {}", r.out, r.err)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            flat.contains("HTTP 503") && flat.contains("Service Unavailable"),
            "{flat}"
        );
        assert!(!flat.contains("<h1>"), "{flat}");
        if json_flag {
            let doc = r.json();
            assert_eq!(doc["verified"], json!(false));
            assert_eq!(doc["http_status"], json!(503));
            assert_eq!(
                doc["server_message"],
                json!("Service Unavailable Try later")
            );
            assert_eq!(doc["plan_hash"].as_str().unwrap().len(), 64);
            assert_eq!(doc["updated"], Value::Null);
        }
    }
}

#[test]
fn changes_update_rejection_names_the_field() {
    let body = json!({"description": "Validation failed", "errors": [
        {"field": "subject", "message": "Synthetic too long"}]});
    let r = run(json!({"status": 422, "json": body}), true);
    assert_eq!(r.code, 5, "{} / {}", r.out, r.err);
    let flat = format!("{} {}", r.out, r.err)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        flat.contains("HTTP 422") && flat.contains("subject: Synthetic too long"),
        "{flat}"
    );
    let _ = DOMAIN;
}
