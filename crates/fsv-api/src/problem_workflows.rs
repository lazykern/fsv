//! Captured problem creation, body-only notes and self-owned basic tasks
//! (`problem_workflows.py`).

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::resource::{PROBLEMS, parse_id};
use fsv_forms::LookupResolver;
use fsv_forms::payload::{check, module_body};
use fsv_py::{Dumps, py_str, truthy};

use crate::formmeta::{
    MetaStore, apply_answers, find_field, form_err, is_requester, load_meta, resolve_requester,
    rules_gate, rules_unavailable_warning,
};
use crate::properties::dumps_sorted;
use crate::readback::{same_text, strict_equal, submitted_value_matches};
use crate::reviewed_write::{WritePlan, execute, refused};
use crate::service::{
    Api, Journal, MAX_DETAIL_PAGES, collect_pages, obj, plan_hash, snapshot_hash,
};
use crate::watchers::{positive_id, self_id};

/// Port of fsv.problem_workflows.ProblemPlan: a [`WritePlan`] plus the
/// inputs it is rebuilt from (not part of the hash).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProblemPlan {
    pub plan: WritePlan,
    pub inputs_json: String,
    /// Review warnings that are not part of the write hash. `"[]"` when none.
    pub warnings_json: String,
}

impl ProblemPlan {
    pub fn hash(&self) -> String {
        self.plan.hash()
    }

    pub fn review(&self) -> Map<String, Value> {
        let mut out = self.plan.review();
        let warnings: Value =
            serde_json::from_str(&self.warnings_json).unwrap_or(Value::Array(vec![]));
        if warnings.as_array().is_some_and(|a| !a.is_empty()) {
            out.insert("warnings".into(), warnings);
        }
        out
    }

    pub fn payload(&self) -> Value {
        self.plan.payload()
    }
}

/// The create form's top-level keys, in payload order.
const TOP: [&str; 5] = ["subject", "description", "status", "priority", "impact"];

/// Port of fsv.problem_workflows.record: the exact problem.
pub fn record(c: &dyn Api, target: i64) -> Result<Map<String, Value>> {
    let raw = c.int_get(&format!("problems/{target}"), None)?;
    let row = match &raw {
        Value::Object(m) => m.get("problem").unwrap_or(&raw),
        _ => &Value::Null,
    };
    match row {
        Value::Object(r) if py_str(r.get("id").unwrap_or(&Value::Null)) == target.to_string() => {
            Ok(r.clone())
        }
        _ => Err(refused("exact problem identity is unavailable")),
    }
}

/// Port of fsv.problem_workflows.inventory: every note, task, or linked
/// solution of the problem, sorted by id. The native linked-solutions route
/// returns KB articles, not a `solutions` key (`d1c158b`).
pub fn inventory(c: &dyn Api, target: i64, kind: &str) -> Result<Vec<Value>> {
    let key = if kind == "solutions" {
        "articles"
    } else {
        kind
    };
    let params = obj(json!({"page": 1, "per_page": 30}));
    let pages = collect_pages(
        |path, params| c.int_get(path, params),
        &format!("problems/{target}/{kind}"),
        key,
        Some(&params),
        &format!("problem {kind}"),
        MAX_DETAIL_PAGES,
    )?;
    if !pages.meta.get("complete").is_some_and(truthy) {
        return Err(refused(format!(
            "complete problem {kind} inventory is unavailable"
        )));
    }
    let mut keyed = pages
        .rows
        .into_iter()
        .map(|row| Ok((positive_id(row.get("id").unwrap_or(&Value::Null))?, row)))
        .collect::<Result<Vec<(i64, Value)>>>()?;
    let ids: HashSet<i64> = keyed.iter().map(|(id, _)| *id).collect();
    if ids.len() != keyed.len() {
        return Err(refused("duplicate problem child identities"));
    }
    keyed.sort_by_key(|(id, _)| *id);
    Ok(keyed.into_iter().map(|(_, row)| row).collect())
}

