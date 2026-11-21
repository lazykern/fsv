//! Reviewed planning-field update (changes and problems): description text
//! plus file uploads (`planning_files.py`).
//!
//! `changes|problems update ID --planning FIELD [--description TEXT] [--file F ...]`
//! is one reviewed transaction of two kinds of step: one attachment upload
//! (`POST /api/_/attachments`) per new file, then the planning-field save
//! (`PUT .../planning-fields/<name>` for an existing row, `POST
//! .../planning-fields?id=<name>` for a new one) carrying the kept and the
//! uploaded attachment IDs. The plan hash binds the tenant, record, section,
//! description, local file names + sizes + sha256, the duplicate decisions,
//! the size limit and a record-state token. The journal gets one entry per
//! upload and one for the save; an unknown step is never replayed (no
//! duplicate attachments). Readback re-reads the planning inventory and
//! compares the row's attachment IDs and names and its description.
//!
//! Problems use the same captured wire on `problems/<id>/planning-fields`
//! for their three native analysis sections; the size limit then comes from
//! the problem form metadata.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::{Map, Value, json};

use fsv_core::client::Body;
use fsv_core::error::{Error, Result};
use fsv_core::journal::sha256_hex;
use fsv_py::{Dumps, ValueExt, py_str};

use crate::attachments::{LocalFile, NOT_PUBLISHED, local_files, owner_row, resolve_limit};
use crate::create::{
    Prompter, match_planning_row, planning_server_key, resolve_planning_definition,
};
use crate::planning_text::{ChangeSchema, definitions, inventory, quote, section_content};
use crate::readback::{same_text, submitted_value_matches};
use crate::reviewed_write::refused;
use crate::service::{Api, Journal, last_event, obj, plan_hash, replay_refusal, snapshot_hash};
use crate::watchers::positive_id;

pub const ACTION: &str = "change_planning_update";
pub const PROBLEM_ACTION: &str = "problem_planning_update";
pub const DUPLICATE_MODES: [&str; 4] = ["prompt", "skip", "replace", "append"];

/// `ACTIONS[module]`.
pub fn action_for(module: &str) -> &'static str {
    if module == "problems" {
        PROBLEM_ACTION
    } else {
        ACTION
    }
}

/// `FORM_FIELDS[module]`.
fn form_fields(module: &str) -> &'static str {
    if module == "problems" {
        "problem_form_fields"
    } else {
        "change_form_fields"
    }
}

/// A failed send: the error and, for a non-2xx step answer, its HTTP status
/// (`ReviewedWriteError(..., http_status=...)`).
#[derive(Debug)]
pub struct Failure {
    pub error: Error,
    pub http_status: Option<u16>,
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Failure {
            error,
            http_status: None,
        }
    }
}

impl Failure {
    /// `exc.as_dict()` for a `ReviewedWriteError`.
    pub fn as_dict(&self) -> Map<String, Value> {
        let mut out = obj(json!({"error": self.error.to_string()}));
        if let Some(status) = self.http_status {
            out.insert("http_status".into(), json!(status));
            out.insert("server_message".into(), json!(""));
        }
        out
    }
}

/// A `ReviewedWriteError` raised by the CLI driver.
pub fn refusal(msg: impl Into<String>) -> Failure {
    refused(msg).into()
}

/// `_resolve_section`.
pub fn resolve_section(module: &str, section: &str, defs: &[Value]) -> Result<Value> {
    if module == "changes" {
        return resolve_planning_definition(section, defs);
    }
    let raw = fsv_core::pystr::strip(section);
    let hits: Vec<&Value> = defs
        .iter()
        .filter(|d| {
            raw == py_str(&d["name"])
                || fsv_core::pystr::casefold(raw) == fsv_core::pystr::casefold(&py_str(&d["label"]))
        })
        .collect();
    if let [one] = hits[..] {
        return Ok(one.clone());
    }
    let known: Vec<String> = defs
        .iter()
        .map(|d| format!("{} ({})", py_str(&d["label"]), py_str(&d["name"])))
        .collect();
    Err(Error::value(format!(
        "unknown problem planning section {}; choices: {}",
        fsv_py::py_repr_str(section),
        known.join(", ")
    )))
}

/// `_att_name`.
pub fn att_name(att: &Value) -> String {
    ["name", "content_file_name"]
        .iter()
        .find_map(|k| att.get_truthy(k))
        .map(py_str)
        .unwrap_or_default()
}

/// `_att_size`: `att.get("size", att.get("content_file_size"))`.
pub fn att_size(att: &Value) -> Value {
    att.get("size")
        .or_else(|| att.get("content_file_size"))
        .cloned()
        .unwrap_or(Value::Null)
}

/// `_existing_attachments`.
fn existing_attachments(row: Option<&Value>) -> Result<Vec<Value>> {
    let Some(row) = row else {
        return Ok(Vec::new());
    };
    let value = match row.get("attachments") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) if a.iter().all(Value::is_object) => a.clone(),
        _ => {
            return Err(refused(
                "planning attachment inventory unavailable; no write attempted",
            ));
        }
    };
    let mut ids = Vec::new();
    for a in &value {
        ids.push(positive_id(a.get("id").unwrap_or(&Value::Null))?);
    }
    if ids.iter().collect::<BTreeSet<_>>().len() != ids.len() {
        return Err(refused(
            "planning attachment inventory has duplicate IDs; no write attempted",
        ));
    }
    Ok(value)
}

