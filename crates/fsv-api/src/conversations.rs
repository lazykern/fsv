//! Reviewed reply or note-with-status from blocked UI submissions
//! (`ticket_conversation.py`).

use serde_json::{Map, Value, json};

use fsv_core::client::Body;
use fsv_core::error::{Error, Result};
use fsv_forms::payload::format_requester;
use fsv_forms::{FormState, build_br_validation_excludes};
use fsv_py::{py_str, truthy};

use crate::formmeta::{MetaStore, RULES_UNAVAILABLE_KEY, existing_from_record, is_status, load_meta, resolve_requester};
use crate::notes::{self, BodyFormat, EMAIL, note_id, returned_note, valid_recipient};
use crate::properties::{dumps_sorted, record};
use crate::service::{Api, Journal, definite_rejection, last_event, load_conversations, obj, plan_hash};

/// `ConversationError`: a safe user-facing failure.
fn refused(msg: impl Into<String>) -> Error {
    Error::exit(2, msg)
}

const UNKNOWN: &str = "conversation outcome unknown; inspect ticket and journal before retry";

/// `recipients`: each bare email resolved to exactly one non-deleted user,
/// formatted `Name <email>`.
pub fn recipients(c: &dyn Api, emails: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for email in emails {
        if !EMAIL.is_match(email) {
            return Err(Error::value("CC must be a plain email address"));
        }
        let one = || -> Result<String> {
            let person = resolve_requester(c, email, None)?;
            let pe = person.email.clone().unwrap_or_default();
            if pe.is_empty() || fsv_core::pystr::casefold(&pe) != fsv_core::pystr::casefold(email) {
                return Err(Error::value("not an exact email match"));
            }
            let value = format_requester(&pe, Some(&person.display)).map_err(|e| Error::value(e.to_string()))?;
            if !valid_recipient(&value) {
                return Err(Error::value("invalid formatted address"));
            }
            Ok(value)
        };
        out.push(one().map_err(|_| refused("each CC email must resolve to exactly one non-deleted user with a valid name and email"))?);
    }
    Ok(out)
}

fn status_inner(c: &dyn Api, store: &dyn MetaStore, cid: i64, label: &str) -> Result<(i64, String, String)> {
    let bad = |m: &str| Error::value(m.to_string());
    let row = record(c, cid)?;
    let meta = load_meta(c, store, "ticket", "edit", true, &json!(cid))?;
    if meta.rules.get(RULES_UNAVAILABLE_KEY).is_some_and(truthy) {
        return Err(bad("rules unavailable"));
    }
    let spec = meta.spec();
    let mut state: FormState = meta.state("edit", None, Some(existing_from_record(&spec, &row)))?;
    let fields: Vec<_> = spec.fields().iter().filter(|f| is_status(f)).collect();
    if fields.len() != 1 {
        return Err(bad("status field not unique"));
    }
    let field = fields[0];
    let want = fsv_core::pystr::casefold(label);
    let choices: Vec<_> = field
        .choices
        .iter()
        .filter(|ch| ch.visible && (fsv_core::pystr::casefold(&ch.label) == want || fsv_core::pystr::casefold(&ch.value) == want))
        .collect();
    let id = match choices.as_slice() {
        [one] => one.id.as_ref().and_then(|v| if v.is_i64() || v.is_u64() { v.as_i64() } else { None }).filter(|i| *i > 0),
        _ => None,
    };
    let Some(id) = id else { return Err(bad("status choice not unique")) };
    state.set(&field.api_name, fsv_forms::Value::from(choices[0].value.clone())).map_err(|e| Error::other(e.to_string()))?;
    let errors = state.errors().map_err(|e| Error::other(e.to_string()))?;
    if !state.evaluated.visible.contains(&field.api_name)
        || !errors.is_empty()
        || !state.rules.unavailable_dependencies("edit").is_empty()
        || state.rules.warnings().iter().any(|w| w.contains("unknown field") || w.contains("none was given"))
    {
        return Err(bad("status form rules or context unavailable"));
    }
    let token = build_br_validation_excludes(&state.evaluated, "ticket", &Value::Null, &Value::Null).map_err(|e| Error::other(e.to_string()))?;
    Ok((id, token, plan_hash(&Value::Object(row))))
}

