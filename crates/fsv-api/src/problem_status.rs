//! Reviewed problem status transitions and captured closure preconditions
//! (`problem_status.py`).

use std::sync::Arc;

use serde_json::{Map, Value, json};

use fsv_core::ApiError;
use fsv_core::error::{Error, Result};
use fsv_core::pystr::{casefold, strip};
use fsv_core::resource::{PROBLEMS, parse_id};
use fsv_forms::LookupResolver;
use fsv_py::{py_str, truthy};

use crate::formmeta::{MetaStore, RULES_UNAVAILABLE_KEY, existing_from_record, load_meta};
use crate::planning_text::{self, PROBLEM_SECTIONS};
use crate::problem_workflows::{inventory, record};
use crate::readback::submitted_value_matches;
use crate::reviewed_write::{WritePlan, execute_rejecting, refused};
use crate::service::{Api, Journal, obj, snapshot_hash};
use crate::task_status;
use crate::watchers::positive_id;

pub const ACTION: &str = "problem_status";

const SIDE_EFFECTS: &str = "Server may auto-fill empty planned_start_date and planned_end_date on status transitions; readback reports these values.";

/// Port of problem_status.StatusPlan: the write plus the runtime status
/// label and the prior record. The hash is the write's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusPlan {
    pub write: WritePlan,
    pub label: String,
    pub before_json: String,
}

impl StatusPlan {
    pub fn hash(&self) -> String {
        self.write.hash()
    }

    /// Port of problem_status.StatusPlan.review.
    pub fn review(&self) -> Map<String, Value> {
        let mut out = self.write.review();
        out.insert("to".into(), json!(self.label));
        out.insert(
            "before".into(),
            serde_json::from_str(&self.before_json).unwrap_or(Value::Null),
        );
        out.insert("side_effects".into(), json!(SIDE_EFFECTS));
        out
    }
}

/// Port of problem_status.closure_context: every native analysis section
/// has visible text and a nondeleted workaround or permanent article is linked.
pub fn closure_context(c: &dyn Api, target: i64) -> Result<Map<String, Value>> {
    let rows = planning_text::inventory(c, "problems", target)?;
    let solutions = inventory(c, target, "solutions")?;
    let mut missing: Vec<String> = Vec::new();
    for (_, key, title) in PROBLEM_SECTIONS {
        let matches: Vec<&Value> = rows
            .iter()
            .filter(|r| r.get("name") == Some(&json!(key)))
            .collect();
        let content = match matches[..] {
            [one] => planning_text::section_content(Some(one)),
            _ => None,
        };
        if content.is_none_or(|t| strip(&fsv_core::output::strip_html(Some(&t))).is_empty()) {
            missing.push(title.to_string());
        }
    }
    let linked = solutions.iter().any(|row| {
        !row.get("deleted").is_some_and(truthy)
            && matches!(
                row.get("article_type"),
                Some(Value::Number(n))
                    if (n.is_i64() || n.is_u64()) && matches!(n.as_i64(), Some(1 | 2))
            )
    });
    if !linked {
        missing.push("linked workaround / solution".into());
    }
    if !missing.is_empty() {
        return Err(refused(format!(
            "violated_closure_rules: missing {}",
            missing.join(", ")
        )));
    }
    Ok(obj(json!({"analysis": rows, "solutions": solutions})))
}

/// Python `before.get(name, before.get("custom_fields", {}).get(name))`.
fn closure_value<'a>(before: &'a Map<String, Value>, name: Option<&str>) -> Option<&'a Value> {
    let name = name?;
    before
        .get(name)
        .or_else(|| before.get("custom_fields")?.get(name))
}

