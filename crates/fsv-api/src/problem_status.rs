//! Reviewed problem status transitions and captured closure preconditions
//! (`problem_status.py`).

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{Map, Value, json};

use fsv_core::ApiError;
use fsv_core::error::{Error, Result};
use fsv_core::pystr::{casefold, strip};
use fsv_core::resource::{PROBLEMS, parse_id};
use fsv_forms::LookupResolver;
use fsv_py::{py_str, truthy};

use crate::formmeta::{MetaStore, existing_from_record, rules_gate, rules_unavailable_warning};
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
    /// Review warnings that are not part of the write hash. `"[]"` when none.
    pub warnings_json: String,
    /// Runtime status labels by ID (not hashed: the metadata is bound through
    /// `context_hash`). Used to check a workflow rewrite against the readback.
    pub status_labels_json: String,
    /// Closure preflight summary shown up front in the review (bound through
    /// `context_hash`, which hashes the closure inventories). `"{}"` when none.
    pub closure_check_json: String,
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
        let closure_check: Value =
            serde_json::from_str(&self.closure_check_json).unwrap_or(Value::Null);
        if closure_check.as_object().is_some_and(|m| !m.is_empty()) {
            out.insert("closure_check".into(), closure_check);
        }
        let warnings: Value =
            serde_json::from_str(&self.warnings_json).unwrap_or(Value::Array(vec![]));
        if warnings.as_array().is_some_and(|a| !a.is_empty()) {
            out.insert("warnings".into(), warnings);
        }
        out
    }
}

const SOLUTION_RULE: &str = "check_workaround_or_solution_added";
const SOLUTION_EVIDENCE: &str = "observed on one tenant: only 2 of 62 closed problems had a linked workaround / solution, so this closure rule is tenant configuration, not a platform rule";