/// `status_context`: the Pending status id, its business-rule token and the
/// record hash, from fresh edit-form metadata. Only Pending was captured.
pub fn status_context(c: &dyn Api, store: &dyn MetaStore, cid: i64, label: &str) -> Result<(i64, String, String)> {
    if fsv_core::pystr::casefold(label) != "pending" {
        return Err(Error::value("only the Pending status transition was captured; use the web form for other statuses"));
    }
    status_inner(c, store, cid, label)
        .map_err(|_| refused("could not resolve Pending and evaluate ticket rules from fresh metadata; no send attempted"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationPlan {
    /// `reply` | `note`.
    pub kind: String,
    pub ticket_id: i64,
    pub tenant: String,
    pub payload_json: String,
    pub status_label: Option<String>,
    pub record_hash: Option<String>,
}

impl ConversationPlan {
    pub fn payload(&self) -> Value {
        serde_json::from_str(&self.payload_json).unwrap_or(Value::Null)
    }

    pub fn path(&self) -> String {
        format!("tickets/{}/{}", self.ticket_id, if self.kind == "reply" { "reply" } else { "notes" })
    }

    pub fn hash(&self) -> String {
        plan_hash(&json!({"tenant": self.tenant, "path": self.path(), "payload": self.payload(), "record_hash": self.record_hash}))
    }

    pub fn review(&self) -> Map<String, Value> {
        obj(json!({"action": format!("ticket_{}", self.kind), "method": "POST", "path": format!("/api/_/{}", self.path()),
                   "payload": self.payload(), "status_label": self.status_label, "plan_hash": self.hash(),
                   "warning": "A reply is public and may notify recipients; notes may also trigger tenant workflows."}))
    }
}

/// Options of [`build_plan`] (Python keyword arguments).
#[derive(Debug, Clone)]
pub struct ConversationArgs<'a> {
    /// `reply` | `note`.
    pub kind: &'a str,
    pub cc: &'a [String],
    pub public: bool,
    pub notify: &'a [String],
    pub format: BodyFormat,
    pub set_status: Option<&'a str>,
}

impl Default for ConversationArgs<'_> {
    fn default() -> Self {
        ConversationArgs { kind: "reply", cc: &[], public: false, notify: &[], format: BodyFormat::Plain, set_status: None }
    }
}

/// `build_plan`: the exact reply/note payload, with the status change and
/// its rules token when `set_status` is given.
pub fn build_plan(c: &dyn Api, store: &dyn MetaStore, ticket_id: &str, body: &str, args: &ConversationArgs<'_>) -> Result<ConversationPlan> {
    if args.kind != "reply" && args.kind != "note" {
        return Err(Error::value("invalid conversation kind"));
    }
    let tenant = c.domain();
    let note = notes::build_plan(ticket_id, body, args.public, args.notify, args.format, &tenant)?;
    let mut payload = if args.kind == "reply" {
        if args.cc.iter().any(|v| !valid_recipient(v)) {
            return Err(Error::value("invalid CC recipient"));
        }
        json!({"body": note.body_html, "conversation_from_summary": false, "cc_emails": args.cc,
               "full_text": note.body_html, "translate_reply": false, "freddy_message_id": null})
    } else {
        note.payload()
    };
    let mut record_hash = None;
    if let Some(label) = args.set_status {
        let (status_id, token, rh) = status_context(c, store, note.ticket_id, label)?;
        record_hash = Some(rh);
        payload["helpdesk_ticket"] = json!({"status": status_id, "br_validation_excludes": token});
    }
    Ok(ConversationPlan {
        kind: args.kind.to_string(),
        ticket_id: note.ticket_id,
        tenant,
        payload_json: dumps_sorted(&payload),
        status_label: args.set_status.map(str::to_string),
        record_hash,
    })
}

/// `send`: one POST after review, public consent and a fresh status check,
/// then a read-back by the returned conversation id.
pub fn send(plan: &ConversationPlan, c: &dyn Api, journal: &dyn Journal, store: &dyn MetaStore, reviewed_hash: Option<&str>, public_confirmed: bool) -> Result<Value> {
    let Some(reviewed) = reviewed_hash else { return Ok(json!({"status": "cancelled"})) };
    let hash = plan.hash();
    if hash != reviewed || plan.tenant.is_empty() || plan.tenant != c.domain() {
        return Err(refused("tenant or review hash changed; review again"));
    }
    let payload = plan.payload();
    let private = payload.get("private").is_none_or(truthy);
    if (plan.kind == "reply" || !private) && !public_confirmed {
        return Err(refused("public conversation requires explicit confirmation"));
    }
    let action = format!("ticket_{}_reviewed", plan.kind);
    if matches!(last_event(journal, &action, &hash).as_deref(), Some("started" | "done")) {
        return Err(refused("conversation was sent or outcome unknown; inspect ticket and journal before retry"));
    }
    if let Some(label) = &plan.status_label {
        let (status_id, token, record_hash) = status_context(c, store, plan.ticket_id, label)?;
        if Some(&record_hash) != plan.record_hash.as_ref() || payload.get("helpdesk_ticket") != Some(&json!({"status": status_id, "br_validation_excludes": token})) {
            return Err(refused("ticket or status metadata changed; review again"));
        }
    }
    let Ok(csrf) = c.get_csrf() else {
        return Err(refused("could not prepare session; no send attempted"));
    };
    let target = json!(plan.ticket_id);
    journal.record_plan("started", &hash, &action, Some(&target), None);
    if last_event(journal, &action, &hash).as_deref() != Some("started") {
        return Err(refused("could not persist journal; no send attempted"));
    }
    let response = match c.send_once("POST", &plan.path(), Body::Raw(plan.payload_json.clone().into_bytes()), None, &csrf) {
        Ok(r) => r,
        Err(e) => {
            if let Some(status) = definite_rejection(&e) {
                journal.record_plan("failed", &hash, &action, Some(&target), None);
                return Err(refused(format!("conversation rejected (HTTP {status}); no retry")));
            }
            return Err(refused(UNKNOWN));
        }
    };
    let Some(conv_id) = note_id(returned_note(&response)) else {
        return Err(refused("response has no exact conversation ID; outcome unknown, do not resend"));
    };
    journal.record_plan("done", &hash, &action, Some(&target), Some(&conv_id));
    let mut result = obj(json!({"status": "submitted", "conversation_id": conv_id, "ticket_id": plan.ticket_id,
                                "plan_hash": hash, "verified": false}));
    let readback = || -> Result<(Map<String, Value>, bool)> {
        let pages = load_conversations(c, plan.ticket_id)?;
        let want = py_str(&conv_id);
        let exact = pages.rows.iter().find(|r| py_str(r.get("id").unwrap_or(&Value::Null)) == want);
        let mut expected = vec![("body", payload["body"].clone())];
        if plan.kind == "reply" {
            expected.push(("cc_emails", payload["cc_emails"].clone()));
        } else {
            expected.push(("private", payload["private"].clone()));
            expected.push(("notify_emails", payload["notify_emails"].clone()));
        }
        let mut fields = Map::new();
        for (k, v) in expected {
            let state = match exact.and_then(|e| e.get(k)) {
                None => "unavailable",
                Some(got) if *got == v => "matched",
                Some(_) => "mismatched",
            };
            fields.insert(k.into(), json!(state));
        }
        if plan.status_label.is_some() {
            let row = record(c, plan.ticket_id)?;
            let state = match row.get("status") {
                None => "unavailable",
                Some(s) if *s == payload["helpdesk_ticket"]["status"] => "matched",
                Some(_) => "mismatched",
            };
            fields.insert("ticket_status".into(), json!(state));
        }
        let complete = pages.missing.is_none() && !pages.meta.get("partial").is_some_and(truthy);
        let verified = exact.is_some() && complete && fields.values().all(|v| v == "matched");
        Ok((fields, verified))
    };
    if let Ok((fields, verified)) = readback() {
        result.insert("fields".into(), Value::Object(fields));
        result.insert("verified".into(), json!(verified));
    }
    if result["verified"] != Value::Bool(true) {
        result.insert("warning".into(), json!("Conversation or ticket fields were not fully verified on read-back; do not resend."));
    }
    Ok(Value::Object(result))
}
