//! Reviewed ticket transitions; resolution text persists before the status
//! write (`ticket_lifecycle.py`).

use std::sync::Arc;

use serde_json::{Map, Value, json};

use fsv_core::client::Body;
use fsv_core::error::{Error, Result};
use fsv_core::pystr::{casefold, strip};
use fsv_core::resource::{TICKETS, parse_id};
use fsv_forms::payload::encode_value;
use fsv_forms::{FormState, Kind, LookupResolver, StateOptions, build_br_validation_excludes};

use crate::formmeta::{
    MetaStore, RULES_UNAVAILABLE_KEY, existing_from_record, form_err, is_status, load_meta,
};
use crate::notes::{self, BodyFormat};
use crate::properties::record;
use crate::readback::{py_equal, resolution_text, same_text, submitted_value_matches};
use crate::reviewed_write::refused;
use crate::service::{Api, Journal, last_event, obj, plan_hash, snapshot_hash};

pub const ACTION: &str = "ticket_transition";

const WARNING: &str = "Resolution save and transition are separate writes, not atomic. Unknown or partial outcomes must not be resent. Tenant workflows may notify people.";
const ACK_WARNING: &str = " Acknowledged blocking rule messages are tenant policy: the web form blocks submit on them, and the server or a workflow may still reject or revert the transition.";
/// How far [`send`] got before a failure (`_partial_report`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Start,
    NoteSent,
    NoteSaved,
    StatusSent,
}

/// `_status_text`: the readback status (and name) for a partial report.
fn status_text(row: Option<&Map<String, Value>>) -> String {
    let Some(row) = row else {
        return "status unavailable on readback".into();
    };
    let Some(status) = row.get("status").filter(|s| !s.is_null()) else {
        return "status unavailable on readback".into();
    };
    let name = match row.get("status_name") {
        Some(Value::String(n)) if !n.is_empty() => format!(" {n}"),
        _ => String::new(),
    };
    format!("readback status {}{name}", fsv_py::py_str(status))
}

/// `_next_step`: the readback-based next step after a held status.
fn next_step(plan: &LifecyclePlan) -> String {
    let t = plan.target;
    format!(
        "Next: inspect `fsv tickets get {t}` and `fsv tickets activity {t}` for the workflow/\
         approval/rule entry; once it no longer holds the ticket, review the status alone with \
         `fsv tickets status {t} {} --dry-run` (the note is already saved). Do not resend this plan.",
        plan.status
    )
}

/// `_partial_report`: the explicit outcome of a failure between or after
/// the two transition writes. Reads the ticket once more; a failed read
/// reports the status as unavailable.
fn partial_report(c: &dyn Api, plan: &LifecyclePlan, phase: Phase, exc: &Error) -> String {
    let row = record(c, plan.target).ok();
    let status = status_text(row.as_ref());
    let mut reason = match exc {
        Error::Exit { message, .. } => message.clone(),
        _ => String::new(),
    };
    if let Some(api) = exc.as_api().filter(|a| (400..500).contains(&a.status)) {
        let msg = fsv_core::error::server_message(&api.body, 240);
        reason = if msg.is_empty() {
            format!("HTTP {}", api.status)
        } else {
            format!("HTTP {}: {msg}", api.status)
        };
    }
    match phase {
        Phase::NoteSaved => format!(
            "resolution note saved and verified; status unchanged ({status}): transition not attempted because {}. {}",
            if reason.is_empty() {
                "the ticket changed after the note save"
            } else {
                &reason
            },
            next_step(plan)
        ),
        Phase::StatusSent if plan.note_html.is_some() => {
            let what = if reason.is_empty() {
                "the status write outcome is unknown".to_string()
            } else {
                format!("the status write was rejected ({reason})")
            };
            format!(
                "resolution note saved; {what}; {status}. {}",
                next_step(plan)
            )
        }
        Phase::NoteSent => format!(
            "resolution note save outcome unknown{}; transition not attempted; {status}. Inspect the ticket and journal; do not resend this plan.",
            if reason.is_empty() {
                String::new()
            } else {
                format!(" ({reason})")
            }
        ),
        _ => format!(
            "transition outcome unknown or partial{}; {status}; inspect ticket and journal; do not resend",
            if reason.is_empty() {
                String::new()
            } else {
                format!(" ({reason})")
            }
        ),
    }
}

