//! Create and update changes through the form engine (`change_flow.py`).
//!
//! Every face ends here: a [`Plan`] (engine state + exact request body +
//! files + plan hash + review), then confirmation, a metadata freshness
//! check, and the writes, journaled under the plan hash.
//!
//! Order of writes on create: the change itself, then attachments (uploaded
//! only now, after confirmation), then planning fields. A failure after the
//! change exists reports the created id and every uploaded attachment id.

use std::collections::HashSet;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use indexmap::IndexMap;
use serde_json::{Map, Value as Json, json};

use fsv_api::service::{Journal, PlanEvent};
use fsv_core::error::{Error, Result};
use fsv_core::journal::PlanStep;
use fsv_core::review::{Review, ReviewAttachment, ReviewMessage};
use fsv_core::safety::{RiskContext, norm, norm_value, write_risks};
use fsv_forms::payload::encode_value;
use fsv_forms::plan::PlanFiles;
use fsv_forms::rules::ChoiceRestriction;
use fsv_forms::{
    Choice, Conflict, EvaluatedForm, Field, FormError, FormSpec, FormState, Kind, LookupValue,
    ModuleOptions, Value, build_change_payload, plan_hash, plan_matches,
};
use fsv_py::truthy;

use crate::drafts::{
    Draft, TemplateOptions, expanduser, load_hints, matching_hints, parse_template, render_template,
};
use crate::meta::{
    ClientResolver, FormMeta, MetaSource, Resolver, apply_answers, existing_from_record,
    find_field, form_err, is_description, is_requester, is_status, name_existing, record_key,
    rules_warnings,
};

pub const DUPLICATE_WINDOW_HOURS: f64 = 24.0;

/// `FlowExit.__init__`: stop with a message and a documented exit code
/// (`FlowExit(code, message)`).
pub fn flow_exit(code: i32, message: impl Into<String>) -> Error {
    Error::exit(code, message)
}

/// The change was written, then a later step (upload, planning) failed.
#[derive(Debug)]
pub struct PartialFailure {
    pub created_id: Json,
    pub uploaded: Vec<Json>,
    pub steps: Vec<Json>,
    pub error: Error,
}

impl PartialFailure {
    pub fn new(
        created_id: Json,
        uploaded: Vec<Json>,
        steps: Vec<Json>,
        error: Error,
    ) -> PartialFailure {
        PartialFailure {
            created_id,
            uploaded,
            steps,
            error,
        }
    }

    /// Exit code of the cause (`raise PartialFailure(...) from e`).
    pub fn exit_code(&self) -> i32 {
        match &self.error {
            e @ (Error::Api(_) | Error::Session(_)) => e.exit_code(),
            _ => 1,
        }
    }
}

impl std::fmt::Display for PartialFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ids: Vec<String> = self
            .uploaded
            .iter()
            .map(|u| fsv_py::py_str(&u["id"]))
            .collect();
        let ids = if ids.is_empty() {
            "none".to_string()
        } else {
            ids.join(", ")
        };
        write!(
            f,
            "change {} was written, then a later step failed: {}. Uploaded attachment ids: {ids}",
            fsv_py::py_str(&self.created_id),
            self.error
        )
    }
}

impl std::error::Error for PartialFailure {}

/// What [`execute`] fails with.
#[derive(Debug)]
pub enum ExecError {
    Failed(Error),
    Partial(Box<PartialFailure>),
}

impl From<Error> for ExecError {
    fn from(e: Error) -> Self {
        ExecError::Failed(e)
    }
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecError::Failed(e) => e.fmt(f),
            ExecError::Partial(p) => p.fmt(f),
        }
    }
}

impl ExecError {
    pub fn exit_code(&self) -> i32 {
        match self {
            ExecError::Failed(e) => e.exit_code(),
            ExecError::Partial(p) => p.exit_code(),
        }
    }
}

/// Prompts allowed?
pub fn interactive() -> bool {
    std::io::stdin().is_terminal()
}

/// `{"name", "size", "sha256", "path"}` of one file.
pub fn sha256_file(path: &str) -> std::io::Result<Json> {
    let d = fsv_forms::plan::file_digest(path)?;
    Ok(json!({"name": d.name, "size": d.size, "sha256": d.sha256, "path": path}))
}

/// One planning field a plan writes after the change exists.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanningEntry {
    pub label: String,
    pub field_id: String,
    pub field_label: String,
    pub files: Vec<String>,
    pub description_html: Option<String>,
}

#[derive(Clone)]
pub struct Plan {
    /// `create` | `update`.
    pub action: String,
    pub meta: Arc<FormMeta>,
    pub state: FormState,
    pub draft: Draft,
    pub payload: Option<Map<String, Json>>,
    pub files: IndexMap<String, Vec<String>>,
    pub planning: Vec<PlanningEntry>,
    pub review: Review,
    pub errors: Vec<String>,
    pub requester: Option<LookupValue>,
    pub target: Option<i64>,
    pub hash: String,
    pub risks: Vec<String>,
}

fn is_file(p: &str) -> bool {
    Path::new(p).is_file()
}

impl Plan {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn to_json(&self) -> Json {
        let attachments: Vec<Json> = self
            .files
            .get("attachments")
            .into_iter()
            .flatten()
            .filter(|p| is_file(p))
            .filter_map(|p| sha256_file(p).ok())
            .collect();
        let planning: Vec<Json> = self
            .planning
            .iter()
            .map(|p| {
                let files: Vec<Json> = p.files.iter().filter(|f| is_file(f)).filter_map(|f| sha256_file(f).ok()).collect();
                json!({"label": p.label, "field": p.field_id, "description_html": p.description_html, "files": files})
            })
            .collect();
        json!({
            "plan_hash": if self.hash.is_empty() { Json::Null } else { json!(self.hash) },
            "action": self.action,
            "target": self.target,
            "payload": if self.ok() { self.payload.clone().map(Json::Object).unwrap_or(Json::Null) } else { Json::Null },
            "after": {"attachments": attachments, "planning": planning},
            "review": self.review.to_dict(),
            "warnings": self.review.warnings,
            "risks": self.risks,
            "requires_confirmation": !self.risks.is_empty(),
            "errors": self.errors,
        })
    }
}

// ---------------------------------------------------------------------------
// building plans

pub fn field_by_type<'a>(spec: &'a FormSpec, field_type: &str) -> Option<&'a Field> {
    spec.fields().iter().find(|f| f.field_type == field_type)
}

/// Old create/update flags as engine answers (keyed by internal name).
pub fn legacy_answers(
    meta: &FormMeta,
    subject: Option<&str>,
    priority: Option<&str>,
    agent: Option<&str>,
    group: Option<&str>,
) -> Result<Map<String, Json>> {
    let spec = meta.spec();
    let mut out = Map::new();
    for (ftype, value) in [
        ("default_subject", subject),
        ("default_priority", priority),
        ("default_agent", agent),
        ("default_group", group),
    ] {
        let Some(value) = value else { continue };
        let Some(f) = field_by_type(&spec, ftype) else {
            return Err(flow_exit(
                2,
                format!(
                    "this form has no {} field",
                    ftype.trim_start_matches("default_")
                ),
            ));
        };
        out.insert(f.api_name.clone(), json!(value));
    }
    Ok(out)
}