/// Port of problem_status.build_plan: one status PUT, checked against
/// fresh form rules and, for Closed, the captured closure preconditions.
pub fn build_plan(
    c: &dyn Api,
    store: &dyn MetaStore,
    target: &str,
    to: &str,
    resolver: Option<Arc<dyn LookupResolver>>,
) -> Result<StatusPlan> {
    let other = |e: fsv_forms::FormError| Error::other(e.to_string());
    let target = positive_id(&json!(parse_id(target, &PROBLEMS)?))?;
    let before = record(c, target)?;
    if !before.contains_key("status") {
        return Err(refused("current problem status unavailable"));
    }
    let meta = load_meta(c, store, "problem", "edit", true, &json!(target))?;
    if meta.rules.get(RULES_UNAVAILABLE_KEY).is_some_and(truthy) {
        return Err(refused("problem rules unavailable"));
    }
    let raw_fields = meta.raw_fields();
    let (status_id, label) =
        task_status::resolve(to, &raw_fields).map_err(|e| Error::value(e.0))?;
    let spec = meta.spec();
    let mut state = meta.state("edit", resolver, Some(existing_from_record(&spec, &before)))?;
    let field = spec.field("status").map_err(other)?;
    state
        .set(&field.api_name, fsv_forms::Value::from(status_id))
        .map_err(other)?;
    if !state.rules.unavailable_dependencies("edit").is_empty()
        || !state.evaluated.messages.is_empty()
        || state
            .rules
            .warnings()
            .iter()
            .any(|w| w.contains("unknown field") || w.contains("none was given"))
    {
        return Err(refused(
            "problem rules need unavailable context or acknowledgement; use web form",
        ));
    }
    let errors = state.errors().map_err(other)?;
    if !state.evaluated.visible.contains(&field.api_name) || !errors.is_empty() {
        return Err(refused(format!(
            "problem form validation failed: {}",
            errors.join("; ")
        )));
    }
    let mut closure = Map::new();
    if casefold(&label) == "closed" {
        closure = closure_context(c, target)?;
        let mut missing = Vec::new();
        for raw in &raw_fields {
            if raw.get("required_for_closure") != Some(&json!(true)) {
                continue;
            }
            let name = raw.get("name");
            let value = closure_value(&before, name.and_then(Value::as_str));
            let empty = match value {
                None | Some(Value::Null) => true,
                Some(Value::String(s)) => s.is_empty(),
                Some(Value::Array(a)) => a.is_empty(),
                Some(_) => false,
            };
            if empty {
                let shown = raw
                    .get("label")
                    .filter(|v| truthy(v))
                    .or(name)
                    .map_or_else(|| "None".to_string(), py_str);
                missing.push(shown);
            }
        }
        if !missing.is_empty() {
            return Err(refused(format!(
                "violated_closure_rules: missing required closure fields: {}",
                missing.join(", ")
            )));
        }
    }
    let context = json!({"before": before, "fields": meta.form_fields, "rules": meta.rules,
                         "closure": closure});
    Ok(StatusPlan {
        write: WritePlan {
            action: ACTION.into(),
            target,
            tenant: c.domain(),
            method: "PUT".into(),
            path: format!("problems/{target}"),
            payload_json: fsv_py::Dumps::default().dump(&json!({"status": status_id})),
            context_hash: snapshot_hash(&context)?,
        },
        label,
        before_json: fsv_py::Dumps::default()
            .sort_keys(true)
            .dump(&Value::Object(before)),
    })
}

/// Port of problem_status.rejection: a server closure-rule refusal names
/// the missing prerequisites.
pub fn rejection(e: &ApiError) -> String {
    let errors = e.body.get("errors").and_then(Value::as_array);
    for error in errors.into_iter().flatten() {
        if error.get("code") != Some(&json!("violated_closure_rules")) {
            continue;
        }
        let desc = error.get("description").and_then(Value::as_object);
        let details: Vec<&str> = [
            (
                "check_analysis_completed",
                "Root Cause, Impact and Symptoms must be filled",
            ),
            (
                "check_workaround_or_solution_added",
                "a workaround / solution must be linked",
            ),
        ]
        .iter()
        .filter(|(k, _)| desc.is_some_and(|d| d.contains_key(*k)))
        .map(|(_, text)| *text)
        .collect();
        return format!(
            "violated_closure_rules: {}",
            if details.is_empty() {
                "closure prerequisites rejected; inspect analysis and solutions".to_string()
            } else {
                details.join("; ")
            }
        );
    }
    format!("write rejected (HTTP {}); no retry", e.status)
}

/// Port of problem_status.send: one PUT, then an independent record read.
pub fn send(
    plan: &StatusPlan,
    c: &dyn Api,
    store: &dyn MetaStore,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
    resolver: Option<Arc<dyn LookupResolver>>,
) -> Result<Value> {
    let target = plan.write.target.to_string();
    let rebuild = || build_plan(c, store, &target, &plan.label, resolver.clone()).map(|p| p.hash());
    let verify = |_: &Value| -> Result<Map<String, Value>> {
        let row = record(c, plan.write.target)?;
        let want = &plan.write.payload()["status"];
        let verified = row
            .get("status")
            .is_some_and(|s| submitted_value_matches(s, want));
        let mut readback = Map::new();
        for k in ["status", "planned_start_date", "planned_end_date"] {
            if let Some(v) = row.get(k) {
                readback.insert(k.into(), v.clone());
            }
        }
        let unavailable: Vec<&str> = ["planned_start_date", "planned_end_date"]
            .into_iter()
            .filter(|k| !row.contains_key(*k))
            .collect();
        Ok(obj(json!({
            "problem_id": plan.write.target, "verified": verified, "readback": readback,
            "unavailable": unavailable,
        })))
    };
    execute_rejecting(
        &plan.write,
        &plan.hash(),
        c,
        journal,
        reviewed_hash,
        &rebuild,
        &verify,
        Some(&rejection),
    )
}

#[cfg(test)]
mod tests;