/// Port of fsv.problem_workflows.create_plan: a problem from fresh form
/// fields and rules, with the requester resolved by `requester` only.
pub fn create_plan(
    c: &dyn Api,
    store: &dyn MetaStore,
    answers: &Map<String, Value>,
    requester: &str,
    resolver: Option<Arc<dyn LookupResolver>>,
) -> Result<ProblemPlan> {
    let meta = load_meta(c, store, "problem", "new", true, &Value::Null)?;
    let touched: Vec<&str> = answers.keys().map(String::as_str).collect();
    let blockers = rules_gate(&meta.rules, &meta.spec(), &touched);
    if !blockers.is_empty() {
        return Err(refused(format!(
            "problem rules are unavailable and this write depends on them: {}",
            blockers.join("; ")
        )));
    }
    let warnings = rules_unavailable_warning(&meta.rules)
        .into_iter()
        .collect::<Vec<_>>();
    let person = resolve_requester(c, requester, None)?;
    let Some(email) = person.email.clone().filter(|e| !e.is_empty()) else {
        return Err(Error::value("resolved requester needs an email"));
    };
    let mut state = meta.state("new", resolver, None)?;
    let spec = meta.spec();
    let fields: Vec<String> = spec
        .fields()
        .iter()
        .filter(|f| is_requester(f))
        .map(|f| f.api_name.clone())
        .collect();
    let [requester_name] = &fields[..] else {
        return Err(refused("unique problem requester field is unavailable"));
    };
    for key in answers.keys() {
        if is_requester(find_field(&spec, key).map_err(form_err)?) {
            return Err(refused(
                "requester answers are not allowed; use --requester",
            ));
        }
    }
    state
        .set(requester_name, fsv_forms::Value::Lookup(person.clone()))
        .map_err(form_err)?;
    let errors = apply_answers(&mut state, answers, None)?;
    if !errors.is_empty() {
        return Err(refused(errors.join("; ")));
    }
    let gate = state.rule_gate(&[]).map_err(form_err)?;
    if !gate.blockers.is_empty() {
        return Err(refused(format!(
            "problem rules reference unavailable fields or context: {}",
            gate.blockers.join("; ")
        )));
    }
    if !state.evaluated.messages.is_empty() {
        return Err(refused("problem form has rule messages; use the web form"));
    }
    check(&state, false, std::slice::from_ref(requester_name)).map_err(form_err)?;
    let (mut top, custom) =
        module_body(&state, &[], &TOP, state.evaluated.values.get("status")).map_err(form_err)?;
    top.shift_remove("requester");
    let mut extra: Vec<&str> = top
        .keys()
        .filter(|k| !TOP.contains(&k.as_str()))
        .map(String::as_str)
        .collect();
    extra.sort_unstable();
    if !extra.is_empty() {
        return Err(refused(format!(
            "problem create sends only subject, description, status, priority, impact, requester and custom fields; uncaptured on create: {}; create first, then set them with `problems update`",
            extra.join(", ")
        )));
    }
    if !TOP.iter().all(|k| top.contains_key(*k)) {
        return Err(refused(
            "subject, description, status, priority and impact must be selected from the form",
        ));
    }
    let mut payload = top;
    payload.insert("custom_fields".into(), Value::Object(custom));
    payload.insert("email".into(), json!(email));
    let context = plan_hash(
        &json!({"fields": meta.form_fields, "rules": meta.rules, "requester": person.to_string()}),
    );
    Ok(ProblemPlan {
        plan: WritePlan {
            action: "problem_create".into(),
            target: 0,
            tenant: c.domain(),
            method: "POST".into(),
            path: "problems".into(),
            payload_json: dumps_sorted(&Value::Object(payload)),
            context_hash: context,
        },
        inputs_json: Dumps::default()
            .sort_keys(true)
            .dump(&json!({"answers": answers, "requester": requester})),
        warnings_json: serde_json::to_string(&warnings).unwrap_or_else(|_| "[]".into()),
    })
}

/// `_choice`: the id of the unique active `label` choice of field `name`.
fn choice(fields: &[Value], name: &str, label: &str) -> Result<i64> {
    let matches: Vec<&Value> = fields
        .iter()
        .filter(|f| f.get("name") == Some(&json!(name)))
        .collect();
    let [field] = matches[..] else {
        return Err(refused(format!(
            "unique runtime {name} metadata is unavailable"
        )));
    };
    let want = fsv_core::pystr::casefold(label);
    let empty = Vec::new();
    let choices: Vec<&Value> = field
        .get("choices")
        .and_then(Value::as_array)
        .unwrap_or(&empty)
        .iter()
        .filter(|c| {
            c.is_object()
                && !["deleted", "is_deleted", "deactivated", "deleted_at"]
                    .iter()
                    .any(|k| c.get(*k).is_some_and(truthy))
                && {
                    let text = ["value", "label", "name"]
                        .iter()
                        .filter_map(|k| c.get(*k))
                        .find(|v| truthy(v))
                        .map(py_str)
                        .unwrap_or_default();
                    fsv_core::pystr::casefold(&text) == want
                }
        })
        .collect();
    // Native problem_task_fields serialize choice ids as digit strings; the
    // accepted task-create wire sends the same identities as JSON integers.
    let ident = match choices[..] {
        [one] => crate::choice_ids::runtime_choice_id(one.get("id"), 0),
        _ => None,
    };
    ident.ok_or_else(|| {
        refused(format!(
            "active runtime {name} choice is unavailable or ambiguous"
        ))
    })
}

