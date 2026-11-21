//! Reviewed problem notes: plan-hash bound send, ambiguity journal and an
//! independent readback of the note body by listing the problem's notes
//! (`problem_notes.py`).
//!
//! The captured browser body is `{"body": ...}` only: the tenant rejects a
//! `private` key with HTTP 400, so visibility is not controllable and the
//! tenant default applies.

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::resource::{PROBLEMS, format_id, parse_id};
use fsv_py::Dumps;

use crate::readback::same_text;
use crate::reviewed_write::{WritePlan, execute_guarded};
use crate::service::{Api, Journal, collect_pages, obj};

pub const ACTION: &str = "problem_note";
pub const VISIBILITY: &str = "problem notes accept only a body (captured `{body}`; the tenant rejects `private` with HTTP 400): visibility is not controllable and the tenant default applies";

fn dumps_sorted(v: &Value) -> String {
    Dumps::default().ensure_ascii(false).sort_keys(true).dump(v)
}

/// Port of problem_notes.build_plan: pure local preparation; the body is
/// sent verbatim (HTML or plain text). `sends` is the answered-send count
/// of `journal.repeat_token`; zero keeps the pre-token hash.
pub fn build_plan(
    journal: &dyn Journal,
    tenant: &str,
    target: &str,
    body: &str,
) -> Result<WritePlan> {
    let cid = parse_id(&fsv_py::py_str(&json!(target)), &PROBLEMS)?;
    if cid <= 0 {
        return Err(Error::value("problem ID must be positive"));
    }
    if fsv_core::pystr::strip(body).is_empty() || body.contains('\0') {
        return Err(Error::value(
            "note body must be nonempty text without NUL characters",
        ));
    }
    let token = journal.repeat_token(&[ACTION], &json!(cid), None);
    let sends = token.get("sends").and_then(Value::as_i64).unwrap_or(0);
    Ok(WritePlan {
        action: ACTION.into(),
        target: cid,
        tenant: tenant.into(),
        method: "POST".into(),
        path: format!("problems/{cid}/notes"),
        payload_json: dumps_sorted(&json!({"body": body})),
        context_hash: if sends != 0 {
            dumps_sorted(&Value::Object(token))
        } else {
            String::new()
        },
    })
}

/// Port of problem_notes.review.
pub fn review(plan: &WritePlan) -> Map<String, Value> {
    let mut out = plan.review();
    out.insert(
        "id".into(),
        json!(format_id(&json!({"id": plan.target}), &PROBLEMS)),
    );
    out.insert("body".into(), plan.payload());
    let state = if plan.context_hash.is_empty() {
        json!({"sends": 0})
    } else {
        serde_json::from_str(&plan.context_hash).unwrap_or(Value::Null)
    };
    out.insert("record_state".into(), state);
    out.insert(
        "warnings".into(),
        json!([format!("visibility: {VISIBILITY}")]),
    );
    out
}

fn returned_id(response: &Value) -> Option<Value> {
    let note = match response.get("note") {
        Some(n @ Value::Object(_)) => n,
        _ => response,
    };
    match note.get("id")? {
        v @ Value::Number(n) if (n.is_i64() || n.is_u64()) && n.as_i64().is_none_or(|i| i > 0) => {
            Some(v.clone())
        }
        Value::String(s)
            if !s.is_empty()
                && s.bytes().all(|b| b.is_ascii_digit())
                && s.bytes().any(|b| b != b'0') =>
        {
            Some(Value::String(s.clone()))
        }
        _ => None,
    }
}

