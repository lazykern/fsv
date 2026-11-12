//! Reviewed ticket notes using the blocked UI contract, not the reply
//! endpoint (`ticket_notes.py`). No accepted live write is implied.

use serde_json::{Map, Value, json};

use fsv_core::client::Body;
use fsv_core::error::{Error, Result};
use fsv_core::resource::{TICKETS, parse_id};

use crate::associations::SendOpts;
use crate::service::{
    Api, Journal, definite_rejection, dumps_bytes, last_event, load_conversations, obj, plan_hash,
};

const ACTION: &str = "ticket_note";
const UNKNOWN: &str = "note outcome unknown; check the exact ticket and journal before any retry";

pub use crate::recipients::{EMAIL, valid_recipient};

/// `NoteError`: a safe user-facing failure (the CLI exits 2).
pub(crate) fn refused(msg: impl Into<String>) -> Error {
    Error::exit(2, msg)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyFormat {
    Plain,
    Html,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotePlan {
    pub ticket_id: i64,
    pub body_html: String,
    pub public: bool,
    pub notify_emails: Vec<String>,
    pub tenant: String,
}

impl NotePlan {
    pub fn path(&self) -> String {
        format!("tickets/{}/notes", self.ticket_id)
    }

    pub fn payload(&self) -> Value {
        json!({"body": self.body_html, "conversation_from_summary": false,
               "private": !self.public, "notify_emails": self.notify_emails})
    }

    pub fn hash(&self) -> String {
        plan_hash(&json!({"tenant": self.tenant, "path": self.path(), "payload": self.payload()}))
    }

    pub fn review(&self) -> Map<String, Value> {
        obj(json!({
            "action": "ticket_note", "tenant": self.tenant, "ticket_id": self.ticket_id,
            "visibility": if self.public { "public note (visible to customer)" } else { "private note" },
            "path": format!("/api/_/{}", self.path()), "method": "POST", "payload": self.payload(),
            "body_text": fsv_core::output::strip_html(Some(&self.body_html)), "plan_hash": self.hash(),
            "risks": fsv_core::safety::note_risks(self.public),
            "warning": "Not a reply. Empty notify_emails does not disable tenant workflows or guarantee no notifications.",
        }))
    }
}

/// `build_plan`: pure local preparation. Plain text is escaped into div
/// paragraphs with `<br>` line breaks; explicit HTML is kept.
pub fn build_plan(
    ticket_id: &str,
    body: &str,
    public: bool,
    notify_emails: &[String],
    format: BodyFormat,
    tenant: &str,
) -> Result<NotePlan> {
    let tid = parse_id(ticket_id, &TICKETS)?;
    if tid <= 0 {
        return Err(Error::value("ticket ID must be positive"));
    }
    if fsv_core::pystr::strip(body).is_empty() || body.contains('\0') {
        return Err(Error::value(
            "note body must be nonempty text without NUL characters",
        ));
    }
    let body_html = match format {
        BodyFormat::Html => {
            if fsv_core::output::strip_html(Some(body)).is_empty() {
                return Err(Error::value("HTML note body must contain visible text"));
            }
            body.to_string()
        }
        BodyFormat::Plain => body
            .replace("\r\n", "\n")
            .split("\n\n")
            .filter(|p| !fsv_core::pystr::strip(p).is_empty())
            .map(|p| {
                format!(
                    "<div>{}</div>",
                    fsv_core::pystr::html_escape(p, true).replace('\n', "<br>")
                )
            })
            .collect(),
    };
    if notify_emails.iter().any(|e| !valid_recipient(e)) {
        return Err(Error::value(
            "notify_emails contains an invalid email address",
        ));
    }
    Ok(NotePlan {
        ticket_id: tid,
        body_html,
        public,
        notify_emails: notify_emails.to_vec(),
        tenant: tenant.to_string(),
    })
}

pub(crate) fn check_plan(plan: &NotePlan, tenant: &str) -> Result<()> {
    let new = build_plan(
        &plan.ticket_id.to_string(),
        &plan.body_html,
        plan.public,
        &plan.notify_emails,
        BodyFormat::Html,
        tenant,
    )?;
    if new != *plan {
        return Err(refused(
            "note plan or tenant changed; review a new plan before sending",
        ));
    }
    Ok(())
}

/// `_duplicate`: refuse a plan whose journal shows it started or finished.
pub(crate) fn duplicate(journal: &dyn Journal, plan: &NotePlan) -> Result<()> {
    if matches!(
        last_event(journal, ACTION, &plan.hash()).as_deref(),
        Some("started" | "done")
    ) {
        return Err(refused(
            "this note was sent or its outcome is unknown; check the exact ticket and journal before any retry",
        ));
    }
    Ok(())
}

/// `_returned_note`: supported response envelopes, not a live contract.
pub(crate) fn returned_note(response: &Value) -> Option<&Value> {
    let m = response.as_object()?;
    for key in ["note", "conversation"] {
        if let Some(v) = m.get(key).filter(|v| v.is_object()) {
            return Some(v);
        }
    }
    m.contains_key("id").then_some(response)
}

/// `_note_id`: a positive int, or a digit string kept as a string.
pub(crate) fn note_id(note: Option<&Value>) -> Option<Value> {
    match note?.get("id")? {
        v @ Value::Number(n) if n.as_i64().is_some_and(|i| i > 0) => Some(v.clone()),
        v @ Value::String(s)
            if !s.is_empty()
                && s.bytes().all(|b| b.is_ascii_digit())
                && s.parse::<u128>().is_ok_and(|n| n > 0) =>
        {
            Some(v.clone())
        }
        _ => None,
    }
}

/// `send`: at most one POST after exact-body review and separate public
/// consent. Read-back matches only the returned id.
pub fn send(
    plan: &NotePlan,
    c: &dyn Api,
    journal: &dyn Journal,
    opts: SendOpts<'_>,
    public_confirmed: bool,
) -> Result<Value> {
    check_plan(plan, &c.domain())?;
    if opts.dry_run {
        let mut out = obj(json!({"status": "dry_run"}));
        out.extend(plan.review());
        return Ok(Value::Object(out));
    }
    if opts.read_only {
        return Err(refused("read-only mode: no note was sent"));
    }
    let Some(reviewed) = opts.reviewed_hash else {
        return Ok(json!({"status": "cancelled", "ticket_id": plan.ticket_id}));
    };
    let hash = plan.hash();
    if reviewed != hash {
        return Err(refused(
            "note review hash does not match; review the exact body again",
        ));
    }
    if plan.public && !public_confirmed {
        return Err(refused(
            "public note requires separate explicit confirmation; no note was sent",
        ));
    }
    if plan.tenant.is_empty() {
        return Err(refused("configure a tenant before sending a note"));
    }
    duplicate(journal, plan)?;
    let Ok(csrf) = c.get_csrf() else {
        return Err(refused(
            "could not prepare the session; no note POST was attempted",
        ));
    };
    let target = json!(plan.ticket_id);
    journal.record_plan("started", &hash, ACTION, Some(&target), None);
    if last_event(journal, ACTION, &hash).as_deref() != Some("started") {
        return Err(refused(
            "could not persist the note journal; no note was sent",
        ));
    }
    let response = match c.send_once(
        "POST",
        &plan.path(),
        Body::Raw(dumps_bytes(&plan.payload(), false)),
        None,
        &csrf,
    ) {
        Ok(r) => r,
        Err(e) => {
            if let Some(status) = definite_rejection(&e) {
                journal.record_plan("failed", &hash, ACTION, Some(&target), None);
                return Err(refused(format!(
                    "note rejected (HTTP {status}); no automatic retry"
                )));
            }
            return Err(refused(UNKNOWN));
        }
    };
    let note = returned_note(&response).cloned();
    let Some(nid) = note_id(note.as_ref()) else {
        return Err(refused(
            "note response has no exact note ID; outcome unknown, check the ticket before any retry",
        ));
    };
    journal.record_plan("done", &hash, ACTION, Some(&target), Some(&nid));
    let mut result = obj(json!({
        "status": "submitted", "ticket_id": plan.ticket_id, "note_id": nid, "note": note,
        "plan_hash": hash, "verified": false, "readback": null,
        "identity_verified": false, "fields_verified": false, "readback_complete": false,
    }));
    match load_conversations(c, plan.ticket_id) {
        Ok(pages) => {
            let want = fsv_py::py_str(&nid);
            let exact = pages
                .rows
                .iter()
                .find(|r| note_id(Some(r)).is_some() && fsv_py::py_str(&r["id"]) == want)
                .cloned();
            let complete =
                pages.missing.is_none() && !pages.meta.get("partial").is_some_and(fsv_py::truthy);
            result.insert("readback".into(), exact.clone().unwrap_or(Value::Null));
            result.insert("readback_complete".into(), json!(complete));
            result.insert("identity_verified".into(), json!(exact.is_some()));
            if let Some(exact) = exact {
                let expected = [
                    ("body", json!(plan.body_html)),
                    ("private", json!(!plan.public)),
                    ("notify_emails", json!(plan.notify_emails)),
                ];
                let mut fields = Map::new();
                for (key, value) in expected {
                    let actual_key = if key == "notify_emails"
                        && exact.get(key).is_none()
                        && exact.get("to_emails").is_some()
                    {
                        "to_emails"
                    } else {
                        key
                    };
                    let state = match exact.get(actual_key) {
                        None => "unavailable",
                        Some(v) if key == "body" && crate::readback::same_text(v, &value) => {
                            "matched"
                        }
                        Some(v) if key != "body" && *v == value => "matched",
                        Some(_) => "mismatched",
                    };
                    fields.insert(key.into(), json!(state));
                }
                let fields_verified = fields.values().all(|v| v == "matched");
                result.insert("fields".into(), Value::Object(fields));
                result.insert("fields_verified".into(), json!(fields_verified));
                result.insert("verified".into(), json!(fields_verified && complete));
            }
            if result["verified"] != Value::Bool(true) {
                result.insert(
                    "warning".into(),
                    json!(
                        "Read-back did not fully verify the note fields and history; do not resend."
                    ),
                );
            }
        }
        Err(_) => {
            result.insert(
                "warning".into(),
                json!("Note returned an ID but conversation read-back failed; do not resend."),
            );
        }
    }
    Ok(Value::Object(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::testing::*;

    const T: &str = "acme.freshservice.com";

    #[test]
    fn plain_body_escapes_paragraphs() {
        let p = build_plan(
            "INC-5",
            "a < b\r\nline2\n\n\n\nnext \"q\"",
            false,
            &[],
            BodyFormat::Plain,
            T,
        )
        .unwrap();
        assert_eq!(
            p.body_html,
            "<div>a &lt; b<br>line2</div><div>next &quot;q&quot;</div>"
        );
        assert_eq!(
            p.payload(),
            json!({"body": p.body_html, "conversation_from_summary": false, "private": true, "notify_emails": []})
        );
        assert_eq!(
            p.hash(),
            plan_hash(&json!({"tenant": T, "path": "tickets/5/notes", "payload": p.payload()}))
        );
    }

    #[test]
    fn validation_messages() {
        let e = |r: Result<NotePlan>| r.unwrap_err().to_string();
        assert_eq!(
            e(build_plan("5", "  ", false, &[], BodyFormat::Plain, T)),
            "note body must be nonempty text without NUL characters"
        );
        assert_eq!(
            e(build_plan("5", "<p> </p>", false, &[], BodyFormat::Html, T)),
            "HTML note body must contain visible text"
        );
        assert_eq!(
            e(build_plan(
                "5",
                "x",
                false,
                &["bad".into()],
                BodyFormat::Plain,
                T
            )),
            "notify_emails contains an invalid email address"
        );
        assert!(
            build_plan(
                "5",
                "x",
                false,
                &["Ann Lee <ann@x.io>".into(), "b@y.co".into()],
                BodyFormat::Plain,
                T
            )
            .is_ok()
        );
        assert!(build_plan("CHN-5", "x", false, &[], BodyFormat::Plain, T).is_err());
    }

    /// Server-normalized readback (`gen_fsv_api_note_readback.py`).
    #[test]
    fn normalized_readback_matches_python() {
        let oracle: Value =
            serde_json::from_str(include_str!("testdata/note_readback_oracle.json")).unwrap();
        for (name, case) in oracle.as_object().unwrap() {
            if name == "_python" {
                continue;
            }
            let mut row = json!({"id": 99001, "body": case["row_body"], "private": true});
            match &case["recipients"] {
                Value::Null => {}
                Value::String(_) => {
                    row["notify_emails"] = json!([]);
                    row["to_emails"] = json!(["x@example.com"]);
                }
                r => row["to_emails"] = r.clone(),
            }
            let api = FakeApi::new()
                .ok("POST tickets/123/notes", json!({"note": {"id": 99001}}))
                .ok(
                    "GET tickets/123/conversations",
                    json!({"conversations": [row], "meta": {"count": 1}}),
                );
            let j = MemJournal::default();
            let p = build_plan("123", "Acme & Globex", false, &[], BodyFormat::Plain, T).unwrap();
            assert_eq!(p.hash(), case["plan_hash"], "{name}");
            let h = p.hash();
            let out = send(
                &p,
                &api,
                &j,
                SendOpts {
                    reviewed_hash: Some(&h),
                    ..Default::default()
                },
                false,
            )
            .unwrap();
            assert_eq!(out, case["result"], "{name}");
            assert_eq!(
                api.writes()[0].body,
                Some(case["requests"][0]["body"].clone()),
                "{name}"
            );
        }
    }

    #[test]
    fn send_reads_back_by_id() {
        let api = FakeApi::new()
            .ok("POST tickets/5/notes", json!({"conversation": {"id": "77"}}))
            .ok("GET tickets/5/conversations", json!({"conversations": [{"id": 77, "body": "<div>hi</div>", "private": true, "notify_emails": []}]}));
        let j = MemJournal::default();
        let p = build_plan("5", "hi", false, &[], BodyFormat::Plain, T).unwrap();
        let h = p.hash();
        let out = send(
            &p,
            &api,
            &j,
            SendOpts {
                reviewed_hash: Some(&h),
                ..Default::default()
            },
            false,
        )
        .unwrap();
        assert_eq!(out["note_id"], "77");
        assert_eq!(out["verified"], true);
        assert_eq!(
            out["fields"],
            json!({"body": "matched", "private": "matched", "notify_emails": "matched"})
        );
        assert_eq!(j.events(), ["started", "done"]);
        let raw = api.writes()[0].raw.clone().unwrap();
        assert_eq!(
            String::from_utf8(raw).unwrap(),
            r#"{"body": "<div>hi</div>", "conversation_from_summary": false, "private": true, "notify_emails": []}"#
        );
    }

    #[test]
    fn valid_recipient_forms() {
        let got: Vec<bool> = [
            "a@b.co",
            "Ann Lee <ann@x.io>",
            "bad",
            "a@b",
            "x\n@y.co",
            "Ann <a@b.co> ",
            "<a@b.co>",
        ]
        .iter()
        .map(|v| valid_recipient(v))
        .collect();
        assert_eq!(got, [true, true, false, false, false, false, false]);
    }

    #[test]
    fn plan_hash_review_and_ids_match_python() {
        let p = build_plan(
            "INC-5",
            "a < b\r\nline2\n\n\n\nnext \"q\"",
            false,
            &[],
            BodyFormat::Plain,
            T,
        )
        .unwrap();
        assert_eq!(
            p.hash(),
            "9d9e097c80db19d0b3df42b6805108b75e3909f53d1437262e4539930b418e4d"
        );
        assert_eq!(p.path(), "tickets/5/notes");
        let h = build_plan(
            "5",
            "<p>x</p>",
            true,
            &["b@y.co".into()],
            BodyFormat::Html,
            T,
        )
        .unwrap();
        assert_eq!(
            Value::Object(h.review()),
            json!({"action": "ticket_note", "tenant": T, "ticket_id": 5, "visibility": "public note (visible to customer)",
                   "path": "/api/_/tickets/5/notes", "method": "POST",
                   "payload": {"body": "<p>x</p>", "conversation_from_summary": false, "private": false, "notify_emails": ["b@y.co"]},
                   "body_text": "x", "plan_hash": "45f1cf2e653ffd77e490218f577da59c65af9ea215f5c1a32cef61386d9df9bd",
                   "risks": ["public note: visible to the requester and may send notifications"],
                   "warning": "Not a reply. Empty notify_emails does not disable tenant workflows or guarantee no notifications."})
        );
        assert_eq!(
            build_plan("CHN-5", "x", false, &[], BodyFormat::Plain, T)
                .unwrap_err()
                .to_string(),
            "prefix CHN not valid for tickets (expect INC or SR)"
        );
        assert_eq!(
            build_plan("0", "x", false, &[], BodyFormat::Plain, T)
                .unwrap_err()
                .to_string(),
            "ticket ID must be positive"
        );
    }

    #[test]
    fn check_plan_rebuilds_and_compares() {
        let good = build_plan("5", "hi", false, &[], BodyFormat::Plain, T).unwrap();
        assert!(check_plan(&good, T).is_ok());
        let odd = NotePlan {
            body_html: "<div>x</div><script>".into(),
            ..good.clone()
        };
        assert!(check_plan(&odd, T).is_ok());
        let other = NotePlan {
            tenant: "other.freshservice.com".into(),
            ..good
        };
        assert_eq!(
            check_plan(&other, T).unwrap_err().to_string(),
            "note plan or tenant changed; review a new plan before sending"
        );
    }

    #[test]
    fn duplicate_blocks_started_and_done_only() {
        let good = build_plan("5", "hi", false, &[], BodyFormat::Plain, T).unwrap();
        let j = MemJournal::default();
        assert!(duplicate(&j, &good).is_ok());
        j.record_plan("started", &good.hash(), ACTION, None, None);
        assert_eq!(
            duplicate(&j, &good).unwrap_err().to_string(),
            "this note was sent or its outcome is unknown; check the exact ticket and journal before any retry"
        );
        j.record_plan("failed", &good.hash(), ACTION, None, None);
        assert!(duplicate(&j, &good).is_ok());
    }

    #[test]
    fn returned_note_and_note_id() {
        let rs = [
            json!({"note": {"id": 1}}),
            json!({"conversation": {"id": 2}, "note": "x"}),
            json!({"id": 3}),
            json!({"x": 1}),
            json!([1]),
        ];
        let got: Vec<Option<Value>> = rs.iter().map(|r| returned_note(r).cloned()).collect();
        assert_eq!(
            got,
            [
                Some(json!({"id": 1})),
                Some(json!({"id": 2})),
                Some(json!({"id": 3})),
                None,
                None
            ]
        );
        let ids = [
            json!({"id": 1}),
            json!({"id": "77"}),
            json!({"id": "0"}),
            json!({"id": 0}),
            json!({"id": true}),
            json!({"id": "7a"}),
            json!({"id": 1.0}),
        ];
        let got: Vec<Option<Value>> = ids.iter().map(|n| note_id(Some(n))).collect();
        assert_eq!(
            got,
            [
                Some(json!(1)),
                Some(json!("77")),
                None,
                None,
                None,
                None,
                None
            ]
        );
        assert_eq!(note_id(None), None);
    }

    #[test]
    fn send_full_result_and_failed_readback() {
        let api = FakeApi::new()
            .ok("POST tickets/5/notes", json!({"conversation": {"id": "77"}}))
            .ok("GET tickets/5/conversations", json!({"conversations": [{"id": 77, "body": "<div>hi</div>", "private": true, "notify_emails": []}]}));
        let p = build_plan("5", "hi", false, &[], BodyFormat::Plain, T).unwrap();
        let h = p.hash();
        assert_eq!(
            h,
            "6a1cb720ba3db32fb4b46fc60d96a34dd122fce1e3692e10f5182cc18ad61962"
        );
        let opts = SendOpts {
            reviewed_hash: Some(&h),
            ..Default::default()
        };
        assert_eq!(
            send(&p, &api, &MemJournal::default(), opts, false).unwrap(),
            json!({"status": "submitted", "ticket_id": 5, "note_id": "77", "note": {"id": "77"}, "plan_hash": h, "verified": true,
                   "readback": {"id": 77, "body": "<div>hi</div>", "private": true, "notify_emails": []},
                   "identity_verified": true, "fields_verified": true, "readback_complete": true,
                   "fields": {"body": "matched", "private": "matched", "notify_emails": "matched"}})
        );
        let api = FakeApi::new()
            .ok("POST tickets/5/notes", json!({"note": {"id": 9}}))
            .on("GET tickets/5/conversations", Reply::Status(500));
        assert_eq!(
            send(&p, &api, &MemJournal::default(), opts, false).unwrap(),
            json!({"status": "submitted", "ticket_id": 5, "note_id": 9, "note": {"id": 9}, "plan_hash": h, "verified": false,
                   "readback": null, "identity_verified": false, "fields_verified": false, "readback_complete": false,
                   "warning": "Read-back did not fully verify the note fields and history; do not resend."})
        );
    }

    #[test]
    fn public_needs_confirmation_and_unknown_outcomes_stay_started() {
        let api = FakeApi::new().on("POST tickets/5/notes", Reply::Status(500));
        let j = MemJournal::default();
        let p = build_plan("5", "hi", true, &[], BodyFormat::Plain, T).unwrap();
        let h = p.hash();
        let opts = SendOpts {
            reviewed_hash: Some(&h),
            ..Default::default()
        };
        assert_eq!(
            send(&p, &api, &j, opts, false).unwrap_err().to_string(),
            "public note requires separate explicit confirmation; no note was sent"
        );
        assert_eq!(
            send(&p, &api, &j, opts, true).unwrap_err().to_string(),
            UNKNOWN
        );
        assert_eq!(j.events(), ["started"]);
        assert_eq!(
            send(
                &p,
                &api,
                &j,
                SendOpts {
                    dry_run: true,
                    ..opts
                },
                false
            )
            .unwrap()["status"],
            "dry_run"
        );
    }
}