thread_local! {
    static CLOSURE_RULES_DIR: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Tests point the proven-rule file at a temp directory.
#[cfg(test)]
pub(crate) fn set_closure_rules_dir(path: Option<PathBuf>) {
    CLOSURE_RULES_DIR.with(|slot| *slot.borrow_mut() = path);
}

fn rules_path() -> PathBuf {
    let dir = CLOSURE_RULES_DIR.with(|slot| {
        slot.borrow()
            .clone()
            .unwrap_or_else(|| fsv_core::config::cache_dir().join("forms"))
    });
    dir.join(format!(
        "{}--problem-closure-rules.json",
        fsv_core::config::cache_domain_key(None)
    ))
}

fn proven_closure_rules() -> std::collections::BTreeSet<String> {
    let Ok(text) = std::fs::read_to_string(rules_path()) else {
        return std::collections::BTreeSet::new();
    };
    let Ok(data) = serde_json::from_str::<Value>(&text) else {
        return std::collections::BTreeSet::new();
    };
    data.get("codes")
        .and_then(Value::as_array)
        .map(|codes| codes.iter().map(fsv_py::py_str).collect())
        .unwrap_or_default()
}

fn remember_closure_rules(codes: &[&str]) {
    if codes.is_empty() {
        return;
    }
    let mut known = proven_closure_rules();
    known.extend(codes.iter().map(|c| (*c).to_string()));
    let path = rules_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let list: Vec<&String> = known.iter().collect();
    let _ = std::fs::write(
        path,
        serde_json::to_string(&json!({"codes": list})).unwrap_or_default(),
    );
}

/// Port of problem_status.closure_context. Missing analysis blocks. A
/// missing linked solution blocks only after this tenant has proven that
/// rule; otherwise it is a review warning.
pub fn closure_context(c: &dyn Api, target: i64) -> Result<Map<String, Value>> {
    let rows = planning_text::inventory(c, "problems", target)?;
    let solutions = inventory(c, target, "solutions")?;
    let mut missing: Vec<String> = Vec::new();
    let mut solution_warnings: Vec<String> = Vec::new();
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
    let linked = solutions.iter().any(linked_solution);
    if !linked {
        if proven_closure_rules().contains(SOLUTION_RULE) {
            missing.push("linked workaround / solution".into());
        } else {
            solution_warnings.push(format!(
                "no linked workaround / solution: some tenants reject closure without one ({SOLUTION_RULE}); not blocked because this tenant has not proven that rule ({SOLUTION_EVIDENCE}); the server decides"
            ));
        }
    }
    if !missing.is_empty() {
        let remembered = if missing.iter().any(|m| m == "linked workaround / solution") {
            " (this tenant rejected a closure without a linked workaround / solution earlier; fsv remembers that per tenant, so the dry-run blocks instead of letting the server refuse)"
        } else {
            ""
        };
        return Err(refused(format!(
            "violated_closure_rules: missing {}{remembered}",
            missing.join(", ")
        )));
    }
    Ok(obj(
        json!({"analysis": rows, "solutions": solutions, "warnings": solution_warnings}),
    ))
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
    if before.get("deleted") == Some(&json!(true)) {
        return Err(refused(format!(
            "problem {target} is deleted (in trash); restore it before changing status"
        )));
    }
    let rec_ws = crate::formmeta::record_workspace_in(&before);
    let meta =
        crate::formmeta::load_meta_in(c, store, "problem", "edit", true, &json!(target), &rec_ws)?;
    let blockers = rules_gate(&meta.rules, &meta.spec(), &["status"]);
    if !blockers.is_empty() {
        return Err(refused(format!(
            "problem rules are unavailable and this write depends on them: {}",
            blockers.join("; ")
        )));
    }
    let mut warnings = rules_unavailable_warning(&meta.rules)
        .into_iter()
        .chain(meta.form_notes())
        .collect::<Vec<_>>();
    let raw_fields = meta.raw_fields();
    let (status_id, label) =
        task_status::resolve_of(to, &raw_fields, "problem").map_err(|e| Error::value(e.0))?;
    let spec = meta.spec();
    let mut state = meta.state("edit", resolver, Some(existing_from_record(&spec, &before)))?;
    let field = spec.field("status").map_err(other)?;
    state
        .set(&field.api_name, fsv_forms::Value::from(status_id))
        .map_err(other)?;
    let gate = state.rule_gate(&[]).map_err(other)?;
    if !gate.blockers.is_empty() || !state.evaluated.messages.is_empty() {
        let detail = gate
            .blockers
            .iter()
            .map(|b| format!("; {b}"))
            .collect::<String>();
        return Err(refused(format!(
            "problem rules need unavailable context or acknowledgement; use web form{detail}"
        )));
    }
    let errors = state.errors().map_err(other)?;
    if !state.evaluated.visible.contains(&field.api_name) || !errors.is_empty() {
        return Err(refused(format!(
            "problem form validation failed: {}",
            errors.join("; ")
        )));
    }
    let mut closure = Map::new();
    let all: Vec<&fsv_forms::Choice> = field.choices.iter().collect();
    let closing = match crate::status_semantics::closes(&all, "problem", Some(&json!(status_id))) {
        Ok(Some(closing)) => closing,
        _ => {
            return Err(refused(format!(
                "cannot tell from the runtime problem status metadata whether {} closes the problem (built-in status ids are missing); nothing was sent",
                fsv_py::py_repr_str(&label)
            )));
        }
    };
    if closing {
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
        if let Some(extra) = closure.get("warnings").and_then(Value::as_array) {
            for item in extra {
                if let Some(text) = item.as_str() {
                    warnings.push(text.to_string());
                }
            }
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
        warnings_json: serde_json::to_string(&warnings).unwrap_or_else(|_| "[]".into()),
        status_labels_json: status_labels_json(&raw_fields)?,
        closure_check_json: fsv_py::Dumps::default().dump(&if closure.is_empty() {
            json!({})
        } else {
            closure_summary(&closure)
        }),
    })
}

/// `json.dumps({str(id): label for active choices}, sort_keys=True)`.
fn status_labels_json(raw_fields: &[Value]) -> Result<String> {
    let choices =
        task_status::active_choices_of(raw_fields, "problem").map_err(|e| Error::value(e.0))?;
    let labels: Map<String, Value> = choices
        .into_iter()
        .map(|(id, label)| (id.to_string(), json!(label)))
        .collect();
    Ok(fsv_py::Dumps::default()
        .sort_keys(true)
        .dump(&Value::Object(labels)))
}

/// A live (not deleted) workaround (1) or solution (2) article row.
fn linked_solution(row: &Value) -> bool {
    !row.get("deleted").is_some_and(truthy)
        && matches!(
            row.get("article_type"),
            Some(Value::Number(n))
                if (n.is_i64() || n.is_u64()) && matches!(n.as_i64(), Some(1 | 2))
        )
}

/// Port of problem_status.closure_summary: the up-front closure preflight
/// for the review (no values, only outcomes).
fn closure_summary(closure: &Map<String, Value>) -> Value {
    let linked = closure
        .get("solutions")
        .and_then(Value::as_array)
        .map_or(0, |rows| rows.iter().filter(|r| linked_solution(r)).count());
    let solution = if linked > 0 {
        format!("linked ({linked})")
    } else if proven_closure_rules().contains(SOLUTION_RULE) {
        // Unreachable: closure_context refuses first.
        "missing: required by this tenant (remembered rejection)".into()
    } else {
        "missing: this tenant may require a linked workaround / solution for closure (not proven here; the server decides and fsv remembers a rejection for later dry-runs)".into()
    };
    json!({"analysis": "Root Cause, Impact and Symptoms are filled", "linked_solution": solution})
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
        let proven: Vec<&str> = [
            "check_analysis_completed",
            "check_workaround_or_solution_added",
        ]
        .into_iter()
        .filter(|k| desc.is_some_and(|d| d.contains_key(*k)))
        .collect();
        remember_closure_rules(&proven);
        return format!(
            "violated_closure_rules: {}",
            if details.is_empty() {
                "closure prerequisites rejected; inspect analysis and solutions".to_string()
            } else {
                details.join("; ")
            }
        );
    }
    if e.status == 405 {
        return "write rejected (HTTP 405): the problem is not editable (deleted or in trash?); no retry"
            .into();
    }
    format!("write rejected (HTTP {}); no retry", e.status)
}