/// Answers without status/requester, the status and requester raw values,
/// and errors.
pub struct Special {
    pub answers: Map<String, Json>,
    pub status_raw: Option<Json>,
    pub requester_raw: Option<Json>,
    pub errors: Vec<String>,
}

pub fn split_special(meta: &FormMeta, draft: &Draft) -> Special {
    let spec = meta.spec();
    let mut out = Special {
        answers: Map::new(),
        status_raw: None,
        requester_raw: None,
        errors: Vec::new(),
    };
    for (k, v) in &draft.answers {
        match find_field(&spec, k) {
            Err(_) => {
                out.answers.insert(k.clone(), v.clone());
            }
            Ok(f) if is_status(f) => out.status_raw = Some(v.clone()),
            Ok(f) if is_requester(f) => out.requester_raw = Some(v.clone()),
            Ok(_) => {
                out.answers.insert(k.clone(), v.clone());
            }
        }
    }
    if let Some(html) = &draft.description_html {
        match spec.fields().iter().find(|f| is_description(f)) {
            None => out.errors.push("this form has no description field".into()),
            Some(desc) => {
                out.answers
                    .retain(|k, _| spec.find(k).is_none_or(|f| f.api_name != desc.api_name));
                out.answers.insert(desc.api_name.clone(), json!(html));
            }
        }
    }
    out
}

pub fn acknowledge(state: &mut FormState, acks: &[String], ack_all: bool) -> Vec<String> {
    let mut warnings = Vec::new();
    if ack_all || acks.iter().any(|a| norm(a) == "all") {
        state.acknowledge_all();
        return warnings;
    }
    for a in acks {
        if state
            .acknowledge(&[fsv_forms::Ack::Text(a.clone())])
            .is_err()
        {
            warnings.push(format!(
                "ack {}: no blocking message matches (ignored)",
                fsv_py::py_repr_str(a)
            ));
        }
    }
    warnings
}

// planning-field identities (`create.py`)

fn planning_key(value: &str) -> String {
    let mut text = fsv_core::pystr::casefold(value);
    for prefix in ["planning-field-", "cf_"] {
        if let Some(rest) = text.strip_prefix(prefix) {
            text = rest.to_string();
        }
    }
    text.chars().filter(|c| c.is_alphanumeric()).collect()
}

fn singular_key(value: &str) -> String {
    let mut text = fsv_core::pystr::casefold(value);
    for prefix in ["planning-field-", "cf_"] {
        if let Some(rest) = text.strip_prefix(prefix) {
            text = rest.to_string();
        }
    }
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    let flush = |current: &mut String, words: &mut Vec<String>| {
        let w = std::mem::take(current);
        words.push(
            w.strip_suffix('s')
                .map_or_else(|| w.clone(), str::to_string),
        );
    };
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            current.push(ch);
        } else if !current.is_empty() {
            flush(&mut current, &mut words);
        }
    }
    if !current.is_empty() {
        flush(&mut current, &mut words);
    }
    words.concat()
}

fn jtext(v: Option<&Json>) -> String {
    v.filter(|v| truthy(v))
        .map(fsv_py::py_str)
        .unwrap_or_default()
}

fn norm_label(v: Option<&Json>) -> String {
    let t = jtext(v);
    fsv_core::pystr::casefold(&fsv_core::pystr::collapse_ws(
        &fsv_core::pystr::html_unescape(&t),
    ))
}

/// The account's planning-field definitions, in metadata order.
pub fn planning_definitions(schema_fields: &[Json]) -> Vec<&Json> {
    schema_fields
        .iter()
        .filter(|f| {
            f.is_object()
                && fsv_forms::spec::is_planning_type(
                    f.get("field_type").and_then(Json::as_str).unwrap_or(""),
                )
        })
        .collect()
}

/// The key `/changes/<id>/planning-fields/<key>` takes for this field on
/// this account (its name from the metadata, else its id).
pub fn planning_server_key(defn: &Json) -> String {
    match defn.get("name").filter(|v| truthy(v)) {
        Some(n) => fsv_py::py_str(n),
        None => fsv_py::py_str(defn.get("id").unwrap_or(&Json::Null)),
    }
}

/// Resolve an exact server identity or a unique exact current label.
pub fn resolve_planning_definition<'a>(
    value: &str,
    schema_fields: &'a [Json],
) -> std::result::Result<&'a Json, String> {
    let raw = fsv_core::pystr::strip(value).to_string();
    let defs = planning_definitions(schema_fields);
    let ambiguous = || {
        format!(
            "ambiguous planning field: {}; select an exact identity",
            fsv_py::py_repr_str(value)
        )
    };
    let identities: Vec<&Json> = defs
        .iter()
        .copied()
        .filter(|f| !raw.is_empty() && (raw == jtext(f.get("name")) || raw == jtext(f.get("id"))))
        .collect();
    let labels: Vec<&Json> = defs
        .iter()
        .copied()
        .filter(|f| !raw.is_empty() && norm_label(f.get("label")) == norm_label(Some(&json!(raw))))
        .collect();
    let matches = if identities.is_empty() {
        labels
    } else {
        identities
    };
    if matches.len() > 1 {
        return Err(ambiguous());
    }
    if let Some(m) = matches.first() {
        return Ok(m);
    }
    let mut aliases = vec![raw.clone()];
    if let Some(rest) = raw.strip_prefix("planning-field-") {
        aliases.push(rest.to_string());
    }
    let extra: Vec<String> = aliases
        .iter()
        .filter_map(|a| a.strip_prefix("cf_").map(str::to_string))
        .collect();
    aliases.extend(extra);
    let mut keys: HashSet<String> = aliases
        .iter()
        .flat_map(|a| [planning_key(a), singular_key(a)])
        .collect();
    keys.remove("");
    let mut found = Vec::new();
    for f in &defs {
        let name = fsv_py::py_str(f.get("name").unwrap_or(&Json::Null));
        let mut cands: Vec<String> = [
            "name",
            "id",
            "label",
            "export_label",
            "helptext",
            "description",
        ]
        .iter()
        .filter_map(|k| f.get(*k).filter(|v| truthy(v)).map(fsv_py::py_str))
        .collect();
        cands.extend([
            format!("planning-field-{name}"),
            format!("planning-field-cf_{name}"),
            format!("cf_{name}"),
        ]);
        let ck: HashSet<String> = cands
            .iter()
            .flat_map(|c| [planning_key(c), singular_key(c)])
            .collect();
        if !keys.is_disjoint(&ck) {
            found.push(*f);
        }
    }
    if found.len() > 1 {
        return Err(ambiguous());
    }
    if let Some(m) = found.first() {
        return Ok(m);
    }
    let known: Vec<String> = defs
        .iter()
        .map(|f| {
            let l = f
                .get("label")
                .filter(|v| truthy(v))
                .or_else(|| f.get("name"))
                .unwrap_or(&Json::Null);
            fsv_py::py_str(l)
        })
        .collect();
    Err(format!(
        "planning field not found: {}; choices: {}",
        fsv_py::py_repr_str(value),
        known.join(", ")
    ))
}