/// Port of problem_notes.send.
pub fn send(
    plan: &WritePlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let payload = plan.payload();
    let body = payload.get("body").cloned().unwrap_or(Value::Null);
    let body_text = body.as_str().unwrap_or_default().to_string();
    let tenant = plan.tenant.clone();
    let rebuild = || -> Result<String> {
        let mut p = build_plan(journal, &tenant, &plan.target.to_string(), &body_text)?;
        p.tenant = tenant.clone();
        Ok(p.hash())
    };
    let verify = |response: &Value| -> Result<Map<String, Value>> {
        let nid = returned_id(response);
        let mut out = obj(json!({
            "note_id": nid, "verified": false, "readback": null,
            "identity_verified": false, "readback_complete": false,
        }));
        let Some(nid) = nid else {
            out.insert(
                "warning".into(),
                json!("note response has no exact note ID; outcome unknown, check the problem before any retry"),
            );
            return Ok(out);
        };
        let params = obj(json!({"page": 1, "per_page": 30, "include": "user"}));
        let pages = collect_pages(
            |p, q| c.int_get(p, q),
            &format!("problems/{}/notes", plan.target),
            "notes",
            Some(&params),
            "notes",
            50,
        )?;
        let want = fsv_py::py_str(&nid);
        let exact = pages
            .rows
            .iter()
            .find(|r| r.is_object() && fsv_py::py_str(r.get("id").unwrap_or(&Value::Null)) == want)
            .cloned();
        let complete =
            pages.missing.is_none() && !pages.meta.get("partial").is_some_and(fsv_py::truthy);
        out.insert("readback".into(), exact.clone().unwrap_or(Value::Null));
        out.insert("readback_complete".into(), json!(complete));
        out.insert("identity_verified".into(), json!(exact.is_some()));
        if let Some(exact) = exact {
            let actual = if exact.get("body").is_some() {
                exact.get("body")
            } else {
                exact.get("body_html")
            };
            let state = match actual {
                None | Some(Value::Null) => "unavailable",
                Some(v) if same_text(v, &body) => "matched",
                Some(_) => "mismatched",
            };
            out.insert("fields".into(), json!({"body": state}));
            out.insert("verified".into(), json!(state == "matched" && complete));
        }
        Ok(out)
    };
    let guards: Vec<String> = if plan.context_hash.is_empty() {
        Vec::new()
    } else {
        let mut older = plan.clone();
        older.context_hash = String::new();
        vec![older.hash()]
    };
    execute_guarded(
        plan,
        &plan.hash(),
        c,
        journal,
        reviewed_hash,
        &rebuild,
        &verify,
        None,
        &guards,
    )
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::service::testing::*;
    use fsv_core::ApiError;

    const T: &str = "acme.freshservice.com";

    /// `test_problem_add_note._setup`: a note list that grows on POST.
    fn world(
        stored: Option<&'static str>,
        post_status: Option<u16>,
    ) -> (FnApi, Arc<Mutex<Vec<Value>>>) {
        let notes: Arc<Mutex<Vec<Value>>> = Arc::default();
        let shared = notes.clone();
        let api = FnApi::new(move |call| {
            if call.method == "GET" {
                let rows = shared.lock().unwrap().clone();
                return Ok(json!({"notes": rows, "meta": {"total_count": rows.len()}}));
            }
            if let Some(status) = post_status {
                return Err(ApiError::new(
                    status,
                    json!({"description": "Validation failed",
                    "errors": [{"field": "body", "message": "Synthetic body too long"}]}),
                )
                .into());
            }
            let body = call.body.clone().unwrap_or(Value::Null);
            let text = body["body"].as_str().unwrap_or_default();
            let stored = stored.map_or_else(|| format!("<div>{text}</div>"), str::to_string);
            let mut rows = shared.lock().unwrap();
            let id = 9000000077i64 + rows.len() as i64;
            rows.push(json!({"id": id, "body": stored, "user_id": 7}));
            Ok(json!({"note": {"id": id, "body": text}}))
        });
        (api, notes)
    }

    fn plan(j: &MemJournal) -> WritePlan {
        build_plan(j, T, "PRB-12", "Acme evidence").unwrap()
    }

    #[test]
    fn reviewed_send_posts_once_and_verifies_by_listing_notes() {
        let (c, _) = world(None, None);
        let j = MemJournal::default();
        let p = plan(&j);
        let r = send(&p, &c, &j, Some(&p.hash())).unwrap();
        assert_eq!(r["verified"], true);
        assert_eq!(r["fields"], json!({"body": "matched"}));
        assert_eq!(r["readback_complete"], true);
        assert_eq!(r["plan_hash"], json!(p.hash()));
        assert_eq!(c.writes().len(), 1);
        assert_eq!(c.writes()[0].path, "problems/12/notes");
        assert_eq!(c.writes()[0].body, Some(json!({"body": "Acme evidence"})));
        assert_eq!(j.events(), ["started", "done"]);
        let again = send(&p, &c, &j, Some(&p.hash())).unwrap_err().to_string();
        assert!(again.contains("already sent at") || again.contains("nothing was sent now"));
        assert_eq!(c.writes().len(), 1);
    }

    #[test]
    fn repeat_after_a_verified_send_plans_a_fresh_hash() {
        let (c, _) = world(None, None);
        let j = MemJournal::default();
        let first = plan(&j);
        assert_eq!(review(&first)["record_state"], json!({"sends": 0}));
        send(&first, &c, &j, Some(&first.hash())).unwrap();
        let second = plan(&j);
        assert_ne!(second.hash(), first.hash());
        assert_eq!(review(&second)["record_state"], json!({"sends": 1}));
        assert_eq!(second.context_hash, r#"{"sends": 1}"#);
        assert_eq!(
            send(&second, &c, &j, Some(&second.hash())).unwrap()["verified"],
            true
        );
        assert_eq!(c.writes().len(), 2);
        let stale = j
            .stale_plan_refusal(Some(&first.hash()), &[ACTION])
            .unwrap();
        assert!(stale.contains("already sent at"));
        assert_eq!(j.stale_plan_refusal(Some(&"0".repeat(64)), &[ACTION]), None);
    }

    #[test]
    fn unresolved_pre_token_plan_still_refuses() {
        let (c, _) = world(None, None);
        let j = MemJournal::default();
        let first = plan(&j);
        j.record_verified("x", ACTION, Some(&json!(12)));
        let second = plan(&j);
        assert_eq!(second.context_hash, r#"{"sends": 1}"#);
        j.record_plan("started", &first.hash(), ACTION, Some(&json!(12)), None);
        let e = send(&second, &c, &j, Some(&second.hash()))
            .unwrap_err()
            .to_string();
        assert!(e.contains("outcome unknown"));
        assert!(c.writes().is_empty());
    }

    #[test]
    fn body_mismatch_and_missing_note_are_not_verified() {
        let (c, _) = world(Some("<div>Globex text</div>"), None);
        let j = MemJournal::default();
        let p = plan(&j);
        let r = send(&p, &c, &j, Some(&p.hash())).unwrap();
        assert_eq!(
            (r["verified"].clone(), r["fields"].clone()),
            (json!(false), json!({"body": "mismatched"}))
        );
        assert!(r["warning"].as_str().unwrap().contains("do not resend"));
        assert!(j.verified.lock().unwrap().is_empty());

        let c = FnApi::new(|call| {
            Ok(if call.method == "GET" {
                json!({"notes": [], "meta": {"total_count": 0}})
            } else {
                json!({"note": {"id": 9000000077i64}})
            })
        });
        let j = MemJournal::default();
        let p = plan(&j);
        let r = send(&p, &c, &j, Some(&p.hash())).unwrap();
        assert_eq!(
            (r["verified"].clone(), r["identity_verified"].clone()),
            (json!(false), json!(false))
        );
    }

    #[test]
    fn server_rejection_names_status_and_message() {
        let (c, _) = world(None, Some(422));
        let j = MemJournal::default();
        let p = plan(&j);
        let e = send(&p, &c, &j, Some(&p.hash())).unwrap_err().to_string();
        assert!(
            e.contains("HTTP 422") && e.contains("Synthetic body too long"),
            "{e}"
        );
        assert_eq!(j.events(), ["started", "failed"]);
    }

    #[test]
    fn response_without_a_note_id_is_unknown() {
        let c = FnApi::new(|call| {
            Ok(if call.method == "GET" {
                json!({"notes": []})
            } else {
                json!({})
            })
        });
        let j = MemJournal::default();
        let p = plan(&j);
        let r = send(&p, &c, &j, Some(&p.hash())).unwrap();
        assert_eq!(r["verified"], false);
        assert!(r["warning"].as_str().unwrap().contains("no exact note ID"));
    }

    #[test]
    fn plan_is_the_captured_body_only() {
        let j = MemJournal::default();
        let p = plan(&j);
        assert_eq!(
            (p.method.as_str(), p.path.as_str(), p.payload()),
            (
                "POST",
                "problems/12/notes",
                json!({"body": "Acme evidence"})
            )
        );
        assert_eq!(p.hash().len(), 64);
        for bad in ["", "  ", "a\0b"] {
            assert!(build_plan(&j, T, "PRB-12", bad).is_err());
        }
        assert!(build_plan(&j, T, "PRB-0", "x").is_err());
    }
}