/// `(verified, note)`. A status that is neither the sent value nor the
/// pre-write value was rewritten by a workflow.
fn status_readback(before: &Value, after: &Value, sent: &Value) -> (bool, Option<String>) {
    if after.is_null() || after.is_boolean() {
        return (false, None);
    }
    if submitted_value_matches(after, sent) {
        return (true, None);
    }
    if !before.is_null() && fsv_py::py_str(after) != fsv_py::py_str(before) {
        return (
            true,
            Some(format!(
                "status rewritten server-side to {} after the write (workflow)",
                fsv_py::py_str(after)
            )),
        );
    }
    (false, None)
}

/// `(created_at, content text, sub_contents text, actor name)` of the
/// activity rows with an aware `created_at`.
fn activity_rows(rows: &[Value]) -> Vec<(jiff::Timestamp, String, String, String)> {
    let html = |v: &Value| {
        let raw = match v {
            Value::String(s) => s.clone(),
            other => py_str(other),
        };
        fsv_core::output::strip_html(Some(&raw))
    };
    let mut out = Vec::new();
    for row in rows {
        let Some(row) = row.as_object() else {
            continue;
        };
        let created = row.get("created_at").map_or("None".to_string(), py_str);
        let Some((dt, Some(off))) =
            fsv_forms::pydt::datetime_fromisoformat(&created.replace('Z', "+00:00"))
        else {
            continue;
        };
        let Ok(at) = off.to_timestamp(dt) else {
            continue;
        };
        let sub = match row.get("sub_contents") {
            Some(Value::Array(subs)) => subs
                .iter()
                .filter(|s| s.is_string())
                .map(html)
                .collect::<Vec<_>>()
                .join(" , "),
            _ => String::new(),
        };
        let content = row
            .get("content")
            .filter(|v| truthy(v))
            .map_or(String::new(), html);
        let actor = row
            .get("actor")
            .and_then(Value::as_object)
            .and_then(|a| a.get("name"))
            .filter(|v| truthy(v))
            .map_or(String::new(), py_str);
        out.push((at, content, sub, actor));
    }
    out
}

/// The label after `set status as` in whitespace-normalized text.
fn set_status(text: &str) -> Option<String> {
    static SET_STATUS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"set status as\s*(.+?)\s*(?:,|$)").expect("regex")
    });
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    SET_STATUS
        .captures(&joined)
        .map(|m| m[1].trim().to_string())
}

/// A workflow triggered by the write can be stamped a moment before the
/// write's own activity row (a closure also fills planned dates in the same
/// save). Rows older than this before the write's entry are not its evidence.
const WORKFLOW_SKEW: jiff::SignedDuration = jiff::SignedDuration::from_secs(5);