pub fn resolve_planning(meta: &FormMeta, draft: &Draft) -> (Vec<PlanningEntry>, Vec<String>) {
    let raw = meta.raw_fields();
    let mut out = Vec::new();
    let mut errors = Vec::new();
    for (label, entry) in &draft.planning {
        match resolve_planning_definition(label, &raw) {
            Err(e) => errors.push(e),
            Ok(defn) => out.push(PlanningEntry {
                label: label.clone(),
                field_id: planning_server_key(defn),
                field_label: defn
                    .get("label")
                    .filter(|v| truthy(v))
                    .map_or_else(|| label.clone(), fsv_py::py_str),
                files: entry.files.clone(),
                description_html: entry.description_html.clone(),
            }),
        }
    }
    (out, errors)
}

pub fn file_errors(paths: &[String]) -> Vec<String> {
    paths
        .iter()
        .filter(|p| !is_file(p))
        .map(|p| format!("file not found: {p}"))
        .collect()
}

/// Genuine rule conflicts: two rules that BOTH match in the final state and
/// give opposite actions on one field property. Rules that matched earlier
/// in the event replay but no longer match are not conflicts.
pub fn real_conflicts(ev: &EvaluatedForm) -> Vec<Conflict> {
    let current: HashSet<String> = ev.matched.iter().map(fsv_py::py_str).collect();
    let mut out = Vec::new();
    for c in &ev.conflicts {
        let live: Vec<_> = c
            .decisions
            .iter()
            .filter(|d| current.contains(&fsv_py::py_str(&d.rule_id)))
            .cloned()
            .collect();
        let ids: HashSet<String> = live.iter().map(|d| fsv_py::py_str(&d.rule_id)).collect();
        if ids.len() < 2 {
            continue;
        }
        let positive: HashSet<bool> = live
            .iter()
            .map(|d| ["show", "mandate", "disable"].contains(&d.action.as_str()))
            .collect();
        if positive.len() < 2 {
            continue;
        }
        let winner = if live.contains(&c.winner) {
            c.winner.clone()
        } else {
            live[live.len() - 1].clone()
        };
        out.push(Conflict {
            field: c.field.clone(),
            property: c.property.clone(),
            decisions: live,
            winner,
        });
    }
    out
}

/// (choice, restriction) when a rule limits `name` to exactly one real
/// (non-empty) choice.
pub fn only_option(state: &FormState, name: &str) -> Result<Option<(Choice, ChoiceRestriction)>> {
    let Some(r) = state.evaluated.restricted.get(name) else {
        return Ok(None);
    };
    let offered: Vec<Choice> = state
        .choices(name)
        .map_err(form_err)?
        .into_iter()
        .filter(|c| c.visible && !c.label.is_empty() && !c.value.is_empty())
        .collect();
    Ok(if offered.len() == 1 {
        Some((offered[0].clone(), r.clone()))
    } else {
        None
    })
}

pub fn rule_of(r: &ChoiceRestriction) -> String {
    if r.rule_name.is_empty() {
        fsv_py::py_str(&r.rule_id)
    } else {
        r.rule_name.clone()
    }
}

/// Review-row suffix: how a rule shaped this value.
pub fn auto_suffix(state: &FormState, name: &str) -> Result<String> {
    let ev = &state.evaluated;
    if ev.populated.contains_key(name) {
        return Ok(" (auto)".into());
    }
    if let Some((choice, r)) = only_option(state, name)?
        && norm(&state.display(name, None).map_err(form_err)?) == norm(&choice.label)
    {
        return Ok(format!(" (only option allowed by rule {})", rule_of(&r)));
    }
    if let Some(r) = ev.restricted.get(name) {
        return Ok(format!(" (choices limited by rule {})", rule_of(r)));
    }
    Ok(String::new())
}

pub fn unsent_suffix(f: &Field) -> &'static str {
    if f.sent {
        ""
    } else {
        " (not sent: fsv does not know its request key yet)"
    }
}

fn label_of(state: &FormState, name: &str) -> String {
    state
        .spec
        .get(name)
        .map_or_else(|| name.to_string(), |f| f.label.clone())
}

pub fn notes(state: &FormState) -> Result<Vec<String>> {
    let ev = &state.evaluated;
    let mut out = Vec::new();
    for (name, rid) in &ev.populated {
        if ev.is_visible(name) {
            out.push(format!(
                "{} = {} (filled in by rule {})",
                label_of(state, name),
                state.display(name, None).map_err(form_err)?,
                fsv_py::py_str(rid)
            ));
        }
    }
    for (name, r) in &ev.restricted {
        if ev.is_visible(name) {
            let offered: Vec<String> = state
                .choices(name)
                .map_err(form_err)?
                .into_iter()
                .filter(|c| c.visible)
                .map(|c| c.label)
                .collect();
            let shown = if offered.is_empty() {
                "(none)".to_string()
            } else {
                offered.join(", ")
            };
            out.push(format!(
                "{}: choices limited to {shown} by rule {}",
                label_of(state, name),
                fsv_py::py_repr_str(&rule_of(r))
            ));
        }
    }
    let pos = |n: &String| ev.visible.iter().position(|v| v == n).unwrap_or(1 << 20);
    let mut disabled: Vec<&String> = ev.disabled.iter().collect();
    disabled.sort_by_key(|n| pos(n));
    for name in disabled {
        let d = ev.decision(name, "disabled");
        out.push(format!(
            "{}: read-only{}",
            label_of(state, name),
            d.map(|d| format!(" ({})", d.describe()))
                .unwrap_or_default()
        ));
    }
    for (name, (old, rid)) in &ev.overridden {
        out.push(format!(
            "{}: your value {} was replaced by rule {}",
            label_of(state, name),
            fsv_py::py_repr_str(&state.display(name, Some(old)).map_err(form_err)?),
            fsv_py::py_str(rid)
        ));
    }
    out.extend(
        real_conflicts(ev)
            .iter()
            .map(|c| format!("conflict: {}", c.describe())),
    );
    Ok(out)
}

pub fn messages(state: &FormState) -> Vec<ReviewMessage> {
    state
        .evaluated
        .messages
        .iter()
        .map(|m| ReviewMessage {
            text: m.text.clone(),
            blocking: m.blocking,
            acknowledged: m.blocking && state.acknowledged.contains(&m.key()),
            rule: if m.rule_name.is_empty() {
                m.rule_id.clone()
            } else {
                json!(m.rule_name)
            },
        })
        .collect()
}

pub fn dropped(state: &FormState) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for (f, v, d) in state.dropped() {
        let why = match &d {
            Some(d) if d.action == "hide" => {
                format!("hidden by rule {}", fsv_py::py_repr_str(&d.rule_name))
            }
            _ => "hidden".to_string(),
        };
        out.push(format!(
            "{} = {} ({why})",
            f.label,
            state.display(&f.api_name, Some(&v)).map_err(form_err)?
        ));
    }
    Ok(out)
}

/// Matching hints for `section` and hints-file problems (`None`: the
/// default `hints.yaml`).
pub fn hint_lines(
    state: &FormState,
    section: &str,
    path: Option<&Path>,
) -> Result<(Vec<String>, Vec<String>)> {
    match load_hints(section, path) {
        Ok(hints) => matching_hints(state, &hints),
        Err(Error::Value(m)) => Ok((Vec::new(), vec![m])),
        Err(e) => Err(e),
    }
}