fn dumps(v: &Value) -> String {
    fsv_py::Dumps::default().sort_keys(true).dump(v)
}

/// `context(client, cid, status, answers, resolution=...)`: the transition
/// payload, the record+metadata fingerprint JSON and the runtime status label.
#[allow(clippy::type_complexity)]
pub fn context(
    c: &dyn Api,
    store: &dyn MetaStore,
    cid: i64,
    status: &str,
    answers: &Map<String, Value>,
    resolution: Option<&str>,
    resolver: Option<Arc<dyn LookupResolver>>,
) -> Result<(Map<String, Value>, String, String, Vec<String>)> {
    context_ack(
        c, store, cid, status, answers, resolution, resolver, None, false,
    )
}

/// [`context`] over an already read `row`, with `ack` acknowledging the
/// blocking rule messages. Acknowledged texts join the fingerprint only
/// when there are any, so unacknowledged plan hashes stay stable.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn context_ack(
    c: &dyn Api,
    store: &dyn MetaStore,
    cid: i64,
    status: &str,
    answers: &Map<String, Value>,
    resolution: Option<&str>,
    resolver: Option<Arc<dyn LookupResolver>>,
    row: Option<Map<String, Value>>,
    ack: bool,
) -> Result<(Map<String, Value>, String, String, Vec<String>)> {
    let other = |e: fsv_forms::FormError| Error::other(e.to_string());
    let row = match row {
        Some(row) => row,
        None => record(c, cid)?,
    };
    let meta = load_meta(c, store, "ticket", "edit", true, &json!(cid))?;
    if meta
        .rules
        .get(RULES_UNAVAILABLE_KEY)
        .is_some_and(fsv_py::truthy)
    {
        return Err(refused("ticket rules unavailable; no write attempted"));
    }
    let spec = meta.spec();
    let mut state = FormState::new(
        spec.clone(),
        Some(meta.engine()?),
        "edit",
        None,
        StateOptions {
            resolver,
            existing: Some(existing_from_record(&spec, &row)),
            ..StateOptions::default()
        },
    )
    .map_err(form_err)?;
    let fields: Vec<_> = spec.fields().iter().filter(|f| is_status(f)).collect();
    if fields.len() != 1 {
        return Err(refused("status metadata is not unique"));
    }
    let field = fields[0];
    let want = casefold(status);
    let choices: Vec<_> = field
        .choices
        .iter()
        .filter(|ch| ch.visible && (casefold(&ch.label) == want || casefold(&ch.value) == want))
        .collect();
    let id = match choices.as_slice() {
        [one] => crate::choice_ids::runtime_choice_id(one.id.as_ref(), 1),
        _ => None,
    };
    let Some(id) = id else {
        return Err(refused(
            "status must identify one visible runtime status label",
        ));
    };
    let choice = choices[0];
    let label = casefold(&choice.label);
    let closing = label == "resolved" || label == "closed";
    state
        .set(
            &field.api_name,
            fsv_forms::Value::from(choice.value.clone()),
        )
        .map_err(other)?;
    let mut seen: Vec<String> = Vec::new();
    for (name, value) in answers {
        let f = spec.field(name).map_err(other)?;
        if seen.contains(&f.api_name)
            || !f.is_custom
            || !f.sent
            || f.synthetic
            || f.kind == Kind::Notice
        {
            return Err(refused(format!(
                "{}: --set requires unique writable custom fields",
                f.label
            )));
        }
        seen.push(f.api_name.clone());
        state
            .set(&f.api_name, fsv_forms::Value::from_json(value))
            .map_err(other)?;
    }
    if !state.evaluated.visible.contains(&field.api_name) {
        return Err(refused(
            "status is hidden on the ticket form; no write attempted",
        ));
    }
    let gate = state.rule_gate(&[]).map_err(other)?;
    if !gate.blockers.is_empty() {
        return Err(refused(format!(
            "ticket rules block this transition; no write attempted: {}",
            gate.blockers.join("; ")
        )));
    }
    // Closure flags are separate from ordinary edit requiredness. Hidden
    // fields are not requested by the native closure form.
    let mut required = state.evaluated.required.clone();
    if closing {
        for raw in meta.raw_fields() {
            if raw.get("required_for_closure") == Some(&Value::Bool(true)) {
                let name = raw.get("name").map_or(String::new(), fsv_py::py_str);
                let Some(f) = spec.find(&name) else {
                    let shown = ["name", "label"]
                        .iter()
                        .find_map(|k| raw.get(*k).filter(|v| fsv_py::truthy(v)))
                        .map_or("None".to_string(), fsv_py::py_str);
                    return Err(refused(format!("unknown required fields: {shown}")));
                };
                required.insert(f.api_name.clone());
            }
        }
    }
    let mut missing = Vec::new();
    for f in state.visible_fields() {
        if !required.contains(&f.api_name) {
            continue;
        }
        let value = state.get(&f.api_name).map_err(other)?;
        if fsv_forms::rules::is_empty(value)
            || (f.kind == Kind::Checkbox && !matches!(value, Some(fsv_forms::Value::Bool(true))))
        {
            missing.push(format!("{} ({})", f.label, f.api_name));
        }
    }
    if !missing.is_empty() {
        return Err(refused(format!(
            "required fields: {}; supply --set FIELD=VALUE",
            missing.join(", ")
        )));
    }
    if ack {
        state.acknowledge_all();
    }
    let acknowledged = crate::properties::acknowledged_texts(&state);
    let errors = state.errors().map_err(other)?;
    if !errors.is_empty() {
        return Err(refused(format!(
            "ticket form validation failed: {}{}",
            errors.join("; "),
            crate::properties::ack_hint(&state)
        )));
    }
    let mut custom = Map::new();
    for name in &seen {
        let f = spec.field(name).map_err(other)?;
        if !state.evaluated.visible.contains(name) {
            return Err(refused(format!("{}: answered field is hidden", f.label)));
        }
        let value = state
            .get(name)
            .map_err(other)?
            .cloned()
            .unwrap_or(fsv_forms::Value::Null);
        custom.insert(
            name.clone(),
            encode_value(&spec, f, &value, &state.evaluated.values).map_err(other)?,
        );
    }
    let persisted = match resolution {
        Some(r) => r.to_string(),
        None => resolution_text(&Value::Object(row.clone())).unwrap_or_default(),
    };
    if closing && strip(&persisted).is_empty() {
        return Err(refused(
            "a persisted resolution note is required; use tickets resolve ID --note TEXT first",
        ));
    }
    let token =
        build_br_validation_excludes(&state.evaluated, "ticket", &Value::Null, &Value::Null)
            .map_err(other)?;
    let mut payload = obj(json!({"status": id, "br_validation_excludes": token}));
    // Native transitions preserve routing, including explicit nulls.
    for key in ["group_id", "responder_id"] {
        let Some(v) = row.get(key) else {
            return Err(refused(format!("ticket routing field unavailable: {key}")));
        };
        payload.insert(key.into(), v.clone());
    }
    if !custom.is_empty() {
        payload.insert("custom_fields".into(), Value::Object(custom));
    }
    let mut bound = json!({
        "record": snapshot_hash(&Value::Object(row))?,
        "metadata": meta.fingerprint().hash,
    });
    if !acknowledged.is_empty() {
        bound["acknowledged"] = json!(acknowledged);
    }
    let fingerprint = dumps(&bound);
    Ok((payload, fingerprint, choice.label.clone(), gate.warnings))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecyclePlan {
    pub target: i64,
    pub tenant: String,
    pub status: String,
    pub answers_json: String,
    pub payload_json: String,
    pub context_hash: String,
    pub note_html: Option<String>,
    /// Display only. Not part of [`LifecyclePlan::hash`].
    pub rule_warnings: Vec<String>,
}

impl LifecyclePlan {
    pub fn payload(&self) -> Map<String, Value> {
        serde_json::from_str(&self.payload_json).unwrap_or_default()
    }

    pub fn answers(&self) -> Map<String, Value> {
        serde_json::from_str(&self.answers_json).unwrap_or_default()
    }

    pub fn hash(&self) -> String {
        plan_hash(&json!({
            "target": self.target, "tenant": self.tenant, "status": self.status,
            "answers_json": self.answers_json, "payload_json": self.payload_json,
            "context_hash": self.context_hash, "note_html": self.note_html,
        }))
    }

    pub fn review(&self) -> Map<String, Value> {
        let payload = self.payload();
        let path = format!("/api/_/tickets/{}", self.target);
        let mut steps = Vec::new();
        if let Some(html) = &self.note_html {
            steps.push(json!({"method": "PUT", "path": path, "payload": {
                "resolution_notes": html,
                "br_validation_excludes": payload.get("br_validation_excludes"),
            }}));
        }
        steps.push(json!({"method": "PUT", "path": path, "payload": payload}));
        let mut out = obj(
            json!({"action": ACTION, "status_label": self.status, "steps": steps,
                   "plan_hash": self.hash(), "warning": WARNING}),
        );
        if !self.rule_warnings.is_empty() {
            out.insert("rule_warnings".into(), json!(self.rule_warnings));
        }
        let acknowledged = self.acknowledged();
        if !acknowledged.is_empty() {
            out.insert("acknowledged_rule_messages".into(), json!(acknowledged));
            out.insert("warning".into(), json!(format!("{WARNING}{ACK_WARNING}")));
        }
        out
    }

    /// Acknowledged blocking rule message texts bound in `context_hash`.
    pub fn acknowledged(&self) -> Vec<String> {
        serde_json::from_str::<Value>(&self.context_hash)
            .ok()
            .and_then(|v| serde_json::from_value(v.get("acknowledged")?.clone()).ok())
            .unwrap_or_default()
    }

    fn record_hash(&self) -> Option<Value> {
        serde_json::from_str::<Value>(&self.context_hash)
            .ok()
            .and_then(|v| v.get("record").cloned())
    }

    fn metadata_hash(&self) -> Option<Value> {
        serde_json::from_str::<Value>(&self.context_hash)
            .ok()
            .and_then(|v| v.get("metadata").cloned())
    }
}

/// `build_plan(client, target, status, note=..., answers=...)`; `note` is
/// plain text saved as the resolution before a Resolved transition.
pub fn build_plan(
    c: &dyn Api,
    store: &dyn MetaStore,
    target: &str,
    status: &str,
    note: Option<&str>,
    answers: Option<&Map<String, Value>>,
    resolver: Option<Arc<dyn LookupResolver>>,
) -> Result<LifecyclePlan> {
    build_plan_ack(c, store, target, status, note, answers, resolver, false)
}

/// [`build_plan`] with `ack=True`: blocking rule messages are acknowledged
/// and bound to the review hash.
#[allow(clippy::too_many_arguments)]
pub fn build_plan_ack(
    c: &dyn Api,
    store: &dyn MetaStore,
    target: &str,
    status: &str,
    note: Option<&str>,
    answers: Option<&Map<String, Value>>,
    resolver: Option<Arc<dyn LookupResolver>>,
    ack: bool,
) -> Result<LifecyclePlan> {
    let cid = parse_id(target, &TICKETS)?;
    if cid <= 0 {
        return Err(Error::value("ticket ID must be positive"));
    }
    if note.is_some() && casefold(status) != "resolved" {
        return Err(Error::value("--note is supported by resolve only"));
    }
    let tenant = c.domain();
    let html = match note {
        Some(n) => Some(
            notes::build_plan(&cid.to_string(), n, false, &[], BodyFormat::Plain, &tenant)?
                .body_html,
        ),
        None => None,
    };
    let empty = Map::new();
    let answers = answers.unwrap_or(&empty);
    let (payload, fingerprint, label, warnings) = context_ack(
        c,
        store,
        cid,
        status,
        answers,
        html.as_deref(),
        resolver,
        None,
        ack,
    )?;
    Ok(LifecyclePlan {
        target: cid,
        tenant,
        status: label,
        answers_json: dumps(&Value::Object(answers.clone())),
        payload_json: dumps(&Value::Object(payload)),
        context_hash: fingerprint,
        note_html: html,
        rule_warnings: warnings,
    })
}

/// The readback holds every submitted key: custom fields by submitted
/// value, others by Python `==` and never a boolean.
pub fn matches(row: &Map<String, Value>, payload: &Map<String, Value>) -> bool {
    payload.iter().all(|(key, value)| match key.as_str() {
        "br_validation_excludes" => true,
        "custom_fields" => submitted_value_matches(row.get(key).unwrap_or(&Value::Null), value),
        _ => row
            .get(key)
            .is_some_and(|v| !v.is_boolean() && py_equal(v, value)),
    })
}

/// `(verified, notes)` for a transition readback. The status and the
/// resolution note decide the outcome. A status that moved away from the
/// pre-write status is a workflow rewrite, verified with a note; so is any
/// other submitted key the server rewrote. An unchanged status or a lost
/// note is a mismatch.
pub fn readback(
    before: Option<&Map<String, Value>>,
    row: &Map<String, Value>,
    payload: &Map<String, Value>,
    note_html: Option<&str>,
) -> (bool, Vec<String>) {
    let mut notes = Vec::new();
    if let Some(html) = note_html {
        let saved = resolution_text(&Value::Object(row.clone())).map_or(Value::Null, Value::String);
        if !same_text(&saved, &json!(html)) {
            return (false, vec!["resolution note not found on readback".into()]);
        }
    }
    let want = payload.get("status").unwrap_or(&Value::Null);
    let got = match row.get("status") {
        None | Some(Value::Null | Value::Bool(_)) => {
            return (false, vec!["status missing on readback".into()]);
        }
        Some(got) => got,
    };
    if fsv_py::py_str(got) != fsv_py::py_str(want) {
        let prior = before
            .and_then(|b| b.get("status"))
            .filter(|v| !v.is_null());
        match prior {
            Some(prior) if fsv_py::py_str(got) != fsv_py::py_str(prior) => notes.push(format!(
                "status rewritten server-side to {} after the write (workflow)",
                fsv_py::py_str(got)
            )),
            _ => {
                return (
                    false,
                    vec![format!("status is still {}", fsv_py::py_str(got))],
                );
            }
        }
    }
    for (key, value) in payload {
        if matches!(key.as_str(), "status" | "br_validation_excludes") {
            continue;
        }
        let same = if key == "custom_fields" {
            submitted_value_matches(row.get(key).unwrap_or(&Value::Null), value)
        } else {
            row.get(key)
                .is_some_and(|v| !v.is_boolean() && py_equal(v, value))
        };
        if !same {
            notes.push(format!(
                "{key} differs on readback (server-side workflow or normalization)"
            ));
        }
    }
    (true, notes)
}

/// `send(plan, client, reviewed_hash=...)`: the optional resolution PUT,
/// then the transition PUT, each verified by readback; never retried.
pub fn send(
    plan: &LifecyclePlan,
    c: &dyn Api,
    store: &dyn MetaStore,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
    resolver: Option<Arc<dyn LookupResolver>>,
) -> Result<Value> {
    let Some(reviewed) = reviewed_hash else {
        return Ok(json!({"status": "cancelled", "verified": false}));
    };
    let hash = plan.hash();
    if plan.tenant.is_empty() || c.domain() != plan.tenant || reviewed != hash {
        return Err(refused("tenant or full review hash changed; review again"));
    }
    if matches!(
        last_event(journal, ACTION, &hash).as_deref(),
        Some("started" | "done")
    ) {
        return Err(refused(
            "transition was sent or outcome unknown; inspect ticket and journal before retry",
        ));
    }
    let answers = plan.answers();
    let before = record(c, plan.target)?;
    let ack = !plan.acknowledged().is_empty();
    let (payload, fingerprint, _, _) = context_ack(
        c,
        store,
        plan.target,
        &plan.status,
        &answers,
        plan.note_html.as_deref(),
        resolver.clone(),
        Some(before.clone()),
        ack,
    )?;
    if fingerprint != plan.context_hash
        || !py_equal(
            &Value::Object(payload.clone()),
            &Value::Object(plan.payload()),
        )
    {
        return Err(refused("ticket or metadata changed; review again"));
    }
    let csrf = c.get_csrf()?;
    let target = json!(plan.target);
    journal.record_plan("started", &hash, ACTION, Some(&target), None);
    if last_event(journal, ACTION, &hash).as_deref() != Some("started") {
        return Err(refused(
            "could not persist ambiguity journal; no write attempted",
        ));
    }
    let path = format!("tickets/{}", plan.target);
    let put = |body: &Value| -> Result<Value> {
        c.send_once(
            "PUT",
            &path,
            Body::Raw(dumps(body).into_bytes()),
            None,
            &csrf,
        )
    };
    let note_saved = |after: &Map<String, Value>, html: &str| {
        same_text(
            &resolution_text(&Value::Object(after.clone())).map_or(Value::Null, Value::String),
            &json!(html),
        )
    };
    // The note and the status are separate writes, and tenant workflows or
    // rules can act between them; the phase names what happened.
    let phase = std::cell::Cell::new(Phase::Start);
    let attempt = || -> Result<(bool, Vec<String>, Map<String, Value>)> {
        if let Some(html) = &plan.note_html {
            let baseline = record(c, plan.target)?;
            if Some(json!(snapshot_hash(&Value::Object(baseline.clone()))?)) != plan.record_hash() {
                return Err(refused("ticket changed before note save"));
            }
            phase.set(Phase::NoteSent);
            put(&json!({"resolution_notes": html,
                        "br_validation_excludes": payload.get("br_validation_excludes")}))?;
            let after = record(c, plan.target)?;
            if !note_saved(&after, html) {
                return Err(refused(
                    "resolution save was not verified; transition not attempted",
                ));
            }
            phase.set(Phase::NoteSaved);
            // A saved note may update derived fields, but must not silently
            // change routing, status, or existing answers.
            let changed: Vec<&str> = ["status", "group_id", "responder_id", "custom_fields"]
                .into_iter()
                .filter(|key| {
                    !after
                        .get(*key)
                        .is_some_and(|v| py_equal(v, baseline.get(*key).unwrap_or(&Value::Null)))
                })
                .collect();
            if !changed.is_empty() {
                return Err(refused(format!(
                    "the ticket changed during the note save ({}), typically a tenant workflow or rule",
                    changed.join(", ")
                )));
            }
            let (fresh, fresh_context, _, _) = context_ack(
                c,
                store,
                plan.target,
                &plan.status,
                &answers,
                None,
                resolver.clone(),
                None,
                ack,
            )?;
            let fresh_meta = serde_json::from_str::<Value>(&fresh_context)
                .ok()
                .and_then(|v| v.get("metadata").cloned());
            if !py_equal(&Value::Object(fresh), &Value::Object(payload.clone()))
                || fresh_meta != plan.metadata_hash()
            {
                return Err(refused(
                    "transition requirements changed after the note save",
                ));
            }
        }
        phase.set(Phase::StatusSent);
        put(&Value::Object(payload.clone()))?;
        let after = record(c, plan.target)?;
        let (verified, notes) =
            readback(Some(&before), &after, &payload, plan.note_html.as_deref());
        Ok((verified, notes, after))
    };
    let notes = match attempt() {
        Err(e) => return Err(refused(partial_report(c, plan, phase.get(), &e))),
        Ok((false, notes, after)) => {
            if plan.note_html.is_some()
                && notes
                    .first()
                    .is_some_and(|n| n.starts_with("status is still"))
            {
                return Err(refused(format!(
                    "resolution note saved; status unchanged ({}): the status write was accepted but \
                     the ticket stayed put, held by a tenant workflow or rule. {}",
                    status_text(Some(&after)),
                    next_step(plan)
                )));
            }
            return Err(refused(format!(
                "transition readback mismatch ({}); outcome unknown; do not resend",
                notes.join("; ")
            )));
        }
        Ok((true, notes, _)) => notes,
    };
    journal.record_plan("done", &hash, ACTION, Some(&target), None);
    let mut out = json!({"status": "submitted", "ticket_id": plan.target, "status_label": plan.status,
              "plan_hash": hash, "verified": true});
    if !notes.is_empty() {
        out["verified_with_note"] = json!(notes);
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