/// Port of problem_status.workflow_activity: `(note, label)` when the
/// activity rows show a workflow that set the status after the write (for
/// example back to the pre-write status). With `requested`, the rows must
/// also show this write's own `set status as <requested>` entry (a closure
/// adds `, set planned end/start date as ...` to the same row), and the
/// workflow entry must not be older than it (beyond [`WORKFLOW_SKEW`]).
fn workflow_note(
    rows: &[Value],
    since: jiff::Timestamp,
    requested: Option<&str>,
) -> Option<(String, String)> {
    let rows: Vec<_> = activity_rows(rows)
        .into_iter()
        .filter(|r| r.0 >= since)
        .collect();
    let mut written = None;
    if let Some(requested) = requested {
        let want = casefold(&requested.split_whitespace().collect::<Vec<_>>().join(" "));
        written = rows
            .iter()
            .filter(|(_, text, _, _)| {
                !casefold(text).contains("workflow")
                    && casefold(&set_status(text).unwrap_or_default()) == want
            })
            .map(|r| r.0)
            .max();
        written?;
    }
    for (at, text, sub, actor) in &rows {
        let too_old = written.is_some_and(|w: jiff::Timestamp| {
            w.checked_sub(WORKFLOW_SKEW).is_ok_and(|limit| *at < limit)
        });
        if !casefold(text).contains("workflow") || too_old {
            continue;
        }
        let Some(label) = set_status(sub).or_else(|| set_status(text)) else {
            continue;
        };
        let Some(requested) = requested else {
            return Some((
                "status rewritten server-side after the write (activity shows a workflow set it)"
                    .into(),
                label,
            ));
        };
        let actor = if actor.is_empty() { "workflow" } else { actor };
        return Some((
            format!(
                "status set to {requested} by this write, then rewritten server-side to {label} by a workflow ({actor}) per the activity log"
            ),
            label,
        ));
    }
    None
}

/// [`workflow_note`] over the first problem activity page; a failed read
/// is no evidence.
fn workflow_activity_note(
    c: &dyn Api,
    target: i64,
    since: jiff::Timestamp,
    requested: Option<&str>,
) -> Option<(String, String)> {
    let rows = crate::service::get_activities(c, &PROBLEMS, target, 1).ok()?;
    workflow_note(&rows, since, requested)
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
    let since = jiff::Timestamp::now()
        .checked_sub(jiff::SignedDuration::from_mins(2))
        .unwrap_or_else(|_| jiff::Timestamp::now());
    let rebuild = || build_plan(c, store, &target, &plan.label, resolver.clone()).map(|p| p.hash());
    let verify = |_: &Value| -> Result<Map<String, Value>> {
        let row = record(c, plan.write.target)?;
        let want = &plan.write.payload()["status"];
        let before_status = serde_json::from_str::<Value>(&plan.before_json)
            .ok()
            .and_then(|v| v.get("status").cloned())
            .unwrap_or(Value::Null);
        let after = row.get("status").cloned().unwrap_or(Value::Null);
        let (mut verified, mut note) = status_readback(&before_status, &after, want);
        let mut conflict = None;
        if !verified && !after.is_null() {
            let found = workflow_activity_note(c, plan.write.target, since, Some(&plan.label));
            note = found.as_ref().map(|f| f.0.clone());
            verified = note.is_some();
            // The workflow's status must be the server's resulting status
            // (compared by runtime label when the plan carries the labels).
            let labels: Value =
                serde_json::from_str(&plan.status_labels_json).unwrap_or(Value::Null);
            let observed = labels.get(py_str(&after)).and_then(Value::as_str);
            if let (Some((_, wf_label)), Some(observed)) = (&found, observed)
                && casefold(observed) != casefold(wf_label)
            {
                verified = false;
                note = None;
                conflict = Some(format!(
                    "activity shows a workflow setting {wf_label}, but the readback status is {observed}; not verified"
                ));
            }
        }
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
        let mut out = obj(json!({
            "problem_id": plan.write.target, "verified": verified, "readback": readback,
            "unavailable": unavailable,
        }));
        if let Some(note) = note {
            out.insert("verified_with_note".into(), json!(note));
        }
        if let Some(conflict) = conflict {
            out.insert("readback_conflict".into(), json!(conflict));
        }
        Ok(out)
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
