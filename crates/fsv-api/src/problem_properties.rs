//! Reviewed problem updates (`problem_properties.py`): planned dates and
//! effort (`problem_planning`) and the other form fields (`problem_update`:
//! priority, subject, description, group, agent, category and custom fields).
//!
//! `PUT problems/{id}` with only the dirty keys. Allowed when the problem
//! form has the field and its rules leave it visible and unlocked.

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::resource::{PROBLEMS, parse_id};
use fsv_forms::pydt::datetime_fromisoformat;
use fsv_py::Dumps;

use crate::formmeta::{MetaStore, existing_from_record, rules_gate, rules_unavailable_warning};
use crate::problem_workflows::record;
use crate::readback::{custom_fields_match, same_instant, same_text, submitted_value_matches};
use crate::reviewed_write::{WritePlan, execute_as, execute_guarded, refused};
use crate::service::{Api, Journal, obj, snapshot_hash};
use crate::tasks::{date, effort, same_effort};
use crate::watchers::positive_id;

pub const ACTION: &str = "problem_planning";
pub const UPDATE_ACTION: &str = "problem_update";
const KEYS: [&str; 3] = ["planned_start_date", "planned_end_date", "planned_effort"];
/// Rich text compared semantically on readback only (the payload is verbatim).
const TEXT_KEYS: [&str; 2] = ["description", "description_html"];

/// Port of problem_properties.PlanningPlan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningPlan {
    pub write: WritePlan,
    pub before_json: String,
    pub warnings_json: String,
    /// Record-state token (`journal.repeat_token`) bound in `context_hash`;
    /// empty for plans without one.
    pub state_json: String,
    /// The context hash without the token (journals written before it was
    /// bound); empty when no token is bound.
    pub prior_context: String,
}

/// `stateful_context`: `(context hash with the token, token JSON, context
/// hash without it)`.
pub(crate) fn stateful_context(
    context: &Value,
    token: &Map<String, Value>,
) -> Result<(String, String, String)> {
    let bound = snapshot_hash(&json!({"context": context, "state": token}))?;
    let state = Dumps::default()
        .sort_keys(true)
        .dump(&Value::Object(token.clone()));
    Ok((bound, state, snapshot_hash(context)?))
}

/// `prior_guards`: the hash of `plan` without its record-state token, so an
/// unresolved send journaled before the token was bound still refuses.
pub(crate) fn prior_guards(plan: &PlanningPlan) -> Vec<String> {
    prior_guard_hashes(&plan.write, &plan.prior_context)
}

/// [`prior_guards`] for any plan carrying a `prior_context`.
pub(crate) fn prior_guard_hashes(write: &WritePlan, prior_context: &str) -> Vec<String> {
    if prior_context.is_empty() || prior_context == write.context_hash {
        return Vec::new();
    }
    let mut older = write.clone();
    older.context_hash = prior_context.to_string();
    vec![older.hash()]
}

impl PlanningPlan {
    pub fn hash(&self) -> String {
        self.write.hash()
    }

    pub fn review(&self) -> Map<String, Value> {
        let mut out = self.write.review();
        out.insert(
            "before".into(),
            serde_json::from_str(&self.before_json).unwrap_or(Value::Null),
        );
        out.insert("after".into(), self.write.payload());
        out.insert(
            "side_effects".into(),
            json!(if self.write.action == ACTION {
                "Tenant workflows may react to planning changes."
            } else {
                "Tenant workflows and notifications may react to these changes."
            }),
        );
        if !self.state_json.is_empty() {
            out.insert(
                "record_state".into(),
                serde_json::from_str(&self.state_json).unwrap_or(Value::Null),
            );
        }
        let warnings: Value = serde_json::from_str(&self.warnings_json).unwrap_or(json!([]));
        if warnings.as_array().is_some_and(|a| !a.is_empty()) {
            out.insert("warnings".into(), warnings);
        }
        out
    }
}

fn precedes(end: &str, start: &str) -> bool {
    let stamp = |text: &str| {
        let (dt, off) = datetime_fromisoformat(&text.replace('Z', "+00:00"))?;
        off?.to_timestamp(dt).ok()
    };
    match (stamp(end), stamp(start)) {
        (Some(end), Some(start)) => end < start,
        _ => false,
    }
}

