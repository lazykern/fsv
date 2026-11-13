//! Reviewed reply or note-with-status from blocked UI submissions
//! (`ticket_conversation.py`).

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use fsv_core::client::Body;
use fsv_core::error::{Error, Result};
use fsv_forms::payload::format_requester;
use fsv_forms::{FormState, build_br_validation_excludes};
use fsv_py::{py_str, truthy};

use crate::formmeta::{
    MetaStore, RULES_UNAVAILABLE_KEY, existing_from_record, is_status, load_meta, resolve_requester,
};
use crate::notes::{self, BodyFormat, EMAIL, note_id, returned_note, valid_recipient};
use crate::properties::{dumps_sorted, record};
use crate::readback::py_equal;
use crate::service::{
    Api, Journal, definite_rejection, last_event, load_conversations, obj, plan_hash, snapshot_hash,
};

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
            let value = format_requester(&pe, Some(&person.display))
                .map_err(|e| Error::value(e.to_string()))?;
            if !valid_recipient(&value) {
                return Err(Error::value("invalid formatted address"));
            }
            Ok(value)
        };
        out.push(one().map_err(|_| refused("each CC email must resolve to exactly one non-deleted user with a valid name and email"))?);
    }
    Ok(out)
}

fn status_inner(
    c: &dyn Api,
    store: &dyn MetaStore,
    cid: i64,
    label: &str,
) -> Result<(i64, String, String)> {
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
        .filter(|ch| {
            ch.visible
                && (fsv_core::pystr::casefold(&ch.label) == want
                    || fsv_core::pystr::casefold(&ch.value) == want)
        })
        .collect();
    let id = match choices.as_slice() {
        [one] => one
            .id
            .as_ref()
            .and_then(|v| {
                if v.is_i64() || v.is_u64() {
                    v.as_i64()
                } else {
                    None
                }
            })
            .filter(|i| *i > 0),
        _ => None,
    };
    let Some(id) = id else {
        return Err(bad("status choice not unique"));
    };
    state
        .set(
            &field.api_name,
            fsv_forms::Value::from(choices[0].value.clone()),
        )
        .map_err(|e| Error::other(e.to_string()))?;
    let errors = state.errors().map_err(|e| Error::other(e.to_string()))?;
    let gate = state
        .rule_gate(&[])
        .map_err(|e| Error::other(e.to_string()))?;
    if !state.evaluated.visible.contains(&field.api_name)
        || !errors.is_empty()
        || !gate.blockers.is_empty()
    {
        let detail = if gate.blockers.is_empty() {
            String::new()
        } else {
            format!(": {}", gate.blockers.join("; "))
        };
        return Err(bad(&format!(
            "status form rules or context unavailable{detail}"
        )));
    }
    let token =
        build_br_validation_excludes(&state.evaluated, "ticket", &Value::Null, &Value::Null)
            .map_err(|e| Error::other(e.to_string()))?;
    Ok((id, token, snapshot_hash(&Value::Object(row))?))
}

/// `email.utils.parseaddr(value)[1]`.
fn parseaddr(value: &str) -> String {
    crate::email_addr::parseaddr(value).1
}

/// `ticket_conversation._email_set`. `None` means the value is not an
/// explicit list of valid recipients.
pub(crate) fn email_set(values: &Value) -> Option<BTreeSet<String>> {
    let list = values.as_array()?;
    let mut out = BTreeSet::new();
    for value in list {
        let text = value.as_str()?;
        let trimmed = text.trim();
        if !crate::recipients::valid_recipient(trimmed) {
            return None;
        }
        out.insert(fsv_core::pystr::casefold(&parseaddr(trimmed)));
    }
    Some(out)
}

/// `ticket_conversation._recipient_readback`. Notification and To are the
/// same observed field. CC and BCC stay distinct: a To address does not
/// prove a blind copy.
pub(crate) fn recipient_readback(
    row: Option<&Map<String, Value>>,
    key: &str,
    expected: &Value,
) -> &'static str {
    let notify = key == "notify_emails" || key == "to_emails";
    let present: Vec<&Value> = match row {
        Some(row) if notify => ["notify_emails", "to_emails"]
            .into_iter()
            .filter_map(|name| row.get(name))
            .collect(),
        Some(row) => row.get(key).into_iter().collect(),
        None => Vec::new(),
    };
    if present.is_empty() {
        return "unavailable";
    }
    match email_set(expected) {
        Some(wanted)
            if present
                .iter()
                .all(|value| email_set(value) == Some(wanted.clone())) =>
        {
            "matched"
        }
        _ => "mismatched",
    }
}

/// `ticket_conversation.sender_readback`: the observed `from_email` is the
/// one submitted address, display name and case aside.
pub(crate) fn sender_readback(row: Option<&Map<String, Value>>, expected: &Value) -> &'static str {
    let Some(actual) = row.and_then(|r| r.get("from_email")) else {
        return "unavailable";
    };
    let wanted = email_set(&json!([expected]));
    if wanted.is_some() && email_set(&json!([actual])) == wanted {
        "matched"
    } else {
        "mismatched"
    }
}

/// `status_context`: the Pending status id, its business-rule token and the
/// record hash, from fresh edit-form metadata. Only Pending was captured.
pub fn status_context(
    c: &dyn Api,
    store: &dyn MetaStore,
    cid: i64,
    label: &str,
) -> Result<(i64, String, String)> {
    if fsv_core::pystr::casefold(label) != "pending" {
        return Err(Error::value(
            "only the Pending status transition was captured; use the web form for other statuses",
        ));
    }
    status_inner(c, store, cid, label)
        .map_err(|_| refused("could not resolve Pending and evaluate ticket rules from fresh metadata; no send attempted"))
}

fn closing_label(label: &str) -> bool {
    matches!(
        fsv_core::pystr::casefold(label).as_str(),
        "resolved" | "closed"
    )
}

pub const SET_STATUS_CHOICES: [&str; 3] = ["Pending", "Resolved", "Closed"];

