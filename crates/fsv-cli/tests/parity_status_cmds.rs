//! Status, lifecycle, conversation and problem parity regressions;
//! expected values from Python fsv at 31a9103.

mod common;
use common::{Env, fixture, get};
use serde_json::{Value, json};

const NO_SESSION: &str = "error: no session; run `fsv auth login`\n";
const WRONG_ID: &str =
    "error: ticket read did not identify the exact target; no update attempted\n";

/// `PropertyError` from the exact-record read is uncaught in Python (exit 1).
#[test]
fn lifecycle_wrong_record_is_an_uncaught_error() {
    let env = Env::new(vec![get(
        "tickets/5",
        json!({"ticket": {"id": 6, "status": 2}}),
    )]);
    for args in [
        &["tickets", "resolve", "5", "--note", "x", "--dry-run"][..],
        &["tickets", "close", "5", "--dry-run"],
        &["tickets", "reopen", "5", "--dry-run"],
        &["tickets", "status", "5", "Open", "--dry-run"],
        &[
            "tickets",
            "reply",
            "5",
            "hi",
            "--set-status",
            "closed",
            "--dry-run",
        ],
    ] {
        let r = env.run(args);
        assert_eq!(r.code, 1, "{args:?}: {}", r.err);
        assert_eq!(r.out, "", "{args:?}");
        assert_eq!(r.err, WRONG_ID, "{args:?}");
    }
}

fn routes_with(name: &str, edit: impl Fn(&mut Value)) -> Vec<Value> {
    let mut routes = fixture(name);
    for r in &mut routes {
        edit(r);
    }
    routes
}

/// A `PropertyError` after the status PUT is not a `ReviewedWriteError`:
/// the partial report gives no rejection reason.
#[test]
fn lifecycle_partial_report_reason_only_for_reviewed_errors() {
    let routes = routes_with("ticket_lifecycle", |r| {
        if r["method"] == "GET" && r["path"] == "/api/_/tickets/123" && r.get("once").is_none() {
            r["json"]["ticket"]["id"] = json!(999);
        }
    });
    let r = Env::new(routes).run(&[
        "tickets",
        "resolve",
        "INC-123",
        "--note",
        "Acme fix <safe>",
        "--yes",
        "--expect-plan",
        "6794e2e526e44c1e65f401df1fdece18dedab4c5289bce634d4b661246c39098",
    ]);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, "");
    assert_eq!(
        r.err,
        "resolution note saved; the status write outcome is unknown; status unavailable on readback. Next: inspect `fsv tickets get 123` and `fsv tickets activity 123` for the workflow/approval/rule entry; once it no longer holds the ticket, review the status alone with `fsv tickets status 123 Resolved --dry-run` (the note is already saved). Do not resend this plan.\n"
    );
}

/// Python `==` on read-back values: `73.0 == 73`.
#[test]
fn property_readback_uses_python_equality() {
    let routes = routes_with("ticket_reviewed", |r| {
        if r["method"] == "GET"
            && r["path"] == "/api/_/tickets/5"
            && r["json"]["ticket"]["priority"] == json!(73)
        {
            r["json"]["ticket"]["priority"] = json!(73.0);
        }
    });
    let r = Env::new(routes).run(&[
        "tickets",
        "update",
        "5",
        "--reviewed",
        "--priority",
        "High",
        "--yes",
        "--expect-plan",
        "3212f8ceb7a5590061f639e5731caeaa0955a7617781938856beb69db8b6e8e7",
    ]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(r.json()["fields"], json!({"priority": "matched"}));
    assert_eq!(r.json()["verified"], json!(true));
}

/// An empty option value is falsy in Python, so it does not count as given.
#[test]
fn empty_option_values_count_as_absent() {
    for args in [
        &[
            "problems",
            "update",
            "5",
            "--planned-effort",
            "2h",
            "--status",
            "",
        ][..],
        &[
            "tickets",
            "reply",
            "5",
            "hi",
            "--inline-image",
            "x.png",
            "--set-status",
            "",
        ],
        &[
            "tickets",
            "reply",
            "5",
            "hi",
            "--attach",
            "x.txt",
            "--set-status",
            "",
        ],
    ] {
        let mut env = Env::new(vec![]);
        env.session = false;
        let r = env.run(args);
        assert_eq!(r.code, 3, "{args:?}: {}", r.err);
        assert_eq!(r.err, NO_SESSION, "{args:?}");
    }
}

/// A `FormError` from the status field is caught: one line, exit 2.
#[test]
fn problem_status_form_error_exits_2() {
    let routes = routes_with("problem_status", |r| {
        if r["path"] == "/api/_/problem_form_fields" {
            for f in r["json"]["problem_fields"].as_array_mut().unwrap() {
                if f["name"] == "status" {
                    f["field_type"] = json!("custom_checkbox");
                }
            }
        }
    });
    let r = Env::new(routes).run(&[
        "problems",
        "status",
        "7",
        "--to",
        "Change Requested",
        "--dry-run",
    ]);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.err, "Status: 2 is not yes/no/true/false\n");
}