fn one(key: &str, raw: Option<&str>, kind: &str) -> Result<Option<String>> {
    let Some(raw) = raw else { return Ok(None) };
    let parsed = if kind == "date" {
        date(Some(raw))?
    } else {
        effort(Some(raw))?
    };
    Ok(Some(parsed.ok_or_else(|| {
        Error::value(format!("{key} cannot be empty"))
    })?))
}

/// Port of problem_properties.build_plan.
pub fn build_plan(
    c: &dyn Api,
    store: &dyn MetaStore,
    target: &str,
    planned_start: Option<&str>,
    planned_end: Option<&str>,
    planned_effort: Option<&str>,
) -> Result<PlanningPlan> {
    let target = positive_id(&json!(parse_id(target, &PROBLEMS)?))?;
    let mut payload = Map::new();
    for (key, raw, kind) in [
        ("planned_start_date", planned_start, "date"),
        ("planned_end_date", planned_end, "date"),
        ("planned_effort", planned_effort, "effort"),
    ] {
        if let Some(value) = one(key, raw, kind)? {
            payload.insert(key.into(), json!(value));
        }
    }
    if payload.is_empty() {
        return Err(Error::value(
            "specify --planned-start, --planned-end or --planned-effort",
        ));
    }
    let before_row = record(c, target)?;
    if before_row.get("deleted") == Some(&json!(true)) {
        return Err(refused(format!(
            "problem {target} is deleted (in trash); restore it before updating"
        )));
    }
    let start = payload
        .get("planned_start_date")
        .or_else(|| before_row.get("planned_start_date"));
    let end = payload
        .get("planned_end_date")
        .or_else(|| before_row.get("planned_end_date"));
    if let (Some(Value::String(start)), Some(Value::String(end))) = (start, end)
        && precedes(end, start)
    {
        return Err(Error::value("planned end must not precede planned start"));
    }
    let rec_ws = crate::formmeta::record_workspace_in(&before_row);
    let meta =
        crate::formmeta::load_meta_in(c, store, "problem", "edit", true, &json!(target), &rec_ws)?;
    let spec = meta.spec();
    for key in payload.keys() {
        if spec.find(key).is_none() {
            return Err(refused(format!(
                "problem form metadata has no {key} field; nothing sent"
            )));
        }
    }
    let touched: Vec<&str> = payload.keys().map(String::as_str).collect();
    let blockers = rules_gate(&meta.rules, &spec, &touched);
    if !blockers.is_empty() {
        return Err(refused(format!(
            "problem rules are unavailable and this write depends on them: {}",
            blockers.join("; ")
        )));
    }
    let warnings = rules_unavailable_warning(&meta.rules)
        .into_iter()
        .chain(meta.form_notes())
        .collect::<Vec<_>>();
    let mut state = meta.state("edit", None, Some(existing_from_record(&spec, &before_row)))?;
    for (key, value) in &payload {
        let field = spec.find(key).expect("checked");
        let text = value.as_str().unwrap_or("");
        state
            .set(&field.api_name, fsv_forms::Value::from(text))
            .map_err(|e| Error::other(e.to_string()))?;
        if !state.evaluated.visible.contains(&field.api_name) {
            return Err(refused(format!(
                "{}: hidden on the problem form",
                field.label
            )));
        }
        if state.evaluated.disabled.contains(&field.api_name) {
            return Err(refused(format!(
                "{}: locked by a problem form rule",
                field.label
            )));
        }
    }
    let gate = state
        .rule_gate(&[])
        .map_err(|e| Error::other(e.to_string()))?;
    if !gate.blockers.is_empty() || !state.evaluated.messages.is_empty() {
        let detail = gate
            .blockers
            .iter()
            .map(|b| format!("; {b}"))
            .collect::<String>();
        return Err(refused(format!(
            "problem rules need unavailable context or acknowledgement; use the web form{detail}"
        )));
    }
    let before: Map<String, Value> = payload
        .keys()
        .map(|k| (k.clone(), before_row.get(k).cloned().unwrap_or(Value::Null)))
        .collect();
    let context = json!({"before": before, "fields": meta.form_fields, "rules": meta.rules});
    Ok(PlanningPlan {
        write: WritePlan {
            action: ACTION.into(),
            target,
            tenant: c.domain(),
            method: "PUT".into(),
            path: format!("problems/{target}"),
            payload_json: Dumps::default()
                .sort_keys(true)
                .dump(&Value::Object(payload)),
            context_hash: snapshot_hash(&context)?,
        },
        before_json: Dumps::default()
            .sort_keys(true)
            .dump(&Value::Object(before)),
        warnings_json: serde_json::to_string(&warnings).unwrap_or_else(|_| "[]".into()),
        state_json: String::new(),
        prior_context: String::new(),
    })
}