pub fn attachment_rows(
    files: &IndexMap<String, Vec<String>>,
    planning: &[PlanningEntry],
) -> Vec<ReviewAttachment> {
    let labels: IndexMap<String, String> = planning
        .iter()
        .map(|p| {
            (
                format!("planning:{}", p.field_id),
                format!("planning: {}", p.label),
            )
        })
        .collect();
    let mut rows = Vec::new();
    for (target, paths) in files {
        for p in paths {
            if let Ok(Some(d)) = is_file(p)
                .then(|| fsv_forms::plan::file_digest(p))
                .transpose()
            {
                rows.push(ReviewAttachment {
                    target: labels
                        .get(target)
                        .cloned()
                        .unwrap_or_else(|| target.clone()),
                    name: d.name,
                    size: d.size,
                    sha256: d.sha256,
                    path: p.clone(),
                });
            }
        }
    }
    rows
}

/// Plan hash over the request, the planning texts, the target and every file.
pub fn hash(
    request: &Json,
    files: &IndexMap<String, Vec<String>>,
    planning: &[PlanningEntry],
    target: Option<i64>,
) -> Result<String> {
    let after: Map<String, Json> = planning
        .iter()
        .map(|p| (p.field_id.clone(), json!(p.description_html)))
        .collect();
    let mut doc = Map::new();
    doc.insert("request".into(), request.clone());
    doc.insert("after".into(), json!({"planning": after}));
    if let Some(t) = target {
        doc.insert("target".into(), json!(t));
    }
    let groups: IndexMap<String, Vec<PathBuf>> = files
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| (k.clone(), v.iter().map(PathBuf::from).collect()))
        .collect();
    Ok(plan_hash(
        &Json::Object(doc),
        Some(&PlanFiles::Fields(groups)),
    )?)
}

fn plan_files(draft: &Draft, planning: &[PlanningEntry]) -> IndexMap<String, Vec<String>> {
    let mut files = IndexMap::new();
    files.insert("attachments".to_string(), draft.attachments.clone());
    for p in planning {
        files.insert(format!("planning:{}", p.field_id), p.files.clone());
    }
    files
}

fn all_paths(files: &IndexMap<String, Vec<String>>) -> Vec<String> {
    files.values().flatten().cloned().collect()
}

/// `state.set` that keeps form errors as problems and fails on others.
fn try_set(state: &mut FormState, name: &str, v: Value, errors: &mut Vec<String>) -> Result<()> {
    match state.set(name, v) {
        Ok(_) => Ok(()),
        Err(e) if e.is_form_error() => {
            errors.push(e.to_string());
            Ok(())
        }
        Err(e) => Err(form_err(e)),
    }
}

fn choice_label(f: &Field, v: &Value) -> String {
    match f.choice(v) {
        Ok(Some(c)) => c.label.clone(),
        _ => v.py_str(),
    }
}

fn payload_problems(e: FormError, problems: &mut Vec<String>) {
    match e {
        FormError::Payload { problems: ps } => {
            for p in ps {
                if !problems.contains(&p) {
                    problems.push(p);
                }
            }
        }
        other => problems.push(other.to_string()),
    }
}

/// Keyword arguments of [`build_create_plan`].
#[derive(Clone, Default)]
pub struct CreateOptions {
    pub resolver: Option<Arc<dyn Resolver>>,
    pub requester: Option<LookupValue>,
    pub ack_all: bool,
    /// `--no-validate`: send without the client-side required check.
    pub allow_incomplete: bool,
    /// Hints file (`None`: `<config>/hints.yaml`).
    pub hints_path: Option<PathBuf>,
}

pub fn build_create_plan(
    meta: &Arc<FormMeta>,
    draft: &Draft,
    opts: &CreateOptions,
) -> Result<Plan> {
    let spec = meta.spec();
    let mut state = meta.state("new", lookup(&opts.resolver), None)?;
    let Special {
        answers,
        status_raw,
        mut errors,
        ..
    } = split_special(meta, draft);
    let mut warnings: Vec<String> = spec
        .warnings
        .iter()
        .cloned()
        .chain(rules_warnings(&meta.rules))
        .collect();
    let req_field = spec.fields().iter().find(|f| is_requester(f));
    let requester = opts.requester.as_ref();
    if let (Some(r), Some(rf)) = (requester, req_field) {
        try_set(
            &mut state,
            &rf.api_name,
            Value::Lookup(r.clone()),
            &mut errors,
        )?;
    }
    let mut failed = HashSet::new();
    errors.extend(apply_answers(&mut state, &answers, Some(&mut failed))?);
    warnings.extend(acknowledge(&mut state, &draft.ack, opts.ack_all));

    let status_field = spec.fields().iter().find(|f| is_status(f));
    let default = status_field
        .and_then(|f| f.default.clone())
        .unwrap_or(Value::Null);
    let mut status_value = default.clone();
    let mut risks = Vec::new();
    if let (Some(raw), Some(sf)) = (
        status_raw
            .as_ref()
            .filter(|r| !r.is_null() && r.as_str() != Some("")),
        status_field,
    ) {
        match state.parse_field(sf, &Value::from_json(raw)) {
            Ok(v) => status_value = v,
            Err(e) if e.is_form_error() => errors.push(e.to_string()),
            Err(e) => return Err(form_err(e)),
        }
        if status_value != default {
            risks.push(format!(
                "status on create -> {} (not the form default; may start approvals/workflows)",
                choice_label(sf, &status_value)
            ));
        }
    }

    let mut skip: HashSet<String> = req_field
        .map(|f| format!("{} is required", f.label))
        .into_iter()
        .collect();
    skip.extend(
        failed
            .iter()
            .map(|n| format!("{} is required", label_of(&state, n))),
    );
    let mut form_problems: Vec<String> = state
        .errors()
        .map_err(form_err)?
        .into_iter()
        .filter(|p| !skip.contains(p))
        .collect();
    if requester.is_none() {
        form_problems.insert(
            0,
            "requester is not set: pass --requester EMAIL, add `requester:` to the draft, \
             or `fsv config set defaults.requester EMAIL`"
                .into(),
        );
    }
    let (planning, perr) = resolve_planning(meta, draft);
    let files = plan_files(draft, &planning);
    let mut problems: Vec<String> = errors
        .into_iter()
        .chain(perr)
        .chain(file_errors(&all_paths(&files)))
        .collect();
    if !opts.allow_incomplete {
        problems.extend(form_problems.iter().cloned());
    } else if !form_problems.is_empty() {
        warnings.push(format!(
            "--no-validate: sending without the client-side required check: {}",
            form_problems.join("; ")
        ));
    }

    let mut payload = None;
    if let Some(r) = requester {
        let mopts = ModuleOptions {
            requester_email: r.email.clone(),
            requester_name: Some(r.display.clone()),
            status: (!status_value.is_null()).then(|| status_value.clone()),
            allow_incomplete: !problems.is_empty() || opts.allow_incomplete,
            ..ModuleOptions::default()
        };
        match build_change_payload(&state, &mopts) {
            Ok(p) => payload = Some(p),
            Err(e) => payload_problems(e, &mut problems),
        }
    }

    let subject = if spec.find("subject").is_some() {
        state.get("subject").ok().flatten().filter(|v| v.truthy())
    } else {
        None
    };
    let mut review = Review::new(format!(
        "New Change{}",
        subject
            .map(|s| format!(": {}", s.py_str()))
            .unwrap_or_default()
    ));
    review.rows.push((
        "Requester".into(),
        requester.map_or_else(|| "(not set)".to_string(), ToString::to_string),
    ));
    let sent = state.payload_values();
    for name in sent.keys() {
        let Some(f) = spec.get(name) else { continue };
        if is_requester(f) || is_status(f) {
            continue;
        }
        let shown = state.display(name, None).map_err(form_err)?
            + &auto_suffix(&state, name)?
            + unsent_suffix(f);
        review.rows.push((f.label.clone(), shown));
    }
    if let Some(sf) = status_field {
        let mut shown = choice_label(sf, &status_value);
        if status_value == default {
            shown.push_str(" (form default)");
        }
        review.rows.push((format!("{} on create", sf.label), shown));
    }
    let mut notify = Vec::new();
    for (ftype, what) in [
        ("default_agent", "assigned agent"),
        ("default_group", "assigned group"),
    ] {
        if let Some(f) = field_by_type(&spec, ftype)
            && sent.contains_key(&f.api_name)
        {
            notify.push(format!(
                "{}: {} ({what}; Freshservice usually notifies on assignment)",
                f.label,
                state.display(&f.api_name, None).map_err(form_err)?
            ));
        }
    }
    if let Some(r) = requester {
        notify.push(format!("Requester: {r} (requester notifications)"));
    }
    let (hints, hint_problems) = hint_lines(&state, "changes", opts.hints_path.as_deref())?;
    notify.extend(hints);
    review.notify = notify;
    review.messages = messages(&state);
    review.dropped = dropped(&state)?;
    review.notes = notes(&state)?;
    review.attachments = attachment_rows(&files, &planning);
    review.risks = risks.clone();
    review.errors = problems.clone();
    review.warnings = warnings.into_iter().chain(hint_problems).collect();
    review.metadata = meta.as_of();
    let mut plan = Plan {
        action: "create".into(),
        meta: meta.clone(),
        state,
        draft: draft.clone(),
        payload,
        files,
        planning,
        review,
        errors: problems,
        requester: opts.requester.clone(),
        target: None,
        hash: String::new(),
        risks,
    };
    if let Some(p) = &plan.payload
        && plan.errors.is_empty()
    {
        plan.hash = hash(&Json::Object(p.clone()), &plan.files, &plan.planning, None)?;
        plan.review.plan_hash = plan.hash.clone();
    }
    Ok(plan)
}