/// `_ask_duplicate`: the same-name decision, only in a terminal.
pub fn ask_duplicate(prompter: &dyn Prompter, name: &str, count: usize) -> Result<String> {
    if !prompter.interactive() {
        return Err(refused(format!(
            "attachment already exists: {name}; use --duplicate skip|replace|append"
        )));
    }
    let answer = prompter.ask(&format!(
        "  duplicate {name} ({count} existing). Choose [s]kip, [r]eplace, [a]ppend: "
    ));
    let answer = fsv_core::pystr::casefold(fsv_core::pystr::strip(&answer));
    Ok(match answer.as_str() {
        "r" | "replace" => "replace",
        "a" | "append" => "append",
        _ => "skip",
    }
    .into())
}

fn file_name(value: &str) -> String {
    Path::new(value)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Port of `PlanningUpdatePlan`.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanningUpdatePlan {
    pub tenant: String,
    pub target: i64,
    pub section: String,
    pub label: String,
    pub method: String,
    pub path: String,
    pub description: Option<String>,
    pub before: String,
    pub kept_ids: Vec<i64>,
    pub replaced: Vec<(i64, String)>,
    pub skipped: Vec<String>,
    pub files: Vec<LocalFile>,
    pub duplicate: String,
    pub limit: Option<i64>,
    pub limit_source: String,
    pub published_limit: Option<i64>,
    pub context_hash: String,
    pub state: Map<String, Value>,
    pub module: String,
}

impl PlanningUpdatePlan {
    pub fn action(&self) -> &'static str {
        action_for(&self.module)
    }

    pub fn noop(&self) -> bool {
        self.files.is_empty()
            && self.replaced.is_empty()
            && match &self.description {
                None => true,
                Some(d) => self.method == "PUT" && same_text(&json!(self.before), &json!(d)),
            }
    }

    pub fn hash(&self) -> String {
        plan_hash(&json!({
            "action": self.action(), "tenant": self.tenant, "target": self.target,
            "section": self.section, "method": self.method, "path": self.path,
            "description": self.description, "kept_attachment_ids": self.kept_ids,
            "replaced": self.replaced.iter().map(|(i, n)| json!([i, n])).collect::<Vec<_>>(),
            "skipped": self.skipped,
            "files": self.files.iter().map(LocalFile::review).collect::<Vec<_>>(),
            "duplicate": self.duplicate, "max_bytes": self.limit,
            "limit_source": self.limit_source, "published_bytes": self.published_limit,
            "context": self.context_hash, "record_state": self.state,
        }))
    }

    /// `payload(uploaded)`: the planning save body.
    pub fn payload(&self, uploaded: &[i64]) -> Value {
        let mut body = Map::new();
        if let Some(d) = &self.description {
            body.insert("description".into(), json!(d));
        }
        if self.method == "POST" || !self.files.is_empty() || !self.replaced.is_empty() {
            let ids: Vec<i64> = self.kept_ids.iter().chain(uploaded).copied().collect();
            if !ids.is_empty() || self.method == "PUT" {
                body.insert("attachments".into(), json!(ids));
            }
        }
        Value::Object(body)
    }

    pub fn review(&self) -> Map<String, Value> {
        let mut steps: Vec<Value> = self
            .files
            .iter()
            .map(|f| json!({"step": "upload", "method": "POST", "path": "/api/_/attachments", "file": f.name}))
            .collect();
        steps.push(json!({"step": "planning", "method": self.method,
                          "path": format!("/api/_/{}", self.path)}));
        let after = self
            .description
            .clone()
            .unwrap_or_else(|| self.before.clone());
        let mut out = obj(json!({
            "action": self.action(), "target": self.target, "section": self.section,
            "label": self.label, "steps": steps,
            "description": {"before": self.before, "after": after,
                            "sent": self.description.is_some()},
            "files": self.files.iter().map(LocalFile::review).collect::<Vec<_>>(),
            "duplicate": self.duplicate, "kept_attachment_ids": self.kept_ids,
            "replaced_attachments": self.replaced.iter().map(|(i, n)| json!({"id": i, "name": n})).collect::<Vec<_>>(),
            "skipped_files": self.skipped, "record_state": self.state,
            "plan_hash": self.hash(),
            "warning": "Uploads and the planning save are not atomic. A failed or unknown step may leave orphan uploads; inspect the change and journal, never replay the transaction.",
        }));
        if !self.files.is_empty() {
            let published = self
                .published_limit
                .map_or_else(|| json!(NOT_PUBLISHED), |n| json!(n));
            let mut limit = obj(json!({
                "max_bytes": self.limit, "source": self.limit_source,
                "published_bytes": published,
                "total_bytes": self.files.iter().map(|f| f.size).sum::<i64>(),
                "applies_to": "total of all files in this submit",
            }));
            if self.limit.is_none() {
                limit.insert(
                    "note".into(),
                    json!("the tenant publishes no attachment size limit; pass --max-bytes N to set one before sending"),
                );
            }
            out.insert("attachment_size_limit".into(), Value::Object(limit));
        }
        out
    }
}

/// The `build_plan` keyword arguments.
pub struct PlanOpts<'a> {
    pub duplicate: &'a str,
    pub backup_replaced: bool,
    pub max_bytes: Option<i64>,
    pub module: &'a str,
}