/// `NothingToChange`: every requested value already matches the record, so
/// no plan exists.
#[derive(Debug)]
pub enum UpdateError {
    NothingToChange(String),
    Failed(Error),
}

impl From<Error> for UpdateError {
    fn from(e: Error) -> Self {
        UpdateError::Failed(e)
    }
}

fn same_value(key: &str, actual: &Value, expected: &Value) -> bool {
    if TEXT_KEYS.contains(&key) {
        same_text(actual, expected)
    } else {
        submitted_value_matches(actual, expected)
    }
}

/// `_changed_keys`: `(changed-key body, the record's current values of those
/// keys)`. A key the record does not expose counts as changed (it cannot be
/// proven equal).
fn changed_keys(
    row: &Map<String, Value>,
    body: &Map<String, Value>,
) -> (Map<String, Value>, Map<String, Value>) {
    let mut changed = Map::new();
    let mut before = Map::new();
    for (key, value) in body {
        if key == "custom_fields" {
            let current = match row.get("custom_fields") {
                Some(c @ Value::Object(_)) => c.clone(),
                _ => json!({}),
            };
            for (name, wanted) in value.as_object().into_iter().flatten() {
                let one = json!({ name: wanted });
                if current.get(name).is_some() && custom_fields_match(&current, &one) {
                    continue;
                }
                for (target, v) in [
                    (&mut changed, wanted.clone()),
                    (
                        &mut before,
                        current.get(name).cloned().unwrap_or(Value::Null),
                    ),
                ] {
                    let slot = target.entry("custom_fields").or_insert_with(|| json!({}));
                    if let Value::Object(m) = slot {
                        m.insert(name.clone(), v);
                    }
                }
            }
        } else if row
            .get(key)
            .is_some_and(|actual| same_value(key, actual, value))
        {
            continue;
        } else {
            changed.insert(key.clone(), value.clone());
            before.insert(key.clone(), row.get(key).cloned().unwrap_or(Value::Null));
        }
    }
    (changed, before)
}