fn empty_json(e: &Json) -> bool {
    matches!(e, Json::Null) || e.as_str() == Some("") || e.as_array().is_some_and(Vec::is_empty)
}

/// Encoded values compare equal when they mean the same to the server.
pub fn normcmp(f: &Field, e: &Json) -> Json {
    if empty_json(e) {
        return Json::Null;
    }
    if f.kind == Kind::Datetime
        && let Some(s) = e.as_str()
    {
        match fsv_forms::pydt::datetime_fromisoformat(&s.replace('Z', "+00:00")) {
            None => return e.clone(),
            Some((local, Some(off))) => {
                let utc = off
                    .to_timestamp(local)
                    .map(|t| t.to_zoned(jiff::tz::TimeZone::UTC).datetime());
                if let Ok(utc) = utc {
                    return json!(
                        fsv_forms::pydt::DateTimeValue::new(utc, jiff::tz::Offset::UTC).isoformat()
                    );
                }
            }
            Some((_, None)) => {}
        }
    }
    if let Json::Array(items) = e {
        let mut v: Vec<String> = items.iter().map(fsv_py::py_str).collect();
        v.sort();
        return json!(v);
    }
    if e.is_boolean() {
        return e.clone();
    }
    if matches!(f.kind, Kind::Text | Kind::Textarea | Kind::Richtext) {
        return json!(fsv_core::pystr::strip(&fsv_py::py_str(e)));
    }
    json!(fsv_py::py_str(e))
}

/// The request value of `v` for `f` (`Null` when empty).
pub fn encode(state: &FormState, f: &Field, v: Option<&Value>) -> Json {
    let Some(v) = v.filter(|v| !v.is_empty()) else {
        return Json::Null;
    };
    if let Ok(j) = encode_value(&state.spec, f, v, &state.evaluated.values) {
        return j;
    }
    match v {
        Value::Lookup(lv) => lv.id.clone(),
        Value::List(items) => Json::Array(
            items
                .iter()
                .map(|x| match x {
                    Value::Lookup(lv) => lv.id.clone(),
                    other => other.to_json().unwrap_or(Json::Null),
                })
                .collect(),
        ),
        other => other.to_json().unwrap_or(Json::Null),
    }
}

/// Keyword arguments of [`build_update_plan`].
#[derive(Clone, Default)]
pub struct UpdateOptions<'a> {
    pub resolver: Option<Arc<dyn Resolver>>,
    pub ack_all: bool,
    /// Status value -> label (default: the status field's choice label).
    pub status_label: Option<&'a dyn Fn(&Json) -> String>,
    pub hints_path: Option<PathBuf>,
}