/// The tenant check precedes the session: no client is opened first.
#[test]
fn add_note_checks_the_tenant_before_the_session() {
    for session in [true, false] {
        let mut env = Env::new(vec![]);
        env.session = session;
        std::fs::create_dir_all(env.config_dir()).unwrap();
        std::fs::write(env.config_dir().join("config.json"), "{}").unwrap();
        let r = env.run(&[
            "tickets",
            "add-note",
            "5",
            "--body",
            "hi",
            "--yes",
            "--expect-plan",
            "0728f357651c6ac051c8d3e84f29cd914bf6749db7e80d470cba3626e52f0611",
        ]);
        assert_eq!(r.code, 2, "session {session}: {}", r.err);
        assert!(
            r.err
                .ends_with("}\nconfigure a tenant before sending a note\n"),
            "session {session}: {}",
            r.err
        );
        assert!(r.requests.is_empty());
    }
    let mut env = Env::new(vec![]);
    env.session = false;
    let r = env.run(&[
        "tickets",
        "add-note",
        "5",
        "--body",
        "hi",
        "--yes",
        "--expect-plan",
        "6a1cb720ba3db32fb4b46fc60d96a34dd122fce1e3692e10f5182cc18ad61962",
    ]);
    assert_eq!(r.code, 2, "{}", r.err);
    assert!(
        r.err
            .ends_with("}\ncould not prepare the session; no note POST was attempted\n"),
        "{}",
        r.err
    );
}

/// `json.dumps` default separators in the proven closure-rule cache.
#[test]
fn closure_rule_cache_uses_python_separators() {
    let env = Env::new(fixture("problem_status_rejected"));
    let r = env.run(&[
        "problems",
        "status",
        "8",
        "--to",
        "closed",
        "--yes",
        "--expect-plan",
        "2e3389e436dc56373dfab815125e119b35c9da3c85bcda2c57aa49d0e62d79b9",
    ]);
    assert_eq!(r.code, 2, "{}", r.err);
    let forms = env.cache_dir().join("forms");
    let file = std::fs::read_dir(&forms)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.to_string_lossy()
                .ends_with("--problem-closure-rules.json")
        })
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(file).unwrap(),
        r#"{"codes": ["check_analysis_completed", "check_workaround_or_solution_added"]}"#
    );
}

/// `str(raw.get("name") or raw.get("label"))` for an unknown closure field.
#[test]
fn unknown_closure_field_shows_the_label_value() {
    let routes = routes_with("ticket_lifecycle", |r| {
        if r["path"] == "/api/_/ticket_form_fields" {
            r["json"]["ticket_fields"]
                .as_array_mut()
                .unwrap()
                .push(json!({
                "id": 9000007999_i64, "name": 0, "label": "", "field_type": "custom_text",
                "required": false, "required_for_closure": true, "position": 99,
                "default_field": false, "choices": [], "nested_fields": []}));
        }
    });
    let r = Env::new(routes).run(&["tickets", "close", "124", "--dry-run"]);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.err, "unknown required fields: \n");
}

/// Python `==` on conversation read-back: `1 == True`.
#[test]
fn conversation_readback_uses_python_equality() {
    fn walk(v: &mut Value) {
        match v {
            Value::Object(m) => {
                if m.get("id") == Some(&json!(901)) && m.get("private") == Some(&json!(true)) {
                    m.insert("private".into(), json!(1));
                }
                m.values_mut().for_each(walk);
            }
            Value::Array(a) => a.iter_mut().for_each(walk),
            _ => {}
        }
    }
    let routes = routes_with("ticket_mail", |r| {
        if r["method"] == "GET" {
            walk(&mut r["json"]);
        }
    });
    let r = Env::new(routes).run(&[
        "tickets",
        "forward",
        "5",
        "see below",
        "--to",
        "ann@acme.com",
        "--sender",
        "support@acme.com",
        "--json",
        "--yes",
        "--expect-plan",
        "5c24fced7a7b4c77e468569a8fa7cbaa16dbda8cb34d184b3fe5daa0188a8263",
    ]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(r.json()["fields"]["private"], json!("matched"));
    assert_eq!(r.json()["verified"], json!(true));
}
