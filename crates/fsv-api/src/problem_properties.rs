//! Reviewed problem planned dates and effort (`problem_properties.py`).
//!
//! `PUT problems/{id}` with only the dirty planning keys. Allowed when the
//! problem form has the field and its rules leave it visible and unlocked.

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::resource::{PROBLEMS, parse_id};
use fsv_forms::pydt::datetime_fromisoformat;
use fsv_py::Dumps;

use crate::formmeta::{
    MetaStore, existing_from_record, load_meta, rules_gate, rules_unavailable_warning,
};
use crate::problem_workflows::record;
use crate::readback::same_instant;
use crate::reviewed_write::{WritePlan, execute_as, refused};
use crate::service::{Api, Journal, obj, snapshot_hash};
use crate::tasks::{date, effort, same_effort};
use crate::watchers::positive_id;

/// Port of problem_properties.PlanningPlan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningPlan {
    pub write: WritePlan,
    pub before_json: String,
    pub warnings_json: String,
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
            json!("Tenant workflows may react to planning changes."),
        );
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
    let meta = load_meta(c, store, "problem", "edit", true, &json!(target))?;
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
            action: "problem_planning".into(),
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
    })
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