/// `status_fields`: the nested `helpdesk_ticket` payload and its record
/// fingerprint. Resolved/Closed use the lifecycle closure checks (routing
/// preserved, persisted resolution required); others are Pending-only.
pub fn status_fields(
    c: &dyn Api,
    store: &dyn MetaStore,
    cid: i64,
    label: &str,
    ack: bool,
) -> Result<(Map<String, Value>, String)> {
    let folded = fsv_core::pystr::casefold(fsv_core::pystr::strip(label));
    if !SET_STATUS_CHOICES
        .iter()
        .any(|c| fsv_core::pystr::casefold(c) == folded)
    {
        // Local usage refusal (exit 2) that names the accepted values, like
        // `tickets resolve --set`; only these transitions have captured wires.
        return Err(Error::value(format!(
            "--set-status {} is not a supported status; choices: {}; nothing was sent",
            fsv_py::py_repr_str(label),
            SET_STATUS_CHOICES.join(", ")
        )));
    }
    let label = fsv_core::pystr::strip(label);
    if closing_label(label) {
        // --ack has the same meaning as tickets close/status --ack: the
        // reviewer read the blocking rule messages; they are bound into the
        // returned fingerprint (and so the review hash) only when used.
        let (payload, fingerprint, _, _) = crate::ticket_lifecycle::context_ack(
            c,
            store,
            cid,
            label,
            &Map::new(),
            None,
            None,
            None,
            ack,
        )?;
        return Ok((payload, fingerprint));
    }
    if ack {
        return Err(refused(
            "--ack applies only with --set-status Resolved or Closed; the captured Pending wire has no blocking-rule acknowledgement",
        ));
    }
    let (status_id, token, record_hash) = status_context(c, store, cid, label)?;
    Ok((
        obj(json!({"status": status_id, "br_validation_excludes": token})),
        record_hash,
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationPlan {
    /// `reply` | `note` | `forward`.
    pub kind: String,
    pub ticket_id: i64,
    pub tenant: String,
    pub payload_json: String,
    pub status_label: Option<String>,
    pub record_hash: Option<String>,
    /// The canned response the body was selected from (`canned.reply_plan`).
    pub canned_id: Option<i64>,
    pub canned_pointer: Option<String>,
    pub canned_hash: Option<String>,
    /// `--ack` with a closing `--set-status` (bound in `record_hash`).
    pub ack: bool,
}

impl ConversationPlan {
    /// Blocking rule messages acknowledged with --ack (bound in `record_hash`).
    pub fn acknowledged(&self) -> Vec<Value> {
        let Some(bound) = self.record_hash.as_deref().filter(|_| self.ack) else {
            return Vec::new();
        };
        match serde_json::from_str::<Value>(bound) {
            Ok(Value::Object(m)) => m
                .get("acknowledged")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    pub fn payload(&self) -> Value {
        serde_json::from_str(&self.payload_json).unwrap_or(Value::Null)
    }

    pub fn path(&self) -> String {
        let tail = match self.kind.as_str() {
            "reply" => "reply",
            "forward" => "forward",
            _ => "notes",
        };
        format!("tickets/{}/{tail}", self.ticket_id)
    }

    pub fn hash(&self) -> String {
        let mut identity = json!({"tenant": self.tenant, "path": self.path(), "payload": self.payload(),
                                  "record_hash": self.record_hash});
        // Preserve all legacy conversation hashes and their durable send guards.
        if let Some(id) = self.canned_id {
            identity["canned"] =
                json!({"id": id, "pointer": self.canned_pointer, "hash": self.canned_hash});
        }
        plan_hash(&identity)
    }

    pub fn review(&self) -> Map<String, Value> {
        let mut out = obj(
            json!({"action": format!("ticket_{}", self.kind), "method": "POST", "path": format!("/api/_/{}", self.path()),
                   "payload": self.payload(), "status_label": self.status_label, "plan_hash": self.hash(),
                   "warning": "Replies and forwards send mail; notes may trigger tenant workflows. Explicit sender addresses are user-asserted, not verified mailbox configuration."}),
        );
        let acknowledged = self.acknowledged();
        if !acknowledged.is_empty() {
            out.insert("acknowledged_rule_messages".into(), json!(acknowledged));
            out.insert(
                "warning".into(),
                json!("Replies and forwards send mail; notes may trigger tenant workflows. Explicit sender addresses are user-asserted, not verified mailbox configuration. Acknowledged blocking rule messages are tenant policy: the web form blocks submit on them, and the server or a workflow may still reject or revert the transition."),
            );
        }
        out
    }
}

/// Options of [`build_plan`] (Python keyword arguments).
#[derive(Debug, Clone)]
pub struct ConversationArgs<'a> {
    /// `reply` | `note` | `forward`.
    pub kind: &'a str,
    pub cc: &'a [String],
    pub public: bool,
    /// Note recipients; for `forward`, the bare `To` addresses.
    pub notify: &'a [String],
    pub format: BodyFormat,
    pub set_status: Option<&'a str>,
    /// `--ack`: acknowledge blocking rule messages of a Resolved/Closed
    /// `set_status`.
    pub ack: bool,
}

/// The `bcc` / `sender` keywords of fsv.ticket_conversation.build_plan
/// (see [`build_plan_mail`]).
#[derive(Debug, Clone, Copy, Default)]
pub struct MailArgs<'a> {
    /// Bare emails; reply only, requires `sender`.
    pub bcc: &'a [String],
    /// A bare support email sent as `from_email` (user-asserted); required
    /// for `forward`.
    pub sender: Option<&'a str>,
}

impl Default for ConversationArgs<'_> {
    fn default() -> Self {
        ConversationArgs {
            kind: "reply",
            cc: &[],
            public: false,
            notify: &[],
            format: BodyFormat::Plain,
            set_status: None,
            ack: false,
        }
    }
}

/// `build_plan`: the exact reply/note payload, with the status change and
/// its rules token when `set_status` is given. No BCC or sender, so a
/// `forward` is refused; use [`build_plan_mail`].
pub fn build_plan(
    c: &dyn Api,
    store: &dyn MetaStore,
    ticket_id: &str,
    body: &str,
    args: &ConversationArgs<'_>,
) -> Result<ConversationPlan> {
    build_plan_mail(c, store, ticket_id, body, args, &MailArgs::default())
}

/// `ticket_conversation.forward_source`: the snapshot of the one matching
/// conversation in a complete ticket history.
pub fn forward_source(c: &dyn Api, ticket_id: i64, conversation_id: i64) -> Result<String> {
    let conversation_id = crate::watchers::positive_id(&json!(conversation_id))?;
    let unavailable = || refused("exact forward source unavailable in complete ticket history");
    let params = obj(json!({"include": "user,phone,feedback", "per_page": 100}));
    let rows = match crate::reviewed_inventory::complete_inventory(
        c,
        &format!("tickets/{ticket_id}/conversations"),
        "conversations",
        Some(&params),
        crate::reviewed_inventory::Family::Internal,
    ) {
        Ok(rows) => rows,
        Err(e @ Error::Session(_)) => return Err(e),
        Err(_) => return Err(unavailable()),
    };
    let want = conversation_id.to_string();
    let matches: Vec<&Value> = rows
        .iter()
        .filter(|row| py_str(row.get("id").unwrap_or(&Value::Null)) == want)
        .collect();
    if matches.len() != 1 {
        return Err(refused(
            "exact forward source unavailable in complete ticket history",
        ));
    }
    snapshot_hash(matches[0])
}

/// `build_plan(..., conversation_id=)` for an exact-conversation forward.
/// `ConversationArgs` stays unchanged so existing CLI literals keep compiling.
pub fn build_plan_forward_source(
    c: &dyn Api,
    store: &dyn MetaStore,
    ticket_id: &str,
    body: &str,
    args: &ConversationArgs<'_>,
    mail: &MailArgs<'_>,
    conversation_id: i64,
) -> Result<ConversationPlan> {
    if args.kind != "forward" {
        return Err(Error::value(
            "conversation selector is only supported for forward",
        ));
    }
    let mut plan = build_plan_mail(c, store, ticket_id, body, args, mail)?;
    let source = forward_source(c, plan.ticket_id, conversation_id)?;
    let mut payload = plan.payload();
    payload["conversation_id"] = json!(conversation_id);
    plan.payload_json = dumps_sorted(&payload);
    plan.record_hash = Some(source);
    Ok(plan)
}

/// Port of fsv.ticket_conversation.build_plan: reply (optionally with BCC
/// and an explicit sender), note, or whole-ticket forward.
pub fn build_plan_mail(
    c: &dyn Api,
    store: &dyn MetaStore,
    ticket_id: &str,
    body: &str,
    args: &ConversationArgs<'_>,
    mail: &MailArgs<'_>,
) -> Result<ConversationPlan> {
    if args.ack && args.set_status.is_none() {
        return Err(Error::value(
            "--ack acknowledges blocking rule messages of a --set-status Resolved/Closed transition; pass it together with --set-status",
        ));
    }
    if !matches!(args.kind, "reply" | "note" | "forward") {
        return Err(Error::value("invalid conversation kind"));
    }
    let tenant = c.domain();
    let note = notes::build_plan(
        ticket_id,
        body,
        args.public,
        args.notify,
        args.format,
        &tenant,
    )?;
    let bare = |v: &String| EMAIL.is_match(v);
    if mail.sender.is_some_and(|s| !EMAIL.is_match(s)) {
        return Err(Error::value(
            "sender must be a plain configured support email address",
        ));
    }
    let (has_cc, has_bcc) = (!args.cc.is_empty(), !mail.bcc.is_empty());
    if args.kind == "forward"
        && (mail.sender.is_none_or(str::is_empty)
            || args.notify.is_empty()
            || args.set_status.is_some()
            || has_cc
            || has_bcc)
    {
        return Err(Error::value(
            "whole-ticket forward requires explicit sender and recipients; no combined status/CC/BCC wire is proven",
        ));
    }
    if args.kind == "note" && (has_bcc || mail.sender.is_some_and(|s| !s.is_empty())) {
        return Err(Error::value("note sender/BCC wire is not proven"));
    }
    if has_bcc && (has_cc || args.set_status.is_some()) {
        return Err(Error::value("combined BCC/CC/status wire is not proven"));
    }
    if has_bcc && (mail.sender.is_none_or(str::is_empty) || !mail.bcc.iter().all(bare)) {
        return Err(Error::value(
            "BCC requires a configured --sender and bare recipient emails",
        ));
    }
    let sender = mail.sender.filter(|s| !s.is_empty());
    let mut payload = match args.kind {
        "reply" => {
            if args.cc.iter().any(|v| !valid_recipient(v)) {
                return Err(Error::value("invalid CC recipient"));
            }
            let mut p = json!({"body": note.body_html, "conversation_from_summary": false, "cc_emails": args.cc,
                               "full_text": note.body_html, "translate_reply": false, "freddy_message_id": null});
            if has_bcc {
                p["bcc_emails"] = json!(mail.bcc);
                if !has_cc && let Some(m) = p.as_object_mut() {
                    m.shift_remove("cc_emails");
                }
            }
            if let Some(s) = sender {
                p["from_email"] = json!(s);
            }
            p
        }
        "forward" => {
            if !args.notify.iter().all(bare) {
                return Err(Error::value("forward requires bare recipient emails"));
            }
            json!({"body": note.body_html, "from_email": sender, "notify_emails": args.notify,
                   "conversation_from_summary": false, "private": true})
        }
        _ => note.payload(),
    };
    let mut record_hash = None;
    if let Some(label) = args.set_status {
        let (fields, rh) = status_fields(c, store, note.ticket_id, label, args.ack)?;
        record_hash = Some(rh);
        payload["helpdesk_ticket"] = Value::Object(fields);
    }
    Ok(ConversationPlan {
        kind: args.kind.to_string(),
        ticket_id: note.ticket_id,
        tenant,
        payload_json: dumps_sorted(&payload),
        status_label: args.set_status.map(str::to_string),
        record_hash,
        canned_id: None,
        canned_pointer: None,
        canned_hash: None,
        ack: args.ack && args.set_status.is_some(),
    })
}

/// `send`: one POST after review, public consent and a fresh status check,
/// then a read-back by the returned conversation id.
pub fn send(
    plan: &ConversationPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    store: &dyn MetaStore,
    reviewed_hash: Option<&str>,
    public_confirmed: bool,
) -> Result<Value> {
    let Some(reviewed) = reviewed_hash else {
        return Ok(json!({"status": "cancelled"}));
    };
    let hash = plan.hash();
    if hash != reviewed || plan.tenant.is_empty() || plan.tenant != c.domain() {
        return Err(refused("tenant or review hash changed; review again"));
    }
    let payload = plan.payload();
    let private = payload.get("private").is_none_or(truthy);
    if (plan.kind == "reply" || plan.kind == "forward" || !private) && !public_confirmed {
        return Err(refused(
            "public conversation requires explicit confirmation",
        ));
    }
    let action = format!("ticket_{}_reviewed", plan.kind);
    if matches!(
        last_event(journal, &action, &hash).as_deref(),
        Some("started" | "done")
    ) {
        return Err(refused(
            "conversation was sent or outcome unknown; inspect ticket and journal before retry",
        ));
    }
    if plan.kind == "forward" && payload.get("conversation_id").is_some() {
        let id = crate::watchers::positive_id(&payload["conversation_id"])?;
        if forward_source(c, plan.ticket_id, id)? != plan.record_hash.as_deref().unwrap_or("") {
            return Err(refused("forward source changed; review again"));
        }
    }
    if plan.canned_id.is_some() {
        crate::canned::freshen(plan, c)?;
    }
    if let Some(label) = &plan.status_label {
        let (fields, record_hash) = status_fields(c, store, plan.ticket_id, label, plan.ack)?;
        if Some(&record_hash) != plan.record_hash.as_ref()
            || !payload
                .get("helpdesk_ticket")
                .is_some_and(|h| py_equal(h, &Value::Object(fields)))
        {
            return Err(refused("ticket or status metadata changed; review again"));
        }
    }
    // Reconfirm formatted CC recipients without silently changing reviewed bytes.
    let strings = |key: &str| -> Vec<String> {
        payload
            .get(key)
            .and_then(Value::as_array)
            .map(|a| a.iter().map(py_str).collect())
            .unwrap_or_default()
    };
    let cc = strings("cc_emails");
    if !cc.is_empty() {
        let emails: Vec<String> = cc.iter().map(|v| parseaddr(v)).collect();
        if recipients(c, &emails)? != cc {
            return Err(refused("CC recipient identity changed; review again"));
        }
    }
    // New recipient variants are looked up again before any send. No opaque
    // user ids or guessed sender identities are substituted into the payload.
    let mut lookups = strings("bcc_emails");
    if plan.kind == "forward" {
        lookups.extend(strings("notify_emails"));
    }
    for address in lookups {
        recipients(c, std::slice::from_ref(&address))?;
    }
    let Ok(csrf) = c.get_csrf() else {
        return Err(refused("could not prepare session; no send attempted"));
    };
    let closing = plan.status_label.as_deref().is_some_and(closing_label);
    let target = json!(plan.ticket_id);
    journal.record_plan("started", &hash, &action, Some(&target), None);
    if last_event(journal, &action, &hash).as_deref() != Some("started") {
        return Err(refused("could not persist journal; no send attempted"));
    }
    let response = match c.send_once(
        "POST",
        &plan.path(),
        Body::Raw(plan.payload_json.clone().into_bytes()),
        None,
        &csrf,
    ) {
        Ok(r) => r,
        Err(e) => {
            if let Some(status) = definite_rejection(&e) {
                journal.record_plan("failed", &hash, &action, Some(&target), None);
                return Err(refused(format!(
                    "conversation rejected (HTTP {status}); no retry"
                )));
            }
            return Err(refused(UNKNOWN));
        }
    };
    let Some(conv_id) = note_id(returned_note(&response)) else {
        return Err(refused(
            "response has no exact conversation ID; outcome unknown, do not resend",
        ));
    };
    if !closing {
        journal.record_plan("done", &hash, &action, Some(&target), Some(&conv_id));
    }
    let mut result = obj(
        json!({"status": "submitted", "conversation_id": conv_id, "ticket_id": plan.ticket_id,
                                "plan_hash": hash, "verified": false}),
    );
    let readback = || -> Result<(Map<String, Value>, bool)> {
        let pages = load_conversations(c, plan.ticket_id)?;
        let want = py_str(&conv_id);
        let exact = pages
            .rows
            .iter()
            .find(|r| py_str(r.get("id").unwrap_or(&Value::Null)) == want);
        let mut expected = vec![("body", payload["body"].clone())];
        if plan.kind == "reply" {
            for key in ["cc_emails", "bcc_emails"] {
                if let Some(v) = payload.get(key) {
                    expected.push((key, v.clone()));
                }
            }
        } else {
            expected.push(("private", payload["private"].clone()));
            // Keep the closure field label. Either observed notification
            // representation can satisfy it.
            let recipient_key = if closing && plan.kind == "note" {
                "to_emails"
            } else {
                "notify_emails"
            };
            expected.push((recipient_key, payload["notify_emails"].clone()));
        }
        if let Some(v) = payload.get("from_email") {
            expected.push(("from_email", v.clone()));
        }
        let mut fields = Map::new();
        let row = exact.and_then(|e| e.as_object());
        for (k, v) in expected {
            let state = if k == "from_email" {
                sender_readback(row, &v)
            } else if matches!(
                k,
                "to_emails" | "notify_emails" | "cc_emails" | "bcc_emails"
            ) {
                recipient_readback(row, k, &v)
            } else {
                match row.and_then(|r| r.get(k)) {
                    None => "unavailable",
                    Some(got) if *got == v => "matched",
                    Some(_) => "mismatched",
                }
            };
            fields.insert(k.into(), json!(state));
        }
        if plan.status_label.is_some() {
            let row = record(c, plan.ticket_id)?;
            let nested = &payload["helpdesk_ticket"];
            let state = match row.get("status") {
                None => "unavailable",
                Some(s) if (s.is_i64() || s.is_u64()) && py_equal(s, &nested["status"]) => {
                    "matched"
                }
                Some(_) => "mismatched",
            };
            fields.insert("ticket_status".into(), json!(state));
            if closing {
                let matched = nested
                    .as_object()
                    .is_some_and(|n| crate::ticket_lifecycle::matches(&row, n));
                fields.insert(
                    "ticket_transition".into(),
                    json!(if matched { "matched" } else { "mismatched" }),
                );
            }
        }
        let complete = pages.missing.is_none() && !pages.meta.get("partial").is_some_and(truthy);
        let verified = exact.is_some() && complete && fields.values().all(|v| v == "matched");
        Ok((fields, verified))
    };
    if let Ok((fields, verified)) = readback() {
        result.insert("fields".into(), Value::Object(fields));
        result.insert("verified".into(), json!(verified));
    }
    if closing && result["verified"] == Value::Bool(true) {
        journal.record_plan("done", &hash, &action, Some(&target), Some(&conv_id));
    }
    if plan.kind == "forward"
        && let Some(source) = payload.get("conversation_id")
    {
        result.insert("source_conversation_id".into(), source.clone());
        result.insert("source_link_readback".into(), json!("unavailable"));
        result.insert("source_link_verified".into(), json!(false));
        result.insert(
            "verification_scope".into(),
            json!("delivery fields; source link is not exposed by independent readback"),
        );
    }
    if result["verified"] != Value::Bool(true) {
        result.insert(
            "warning".into(),
            json!(
                "Conversation or ticket fields were not fully verified on read-back; do not resend."
            ),
        );
    }
    Ok(Value::Object(result))
}

#[cfg(test)]
mod closure_tests;

#[cfg(test)]
mod http_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formmeta::MemStore;
    use crate::service::testing::*;

    const TOKEN: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltdIn0=";
    const ROW_HASH: &str = "1fdf7e536748426619ff7f88ffd5fb0a1b5940b5c4352579c3a462978c2f6a79";
    const REPLY_HASH: &str = "a6558f93a3456b1bf04f15b2e3711aa8d995ae7483bb767c1a7bcd8b5553a2ca";

    #[test]
    fn payload_parses_the_reviewed_json() {
        let oracle: Value =
            serde_json::from_str(include_str!("testdata/coverage_oracle.json")).unwrap();
        let plan = ConversationPlan {
            kind: "note".into(),
            ticket_id: 5,
            tenant: "acme".into(),
            payload_json: r#"{"body": "<p>x</p>", "private": true}"#.into(),
            status_label: None,
            record_hash: None,
            canned_id: None,
            canned_pointer: None,
            canned_hash: None,
            ack: false,
        };
        assert_eq!(plan.payload(), oracle["misc"]["conversation_payload"]);
    }
    const NOTE_HASH: &str = "b968aa46849b709e7f57dcf98d4fe54d0529917405a43d49448bd410639a525a";
    const CC: &str = "Acme User <person@example.com>";
    const STATUS_ERR: &str = "could not resolve Pending and evaluate ticket rules from fresh metadata; no send attempted";
    const WARNING: &str = "Replies and forwards send mail; notes may trigger tenant workflows. Explicit sender addresses are user-asserted, not verified mailbox configuration.";
    const UNVERIFIED: &str =
        "Conversation or ticket fields were not fully verified on read-back; do not resend.";

    fn err<T: std::fmt::Debug>(r: Result<T>) -> String {
        r.unwrap_err().to_string()
    }

    fn row() -> Value {
        json!({"id": 123, "subject": "Test", "status": 2})
    }

    fn api() -> FakeApi {
        ticket_api(json!({"ticket": row()})).ok(
            "GET autocomplete/requesters",
            json!([{"id": 11, "value": "Acme User", "email": "person@example.com"}]),
        )
    }

    fn reply_plan(c: &FakeApi) -> ConversationPlan {
        let cc = vec![CC.to_string()];
        build_plan(
            c,
            &MemStore::default(),
            "123",
            "Hello <team>",
            &ConversationArgs {
                cc: &cc,
                set_status: Some("Pending"),
                ..Default::default()
            },
        )
        .unwrap()
    }

    fn note_plan(c: &FakeApi) -> ConversationPlan {
        let notify = vec!["b@y.co".to_string()];
        build_plan(
            c,
            &MemStore::default(),
            "INC-123",
            "Note",
            &ConversationArgs {
                kind: "note",
                notify: &notify,
                ..Default::default()
            },
        )
        .unwrap()
    }

    #[test]
    fn recipients_resolve_exact_users() {
        let s = |v: &str| vec![v.to_string()];
        assert_eq!(recipients(&api(), &s("person@example.com")).unwrap(), [CC]);
        assert_eq!(
            err(recipients(&api(), &s("Ann <a@b.co>"))),
            "CC must be a plain email address"
        );
        let msg = "each CC email must resolve to exactly one non-deleted user with a valid name and email";
        assert_eq!(err(recipients(&api(), &s("other@example.com"))), msg);
        let down = FakeApi::new().on("GET autocomplete/requesters", Reply::Session);
        assert_eq!(err(recipients(&down, &s("person@example.com"))), msg);
    }

    #[test]
    fn status_context_pending_only() {
        let store = MemStore::default();
        assert_eq!(
            status_context(&api(), &store, 123, "pending").unwrap(),
            (3, TOKEN.to_string(), ROW_HASH.to_string())
        );
        assert_eq!(
            err(status_context(&api(), &store, 123, "Resolved")),
            "only the Pending status transition was captured; use the web form for other statuses"
        );
        let other = ticket_api(json!({"ticket": {"id": 9}}));
        assert_eq!(
            err(status_context(&other, &store, 123, "Pending")),
            STATUS_ERR
        );
        let no_rules = FakeApi::new()
            .ok("GET ticket_form_fields", ticket_form(3))
            .ok("GET tickets/123", json!({"ticket": row()}));
        assert_eq!(
            err(status_context(&no_rules, &store, 123, "Pending")),
            STATUS_ERR
        );
    }

    #[test]
    fn build_plan_reply_and_note() {
        let c = api();
        let p = reply_plan(&c);
        let payload = json!({"body": "<div>Hello &lt;team&gt;</div>", "cc_emails": [CC], "conversation_from_summary": false, "freddy_message_id": null,
                             "full_text": "<div>Hello &lt;team&gt;</div>", "helpdesk_ticket": {"br_validation_excludes": TOKEN, "status": 3}, "translate_reply": false});
        assert_eq!(
            p.payload_json,
            format!(
                "{{\"body\": \"<div>Hello &lt;team&gt;</div>\", \"cc_emails\": [\"{CC}\"], \"conversation_from_summary\": false, \"freddy_message_id\": null, \"full_text\": \"<div>Hello &lt;team&gt;</div>\", \"helpdesk_ticket\": {{\"br_validation_excludes\": \"{TOKEN}\", \"status\": 3}}, \"translate_reply\": false}}"
            )
        );
        assert_eq!(
            (
                p.path(),
                p.hash(),
                p.record_hash.as_deref(),
                p.status_label.as_deref()
            ),
            (
                "tickets/123/reply".to_string(),
                REPLY_HASH.to_string(),
                Some(ROW_HASH),
                Some("Pending")
            )
        );
        assert_eq!(
            Value::Object(p.review()),
            json!({"action": "ticket_reply", "method": "POST", "path": "/api/_/tickets/123/reply", "payload": payload,
            "status_label": "Pending", "plan_hash": REPLY_HASH, "warning": WARNING})
        );
        let n = note_plan(&c);
        assert_eq!(
            n.payload_json,
            r#"{"body": "<div>Note</div>", "conversation_from_summary": false, "notify_emails": ["b@y.co"], "private": true}"#
        );
        assert_eq!(
            (n.path(), n.hash(), n.record_hash),
            ("tickets/123/notes".to_string(), NOTE_HASH.to_string(), None)
        );
        let store = MemStore::default();
        assert_eq!(
            err(build_plan(
                &c,
                &store,
                "123",
                "x",
                &ConversationArgs {
                    kind: "forward",
                    ..Default::default()
                }
            )),
            "whole-ticket forward requires explicit sender and recipients; no combined status/CC/BCC wire is proven"
        );
        assert_eq!(
            err(build_plan(
                &c,
                &store,
                "123",
                "x",
                &ConversationArgs {
                    kind: "call",
                    ..Default::default()
                }
            )),
            "invalid conversation kind"
        );
        let bad = vec!["bad".to_string()];
        assert_eq!(
            err(build_plan(
                &c,
                &store,
                "123",
                "x",
                &ConversationArgs {
                    cc: &bad,
                    ..Default::default()
                }
            )),
            "invalid CC recipient"
        );
    }

    #[test]
    fn send_reply_verifies_conversation_and_status() {
        let p = reply_plan(&api());
        let c = api()
            .ok("GET tickets/123", json!({"ticket": {"id": 123, "subject": "Test", "status": 3}}))
            .ok("POST tickets/123/reply", json!({"conversation": {"id": 456}}))
            .ok("GET tickets/123/conversations", json!({"conversations": [{"id": 456, "body": "<div>Hello &lt;team&gt;</div>", "cc_emails": [CC]}]}));
        let (j, store) = (MemJournal::default(), MemStore::default());
        assert_eq!(
            send(&p, &c, &j, &store, Some(REPLY_HASH), true).unwrap(),
            json!({"status": "submitted", "conversation_id": 456, "ticket_id": 123, "plan_hash": REPLY_HASH, "verified": true,
                   "fields": {"body": "matched", "cc_emails": "matched", "ticket_status": "matched"}})
        );
        assert_eq!(j.events(), ["started", "done"]);
        let w = c.writes();
        assert_eq!(
            (w[0].method.as_str(), w[0].path.as_str()),
            ("POST", "tickets/123/reply")
        );
        assert_eq!(
            String::from_utf8(w[0].raw.clone().unwrap()).unwrap(),
            p.payload_json
        );
        assert_eq!(
            err(send(&p, &c, &j, &store, Some(REPLY_HASH), true)),
            "conversation was sent or outcome unknown; inspect ticket and journal before retry"
        );
        assert_eq!(
            send(&p, &c, &j, &store, None, false).unwrap(),
            json!({"status": "cancelled"})
        );
        assert_eq!(
            err(send(&p, &c, &j, &store, Some("x"), true)),
            "tenant or review hash changed; review again"
        );
        assert_eq!(
            err(send(
                &p,
                &c,
                &MemJournal::default(),
                &store,
                Some(REPLY_HASH),
                false
            )),
            "public conversation requires explicit confirmation"
        );
        let changed = ticket_api(json!({"ticket": {"id": 123, "subject": "Changed", "status": 2}}));
        assert_eq!(
            err(send(
                &p,
                &changed,
                &MemJournal::default(),
                &store,
                Some(REPLY_HASH),
                true
            )),
            "ticket or status metadata changed; review again"
        );
    }

    #[test]
    fn send_note_failures_and_unverified() {
        let n = note_plan(&api());
        let store = MemStore::default();
        let j = MemJournal::default();
        let c = FakeApi::new().ok("POST tickets/123/notes", json!({"note": {"id": "x"}}));
        assert_eq!(
            err(send(&n, &c, &j, &store, Some(NOTE_HASH), false)),
            "response has no exact conversation ID; outcome unknown, do not resend"
        );
        assert_eq!(j.events(), ["started"]);
        let j = MemJournal::default();
        let c = FakeApi::new().on("POST tickets/123/notes", Reply::Status(422));
        assert_eq!(
            err(send(&n, &c, &j, &store, Some(NOTE_HASH), false)),
            "conversation rejected (HTTP 422); no retry"
        );
        assert_eq!(j.events(), ["started", "failed"]);
        let c = FakeApi::new()
            .ok("POST tickets/123/notes", json!({"note": {"id": 7}}))
            .on("GET tickets/123/conversations", Reply::Status(500));
        assert_eq!(
            send(
                &n,
                &c,
                &MemJournal::default(),
                &store,
                Some(NOTE_HASH),
                false
            )
            .unwrap(),
            json!({"status": "submitted", "conversation_id": 7, "ticket_id": 123, "plan_hash": NOTE_HASH, "verified": false,
                   "fields": {"body": "unavailable", "private": "unavailable", "notify_emails": "unavailable"}, "warning": UNVERIFIED})
        );
    }

    fn mail_plan(c: &FakeApi, kind: &str, sender: &str, to: &[String]) -> Result<ConversationPlan> {
        let (bcc, notify): (&[String], &[String]) = if kind == "reply" {
            (to, &[])
        } else {
            (&[], to)
        };
        build_plan_mail(
            c,
            &MemStore::default(),
            "123",
            "Acme update",
            &ConversationArgs {
                kind,
                notify,
                ..Default::default()
            },
            &MailArgs {
                bcc,
                sender: Some(sender),
            },
        )
    }

    /// `test_ticket_conversation.py::test_bcc_and_whole_ticket_forward_exact_wire_hash_and_readback`.
    #[test]
    fn bcc_and_forward_exact_wire_hash_and_readback() {
        let oracle: Value =
            serde_json::from_str(include_str!("testdata/reviewed_oracle.json")).unwrap();
        let to = vec!["person@example.com".to_string()];
        for kind in ["reply", "forward"] {
            let p = mail_plan(&api(), kind, "support@example.com", &to).unwrap();
            let payload = p.payload();
            assert_eq!(payload["from_email"], "support@example.com");
            let key = if kind == "reply" {
                "bcc_emails"
            } else {
                "notify_emails"
            };
            assert_eq!(payload[key], json!(["person@example.com"]));
            assert_eq!(
                p.payload_json, oracle["conversation"][kind]["payload_json"],
                "{kind}"
            );
            assert_eq!(p.hash(), oracle["conversation"][kind]["hash"], "{kind}");
            if kind == "forward" {
                assert!(p.path().ends_with("/forward"));
                assert_eq!(
                    payload,
                    json!({"body": "<div>Acme update</div>", "from_email": "support@example.com",
                    "notify_emails": ["person@example.com"], "conversation_from_summary": false, "private": true})
                );
            } else {
                assert!(payload.get("cc_emails").is_none());
            }
            let changed = mail_plan(&api(), kind, "other@example.com", &to).unwrap();
            assert_ne!(p.hash(), changed.hash());
            let mut row = json!({"id": 456});
            row.as_object_mut()
                .unwrap()
                .extend(payload.as_object().unwrap().clone());
            let c = api()
                .ok(
                    &format!("POST {}", p.path()),
                    json!({"conversation": {"id": 456}}),
                )
                .ok(
                    "GET tickets/123/conversations",
                    json!({"conversations": [row]}),
                );
            let j = MemJournal::default();
            assert_eq!(
                err(send(
                    &p,
                    &c,
                    &j,
                    &MemStore::default(),
                    Some(&p.hash()),
                    false
                )),
                "public conversation requires explicit confirmation"
            );
            let out = send(&p, &c, &j, &MemStore::default(), Some(&p.hash()), true).unwrap();
            assert_eq!(out["verified"], true, "{out}");
            assert_eq!(c.writes().len(), 1);
            assert!(c.gets().contains(&"autocomplete/requesters".to_string()));
        }
    }

    /// `test_email_sets_allow_order_case_and_duplicates_but_not_invalid_data`
    /// and the alias rows from `test_recipient_readback_http`.
    #[test]
    fn recipient_readback_matches_python() {
        let row = Map::from_iter([(
            "bcc_emails".into(),
            json!([
                "SECOND@example.com",
                "Person <person@example.com>",
                "PERSON@example.com"
            ]),
        )]);
        assert_eq!(
            recipient_readback(
                Some(&row),
                "bcc_emails",
                &json!(["person@example.com", "second@example.com"])
            ),
            "matched"
        );
        assert_eq!(
            recipient_readback(
                Some(&Map::from_iter([(
                    "bcc_emails".into(),
                    json!(["not-an-email"])
                )])),
                "bcc_emails",
                &json!([])
            ),
            "mismatched"
        );
        assert_eq!(
            recipient_readback(
                Some(&Map::from_iter([("bcc_emails".into(), json!([]))])),
                "bcc_emails",
                &json!([])
            ),
            "matched"
        );
        assert_eq!(
            recipient_readback(Some(&Map::new()), "bcc_emails", &json!([])),
            "unavailable"
        );
        let forward = Map::from_iter([(
            "to_emails".into(),
            json!(["SECOND@example.com", "Person <PERSON@example.com>"]),
        )]);
        assert_eq!(
            recipient_readback(
                Some(&forward),
                "notify_emails",
                &json!(["person@example.com", "second@example.com"])
            ),
            "matched"
        );
        let disagree = Map::from_iter([
            (
                "to_emails".into(),
                json!(["person@example.com", "second@example.com"]),
            ),
            ("notify_emails".into(), json!(["wrong@example.com"])),
        ]);
        assert_eq!(
            recipient_readback(
                Some(&disagree),
                "notify_emails",
                &json!(["person@example.com", "second@example.com"])
            ),
            "mismatched"
        );
        assert_eq!(
            recipient_readback(
                Some(&Map::from_iter([(
                    "cc_emails".into(),
                    json!(["person@example.com", "second@example.com"])
                )])),
                "notify_emails",
                &json!(["person@example.com", "second@example.com"])
            ),
            "unavailable"
        );
        assert_eq!(
            recipient_readback(
                Some(&Map::from_iter([(
                    "to_emails".into(),
                    json!(["person@example.com"])
                )])),
                "bcc_emails",
                &json!(["person@example.com"])
            ),
            "unavailable"
        );
    }

    fn source_row() -> Value {
        json!({"id": 901, "body": "<div>Original Acme message</div>"})
    }

    fn forward_source_plan(c: &FakeApi) -> Result<ConversationPlan> {
        let to = vec!["person@example.com".to_string()];
        build_plan_forward_source(
            c,
            &MemStore::default(),
            "123",
            "Acme update",
            &ConversationArgs {
                kind: "forward",
                notify: &to,
                ..Default::default()
            },
            &MailArgs {
                sender: Some("support@example.com"),
                ..MailArgs::default()
            },
            901,
        )
    }

    /// `test_missing_source_blocks_forward` and `test_source_drift_blocks_forward`.
    #[test]
    fn exact_forward_source_is_bound_and_drift_sends_nothing() {
        let missing = api().ok(
            "GET tickets/123/conversations",
            json!({"conversations": []}),
        );
        assert!(err(forward_source_plan(&missing)).contains("exact forward source"));
        assert!(missing.writes().is_empty());
        let row = source_row();
        let c = api().ok(
            "GET tickets/123/conversations",
            json!({"conversations": [row]}),
        );
        let p = forward_source_plan(&c).unwrap();
        assert_eq!(p.payload()["conversation_id"], 901);
        assert_eq!(
            p.record_hash.as_deref(),
            Some(snapshot_hash(&source_row()).unwrap().as_str())
        );
        let drifted = api().ok(
            "GET tickets/123/conversations",
            json!({"conversations": [{"id": 901, "body": "Changed"}]}),
        );
        let (j, store) = (MemJournal::default(), MemStore::default());
        assert!(
            err(send(&p, &drifted, &j, &store, Some(&p.hash()), true)).contains("source changed")
        );
        assert!(drifted.writes().is_empty());
        assert!(
            err(build_plan_forward_source(
                &c,
                &store,
                "123",
                "Acme update",
                &ConversationArgs::default(),
                &MailArgs::default(),
                901,
            ))
            .contains("only supported for forward")
        );
    }

    /// `test_formatted_sender_forward_readback`.
    #[test]
    fn formatted_sender_forward_readback() {
        let c = api()
            .ok(
                "POST tickets/123/forward",
                json!({"conversation": {"id": 456}}),
            )
            .ok(
                "GET tickets/123/conversations",
                json!({"conversations": [source_row(), {
                    "id": 456,
                    "body": "<div>Acme update</div>",
                    "private": true,
                    "from_email": "\"Acme Support\" <SUPPORT@example.com>",
                    "to_emails": ["Acme Person <PERSON@example.com>"]
                }]}),
            );
        let p = forward_source_plan(&c).unwrap();
        let (j, store) = (MemJournal::default(), MemStore::default());
        let out = send(&p, &c, &j, &store, Some(&p.hash()), true).unwrap();
        assert_eq!(out["verified"], true, "{out}");
        assert_eq!(out["fields"]["from_email"], "matched");
        assert_eq!(out["source_link_verified"], false);
        assert_eq!(out["source_link_readback"], "unavailable");
        assert_eq!(out["source_conversation_id"], 901);
        assert_eq!(
            out["verification_scope"],
            "delivery fields; source link is not exposed by independent readback"
        );
        assert_eq!(c.writes().len(), 1);
    }

    /// `test_unexposed_source_link_is_unavailable_not_failed` and
    /// `test_delivery_mismatch_still_fails_with_source_unavailable`.
    #[test]
    fn unexposed_source_link_matches_python() {
        let oracle: Value =
            serde_json::from_str(include_str!("testdata/forward_source_link_oracle.json")).unwrap();
        let cases = oracle["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 8);
        for case in cases {
            let mut row = json!({
                "id": 456,
                "body": "<div>Acme update</div>",
                "private": true,
                "from_email": "support@example.com",
                "to_emails": ["person@example.com"]
            });
            for (k, v) in case["extra"].as_object().unwrap() {
                row[k] = v.clone();
            }
            let c = api()
                .ok(
                    "POST tickets/123/forward",
                    json!({"conversation": {"id": 456}}),
                )
                .ok(
                    "GET tickets/123/conversations",
                    json!({"conversations": [source_row(), row]}),
                );
            let p = forward_source_plan(&c).unwrap();
            let (j, store) = (MemJournal::default(), MemStore::default());
            let out = send(&p, &c, &j, &store, Some(&p.hash()), true).unwrap();
            let name = case["name"].as_str().unwrap();
            for key in [
                "verified",
                "fields",
                "source_conversation_id",
                "source_link_readback",
                "source_link_verified",
                "verification_scope",
            ] {
                assert_eq!(out[key], case[key], "{name} {key}");
            }
            assert_eq!(
                json!(out.get("warning").is_some()),
                case["has_warning"],
                "{name}"
            );
            assert_eq!(json!(c.writes().len()), case["posts"], "{name}");
        }
    }

    /// A forward whose server readback stores recipients as `to_emails`.
    #[test]
    fn forward_to_emails_readback_verifies() {
        let to = vec!["person@example.com".to_string()];
        let p = mail_plan(&api(), "forward", "support@example.com", &to).unwrap();
        let c = api()
            .ok(
                &format!("POST {}", p.path()),
                json!({"conversation": {"id": 456}}),
            )
            .ok(
                "GET tickets/123/conversations",
                json!({"conversations": [{
                    "id": 456,
                    "body": "<div>Acme update</div>",
                    "private": true,
                    "from_email": "support@example.com",
                    "to_emails": ["Person <PERSON@example.com>"]
                }]}),
            );
        let (j, store) = (MemJournal::default(), MemStore::default());
        let out = send(&p, &c, &j, &store, Some(&p.hash()), true).unwrap();
        assert_eq!(out["fields"]["notify_emails"], "matched");
        assert_eq!(out["verified"], true, "{out}");
        assert_eq!(c.writes().len(), 1);
    }

    /// `test_bcc_missing_readback_is_unavailable_not_verified`.
    #[test]
    fn bcc_missing_readback_is_unavailable() {
        let to = vec!["person@example.com".to_string()];
        let p = mail_plan(&api(), "reply", "support@example.com", &to).unwrap();
        let c = api()
            .ok("POST tickets/123/reply", json!({"conversation": {"id": 456}}))
            .ok("GET tickets/123/conversations", json!({"conversations": [{"id": 456, "body": "<div>Acme update</div>", "cc_emails": []}]}));
        let (j, store) = (MemJournal::default(), MemStore::default());
        let out = send(&p, &c, &j, &store, Some(&p.hash()), true).unwrap();
        assert_eq!(out["verified"], false);
        assert_eq!(out["fields"]["bcc_emails"], "unavailable");
        assert_eq!(out["fields"]["from_email"], "unavailable");
        assert!(err(send(&p, &c, &j, &store, Some(&p.hash()), true)).contains("was sent"));
        assert_eq!(c.writes().len(), 1);
    }

    /// `test_forward_uncaptured_variants_refused` plus the other refusals.
    #[test]
    fn mail_variants_refused() {
        let c = api();
        let store = MemStore::default();
        let to = vec!["person@example.com".to_string()];
        let named = vec![CC.to_string()];
        let support = Some("support@example.com");
        let plan = |args: ConversationArgs<'_>, mail: MailArgs<'_>| {
            build_plan_mail(&c, &store, "123", "Hello", &args, &mail)
        };
        let reply = ConversationArgs::default;
        let cases = [
            (
                reply(),
                MailArgs {
                    bcc: &to,
                    sender: None,
                },
                "BCC requires a configured --sender and bare recipient emails",
            ),
            (
                ConversationArgs {
                    kind: "forward",
                    notify: &to,
                    set_status: Some("Pending"),
                    ..reply()
                },
                MailArgs {
                    sender: support,
                    ..Default::default()
                },
                "whole-ticket forward requires explicit sender and recipients; no combined status/CC/BCC wire is proven",
            ),
            (
                reply(),
                MailArgs {
                    sender: Some("Support <support@example.com>"),
                    ..Default::default()
                },
                "sender must be a plain configured support email address",
            ),
            (
                ConversationArgs {
                    kind: "note",
                    ..reply()
                },
                MailArgs {
                    sender: support,
                    ..Default::default()
                },
                "note sender/BCC wire is not proven",
            ),
            (
                ConversationArgs {
                    cc: &named,
                    ..reply()
                },
                MailArgs {
                    bcc: &to,
                    sender: support,
                },
                "combined BCC/CC/status wire is not proven",
            ),
            (
                reply(),
                MailArgs {
                    bcc: &named,
                    sender: support,
                },
                "BCC requires a configured --sender and bare recipient emails",
            ),
        ];
        for (args, mail, want) in cases {
            assert_eq!(err(plan(args, mail)), want);
        }
        let sender_only = plan(
            reply(),
            MailArgs {
                sender: support,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(sender_only.payload()["cc_emails"], json!([]));
        assert_eq!(sender_only.payload()["from_email"], "support@example.com");
    }

    /// `test_ticket_conversation_http.py::test_changed_cc_identity_refuses_instead_of_changing_reviewed_payload`.
    #[test]
    fn changed_cc_identity_refuses() {
        let cc = vec![CC.to_string()];
        let p = build_plan(
            &api(),
            &MemStore::default(),
            "123",
            "Acme update",
            &ConversationArgs {
                cc: &cc,
                ..Default::default()
            },
        )
        .unwrap();
        let c = ticket_api(json!({"ticket": row()}))
            .ok(
                "GET autocomplete/requesters",
                json!([{"id": 11, "value": "Different Operator", "email": "person@example.com"}]),
            )
            .ok(
                "POST tickets/123/reply",
                json!({"conversation": {"id": 456}}),
            );
        assert_eq!(
            err(send(
                &p,
                &c,
                &MemJournal::default(),
                &MemStore::default(),
                Some(&p.hash()),
                true
            )),
            "CC recipient identity changed; review again"
        );
        assert!(c.writes().is_empty());
        assert_eq!(
            parseaddr("Acme User <person@example.com>"),
            "person@example.com"
        );
        assert_eq!(parseaddr("person@example.com"), "person@example.com");
    }

    /// `test_ticket_conversation_http.py::test_missing_or_wrong_conversation_readback_never_resends`
    /// and `::test_each_conversation_write_caller_leaves_ambiguity_guard`.
    #[test]
    fn readback_gaps_and_ambiguous_failures_never_resend() {
        let cc = vec![CC.to_string()];
        let p = build_plan(
            &api(),
            &MemStore::default(),
            "123",
            "Acme update",
            &ConversationArgs {
                cc: &cc,
                ..Default::default()
            },
        )
        .unwrap();
        let body = "<div>Acme update</div>";
        for conv in [
            json!({"id": 456, "cc_emails": [CC]}),
            json!({"id": 456, "body": body}),
            json!({"id": 999, "body": body, "cc_emails": [CC]}),
            json!({"id": 456, "body": "<div>Other</div>", "cc_emails": [CC]}),
        ] {
            let c = api()
                .ok(
                    "POST tickets/123/reply",
                    json!({"conversation": {"id": 456}}),
                )
                .ok(
                    "GET tickets/123/conversations",
                    json!({"conversations": [conv]}),
                );
            let (j, store) = (MemJournal::default(), MemStore::default());
            let out = send(&p, &c, &j, &store, Some(&p.hash()), true).unwrap();
            assert_eq!(out["verified"], false);
            assert!(out["warning"].as_str().unwrap().contains("do not resend"));
            assert!(err(send(&p, &c, &j, &store, Some(&p.hash()), true)).contains("was sent"));
            assert_eq!(c.writes().len(), 1);
        }
        let n = note_plan(&api());
        for status in [403, 429, 500] {
            let c = FakeApi::new().on("POST tickets/123/notes", Reply::Status(status));
            let j = MemJournal::default();
            for _ in 0..2 {
                assert!(
                    err(send(
                        &n,
                        &c,
                        &j,
                        &MemStore::default(),
                        Some(&n.hash()),
                        false
                    ))
                    .contains("unknown")
                );
            }
            assert_eq!(c.writes().len(), 1);
        }
    }
}