pub fn build_update_plan(
    meta: &Arc<FormMeta>,
    draft: &Draft,
    record: &Map<String, Json>,
    target: i64,
    opts: &UpdateOptions<'_>,
) -> Result<Plan> {
    let spec = meta.spec();
    let existing = name_existing(
        existing_from_record(&spec, record),
        &spec,
        record,
        opts.resolver.as_deref(),
    );
    let mut state = meta.state("edit", lookup(&opts.resolver), Some(existing.clone()))?;
    let mut answers = draft.answers.clone();
    let mut errors = Vec::new();
    if let Some(html) = &draft.description_html {
        match spec.fields().iter().find(|f| is_description(f)) {
            None => errors.push("this form has no description field".to_string()),
            Some(desc) => {
                answers.retain(|k, _| spec.find(k).is_none_or(|f| f.api_name != desc.api_name));
                answers.insert(desc.api_name.clone(), json!(html));
            }
        }
    }
    let answered: HashSet<String> = answers
        .keys()
        .filter_map(|k| spec.find(k))
        .map(|f| f.api_name.clone())
        .collect();
    let mut failed = HashSet::new();
    errors.extend(apply_answers(&mut state, &answers, Some(&mut failed))?);
    let warnings: Vec<String> = spec
        .warnings
        .iter()
        .cloned()
        .chain(rules_warnings(&meta.rules))
        .chain(acknowledge(&mut state, &draft.ack, opts.ack_all))
        .collect();
    let skip: HashSet<String> = failed
        .iter()
        .map(|n| format!("{} is required", label_of(&state, n)))
        .collect();
    let mut problems: Vec<String> = errors;
    problems.extend(
        state
            .errors()
            .map_err(form_err)?
            .into_iter()
            .filter(|p| !skip.contains(p)),
    );

    let mut top = Map::new();
    let mut custom = Map::new();
    let mut changes = Vec::new();
    let ev = state.evaluated.clone();
    for f in spec.ordered() {
        let name = &f.api_name;
        if matches!(f.kind, Kind::Notice | Kind::Attachment) {
            continue;
        }
        if !answered.contains(name) && !ev.populated.contains_key(name) {
            continue;
        }
        if !ev.is_visible(name) {
            continue;
        }
        let new_v = ev.values.get(name);
        let old_v = existing.get(name);
        let (new_e, old_e) = (encode(&state, f, new_v), encode(&state, f, old_v));
        if normcmp(f, &new_e) == normcmp(f, &old_e) {
            continue;
        }
        let old_shown = match old_v {
            Some(o) => state.display(name, Some(o)).map_err(form_err)?,
            None => String::new(),
        };
        if !f.sent {
            changes.push((
                f.label.clone(),
                old_shown,
                state.display(name, None).map_err(form_err)? + unsent_suffix(f),
            ));
            continue;
        }
        if f.is_custom {
            custom.insert(name.clone(), new_e);
        } else {
            top.insert(record_key(name).to_string(), new_e);
        }
        changes.push((
            f.label.clone(),
            old_shown,
            state.display(name, None).map_err(form_err)? + &auto_suffix(&state, name)?,
        ));
    }
    let mut body = top.clone();
    if !custom.is_empty() {
        body.insert("custom_fields".into(), Json::Object(custom));
    }
    let (planning, perr) = resolve_planning(meta, draft);
    let files = plan_files(draft, &planning);
    problems.extend(perr);
    problems.extend(file_errors(&all_paths(&files)));

    let sf = spec.fields().iter().find(|f| is_status(f));
    let default_label = |v: &Json| -> String {
        match sf.and_then(|f| f.choice(v).ok().flatten()) {
            Some(c) => c.label.clone(),
            None => fsv_py::py_str(v),
        }
    };
    let label_fn: &dyn Fn(&Json) -> String = opts.status_label.unwrap_or(&default_label);
    let risks = write_risks(
        "changes",
        &top,
        &RiskContext {
            current: Some(record),
            status_label: Some(label_fn),
            kind: "",
        },
    );
    let mut review = Review::new(format!("Update change {target}"));
    review.changes = changes;
    let (notify, hint_problems) = hint_lines(&state, "changes", opts.hints_path.as_deref())?;
    review.notify = notify;
    review.messages = messages(&state);
    review.dropped = dropped(&state)?;
    review.notes = notes(&state)?;
    review.attachments = attachment_rows(&files, &planning);
    review.risks = risks.clone();
    review.errors = problems.clone();
    review.warnings = warnings.into_iter().chain(hint_problems).collect();
    review.metadata = meta.as_of();
    let any_files = files.values().any(|v| !v.is_empty());
    let mut plan = Plan {
        action: "update".into(),
        meta: meta.clone(),
        state,
        draft: draft.clone(),
        payload: Some(body),
        files,
        planning,
        review,
        errors: problems,
        requester: None,
        target: Some(target),
        hash: String::new(),
        risks,
    };
    let body = plan.payload.clone().unwrap_or_default();
    if plan.errors.is_empty() && (!body.is_empty() || any_files || !plan.planning.is_empty()) {
        plan.hash = hash(
            &Json::Object(body),
            &plan.files,
            &plan.planning,
            Some(target),
        )?;
        plan.review.plan_hash = plan.hash.clone();
    }
    Ok(plan)
}

// ---------------------------------------------------------------------------
// executing plans

/// `{"step", "ok"}` of each executed step, as the journal keeps them.
fn journal_steps(steps: &[Json]) -> Vec<PlanStep> {
    steps
        .iter()
        .map(|x| PlanStep {
            step: x
                .get("step")
                .and_then(Json::as_str)
                .unwrap_or("")
                .to_string(),
            ok: x.get("ok").is_some_and(truthy),
        })
        .collect()
}

pub fn check_duplicate(plan: &Plan, allow_duplicate: bool, journal: &dyn Journal) -> Result<()> {
    if plan.action != "create" || allow_duplicate {
        return Ok(());
    }
    let Some(hit) = journal.recent_plan(&plan.hash, "create", DUPLICATE_WINDOW_HOURS) else {
        return Ok(());
    };
    let ts = fsv_py::py_str(hit.get("ts").unwrap_or(&Json::Null));
    let what = if hit.get("event").and_then(Json::as_str) == Some("done") {
        format!(
            "was already sent at {ts} (created {})",
            fsv_py::py_str(hit.get("created_id").unwrap_or(&Json::Null))
        )
    } else {
        format!("was started at {ts} with no recorded outcome (it may have been created)")
    };
    Err(flow_exit(
        4,
        format!(
            "this exact plan {what}. Check `fsv journal tail`; pass --allow-duplicate to send it again."
        ),
    ))
}

pub const ATTACH_FLOWS: [&str; 2] = ["after-create", "before-create"];

/// [`resolve_attach_flow`] with the config given.
pub fn resolve_attach_flow_in(option: Option<&str>, config: &Map<String, Json>) -> Result<String> {
    let configured = config
        .get("changes")
        .filter(|v| truthy(v))
        .and_then(|c| c.get("attach_flow"))
        .filter(|v| truthy(v))
        .map(fsv_py::py_str)
        .unwrap_or_default();
    let value = option
        .filter(|o| !o.is_empty())
        .map(str::to_string)
        .or_else(|| (!configured.is_empty()).then_some(configured))
        .unwrap_or_else(|| ATTACH_FLOWS[0].to_string());
    if !ATTACH_FLOWS.contains(&value.as_str()) {
        return Err(flow_exit(
            2,
            format!(
                "attach flow {}: expected one of {}",
                fsv_py::py_repr_str(&value),
                ATTACH_FLOWS.join(", ")
            ),
        ));
    }
    Ok(value)
}

/// Attachment flow for `changes create`: the option, else config
/// `changes.attach_flow`, else `after-create`.
///
/// * `after-create` (default): create the change, upload each file, then
///   PUT `{"attachments": [ids]}` on the new change.
/// * `before-create`: upload each file first (still only after you
///   confirm), then include `attachments: [ids]` in the create body.
///
/// Neither order is verified against what the web UI sends on create.
pub fn resolve_attach_flow(option: Option<&str>) -> Result<String> {
    if let Some(o) = option.filter(|o| !o.is_empty()) {
        return resolve_attach_flow_in(Some(o), &Map::new());
    }
    resolve_attach_flow_in(None, &fsv_core::config::load_config())
}

/// One planning-field write (`create.update_planning_field`).
pub struct PlanningUpdate<'a> {
    pub field_id: &'a str,
    pub description: Option<&'a str>,
    pub file_paths: &'a [String],
    pub duplicate: &'a str,
    pub label: &'a str,
}

