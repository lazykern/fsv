//! `tests/test_ticket_conversation_http.py`: transport-level mail/status
//! safety over a real `fsv_core::Client`; synthetic schemas are not live proof.

use serde_json::{Value, json};

use super::*;
use crate::properties::http_tests::{FAILURES, TENANTS, Wire};

fn err<T: std::fmt::Debug>(r: Result<T>) -> String {
    r.unwrap_err().to_string()
}

fn posts(w: &Wire) -> usize {
    w.writes("POST").len()
}

fn person() -> Vec<String> {
    vec!["person@example.com".into()]
}

fn plan(w: &Wire, kind: &str, cc: &[String], set_status: Option<&str>) -> Result<ConversationPlan> {
    build_plan(
        &w.client,
        &w.store,
        &w.cid.to_string(),
        "Acme update",
        &ConversationArgs {
            kind,
            cc,
            set_status,
            ..Default::default()
        },
    )
}

fn send_plan(w: &Wire, p: &ConversationPlan, public_confirmed: bool) -> Result<Value> {
    send(
        p,
        &w.client,
        &w.journal,
        &w.store,
        Some(&p.hash()),
        public_confirmed,
    )
}

/// Python tests/test_ticket_conversation_http.py::test_exact_pending_http_contract_and_public_consent
#[test]
fn exact_pending_http_contract_and_public_consent() {
    for tenant in TENANTS {
        for kind in ["reply", "note"] {
            let w = Wire::new(tenant);
            let cc = if kind == "reply" {
                recipients(&w.client, &person()).unwrap()
            } else {
                Vec::new()
            };
            let p = plan(&w, kind, &cc, Some("Pending")).unwrap();
            let payload = p.payload();
            assert_eq!(payload["helpdesk_ticket"]["status"], json!(w.pending));
            assert_eq!(posts(&w), 0);
            if kind == "reply" {
                assert!(err(send_plan(&w, &p, false)).contains("confirmation"));
            }
            let result = send_plan(&w, &p, kind == "reply").unwrap();
            assert_eq!(result["verified"], json!(true), "{kind}: {result}");
            let sent = w.writes("POST");
            let tail = if kind == "reply" { "reply" } else { "notes" };
            assert_eq!(sent.len(), 1);
            assert_eq!(
                sent[0].url,
                format!("https://{}/api/_/tickets/{}/{tail}", w.domain, w.cid)
            );
            assert_eq!(sent[0].header("content-type"), Some("application/json"));
            assert_eq!(sent[0].header("x-csrf-token"), Some("synthetic-token"));
            assert_eq!(
                serde_json::from_slice::<Value>(sent[0].body.as_deref().unwrap()).unwrap(),
                payload
            );
            assert!(payload.get("from_email").is_none());
            if kind == "reply" {
                let name = w.server().lookup_name.clone();
                assert_eq!(
                    payload["cc_emails"],
                    json!([format!("{name} <person@example.com>")])
                );
            } else {
                assert_eq!(payload["private"], json!(true));
            }
        }
    }
}

/// Python tests/test_ticket_conversation_http.py::test_missing_or_wrong_conversation_readback_never_resends
#[test]
fn missing_or_wrong_conversation_readback_never_resends() {
    for tenant in TENANTS {
        for missing in ["body", "cc_emails", "wrong-id", "contradict"] {
            let w = Wire::new(tenant);
            w.server().conv_readback = Some(missing);
            let cc = recipients(&w.client, &person()).unwrap();
            let p = plan(&w, "reply", &cc, None).unwrap();
            let result = send_plan(&w, &p, true).unwrap();
            assert_eq!(result["verified"], json!(false), "{missing}");
            assert!(
                result["warning"]
                    .as_str()
                    .unwrap()
                    .contains("do not resend"),
                "{missing}"
            );
            assert!(
                err(send_plan(&w, &p, true)).contains("was sent"),
                "{missing}"
            );
            assert_eq!(posts(&w), 1, "{missing}");
        }
    }
}

/// Python tests/test_ticket_conversation_http.py::test_each_conversation_write_caller_leaves_ambiguity_guard
#[test]
fn each_conversation_write_caller_leaves_ambiguity_guard() {
    for tenant in TENANTS {
        for failure in FAILURES {
            let w = Wire::new(tenant);
            w.server().failure = Some(failure);
            let p = plan(&w, "note", &[], Some("Pending")).unwrap();
            for _ in 0..2 {
                let msg = err(send_plan(&w, &p, false));
                assert!(msg.contains("unknown"), "{failure:?}: {msg}");
            }
            assert_eq!(posts(&w), 1, "{failure:?}");
        }
    }
}

/// Python tests/test_ticket_conversation_http.py::test_changed_cc_identity_refuses_instead_of_changing_reviewed_payload
#[test]
fn changed_cc_identity_refuses_instead_of_changing_reviewed_payload() {
    for tenant in TENANTS {
        let w = Wire::new(tenant);
        let cc = recipients(&w.client, &person()).unwrap();
        let p = plan(&w, "reply", &cc, None).unwrap();
        w.server().lookup_name = "Different Operator".into();
        assert!(err(send_plan(&w, &p, true)).contains("identity changed"));
        assert_eq!(posts(&w), 0);
    }
}

/// Python tests/test_ticket_conversation_http.py::test_resolution_requires_persisted_note_from_http_readback
#[test]
fn resolution_requires_persisted_note_from_http_readback() {
    for tenant in TENANTS {
        for label in ["Resolved", "Closed"] {
            // the fixture carries the built-in Resolved (4) and Closed (5) statuses
            let w = Wire::new(tenant);
            let msg = err(plan(&w, "note", &[], Some(label)));
            assert!(msg.contains("persisted resolution"), "{label}: {msg}");
            assert_eq!(posts(&w), 0);
        }
    }
}