/// Port of problem_properties.build_update_plan: a reviewed problem field
/// update from an already-resolved wire body (top-level keys plus nested
/// `custom_fields`). Unchanged keys are dropped; [`UpdateError::NothingToChange`]
/// when none remain.
pub fn build_update_plan(
    c: &dyn Api,
    store: &dyn MetaStore,
    journal: &dyn Journal,
    target: &str,
    body: &Map<String, Value>,
) -> std::result::Result<PlanningPlan, UpdateError> {
    let target = positive_id(&json!(parse_id(target, &PROBLEMS)?))?;
    let only_empty_custom = body.len() == 1
        && body
            .get("custom_fields")
            .is_some_and(|v| !fsv_py::truthy(v));
    if body.is_empty() || only_empty_custom {
        return Err(Error::value("nothing to update; pass at least one field").into());
    }
    if body.contains_key("status") {
        return Err(refused(
            "problem status changes go through `fsv problems status ID --to STATUS` (a reviewed transition with closure checks); nothing sent",
        )
        .into());
    }
    if body.keys().any(|k| KEYS.contains(&k.as_str())) {
        return Err(refused(
            "problem planned dates/effort are reviewed on their own; send other fields separately",
        )
        .into());
    }
    let before_row = record(c, target)?;
    if before_row.get("deleted") == Some(&json!(true)) {
        return Err(refused(format!(
            "problem {target} is deleted (in trash); restore it before updating"
        ))
        .into());
    }
    let (payload, before) = changed_keys(&before_row, body);
    if payload.is_empty() {
        return Err(UpdateError::NothingToChange(format!(
            "nothing to change: problem {target} already has these values; nothing sent"
        )));
    }
    let rec_ws = crate::formmeta::record_workspace_in(&before_row);
    let meta =
        crate::formmeta::load_meta_in(c, store, "problem", "edit", true, &json!(target), &rec_ws)?;
    let spec = meta.spec();
    let mut wanted: Vec<(String, Value)> = payload
        .iter()
        .filter(|(k, _)| *k != "custom_fields")
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    for (k, v) in payload
        .get("custom_fields")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        wanted.push((k.clone(), v.clone()));
    }
    let mut fields = Vec::new();
    for (key, value) in &wanted {
        let Some(field) = spec.find(key) else {
            return Err(refused(format!(
                "problem form metadata has no {key} field; nothing sent"
            ))
            .into());
        };
        fields.push((field, value));
    }
    let touched: Vec<&str> = fields.iter().map(|(f, _)| f.api_name.as_str()).collect();
    let blockers = rules_gate(&meta.rules, &spec, &touched);
    if !blockers.is_empty() {
        return Err(refused(format!(
            "problem rules are unavailable and this write depends on them: {}",
            blockers.join("; ")
        ))
        .into());
    }
    let warnings = rules_unavailable_warning(&meta.rules)
        .into_iter()
        .chain(meta.form_notes())
        .collect::<Vec<_>>();
    let mut state = meta.state("edit", None, Some(existing_from_record(&spec, &before_row)))?;
    for (field, value) in &fields {
        state
            .set(&field.api_name, fsv_forms::Value::from_json(value))
            .map_err(|e| Error::other(e.to_string()))?;
    }
    for (field, _) in &fields {
        if !state.evaluated.visible.contains(&field.api_name) {
            return Err(refused(format!("{}: hidden on the problem form", field.label)).into());
        }
        if state.evaluated.disabled.contains(&field.api_name) {
            return Err(refused(format!("{}: locked by a problem form rule", field.label)).into());
        }
    }
    let gate = state
        .rule_gate(&[])
        .map_err(|e| Error::other(e.to_string()))?;
    if !gate.blockers.is_empty() || !state.evaluated.messages.is_empty() {
        let detail = gate
            .blockers
            .iter()
            .map(|b| format!("; {b}"))
            .collect::<String>();
        return Err(refused(format!(
            "problem rules need unavailable context or acknowledgement; use the web form{detail}"
        ))
        .into());
    }
    let context = json!({"before": before, "fields": meta.form_fields, "rules": meta.rules});
    let token = journal.repeat_token(
        &[UPDATE_ACTION],
        &json!(target),
        Some(&Value::Object(before_row.clone())),
    );
    let (bound, state_json, prior) = stateful_context(&context, &token)?;
    Ok(PlanningPlan {
        write: WritePlan {
            action: UPDATE_ACTION.into(),
            target,
            tenant: c.domain(),
            method: "PUT".into(),
            path: format!("problems/{target}"),
            payload_json: Dumps::default()
                .sort_keys(true)
                .dump(&Value::Object(payload)),
            context_hash: bound,
        },
        before_json: Dumps::default()
            .sort_keys(true)
            .dump(&Value::Object(before)),
        warnings_json: serde_json::to_string(&warnings).unwrap_or_else(|_| "[]".into()),
        state_json,
        prior_context: prior,
    })
}

/// `update_fields`: per changed key matched, mismatched or unavailable
/// (custom fields as `custom_fields.<name>`).
pub fn update_fields(row: &Map<String, Value>, payload: &Map<String, Value>) -> Map<String, Value> {
    let mut fields = Map::new();
    for (key, expected) in payload {
        if key == "custom_fields" {
            let current = row.get("custom_fields");
            for (name, value) in expected.as_object().into_iter().flatten() {
                let label = format!("custom_fields.{name}");
                let status = match current {
                    Some(c @ Value::Object(m)) if m.contains_key(name) => {
                        if custom_fields_match(c, &json!({ name: value })) {
                            "matched"
                        } else {
                            "mismatched"
                        }
                    }
                    _ => "unavailable",
                };
                fields.insert(label, json!(status));
            }
        } else {
            let status = match row.get(key) {
                None => "unavailable",
                Some(actual) if same_value(key, actual, expected) => "matched",
                Some(_) => "mismatched",
            };
            fields.insert(key.clone(), json!(status));
        }
    }
    fields
}