/// The `fsv.create` write calls a plan makes.
pub trait ChangeWriter {
    /// Upload one file; the attachment id.
    fn upload_file(&self, path: &str) -> Result<Json>;
    /// `POST /api/_/changes`; the created change.
    fn post_change_body(&self, body: &Map<String, Json>) -> Result<Json>;
    fn put_change_body(&self, change_id: &Json, body: &Map<String, Json>) -> Result<Json>;
    fn attach_files_to_change(
        &self,
        change_id: &Json,
        paths: &[String],
        duplicate: &str,
    ) -> Result<()>;
    fn update_planning_field(&self, change_id: &Json, req: &PlanningUpdate<'_>) -> Result<()>;
}

fn basename(p: &str) -> String {
    p.rsplit('/').next().unwrap_or(p).to_string()
}

fn upload_all(
    paths: &[String],
    writer: &dyn ChangeWriter,
    uploaded: &mut Vec<Json>,
) -> Result<Vec<Json>> {
    let mut ids = Vec::new();
    for p in paths {
        let id = writer.upload_file(p)?;
        ids.push(id.clone());
        uploaded.push(json!({"id": id, "name": basename(p)}));
    }
    Ok(ids)
}

/// The flow's resolver as the form state's lookup resolver.
pub(crate) fn lookup(r: &Option<Arc<dyn Resolver>>) -> Option<Arc<dyn fsv_forms::LookupResolver>> {
    r.clone().map(|r| r as Arc<dyn fsv_forms::LookupResolver>)
}

/// Send the plan. Fails with [`ExecError::Partial`] when the change was
/// written but a later step (upload, planning) failed.
pub fn execute(
    plan: &Plan,
    writer: &dyn ChangeWriter,
    journal: &dyn Journal,
    duplicate: &str,
    attach_flow: Option<&str>,
) -> std::result::Result<Json, ExecError> {
    if !plan.ok() {
        return Err(flow_exit(2, format!("plan has problems: {}", plan.errors.join("; "))).into());
    }
    let mut steps: Vec<Json> = Vec::new();
    let mut uploaded: Vec<Json> = Vec::new();
    let paths = plan.files.get("attachments").cloned().unwrap_or_default();
    let create = plan.action == "create";
    let early = create && !paths.is_empty() && resolve_attach_flow(attach_flow)? == "before-create";
    let mut body = plan.payload.clone().unwrap_or_default();
    let target = plan.target.map(|t| json!(t));
    let tref = target.as_ref();
    let event = |created_id, steps, error| PlanEvent {
        target: tref,
        created_id,
        steps,
        error,
    };
    let before = journal.read().len();
    let written = journal.try_record_event(
        "started",
        &plan.hash,
        &plan.action,
        event(None, None, false),
    );
    let entries = journal.read();
    let last = entries.last();
    if written.is_err()
        || entries.len() <= before
        || last.and_then(|r| r.get("plan")).and_then(Json::as_str) != Some(&plan.hash)
        || last.and_then(|r| r.get("event")).and_then(Json::as_str) != Some("started")
    {
        return Err(flow_exit(5, "could not persist the change journal; nothing was sent").into());
    }
    let mut sending = false;
    let first = (|| -> Result<(Json, Json)> {
        if early {
            let ids = upload_all(&paths, writer, &mut uploaded)?;
            body.insert("attachments".into(), Json::Array(ids));
        }
        sending = true;
        if create {
            let result = writer.post_change_body(&body)?;
            let cid = result.get("id").cloned().unwrap_or(Json::Null);
            Ok((cid, result))
        } else {
            let cid = target.clone().unwrap_or(Json::Null);
            let payload = plan.payload.clone().unwrap_or_default();
            let result = if payload.is_empty() {
                json!({"id": cid})
            } else {
                writer.put_change_body(&cid, &payload)?
            };
            Ok((cid, result))
        }
    })();
    let (cid, result) = match first {
        Ok(v) => v,
        Err(e) => {
            // Once the change body may have gone out, only a definite refusal
            // is `failed`; anything else leaves `started` so a resend needs
            // review.
            let rejected = if sending {
                fsv_api::service::definite_rejection(&e).is_some()
            } else {
                matches!(e, Error::Api(_) | Error::Other(_) | Error::Session(_))
            };
            if rejected {
                journal.record_event("failed", &plan.hash, &plan.action, event(None, None, true));
            }
            if !uploaded.is_empty() {
                let shown: Vec<String> = uploaded
                    .iter()
                    .map(|u| {
                        format!(
                            "{} ({})",
                            fsv_py::py_str(&u["id"]),
                            fsv_py::py_str(&u["name"])
                        )
                    })
                    .collect();
                eprintln!(
                    "note: files were uploaded before the change was rejected (unused attachment ids: {})",
                    shown.join(", ")
                );
            }
            return Err(e.into());
        }
    };
    steps.push(json!({"step": plan.action, "ok": true, "id": cid}));
    if early {
        steps.push(json!({"step": "attachments", "ok": true, "ids": body["attachments"]}));
    }
    let after = (|| -> Result<()> {
        if early {
        } else if !paths.is_empty() && create {
            let ids = upload_all(&paths, writer, &mut uploaded)?;
            let mut b = Map::new();
            b.insert("attachments".into(), Json::Array(ids.clone()));
            writer.put_change_body(&cid, &b)?;
            steps.push(json!({"step": "attachments", "ok": true, "ids": ids}));
        } else if !paths.is_empty() {
            writer.attach_files_to_change(&cid, &paths, duplicate)?;
            steps.push(json!({"step": "attachments", "ok": true, "files": paths.iter().map(|p| basename(p)).collect::<Vec<_>>()}));
        }
        for p in &plan.planning {
            let label = if p.field_label.is_empty() {
                &p.label
            } else {
                &p.field_label
            };
            writer.update_planning_field(
                &cid,
                &PlanningUpdate {
                    field_id: &p.field_id,
                    description: p.description_html.as_deref(),
                    file_paths: &p.files,
                    duplicate,
                    label,
                },
            )?;
            steps.push(json!({"step": format!("planning:{}", p.field_id), "ok": true}));
        }
        Ok(())
    })();
    if let Err(e) = after {
        let msg: String = e.to_string().chars().take(200).collect();
        steps.push(json!({"step": "after", "ok": false, "error": msg}));
        let recorded = journal_steps(&steps);
        journal.record_event(
            "done",
            &plan.hash,
            &plan.action,
            event(Some(&cid), Some(&recorded), true),
        );
        return Err(ExecError::Partial(Box::new(PartialFailure::new(
            cid, uploaded, steps, e,
        ))));
    }
    let recorded = journal_steps(&steps);
    journal.record_event(
        "done",
        &plan.hash,
        &plan.action,
        event(Some(&cid), Some(&recorded), false),
    );
    Ok(
        json!({"id": cid, "result": result, "steps": steps, "uploaded": uploaded, "plan_hash": plan.hash}),
    )
}

/// What changed between two plans (rows, payload keys).
pub fn review_diff(old: &Plan, new: &Plan) -> Vec<String> {
    let to_map =
        |rows: &[(String, String)]| -> IndexMap<String, String> { rows.iter().cloned().collect() };
    let (a, b) = (to_map(&old.review.rows), to_map(&new.review.rows));
    let mut keys: Vec<&String> = a.keys().collect();
    keys.extend(b.keys().filter(|k| !a.contains_key(*k)));
    let mut out = Vec::new();
    for k in keys {
        if a.get(k) != b.get(k) {
            let dash = "—".to_string();
            out.push(format!(
                "{k}: {} → {}",
                a.get(k).unwrap_or(&dash),
                b.get(k).unwrap_or(&dash)
            ));
        }
    }
    if out.is_empty() && old.payload != new.payload {
        let empty = Map::new();
        let (op, np) = (
            old.payload.as_ref().unwrap_or(&empty),
            new.payload.as_ref().unwrap_or(&empty),
        );
        let mut keys: Vec<&String> = op.keys().chain(np.keys()).collect();
        keys.sort();
        keys.dedup();
        out.extend(
            keys.into_iter()
                .filter(|k| op.get(*k) != np.get(*k))
                .map(|k| format!("payload {k} changed")),
        );
    }
    if old.errors != new.errors {
        out.extend(
            new.errors
                .iter()
                .filter(|e| !old.errors.contains(e))
                .map(|e| format!("problem: {e}")),
        );
    }
    out
}

/// Re-fetch the metadata. -> (plan to send, diff lines if the plan changed).
pub fn freshen(
    plan: Plan,
    source: &dyn MetaSource,
    rebuild: &mut dyn FnMut(Arc<FormMeta>) -> Result<Plan>,
) -> Result<(Plan, Option<Vec<String>>)> {
    let mode = if plan.meta.form_type == "new_form" {
        "new"
    } else {
        "edit"
    };
    let fresh = source.load_meta(&plan.meta.module, mode, true, &plan.meta.module_id)?;
    if !fresh.fingerprint().changed(&plan.meta.fingerprint()) {
        return Ok((plan, None));
    }
    let new = rebuild(Arc::new(fresh))?;
    if !new.hash.is_empty() && new.hash == plan.hash {
        return Ok((new, None));
    }
    let mut diff = review_diff(&plan, &new);
    if diff.is_empty() {
        diff.push("form metadata changed (fields or rules); plan hash differs".into());
    }
    Ok((new, Some(diff)))
}

// ---------------------------------------------------------------------------
// editor face

/// The real editor: `$VISUAL`/`$EDITOR` on a `.yaml` temp file.
pub fn editor_edit(text: &str) -> Result<Draft> {
    let opts = fsv_core::editor::EditOptions {
        suffix: ".yaml",
        comment_prefix: "#",
        max_rounds: 20,
    };
    fsv_core::editor::edit_text(text, |t| parse_template(t, None), &opts)
}

/// Open the annotated YAML draft; reopen it, regenerated with the errors at
/// the top, until the form is complete. Fails with `EditorAbort`.
pub fn edit_loop(
    build: &mut dyn FnMut(&Draft) -> Result<Plan>,
    draft: Draft,
    title: &str,
    requester_text: Option<&str>,
    edit: &mut dyn FnMut(&str) -> Result<Draft>,
    max_rounds: usize,
) -> Result<(Draft, Plan)> {
    let mut draft = draft;
    let mut plan = build(&draft)?;
    let mut errors: Vec<String> = Vec::new();
    for _ in 0..max_rounds {
        let text = template_for(&plan, &draft, title, &errors, requester_text, None)?;
        let base = parse_template(&text, None)?;
        let new = edit(&text)?;
        let answers: Map<String, Json> = new
            .answers
            .iter()
            .filter(|(k, v)| {
                draft.answers.contains_key(*k)
                    || yaml_norm(Some(v)) != yaml_norm(base.answers.get(*k))
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let mut desc = new.description_html.clone();
        if draft.description_html.is_none()
            && let (Some(d), Some(b)) = (&desc, &base.description_html)
            && fsv_core::pystr::strip(d) == fsv_core::pystr::strip(b)
        {
            desc = None;
        }
        draft = Draft {
            answers,
            requester: new.requester,
            description_html: desc,
            attachments: new.attachments,
            planning: new.planning,
            ack: new.ack,
            preset: draft.preset.clone(),
            base_dir: new.base_dir,
        };
        plan = build(&draft)?;
        if plan.ok() {
            return Ok((draft, plan));
        }
        errors = plan.errors.clone();
    }
    Err(Error::EditorAbort(
        "aborted: too many rounds with errors".into(),
    ))
}

pub fn yaml_norm(v: Option<&Json>) -> Json {
    match v {
        Some(Json::Array(items)) => {
            Json::Array(items.iter().map(|x| json!(norm_value(x))).collect())
        }
        None | Some(Json::Null) => Json::Null,
        Some(Json::String(s)) if s.is_empty() => Json::Null,
        Some(other) => json!(norm_value(other)),
    }
}

pub fn template_for(
    plan: &Plan,
    draft: &Draft,
    title: &str,
    errors: &[String],
    requester_text: Option<&str>,
    base_dir: Option<&str>,
) -> Result<String> {
    let spec = plan.state.spec.clone();
    let mut unknown = Map::new();
    let mut hidden = Map::new();
    for (k, v) in &draft.answers {
        match spec.find(k) {
            None => {
                unknown.insert(k.clone(), v.clone());
            }
            Some(f)
                if !plan.state.evaluated.is_visible(&f.api_name)
                    && !is_status(f)
                    && !is_requester(f) =>
            {
                hidden.insert(k.clone(), v.clone());
            }
            Some(_) => {}
        }
    }
    let mut req = requester_text.map(str::to_string);
    if req.is_none()
        && let Some(r) = &plan.requester
    {
        req = r
            .email
            .clone()
            .filter(|e| !e.is_empty())
            .or_else(|| (!r.display.is_empty()).then(|| r.display.clone()));
    }
    if req.is_none() {
        req = draft.requester.clone();
    }
    render_template(
        &plan.state,
        &TemplateOptions {
            title,
            errors,
            draft: Some(draft),
            requester: req.as_deref(),
            module_cli: None,
            unknown: Some(&unknown),
            hidden: Some(&hidden),
            base_dir,
        },
    )
}

pub fn save_draft(plan: &Plan, draft: &Draft, path: &str) -> Result<String> {
    let p = PathBuf::from(expanduser(path));
    let resolved = fsv_core::journal::resolve_path(&p);
    let parent = resolved
        .parent()
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/".into());
    let text = template_for(
        plan,
        draft,
        "fsv change draft (saved)",
        &[],
        None,
        Some(&parent),
    )?;
    std::fs::write(&p, text)?;
    Ok(p.to_string_lossy().into_owned())
}

/// Plain-text one-liners for logs/notifications.
pub fn plan_text(plan: &Plan) -> String {
    if plan.hash.is_empty() {
        format!("{} plan (incomplete)", plan.action)
    } else {
        format!(
            "{} plan {}",
            plan.action,
            &plan.hash[..plan.hash.len().min(16)]
        )
    }
}

pub fn expect(plan: &Plan, expected: Option<&str>) -> Result<()> {
    let Some(expected) = expected.filter(|e| !e.is_empty()) else {
        return Ok(());
    };
    if plan.hash.is_empty() {
        return Err(flow_exit(
            2,
            format!(
                "--expect-plan given but the plan is incomplete: {}",
                plan.errors.join("; ")
            ),
        ));
    }
    if !plan_matches(expected, &plan.hash) {
        return Err(flow_exit(
            4,
            format!(
                "plan changed: expected {expected}, computed {}. Review again with --dry-run.",
                plan.hash
            ),
        ));
    }
    Ok(())
}

pub fn plain(text: &str) -> String {
    fsv_core::pystr::split_ws(&fsv_forms::spec::plain_text(text)).join(" ")
}

pub fn default_resolver(
    client: Arc<dyn fsv_api::service::Api + Send + Sync>,
) -> Arc<ClientResolver> {
    Arc::new(ClientResolver::new(client))
}

#[cfg(test)]
mod tests;