/// Port of fsv.problem_workflows.child_plan: a body-only note or a basic,
/// self-owned one-minute task (`kind` is `note` or `task`).
pub fn child_plan(c: &dyn Api, target: &str, kind: &str, text: &str) -> Result<ProblemPlan> {
    if !matches!(kind, "note" | "task") {
        return Err(Error::value("unsupported problem child kind"));
    }
    if fsv_core::pystr::strip(text).is_empty() {
        return Err(Error::value("nonempty text is required"));
    }
    let target = positive_id(&json!(parse_id(target, &PROBLEMS)?))?;
    let parent = record(c, target)?;
    let plural = if kind == "note" { "notes" } else { "tasks" };
    let rows = inventory(c, target, plural)?;
    let mut context = json!({"parent": parent, "before": rows});
    let payload = if kind == "note" {
        // Captured body-only note: do not infer privacy or add notification keys.
        json!({"body": text})
    } else {
        let user = self_id(c)?;
        let raw = c.int_get("problem_task_fields", None)?;
        let fields = ["problem_task_fields", "task_fields", "fields"]
            .iter()
            .find_map(|k| raw.get(*k).and_then(Value::as_array))
            .cloned()
            .ok_or_else(|| refused("problem task metadata is unavailable"))?;
        let payload = json!({"title": text, "owner_id": user, "status": choice(&fields, "status", "Open")?,
                             "notify_before": choice(&fields, "notify_before", "Never")?,
                             "planned_effort": "1m"});
        // Requiredness also lives in nested field_options; the parent-provided
        // workspace is omitted by the accepted create body.
        for field in &fields {
            let required = crate::ticket_tasks::required_for_create(field);
            let sent = field.get("name").and_then(Value::as_str).is_some_and(|n| {
                payload.get(n).is_some() || crate::ticket_tasks::CONTEXT_FIELDS.contains(&n)
            });
            if required && !sent {
                return Err(refused("problem task has an uncaptured required field"));
            }
        }
        context["fields"] = Value::Array(fields);
        context["self"] = json!(user);
        payload
    };
    Ok(ProblemPlan {
        plan: WritePlan {
            action: format!("problem_{kind}"),
            target,
            tenant: c.domain(),
            method: "POST".into(),
            path: format!("problems/{target}/{plural}"),
            payload_json: dumps_sorted(&payload),
            context_hash: snapshot_hash(&context)?,
        },
        inputs_json: Dumps::default().dump(&json!({"text": text, "kind": kind})),
        warnings_json: "[]".into(),
    })
}

/// Port of fsv.problem_workflows.send: rebuilt from the plan's inputs, then
/// every payload key read back from the created record.
pub fn send(
    plan: &ProblemPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    store: &dyn MetaStore,
    reviewed_hash: Option<&str>,
    resolver: Option<Arc<dyn LookupResolver>>,
) -> Result<Value> {
    let inputs: Value = serde_json::from_str(&plan.inputs_json)?;
    let create = plan.plan.action == "problem_create";
    let text = |k: &str| {
        inputs
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let rebuild = || -> Result<WritePlan> {
        Ok(if create {
            let answers = inputs
                .get("answers")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            create_plan(c, store, &answers, &text("requester"), resolver.clone())?
        } else {
            child_plan(
                c,
                &plan.plan.target.to_string(),
                &text("kind"),
                &text("text"),
            )?
        }
        .plan)
    };
    let verify = |response: &Value| -> Result<Map<String, Value>> {
        let key = if create {
            "problem"
        } else if text("kind") == "note" {
            "note"
        } else {
            "task"
        };
        let raw = match response {
            Value::Object(m) => m.get(key).unwrap_or(response),
            _ => &Value::Null,
        };
        let exact_id = positive_id(raw.get("id").unwrap_or(&Value::Null))?;
        let row = if create {
            Some(Value::Object(record(c, exact_id)?))
        } else {
            let kind = if key == "note" { "notes" } else { "tasks" };
            inventory(c, plan.plan.target, kind)?
                .into_iter()
                .find(|r| py_str(r.get("id").unwrap_or(&Value::Null)) == exact_id.to_string())
        };
        let matches = |k: &str, actual: &Value, expected: &Value| match k {
            "custom_fields" if create => submitted_value_matches(actual, expected),
            "description" if create => same_text(actual, expected),
            _ => strict_equal(actual, expected),
        };
        let mut fields = Map::new();
        for (k, value) in plan.payload().as_object().into_iter().flatten() {
            let state = match row.as_ref().and_then(|r| r.get(k)) {
                None => "unavailable",
                Some(actual) if matches(k, actual, value) => "matched",
                Some(_) => "mismatched",
            };
            fields.insert(k.clone(), json!(state));
        }
        let verified = row.is_some() && fields.values().all(|v| v == "matched");
        Ok(obj(
            json!({format!("{key}_id"): exact_id, "fields": fields, "verified": verified}),
        ))
    };
    execute(&plan.plan, c, journal, reviewed_hash, &rebuild, &verify)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `problem_workflows._choice(fields, "status", label)` from
    /// `gen_fsv_api_workflows.py`.
    #[test]
    fn choice_matches_python() {
        let oracle: Value =
            serde_json::from_str(include_str!("testdata/workflows_oracle.json")).unwrap();
        for case in oracle["direct"]["problem_choice"].as_array().unwrap() {
            let fields = case["fields"].as_array().unwrap();
            let got = match choice(fields, "status", case["label"].as_str().unwrap()) {
                Ok(id) => json!({"ok": id}),
                Err(e) => json!({"error": e.to_string()}),
            };
            let want = match case.get("ok") {
                Some(ok) => json!({"ok": ok}),
                None => json!({"error": case["error"]}),
            };
            assert_eq!(got, want, "{case}");
        }
    }
}