/// Port of problem_properties.send_update.
pub fn send_update(
    plan: &PlanningPlan,
    c: &dyn Api,
    store: &dyn MetaStore,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let payload = plan.write.payload();
    let payload = payload.as_object().cloned().unwrap_or_default();
    let target = plan.write.target;
    let verify = |_: &Value| -> Result<Map<String, Value>> {
        let fields = update_fields(&record(c, target)?, &payload);
        let verified = !fields.is_empty() && fields.values().all(|v| v == "matched");
        Ok(obj(
            json!({"problem_id": target, "fields": fields, "verified": verified}),
        ))
    };
    let rebuild = || {
        build_update_plan(c, store, journal, &target.to_string(), &payload)
            .map(|p| p.hash())
            .map_err(|e| match e {
                UpdateError::NothingToChange(m) => refused(m),
                UpdateError::Failed(e) => e,
            })
    };
    execute_guarded(
        &plan.write,
        &plan.hash(),
        c,
        journal,
        reviewed_hash,
        &rebuild,
        &verify,
        None,
        &prior_guards(plan),
    )
}

/// Port of problem_properties.send.
pub fn send(
    plan: &PlanningPlan,
    c: &dyn Api,
    store: &dyn MetaStore,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let inputs = plan.write.payload();
    let target = plan.write.target;
    let verify = move |_: &Value| -> Result<Map<String, Value>> {
        let row = record(c, target)?;
        let mut fields = Map::new();
        let Some(inputs) = inputs.as_object() else {
            return Ok(obj(
                json!({"problem_id": target, "fields": {}, "verified": true}),
            ));
        };
        for (key, expected) in inputs {
            let status = if !row.contains_key(key) {
                "unavailable"
            } else if key == "planned_effort" {
                if same_effort(row.get(key).unwrap_or(&Value::Null), expected) {
                    "matched"
                } else {
                    "mismatched"
                }
            } else if same_instant(row.get(key).unwrap_or(&Value::Null), expected) {
                "matched"
            } else {
                "mismatched"
            };
            fields.insert(key.clone(), json!(status));
        }
        let verified = fields.values().all(|v| v == "matched");
        Ok(obj(
            json!({"problem_id": target, "fields": fields, "verified": verified}),
        ))
    };
    let rebuild = || {
        let inputs = plan.write.payload();
        let get = |k: &str| inputs.get(k).and_then(Value::as_str).map(str::to_string);
        let start = get("planned_start_date");
        let end = get("planned_end_date");
        let effort = get("planned_effort");
        build_plan(
            c,
            store,
            &target.to_string(),
            start.as_deref(),
            end.as_deref(),
            effort.as_deref(),
        )
        .map(|p| p.hash())
    };
    execute_as(
        &plan.write,
        &plan.hash(),
        c,
        journal,
        reviewed_hash,
        &rebuild,
        &verify,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formmeta::MemStore;
    use crate::service::testing::*;
    use fsv_core::ApiError;

    #[test]
    fn empty_change_and_inverted_dates() {
        let api = FnApi::new(|_: &Call| Err(ApiError::new(404, json!({})).into()));
        let err = build_plan(&api, &MemStore::default(), "PRB-123", None, None, None).unwrap_err();
        assert_eq!(
            err.to_string(),
            "specify --planned-start, --planned-end or --planned-effort"
        );
        let api = FnApi::new(|call: &Call| {
            assert_eq!(call.path, "problems/123");
            Ok(
                json!({"problem": {"id": 123, "planned_start_date": "2030-01-02T11:00:00Z", "planned_end_date": null}}),
            )
        });
        let err = build_plan(
            &api,
            &MemStore::default(),
            "123",
            None,
            Some("2030-01-02T10:00:00Z"),
            None,
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "planned end must not precede planned start"
        );
    }
}
