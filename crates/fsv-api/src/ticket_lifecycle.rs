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
const UNKNOWN: &str =
    "transition outcome unknown or partial; inspect ticket and journal; do not resend";

fn dumps(v: &Value) -> String {
    fsv_py::Dumps::default().sort_keys(true).dump(v)
}

/// `context(client, cid, status, answers, resolution=...)`: the transition
/// payload, the record+metadata fingerprint JSON and the runtime status label.
pub fn context(
    c: &dyn Api,
    store: &dyn MetaStore,
    cid: i64,
    status: &str,
    answers: &Map<String, Value>,
    resolution: Option<&str>,
    resolver: Option<Arc<dyn LookupResolver>>,
) -> Result<(Map<String, Value>, String, String)> {
    let other = |e: fsv_forms::FormError| Error::other(e.to_string());
    let row = record(c, cid)?;
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
    if !state.evaluated.visible.contains(&field.api_name)
        || !state.rules.unavailable_dependencies("edit").is_empty()
        || state
            .rules
            .warnings()
            .iter()
            .any(|w| w.contains("unknown field") || w.contains("none was given"))
    {
        return Err(refused(
            "ticket rules need unavailable fields or context; no write attempted",
        ));
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
    let errors = state.errors().map_err(other)?;
    if !errors.is_empty() {
        return Err(refused(format!(
            "ticket form validation failed: {}",
            errors.join("; ")
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
    let fingerprint = dumps(&json!({
        "record": snapshot_hash(&Value::Object(row))?,
        "metadata": meta.fingerprint().hash,
    }));
    Ok((payload, fingerprint, choice.label.clone()))
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
        obj(
            json!({"action": ACTION, "status_label": self.status, "steps": steps,
                   "plan_hash": self.hash(), "warning": WARNING}),
        )
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
    let (payload, fingerprint, label) =
        context(c, store, cid, status, answers, html.as_deref(), resolver)?;
    Ok(LifecyclePlan {
        target: cid,
        tenant,
        status: label,
        answers_json: dumps(&Value::Object(answers.clone())),
        payload_json: dumps(&Value::Object(payload)),
        context_hash: fingerprint,
        note_html: html,
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
    let (payload, fingerprint, _) = context(
        c,
        store,
        plan.target,
        &plan.status,
        &answers,
        plan.note_html.as_deref(),
        resolver.clone(),
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
    let attempt = || -> Result<bool> {
        if let Some(html) = &plan.note_html {
            let baseline = record(c, plan.target)?;
            if Some(json!(snapshot_hash(&Value::Object(baseline.clone()))?)) != plan.record_hash() {
                return Err(refused("ticket changed before note save"));
            }
            put(&json!({"resolution_notes": html,
                        "br_validation_excludes": payload.get("br_validation_excludes")}))?;
            let after = record(c, plan.target)?;
            if !note_saved(&after, html) {
                return Err(refused(
                    "resolution save was not verified; transition not attempted",
                ));
            }
            // A saved note may update derived fields, but must not silently
            // change routing, status, or existing answers.
            for key in ["status", "group_id", "responder_id", "custom_fields"] {
                let unchanged = after
                    .get(key)
                    .is_some_and(|v| py_equal(v, baseline.get(key).unwrap_or(&Value::Null)));
                if !unchanged {
                    return Err(refused(
                        "ticket changed during note save; transition not attempted",
                    ));
                }
            }
            let (fresh, fresh_context, _) = context(
                c,
                store,
                plan.target,
                &plan.status,
                &answers,
                None,
                resolver.clone(),
            )?;
            let fresh_meta = serde_json::from_str::<Value>(&fresh_context)
                .ok()
                .and_then(|v| v.get("metadata").cloned());
            if !py_equal(&Value::Object(fresh), &Value::Object(payload.clone()))
                || fresh_meta != plan.metadata_hash()
            {
                return Err(refused("transition requirements changed after note save"));
            }
        }
        put(&Value::Object(payload.clone()))?;
        let after = record(c, plan.target)?;
        Ok(matches(&after, &payload)
            && plan
                .note_html
                .as_deref()
                .is_none_or(|html| note_saved(&after, html)))
    };
    match attempt() {
        Err(_) => return Err(refused(UNKNOWN)),
        Ok(false) => {
            return Err(refused(
                "transition readback mismatch; outcome unknown; do not resend",
            ));
        }
        Ok(true) => {}
    }
    journal.record_plan("done", &hash, ACTION, Some(&target), None);
    Ok(
        json!({"status": "submitted", "ticket_id": plan.target, "status_label": plan.status,
              "plan_hash": hash, "verified": true}),
    )
}

#[cfg(test)]
mod tests;