/// Port of `build_plan`.
#[allow(clippy::too_many_arguments)]
pub fn build_plan(
    c: &dyn Api,
    journal: &dyn Journal,
    prompter: &dyn Prompter,
    schema: &ChangeSchema,
    target: i64,
    section: &str,
    description: Option<&str>,
    paths: &[String],
    opts: &PlanOpts,
) -> Result<PlanningUpdatePlan> {
    let module = opts.module;
    if module != "changes" && module != "problems" {
        return Err(Error::value(
            "planning files are supported for changes and problems",
        ));
    }
    let mode = fsv_core::pystr::casefold(opts.duplicate);
    if !DUPLICATE_MODES.contains(&mode.as_str()) {
        return Err(Error::value(
            "--duplicate must be one of: prompt, skip, replace, append",
        ));
    }
    if opts.backup_replaced {
        return Err(Error::value(format!(
            "--backup-replaced is not on the reviewed planning path; download the attachment first (fsv {module} download ID --planning SECTION), then --duplicate replace"
        )));
    }
    if description.is_some_and(|d| fsv_core::pystr::strip(d).is_empty()) {
        return Err(Error::value("--description must be nonempty text/HTML"));
    }
    if description.is_none() && paths.is_empty() {
        return Err(Error::value("--planning needs --description and/or --file"));
    }
    let cid = positive_id(&json!(target))?;
    let defs = definitions(c, module, schema)?;
    let definition = resolve_section(module, section, &defs)?;
    let rows = inventory(c, module, cid)?;
    let existing = match_planning_row(&definition, &rows)?;
    let key = existing
        .as_ref()
        .and_then(|e| e.get_truthy("name"))
        .map_or_else(|| planning_server_key(&definition), py_str);
    let attachments = existing_attachments(existing.as_ref())?;
    let content = section_content(existing.as_ref());
    let before = if existing.is_some() {
        content.clone().unwrap_or_default()
    } else {
        String::new()
    };
    if let Some(e) = &existing
        && content.is_none()
        && [
            "description",
            "content",
            "description_html",
            "description_text",
            "body",
        ]
        .iter()
        .any(|k| e.get(*k).is_some())
    {
        return Err(refused(
            "planning text state is unavailable; no write attempted",
        ));
    }
    let mut kept: Vec<i64> = Vec::new();
    for a in &attachments {
        kept.push(positive_id(a.get("id").unwrap_or(&Value::Null))?);
    }
    let mut replaced: Vec<(i64, String)> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut upload: Vec<String> = Vec::new();
    for value in paths {
        let name = file_name(value);
        let matches: Vec<&Value> = attachments.iter().filter(|a| att_name(a) == name).collect();
        let mut action = mode.clone();
        if !matches.is_empty() && action == "prompt" {
            action = ask_duplicate(prompter, &name, matches.len())?;
        }
        if !matches.is_empty() && action == "skip" {
            skipped.push(name);
            continue;
        }
        if !matches.is_empty() && action == "replace" {
            for a in &matches {
                let ident = positive_id(a.get("id").unwrap_or(&Value::Null))?;
                if let Some(pos) = kept.iter().position(|k| *k == ident) {
                    kept.remove(pos);
                    replaced.push((ident, name.clone()));
                }
            }
        }
        upload.push(value.clone());
    }
    let mut description = description.map(str::to_string);
    if description.is_none() && !upload.is_empty() && fsv_core::pystr::strip(&before).is_empty() {
        // the server needs text with files
        description = Some(
            paths
                .iter()
                .map(|p| file_name(p))
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    let (mut limit_value, mut source, mut published) = (None, NOT_PUBLISHED.to_string(), None);
    let mut files: Vec<LocalFile> = Vec::new();
    if !upload.is_empty() {
        let meta = c.int_get(form_fields(module), None)?;
        let limit = resolve_limit(c, &meta, opts.max_bytes)?;
        (limit_value, source, published) = (limit.max_bytes, limit.source, limit.published);
        files = local_files(&upload, limit_value)?;
    } else if opts.max_bytes.is_some_and(|n| n <= 0) {
        return Err(refused(
            "--max-bytes must be a positive whole number of bytes",
        ));
    }
    if existing.is_none() && description.is_none() {
        return Err(refused(
            "a new planning section needs --description text; no write attempted",
        ));
    }
    let method = if existing.is_some() { "PUT" } else { "POST" };
    let base = format!("{module}/{cid}/planning-fields");
    let path = if method == "PUT" {
        format!("{base}/{}", quote(&key))
    } else {
        format!("{base}?id={}", quote(&key))
    };
    let owner = owner_row(c, module, cid)?;
    let state = journal.repeat_token(
        &[action_for(module)],
        &json!(cid),
        Some(&Value::Object(owner)),
    );
    let label = definition
        .get_truthy("label")
        .map_or_else(|| section.to_string(), py_str);
    Ok(PlanningUpdatePlan {
        tenant: c.domain(),
        target: cid,
        section: key,
        label,
        method: method.into(),
        path,
        description,
        before,
        kept_ids: kept,
        replaced,
        skipped,
        files,
        duplicate: mode,
        limit: limit_value,
        limit_source: source,
        published_limit: published,
        context_hash: snapshot_hash(&json!({"definitions": defs, "rows": rows}))?,
        state,
        module: module.to_string(),
    })
}

/// Port of `readback`.
pub fn readback(
    c: &dyn Api,
    schema: &ChangeSchema,
    plan: &PlanningUpdatePlan,
    uploaded: &[i64],
) -> Result<Map<String, Value>> {
    let defs = definitions(c, &plan.module, schema)?;
    let definition = defs
        .iter()
        .find(|d| planning_server_key(d) == plan.section)
        .cloned()
        .unwrap_or_else(|| json!({"name": plan.section}));
    let rows = inventory(c, &plan.module, plan.target)?;
    let Some(row) = match_planning_row(&definition, &rows)? else {
        return Ok(obj(json!({
            "section": plan.section, "attachments": "unavailable",
            "description": "unavailable", "verified": false,
        })));
    };
    let mut observed: Vec<(i64, Value)> = Vec::new();
    for a in existing_attachments(Some(&row))? {
        observed.push((positive_id(a.get("id").unwrap_or(&Value::Null))?, a));
    }
    let expected: BTreeSet<i64> = plan.kept_ids.iter().chain(uploaded).copied().collect();
    let seen: BTreeSet<i64> = observed.iter().map(|(i, _)| *i).collect();
    let find = |id: i64| {
        observed
            .iter()
            .rev()
            .find(|(i, _)| *i == id)
            .map(|(_, a)| a)
    };
    let files_ok = seen == expected
        && uploaded.iter().zip(&plan.files).all(|(i, f)| {
            find(*i).is_some_and(|a| {
                att_name(a) == f.name
                    && (att_size(a).is_null()
                        || submitted_value_matches(&att_size(a), &json!(f.size)))
            })
        });
    let content = section_content(Some(&row));
    let desc = match (&plan.description, &content) {
        (None, _) => "not sent",
        (Some(_), None) => "unavailable",
        (Some(d), Some(got)) if same_text(&json!(got), &json!(d)) => "matched",
        _ => "mismatched",
    };
    let mut sorted = observed.clone();
    sorted.sort_by_key(|(i, _)| *i);
    sorted.dedup_by_key(|(i, _)| *i);
    Ok(obj(json!({
        "section": plan.section,
        "attachments": sorted.iter().map(|(i, a)| json!({"id": i, "name": att_name(a)})).collect::<Vec<_>>(),
        "attachments_verified": files_ok, "description": desc,
        "verified": files_ok && matches!(desc, "matched" | "not sent"),
    })))
}

/// Port of `upload_files`: each reviewed file is uploaded once
/// (`POST /api/_/attachments`) after its bytes are checked against the
/// review; every returned ID is journaled before the next step. `uploaded`
/// collects the IDs (also on failure).
#[allow(clippy::too_many_arguments)]
pub fn upload_files(
    c: &dyn Api,
    journal: &dyn Journal,
    csrf: &str,
    files: &[LocalFile],
    uploaded: &mut Vec<i64>,
    hash: &str,
    action: &str,
    target: i64,
) -> Result<()> {
    let upload_action = format!("{action}_upload");
    for file in files {
        let raw = std::fs::read(&file.path).map_err(|e| Error::other(e.to_string()))?;
        if raw.len() as i64 != file.size || sha256_hex(&raw) != file.sha256 {
            return Err(refused("attachment changed after review"));
        }
        let body = json!({
            "content": crate::create::data_uri(&file.mime, &raw),
            "content_file_name": file.name,
            "content_file_size": file.size,
            "content_content_type": file.mime,
        });
        let response = c.send_once(
            "POST",
            "attachments",
            Body::Raw(Dumps::default().dump(&body).into_bytes()),
            None,
            csrf,
        )?;
        let ident = positive_id(match &response {
            Value::Object(m) => m.get("id").unwrap_or(&Value::Null),
            _ => &Value::Null,
        })?;
        if uploaded.contains(&ident) {
            return Err(refused("upload returned duplicate attachment identity"));
        }
        uploaded.push(ident);
        journal.record_plan(
            "uploaded",
            hash,
            &upload_action,
            Some(&json!(target)),
            Some(&json!(ident)),
        );
        let persisted = journal.read().into_iter().rfind(|e| {
            e.get("kind") == Some(&json!(format!("plan:{upload_action}")))
                && e.get("plan") == Some(&json!(hash))
        });
        if persisted.is_none_or(|e| e.get("created_id") != Some(&json!(ident))) {
            return Err(refused("could not persist uploaded attachment identity"));
        }
    }
    Ok(())
}

/// Port of `send`.
pub fn send(
    plan: &PlanningUpdatePlan,
    c: &dyn Api,
    journal: &dyn Journal,
    schema: &ChangeSchema,
    reviewed_hash: Option<&str>,
    rebuild: &dyn Fn() -> Result<PlanningUpdatePlan>,
) -> std::result::Result<Value, Failure> {
    let Some(reviewed) = reviewed_hash else {
        return Ok(json!({"status": "cancelled", "verified": false}));
    };
    let hash = plan.hash();
    if plan.tenant.is_empty() || plan.tenant != c.domain() || reviewed != hash {
        return Err(refused("tenant or full planning review hash changed; review again").into());
    }
    if !plan.files.is_empty() && plan.limit.is_none() {
        return Err(refused(
            "the tenant publishes no attachment size limit; review again with --max-bytes N; no upload attempted",
        )
        .into());
    }
    let action = plan.action();
    if let Some(refusal) = replay_refusal(
        journal,
        action,
        &hash,
        "planning transaction was sent or outcome unknown; do not replay",
    ) {
        return Err(refused(refusal).into());
    }
    let fresh = rebuild()?;
    let paths: Vec<String> = plan.files.iter().map(|f| f.path.clone()).collect();
    if fresh.hash() != hash
        || (!plan.files.is_empty() && local_files(&paths, plan.limit)? != plan.files)
    {
        return Err(refused(format!(
            "{}, planning section or file bytes changed since review; review again",
            plan.module.trim_end_matches('s')
        ))
        .into());
    }
    let Ok(csrf) = c.get_csrf() else {
        return Err(refused("could not prepare session; no write attempted").into());
    };
    journal.record_plan("started", &hash, action, Some(&json!(plan.target)), None);
    if last_event(journal, action, &hash).as_deref() != Some("started") {
        return Err(refused("could not persist planning journal; no write attempted").into());
    }
    let mut uploaded: Vec<i64> = Vec::new();
    let mut step = "upload";
    let transact = |uploaded: &mut Vec<i64>, step: &mut &str| -> Result<()> {
        upload_files(
            c,
            journal,
            &csrf,
            &plan.files,
            uploaded,
            &hash,
            action,
            plan.target,
        )?;
        *step = "planning";
        let body = Dumps::default().dump(&plan.payload(uploaded)).into_bytes();
        c.send_once(&plan.method, &plan.path, Body::Raw(body), None, &csrf)?;
        Ok(())
    };
    if let Err(exc) = transact(&mut uploaded, &mut step) {
        let status = exc.status().filter(|s| *s != 0);
        let detail = status.map_or_else(String::new, |s| format!(" (HTTP {s})"));
        return Err(Failure {
            error: refused(format!(
                "planning transaction failed at the {step} step{detail} or outcome unknown; uploaded IDs: {}; inspect the {} and journal, do not replay",
                Dumps::default().dump(&json!(uploaded)),
                plan.module.trim_end_matches('s')
            )),
            http_status: status,
        });
    }
    journal.record_plan("done", &hash, action, Some(&json!(plan.target)), None);
    let mut result = obj(json!({
        "status": "submitted", "plan_hash": hash, "uploaded_ids": uploaded, "verified": false,
    }));
    match readback(c, schema, plan, &uploaded) {
        Ok(extra) => result.extend(extra),
        Err(_) => {
            result.insert(
                "warning".into(),
                json!("Independent planning readback failed; do not resend."),
            );
        }
    }
    if result.get("verified") == Some(&Value::Bool(true)) {
        journal.record_verified(&hash, action, Some(&json!(plan.target)));
    } else if !result.contains_key("warning") {
        result.insert(
            "warning".into(),
            json!("Planning readback was not fully verified; do not resend."),
        );
    }
    Ok(Value::Object(result))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use base64::Engine as _;
    use fsv_core::ApiError;

    use super::*;
    use crate::service::testing::{Call, FnApi, MemJournal};

    const UPLOAD_BASE: i64 = 701;

    struct NoTerminal;
    impl Prompter for NoTerminal {
        fn interactive(&self) -> bool {
            false
        }
        fn ask(&self, _: &str) -> String {
            String::new()
        }
    }

    #[derive(Default)]
    struct Tenant {
        rows: Vec<Value>,
        limit: Option<i64>,
        uploads: Vec<(i64, Value)>,
        writes: Vec<(String, String, Value)>,
        fail: Option<(&'static str, Error)>,
        updated_at: String,
    }

    type Shared = Arc<Mutex<Tenant>>;

    /// `tests/test_change_planning_reviewed.py` FakeClient: one change, one
    /// planning field, attachment uploads.
    fn tenant(module: &'static str) -> (Shared, FnApi) {
        let key = if module == "changes" {
            "change_planning_fields"
        } else {
            "problem_planning_fields"
        };
        let rows = if module == "changes" {
            vec![
                json!({"name": "acme_plan", "description": "<div>Acme rollout</div>",
                        "attachments": [{"id": 501, "name": "old.txt", "size": 9}]}),
            ]
        } else {
            vec![
                json!({"name": "problem_cause", "description": "<div>Acme cause</div>",
                        "attachments": [{"id": 501, "name": "old.txt", "size": 9}]}),
            ]
        };
        let state: Shared = Arc::new(Mutex::new(Tenant {
            rows,
            limit: Some(4096),
            updated_at: "2026-01-01T00:00:00Z".into(),
            ..Tenant::default()
        }));
        let shared = state.clone();
        let api = FnApi::new(move |call: &Call| {
            let mut s = shared.lock().unwrap();
            if call.method == "GET" {
                let path = call.path.as_str();
                if path == "change_form_fields" || path == "problem_form_fields" {
                    return Ok(match s.limit {
                        None => json!({}),
                        Some(n) => json!({"attachment_settings": {"max_attachment_size_bytes": n}}),
                    });
                }
                if path == format!("{module}/123/planning-fields") {
                    return Ok(json!({key: s.rows.clone()}));
                }
                if path == format!("{module}/123") {
                    let item = module.trim_end_matches('s');
                    return Ok(json!({item: {"id": 123, "updated_at": s.updated_at}}));
                }
                panic!("{path}");
            }
            let body = call.body.clone().unwrap();
            s.writes.push((call.method.clone(), call.path.clone(), body.clone()));
            let step = if call.path == "attachments" { "upload" } else { "planning" };
            if s.fail.as_ref().is_some_and(|(st, _)| *st == step) {
                let (_, e) = s.fail.take().unwrap();
                s.fail = Some((step, Error::other("kept")));
                return Err(e);
            }
            if step == "upload" {
                let ident = UPLOAD_BASE + s.uploads.len() as i64;
                let raw = body["content"].as_str().unwrap().split_once(',').unwrap().1.to_string();
                let data = base64::engine::general_purpose::STANDARD.decode(raw).unwrap();
                let row = json!({"id": ident, "name": body["content_file_name"], "size": data.len().to_string()});
                s.uploads.push((ident, row.clone()));
                return Ok(row);
            }
            let name = if call.method == "POST" {
                call.path.split_once("?id=").unwrap().1.to_string()
            } else {
                call.path.rsplit('/').next().unwrap().to_string()
            };
            let old = s.rows.iter().find(|r| r["name"] == json!(name)).cloned();
            let mut known: Vec<Value> = old
                .as_ref()
                .and_then(|o| o["attachments"].as_array().cloned())
                .unwrap_or_default();
            known.extend(s.uploads.iter().map(|(_, r)| r.clone()));
            let want: Vec<Value> = match body.get("attachments") {
                Some(ids) => ids
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|i| known.iter().find(|k| k["id"] == *i).cloned().unwrap())
                    .collect(),
                None => old.as_ref().and_then(|o| o["attachments"].as_array().cloned()).unwrap_or_default(),
            };
            let row = json!({
                "name": name,
                "description": body.get("description").cloned()
                    .or_else(|| old.as_ref().map(|o| o["description"].clone())).unwrap_or(Value::Null),
                "attachments": want,
            });
            s.rows.retain(|r| r["name"] != json!(name));
            s.rows.push(row.clone());
            s.updated_at = "2026-01-01T00:05:00Z".into();
            Ok(json!({"planning_field": row}))
        })
        .with_fetch(|_| {
            Ok(fsv_core::client::Response::new(200)
                .with_body(b"<html><script>var x = 1;</script></html>".to_vec()))
        });
        (state, api)
    }

    fn schema(_: &dyn Api) -> Result<Value> {
        Ok(json!({"fields": [
            {"name": "acme_plan", "label": "Rollout Plan", "field_type": "planning_field", "id": 901},
            {"name": "acme_backout", "label": "Backout Plan", "field_type": "planning_field", "id": 902}]}))
    }

    /// A file named `name` in a per-test directory (the name is the
    /// attachment identity, so tests cannot share one).
    fn file(name: &str, bytes: &[u8]) -> String {
        let test = std::thread::current().name().unwrap_or("t").replace("::", "_");
        let dir: PathBuf = std::env::temp_dir().join(format!("fsv_planning_files_{}_{test}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        p.to_string_lossy().into_owned()
    }

    #[allow(clippy::too_many_arguments)]
    fn plan(
        c: &FnApi,
        j: &MemJournal,
        module: &str,
        section: &str,
        description: Option<&str>,
        paths: &[String],
        duplicate: &str,
        max_bytes: Option<i64>,
    ) -> Result<PlanningUpdatePlan> {
        build_plan(
            c,
            j,
            &NoTerminal,
            &schema,
            123,
            section,
            description,
            paths,
            &PlanOpts {
                duplicate,
                backup_replaced: false,
                max_bytes,
                module,
            },
        )
    }

    fn rebuilt<'a>(
        c: &'a FnApi,
        j: &'a MemJournal,
        paths: &'a [String],
        desc: Option<&'a str>,
    ) -> impl Fn() -> Result<PlanningUpdatePlan> + 'a {
        rebuilt_with(c, j, paths, desc, "prompt")
    }

    fn rebuilt_with<'a>(
        c: &'a FnApi,
        j: &'a MemJournal,
        paths: &'a [String],
        desc: Option<&'a str>,
        duplicate: &'a str,
    ) -> impl Fn() -> Result<PlanningUpdatePlan> + 'a {
        move || {
            plan(
                c,
                j,
                "changes",
                "Rollout Plan",
                desc,
                paths,
                duplicate,
                None,
            )
        }
    }

    #[test]
    fn dry_run_hash_is_over_file_bytes_and_sends_nothing() {
        let (s, c) = tenant("changes");
        let j = MemJournal::default();
        let f = file("plan_hash.txt", b"Acme plan v2\n");
        let p = plan(
            &c,
            &j,
            "changes",
            "Rollout Plan",
            Some("<div>Acme v2</div>"),
            std::slice::from_ref(&f),
            "prompt",
            None,
        )
        .unwrap();
        let review = p.review();
        assert_eq!(review["section"], json!("acme_plan"));
        assert_eq!(
            review["files"],
            json!([{"name": "plan_hash.txt", "size": 13, "content_type": "text/plain",
                    "sha256": fsv_core::journal::sha256_hex(b"Acme plan v2\n")}])
        );
        let steps: Vec<&str> = review["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["step"].as_str().unwrap())
            .collect();
        assert_eq!(steps, ["upload", "planning"]);
        assert_eq!(review["attachment_size_limit"]["max_bytes"], json!(4096));
        assert_eq!(review["record_state"]["sends"], json!(0));
        assert!(s.lock().unwrap().writes.is_empty());
        std::fs::write(&f, b"Acme plan v3\n").unwrap();
        let again = plan(
            &c,
            &j,
            "changes",
            "Rollout Plan",
            Some("<div>Acme v2</div>"),
            &[f],
            "prompt",
            None,
        )
        .unwrap();
        assert_ne!(again.hash(), p.hash());
    }

    #[test]
    fn reviewed_send_uploads_then_saves_with_journal_and_readback() {
        let (s, c) = tenant("changes");
        let j = MemJournal::default();
        let paths = vec![file("plan_send.txt", b"Acme plan v2\n")];
        let desc = Some("<div>Acme v2</div>");
        let p = plan(
            &c,
            &j,
            "changes",
            "Rollout Plan",
            desc,
            &paths,
            "prompt",
            None,
        )
        .unwrap();
        let build = rebuilt(&c, &j, &paths, desc);
        let out = send(&p, &c, &j, &schema, Some(&p.hash()), &build).unwrap();
        assert_eq!(out["verified"], json!(true), "{out}");
        assert_eq!(out["description"], json!("matched"));
        assert_eq!(out["uploaded_ids"], json!([UPLOAD_BASE]));
        assert_eq!(
            out["attachments"],
            json!([{"id": 501, "name": "old.txt"}, {"id": UPLOAD_BASE, "name": "plan_send.txt"}])
        );
        let w = s.lock().unwrap().writes.clone();
        assert_eq!(
            w.iter()
                .map(|x| (x.0.as_str(), x.1.as_str()))
                .collect::<Vec<_>>(),
            [
                ("POST", "attachments"),
                ("PUT", "changes/123/planning-fields/acme_plan")
            ]
        );
        assert_eq!(
            w[1].2,
            json!({"description": "<div>Acme v2</div>", "attachments": [501, UPLOAD_BASE]})
        );
        let rows = j.read();
        let kinds: Vec<(String, String)> = rows
            .iter()
            .filter(|e| e["plan"] == json!(p.hash()))
            .map(|e| {
                (
                    e["kind"].as_str().unwrap().to_string(),
                    e["event"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        assert!(kinds.contains(&(
            "plan:change_planning_update_upload".into(),
            "uploaded".into()
        )));
        assert_eq!(
            kinds.last().unwrap(),
            &(
                "plan:change_planning_update".to_string(),
                "done".to_string()
            )
        );
        let again = send(&p, &c, &j, &schema, Some(&p.hash()), &build).unwrap_err();
        assert_eq!(again.error.exit_code(), 2);
        assert_eq!(s.lock().unwrap().writes.len(), 2);
    }

    #[test]
    fn duplicate_skip_is_nothing_to_change() {
        let (s, c) = tenant("changes");
        let j = MemJournal::default();
        let old = file("old.txt", b"Acme old\n");
        let p = plan(
            &c,
            &j,
            "changes",
            "Rollout Plan",
            None,
            &[old],
            "skip",
            None,
        )
        .unwrap();
        assert!(p.noop());
        assert_eq!(p.skipped, ["old.txt"]);
        assert!(s.lock().unwrap().writes.is_empty());
    }

    #[test]
    fn duplicate_replace_drops_the_old_id_and_prompt_refuses_without_a_terminal() {
        let (s, c) = tenant("changes");
        let j = MemJournal::default();
        let old = file("old.txt", b"Acme old v2\n");
        let err = plan(
            &c,
            &j,
            "changes",
            "Rollout Plan",
            None,
            std::slice::from_ref(&old),
            "prompt",
            None,
        )
        .unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert!(err.to_string().contains("skip|replace|append"), "{err}");
        let p = plan(
            &c,
            &j,
            "changes",
            "Rollout Plan",
            None,
            std::slice::from_ref(&old),
            "replace",
            None,
        )
        .unwrap();
        let review = p.review();
        assert_eq!(
            review["replaced_attachments"],
            json!([{"id": 501, "name": "old.txt"}])
        );
        assert_eq!(review["kept_attachment_ids"], json!([]));
        assert_eq!(review["description"]["sent"], json!(false));
        let paths = vec![old.clone()];
        let build = rebuilt_with(&c, &j, &paths, None, "replace");
        let out = send(&p, &c, &j, &schema, Some(&p.hash()), &build).unwrap();
        assert_eq!(out["verified"], json!(true), "{out}");
        assert_eq!(
            s.lock().unwrap().writes.last().unwrap().2,
            json!({"attachments": [UPLOAD_BASE]})
        );
        let backup = PlanOpts {
            duplicate: "replace",
            backup_replaced: true,
            max_bytes: None,
            module: "changes",
        };
        let err = build_plan(
            &c,
            &j,
            &NoTerminal,
            &schema,
            123,
            "Rollout Plan",
            None,
            &paths,
            &backup,
        )
        .unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn unknown_step_is_never_replayed() {
        for (step, exc) in [
            (
                "upload",
                Error::Http {
                    kind: "ReadError".into(),
                    message: "disconnect".into(),
                },
            ),
            ("planning", ApiError::new(503, json!({})).into()),
        ] {
            let (s, c) = tenant("changes");
            let j = MemJournal::default();
            let paths = vec![file("unknown_step.txt", b"Acme plan v2\n")];
            let p = plan(
                &c,
                &j,
                "changes",
                "Rollout Plan",
                None,
                &paths,
                "prompt",
                None,
            )
            .unwrap();
            s.lock().unwrap().fail = Some((step, exc));
            let build = rebuilt(&c, &j, &paths, None);
            let err = send(&p, &c, &j, &schema, Some(&p.hash()), &build).unwrap_err();
            let text = err.error.to_string();
            assert_eq!(err.error.exit_code(), 2);
            assert!(
                text.contains("do not replay") && text.contains(&format!("{step} step")),
                "{text}"
            );
            let n = s.lock().unwrap().writes.len();
            s.lock().unwrap().fail = None;
            let again = plan(
                &c,
                &j,
                "changes",
                "Rollout Plan",
                None,
                &paths,
                "prompt",
                None,
            )
            .unwrap();
            assert_eq!(again.hash(), p.hash());
            let build = rebuilt(&c, &j, &paths, None);
            let refused = send(&again, &c, &j, &schema, Some(&again.hash()), &build).unwrap_err();
            assert_eq!(refused.error.exit_code(), 2);
            assert_eq!(s.lock().unwrap().writes.len(), n);
            let uploads = s
                .lock()
                .unwrap()
                .writes
                .iter()
                .filter(|w| w.1 == "attachments")
                .count();
            assert_eq!(uploads, 1);
        }
    }

    #[test]
    fn size_limit_override_and_unpublished_limit() {
        let (s, c) = tenant("changes");
        let j = MemJournal::default();
        let f = file("limit.txt", b"Acme plan v2\n");
        let err = plan(
            &c,
            &j,
            "changes",
            "Rollout Plan",
            None,
            std::slice::from_ref(&f),
            "prompt",
            Some(5),
        )
        .unwrap_err();
        assert!(err.to_string().contains("5-byte size limit"), "{err}");
        s.lock().unwrap().limit = None;
        let p = plan(
            &c,
            &j,
            "changes",
            "Rollout Plan",
            None,
            std::slice::from_ref(&f),
            "prompt",
            None,
        )
        .unwrap();
        let review = p.review();
        assert!(review["attachment_size_limit"]["max_bytes"].is_null());
        assert!(
            review["attachment_size_limit"]["note"]
                .as_str()
                .unwrap()
                .contains("--max-bytes")
        );
        let paths = vec![f.clone()];
        let build = rebuilt(&c, &j, &paths, None);
        let err = send(&p, &c, &j, &schema, Some(&p.hash()), &build).unwrap_err();
        assert!(
            err.error.to_string().contains("--max-bytes"),
            "{}",
            err.error
        );
        assert!(s.lock().unwrap().writes.is_empty());
        let p = plan(
            &c,
            &j,
            "changes",
            "Rollout Plan",
            None,
            &[f],
            "prompt",
            Some(100),
        )
        .unwrap();
        assert_eq!(
            p.review()["attachment_size_limit"]["source"],
            json!("--max-bytes override")
        );
    }

    #[test]
    fn new_section_posts_description_and_files() {
        let (s, c) = tenant("changes");
        let j = MemJournal::default();
        let f = file("backout.txt", b"Acme plan v2\n");
        let p = plan(
            &c,
            &j,
            "changes",
            "Backout Plan",
            None,
            std::slice::from_ref(&f),
            "prompt",
            None,
        )
        .unwrap();
        let review = p.review();
        assert_eq!(
            review["steps"].as_array().unwrap().last().unwrap(),
            &json!({"step": "planning", "method": "POST", "path": "/api/_/changes/123/planning-fields?id=acme_backout"})
        );
        assert_eq!(review["description"]["after"], json!("backout.txt"));
        let paths = vec![f];
        let build = || {
            plan(
                &c,
                &j,
                "changes",
                "Backout Plan",
                None,
                &paths,
                "prompt",
                None,
            )
        };
        let out = send(&p, &c, &j, &schema, Some(&p.hash()), &build).unwrap();
        assert_eq!(out["verified"], json!(true), "{out}");
        assert_eq!(
            s.lock().unwrap().writes.last().unwrap(),
            &(
                "POST".to_string(),
                "changes/123/planning-fields?id=acme_backout".to_string(),
                json!({"description": "backout.txt", "attachments": [UPLOAD_BASE]})
            )
        );
    }

    #[test]
    fn problem_sections_use_the_problem_form_and_the_same_wire() {
        let (_, c) = tenant("problems");
        let j = MemJournal::default();
        let f = file("cause.txt", b"Acme plan v2\n");
        let p = plan(&c, &j, "problems", "Root Cause", None, &[f], "prompt", None).unwrap();
        assert_eq!(p.action(), "problem_planning_update");
        assert_eq!(p.section, "problem_cause");
        assert_eq!(
            (p.method.as_str(), p.path.as_str()),
            ("PUT", "problems/123/planning-fields/problem_cause")
        );
        let err = plan(&c, &j, "problems", "Nope", Some("x"), &[], "prompt", None).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert!(
            err.to_string()
                .contains("unknown problem planning section 'Nope'"),
            "{err}"
        );
    }
}
