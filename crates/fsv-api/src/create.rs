//! Create and clone resources from schema-driven templates (`create.py`).
//!
//! The schema-dependent entry points take the loaded change schema (or its
//! field list) instead of reading the cache themselves. Interactive choices
//! go through a [`Prompter`] so nothing here reads stdin directly.

use std::collections::HashSet;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use base64::Engine as _;
use regex::Regex;
use serde_json::{Map, Value, json};

use fsv_core::client::Body;
use fsv_core::error::{ApiError, Error, Result};
use fsv_forms::pydt::datetime_fromisoformat;
use fsv_forms::pytext::{casefold, html_unescape, split_ws};
use fsv_forms::spec::{is_content_type, is_planning_type};
use fsv_py::{ValueExt, py_repr, py_repr_str, py_str, truthy};

use crate::service::{Api, Journal, obj, py_int, rows_at, unwrap_key};

/// Embedded read models the clone flow copies separately (or not at all).
pub const CLONE_SKIP_FIELDS: &[&str] = &["stats", "planning_field_definitions", "main_attachments", "attachments"];

pub const READ_ONLY_FIELDS: &[&str] = &[
    "id", "display_id", "human_display_id", "change_id", "workspace_id", "import_id", "project_id", "wf_event_id",
    "created_at", "updated_at", "closed_at", "due_by", "nr_due_by", "first_response_time", "ola_stopped_at",
    "status_name", "priority_name", "impact_name", "risk_name", "approval_status", "approval_type", "summary_status",
    "resolution_time_taken", "total_time_spent", "timer_active_agents", "task_due_status", "is_editable",
    "status_ola_timer", "state_flow_id", "state_traversal", "cab_meeting_ids", "trashed", "deleted", "is_escalated",
    "escalation_level", "manual_dueby", "planned", "requester", "department", "agent", "group", "attachments",
    "cloud_files", "shared_attachment_ids", "assoc_assets", "assoc_asset_ids", "assoc_change_ids", "assoc_release_id",
    "blackout_window", "source", "business_calendar_id", "sla_policy_id", "cc_email", "email_config_id", "group_name",
    "owner_name", "description_text", "problem_id", "release_id", "associated_cis_and_services_ids", "sub_category",
    "item_category", "initiated_by", "stack_rank", "taskable_display_id", "taskable_type", "user_id",
    "force_authenticated", "tasks_dependency_type", "notify_before",
];

pub const SKIP_TYPES: &[&str] = &["custom_content", "planning_field"];

/// Embedded FK fields scaffold a `*_id` placeholder instead.
const TEMPLATE_FK_PLACEHOLDERS: &[(&str, &str, &str)] = &[
    ("requester", "requester_id", "<requester email or ID>"),
    ("agent", "agent_id", "<agent email/name or ID>"),
    ("group", "group_id", "<group name or ID>"),
    ("department", "department_id", "<department name or ID>"),
];

/// `READ_ONLY_FIELDS | {...}`: the extra names are already read-only.
pub fn is_task_read_only(key: &str) -> bool {
    READ_ONLY_FIELDS.contains(&key)
}

pub const TASK_OWNER_KEYS: &[&str] = &["owner_id", "agent_id", "responder_id", "owner"];
pub const TASK_DATE_KEYS: &[&str] = &["due_date", "planned_start_date", "planned_end_date", "start_date", "end_date"];
pub const TASK_READONLY_CF: &[&str] = &["team_name"];

const CHANGE_CORE_NAMES: &[&str] = &[
    "requester_id", "email", "group_id", "agent_id", "change_type", "priority", "impact", "risk", "status", "category",
    "subject", "description", "planned_start_date", "planned_end_date", "planned_effort", "department_id",
    "change_window_id",
];

/// `APIError(0, message)`.
fn api0(msg: impl Into<String>) -> Error {
    ApiError::new(0, Value::String(msg.into())).into()
}

fn s(v: Option<&Value>) -> String {
    v.filter(|x| truthy(x)).map(py_str).unwrap_or_default()
}

fn is_id_value_pair(v: &Value) -> bool {
    v.as_object().is_some_and(|m| m.len() == 2 && m.contains_key("id") && m.contains_key("value"))
}

/// Stdin interaction (`sys.stdin.isatty()` + `input()`), replaceable in tests.
pub trait Prompter {
    fn interactive(&self) -> bool;
    /// Write `prompt` to stderr and read one line.
    fn ask(&self, prompt: &str) -> String;
}

/// The process terminal.
pub struct StdinPrompter;

impl Prompter for StdinPrompter {
    fn interactive(&self) -> bool {
        std::io::stdin().is_terminal()
    }
    fn ask(&self, prompt: &str) -> String {
        eprint!("{prompt}");
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        line.trim_end_matches(['\n', '\r']).to_string()
    }
}

// ---------------------------------------------------------------------------
// templates and clones

/// `_build_template`: `level` is `required` | `optional` | `all`.
pub fn build_template(schema_fields: &[Value], level: &str) -> Map<String, Value> {
    let mut template = Map::new();
    for f in schema_fields {
        let Some(name) = f.get_truthy("name").map(py_str) else { continue };
        let ftype = s(f.get("field_type"));
        if SKIP_TYPES.contains(&ftype.as_str()) || is_content_type(&ftype) {
            continue;
        }
        let required = f.get("required").is_some_and(truthy);
        if let Some((_, key, placeholder)) = TEMPLATE_FK_PLACEHOLDERS.iter().find(|(n, _, _)| *n == name) {
            if level == "required" && !required {
                continue;
            }
            template.insert((*key).into(), json!(placeholder));
            continue;
        }
        if READ_ONLY_FIELDS.contains(&name.as_str()) {
            continue;
        }
        if level == "required" && !required {
            continue;
        }
        template.insert(name, field_default(f));
    }
    template
}

fn choice_example(c: &Value) -> Value {
    c.get("value").or_else(|| c.get("name")).or_else(|| c.get("id")).cloned().unwrap_or(Value::Null)
}

/// `_field_default`: a placeholder for a schema field.
pub fn field_default(f: &Value) -> Value {
    let ftype = s(f.get("field_type"));
    let choices = f.list_or_empty("choices");
    let boolish = |c: &Value| {
        c.get("id").is_some_and(Value::is_boolean)
            || c.get("id").is_some_and(|v| *v == json!(1) || *v == json!(0))
            || c.get("value").is_some_and(|v| v == "true" || v == "false")
    };
    if matches!(ftype.as_str(), "default_checkbox" | "custom_checkbox")
        || (!choices.is_empty() && choices.iter().take(2).all(boolish))
    {
        return json!(false);
    }
    if !choices.is_empty() {
        if choices.len() <= 5 {
            return Value::Array(choices.iter().map(choice_example).collect());
        }
        let examples: Vec<String> = choices.iter().take(2).map(|c| py_str(&choice_example(c))).collect();
        return json!(format!("<choose: {}, ...>", examples.join(", ")));
    }
    if ftype.ends_with("_date") {
        return json!("2026-01-01T10:00:00+07:00");
    }
    if ftype.ends_with("_number") || ftype.ends_with("_decimal") {
        return json!(0);
    }
    match ftype.as_str() {
        "default_description" => return json!("<div>description here</div>"),
        "default_subject" => return json!("subject here"),
        "default_requester" | "default_agent" => return json!("<email or name>"),
        "default_group" | "default_department" => return json!("<name or ID>"),
        _ => {}
    }
    if ftype.ends_with("_multi_select_dropdown") {
        return json!([]);
    }
    if let Some(name) = f.get_truthy("name") {
        if py_str(name).ends_with("_id") {
            return json!("<ID>");
        }
    }
    json!("")
}

/// `_get_change_schema_fields` over a loaded change schema.
pub fn change_schema_fields(schema: &Value) -> Vec<Value> {
    let fields = schema.list_or_empty("fields");
    if !fields.is_empty() {
        return fields.to_vec();
    }
    let raw = schema.get("data").unwrap_or(schema);
    if let Some(m) = raw.as_object() {
        let v = m.get("change_fields").or_else(|| m.get("fields"));
        if let Some(a) = v.filter(|v| truthy(v)).and_then(Value::as_array) {
            return a.clone();
        }
    }
    Vec::new()
}

/// `change_template`.
pub fn change_template(schema: &Value, level: &str) -> Map<String, Value> {
    build_template(&change_schema_fields(schema), level)
}

/// Flatten `custom_fields` into the record (lookups `{id, value}` become ids).
fn flatten_custom(change: &mut Map<String, Value>) {
    if let Some(cf) = change.remove("custom_fields") {
        if let Value::Object(cf) = cf {
            for (k, v) in cf {
                let v = if is_id_value_pair(&v) { v["id"].clone() } else { v };
                change.insert(k, v);
            }
        }
    }
}

/// `change_clone_data`: read-only/computed fields stripped, status reset to Open.
pub fn change_clone_data(c: &dyn Api, change_id: i64) -> Result<Map<String, Value>> {
    let data = c.int_get(&format!("changes/{change_id}"), None)?;
    let mut change = obj(unwrap_key(data, "change"));
    flatten_custom(&mut change);
    let mut out = Map::new();
    for (k, v) in change {
        if READ_ONLY_FIELDS.contains(&k.as_str()) || CLONE_SKIP_FIELDS.contains(&k.as_str()) || k.ends_with("planning_fields") {
            continue;
        }
        out.insert(k, v);
    }
    out.insert("status".into(), json!(1));
    Ok(out)
}

/// `_parse_when`: an aware instant (naive values are UTC).
pub fn parse_when(value: &Value) -> Option<jiff::Timestamp> {
    let text = value.as_str().filter(|t| !t.is_empty())?;
    let (dt, off) = datetime_fromisoformat(&text.replace('Z', "+00:00"))?;
    off.unwrap_or(jiff::tz::Offset::UTC).to_timestamp(dt).ok()
}

/// `_task_clone_body`: read-only fields and nulls stripped, status Open;
/// unless `keep_owners`, owners and past dates are dropped.
pub fn task_clone_body(task: &Value, keep_owners: bool, now: Option<jiff::Timestamp>, mut dropped: Option<&mut Vec<String>>) -> Map<String, Value> {
    let now = now.unwrap_or_else(jiff::Timestamp::now);
    let mut out = Map::new();
    let title = task.get_truthy("title").or_else(|| task.get("id")).cloned().unwrap_or(Value::Null);
    for (k, v) in task.as_object().into_iter().flatten() {
        if is_task_read_only(k) || v.is_null() {
            continue;
        }
        if !keep_owners && TASK_OWNER_KEYS.contains(&k.as_str()) {
            if let Some(d) = dropped.as_deref_mut() {
                d.push(format!("task {}: owner not copied", py_repr(&title)));
            }
            continue;
        }
        if TASK_DATE_KEYS.contains(&k.as_str()) {
            if let Some(when) = parse_when(v) {
                if when < now {
                    if let Some(d) = dropped.as_deref_mut() {
                        d.push(format!("task {}: past {k} not copied", py_repr(&title)));
                    }
                    continue;
                }
            }
        }
        out.insert(k.clone(), v.clone());
    }
    out.insert("status".into(), json!(1));
    if let Some(Value::Object(cf)) = out.get("custom_fields") {
        let cf: Map<String, Value> = cf.iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k.clone(), v.clone())).collect();
        if cf.is_empty() {
            out.remove("custom_fields");
        } else {
            out.insert("custom_fields".into(), Value::Object(cf));
        }
    }
    out
}

/// `clone_tasks`: every task of the source change created on the new one.
pub fn clone_tasks(
    c: &dyn Api,
    source_change_id: i64,
    new_change_id: i64,
    keep_owners: bool,
    now: Option<jiff::Timestamp>,
    mut dropped: Option<&mut Vec<String>>,
) -> Result<Vec<Value>> {
    let data = c.int_get(&format!("changes/{source_change_id}/tasks"), None)?;
    let mut created = Vec::new();
    for task in rows_at(&data, "tasks") {
        let body = task_clone_body(&task, keep_owners, now, dropped.as_deref_mut());
        let result = c.int_post(&format!("changes/{new_change_id}/tasks"), &json!({"change_task": body}))?;
        created.push(unwrap_key(result, "task"));
    }
    Ok(created)
}

/// `clone_side_effects`: read-only counts of what the clone flags would copy.
pub fn clone_side_effects(c: &dyn Api, source_change_id: i64) -> Result<Map<String, Value>> {
    let tasks = rows_at(&c.int_get(&format!("changes/{source_change_id}/tasks"), None)?, "tasks");
    let planning = rows_at(&c.int_get(&format!("changes/{source_change_id}/planning-fields"), None)?, "change_planning_fields");
    let with_owner = tasks.iter().filter(|t| TASK_OWNER_KEYS.iter().any(|k| t.get(*k).is_some_and(truthy))).count();
    let assets = get_change_assets(c, source_change_id)?.len();
    let filled = planning
        .iter()
        .filter(|f| !fsv_core::pystr::strip(&s(f.get("description"))).is_empty() || f.get("attachments").is_some_and(truthy))
        .count();
    let atts: usize = planning.iter().map(|f| f.list_or_empty("attachments").len()).sum();
    Ok(obj(json!({
        "tasks": tasks.len(), "tasks_with_owner": with_owner, "assets": assets,
        "planning_fields": filled, "planning_attachments": atts,
    })))
}

/// `clone_assets`: the source's assets associated on the new change.
pub fn clone_assets(c: &dyn Api, source_change_id: i64, new_change_id: i64) -> Result<Vec<i64>> {
    let display = |e: &Value| e.get_truthy("display_id").or_else(|| e.obj_or_empty("config_item").get("display_id").filter(|v| truthy(v))).cloned();
    let ids: Vec<i64> = get_change_assets(c, source_change_id)?.iter().filter_map(display).map(|v| py_int(&v).unwrap_or(0)).collect();
    if ids.is_empty() {
        return Ok(ids);
    }
    c.int_put(&format!("changes/{new_change_id}/assets/associate"), &json!({"item_ids": ids}))?;
    Ok(ids)
}

/// `clone_planning_fields`: text and re-uploaded attachments; returns names.
pub fn clone_planning_fields(c: &dyn Api, source_change_id: i64, new_change_id: i64) -> Result<Vec<String>> {
    let data = c.int_get(&format!("changes/{source_change_id}/planning-fields"), None)?;
    let mut cloned = Vec::new();
    for field in rows_at(&data, "change_planning_fields") {
        let Some(name) = field.get_truthy("name").map(py_str) else { continue };
        let description = s(field.get("description"));
        let attachments = field.list_or_empty("attachments");
        if fsv_core::pystr::strip(&description).is_empty() && attachments.is_empty() {
            continue;
        }
        let mut new_ids: Vec<Value> = Vec::new();
        for att in attachments {
            if let Some(id) = reupload_attachment(c, att)? {
                new_ids.push(id);
            }
        }
        let mut body = obj(json!({"description": description}));
        if !new_ids.is_empty() {
            body.insert("attachments".into(), Value::Array(new_ids));
        }
        c.int_post(&format!("changes/{new_change_id}/planning-fields?id={name}"), &Value::Object(body))?;
        cloned.push(name);
    }
    Ok(cloned)
}

// ---------------------------------------------------------------------------
// attachments

/// `_attachment_url`: the download URL (helpdesk links get `?download=true`).
pub fn attachment_url(att: &Value, domain: &str) -> String {
    let mut url = s(att.get_truthy("attachment_url").or_else(|| att.get("canonical_url")));
    if url.is_empty() {
        if let Some(p) = att.get_truthy("canonical_path") {
            url = format!("https://{domain}{}", py_str(p));
        }
    }
    if !url.is_empty() && !url.contains('?') && url.contains("/helpdesk/attachments/") {
        url.push_str("?download=true");
    }
    url
}

/// `_safe_filename`.
pub fn safe_filename(name: &str) -> String {
    let cleaned = name.replace('/', "-").replace('\0', "");
    let cleaned = fsv_core::pystr::strip(&cleaned);
    if cleaned.is_empty() { "attachment".into() } else { cleaned.to_string() }
}

/// The filename of a `Content-Disposition` header (`Message.get_filename`).
fn disposition_filename(cd: &str) -> Option<String> {
    let mut plain = None;
    for part in cd.split(';').skip(1) {
        let Some((k, v)) = part.split_once('=') else { continue };
        let k = k.trim().to_ascii_lowercase();
        let v = v.trim();
        if k == "filename*" {
            let raw = v.trim_matches('"');
            let encoded = raw.splitn(3, '\'').nth(2).unwrap_or(raw);
            return Some(fsv_core::pystr::unquote(encoded));
        }
        if k == "filename" {
            plain = Some(v.trim_matches('"').to_string());
        }
    }
    plain.filter(|s| !s.is_empty())
}

fn skipped_result(name: &str, path: &Path) -> Value {
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    json!({"name": name, "path": path.to_string_lossy(), "size": size, "skipped": true})
}

/// `download_attachment`: one file into `out_dir` (kept unless `force`).
pub fn download_attachment(c: &dyn Api, att: &Value, out_dir: &Path, force: bool) -> Result<Value> {
    let url = attachment_url(att, &c.domain());
    if url.is_empty() {
        return Err(api0(format!("attachment has no URL: {}", py_repr(att))));
    }
    std::fs::create_dir_all(out_dir).map_err(|e| Error::other(e.to_string()))?;
    let fallback = format!("attachment-{}", att.get("id").map(py_str).unwrap_or_default());
    let mut name = safe_filename(&att.get_truthy("name").map(py_str).unwrap_or(fallback));
    let mut path = out_dir.join(&name);
    if path.exists() && !force {
        return Ok(skipped_result(&name, &path));
    }
    eprintln!("  downloading {name}...");
    let r = c.fetch(&url)?;
    if r.status >= 400 {
        let text: String = String::from_utf8_lossy(&r.body).chars().take(500).collect();
        return Err(ApiError::new(r.status, Value::String(text)).into());
    }
    let cd = r.header("content-disposition").unwrap_or("").to_string();
    if att.get_truthy("name").is_none() && cd.contains("filename=") {
        if let Some(filename) = disposition_filename(&cd) {
            name = safe_filename(&filename);
            path = out_dir.join(&name);
            if path.exists() && !force {
                return Ok(skipped_result(&name, &path));
            }
        }
    }
    std::fs::write(&path, &r.body).map_err(|e| Error::other(e.to_string()))?;
    Ok(json!({"name": name, "path": path.to_string_lossy(), "size": r.body.len(), "skipped": false}))
}

fn now_epoch() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn data_uri(mime: &str, raw: &[u8]) -> String {
    format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(raw))
}

/// `_reupload_attachment`: download then upload again; the new id or `None`.
pub fn reupload_attachment(c: &dyn Api, att: &Value) -> Result<Option<Value>> {
    let url = attachment_url(att, &c.domain());
    if url.is_empty() {
        return Ok(None);
    }
    let r = c.fetch(&url)?;
    if r.status != 200 {
        return Ok(None);
    }
    let mime_full = att.get("content_type").map(py_str).unwrap_or_else(|| "application/octet-stream".into());
    let mime = fsv_core::pystr::strip(mime_full.split(';').next().unwrap_or("")).to_string();
    let name = att.get("name").cloned().unwrap_or_else(|| json!("attachment"));
    let body = json!({
        "content": data_uri(&mime, &r.body),
        "content_updated_at": now_epoch(),
        "content_file_name": name,
        "content_file_size": r.body.len(),
        "content_content_type": mime,
    });
    let result = c.int_post("attachments", &body)?;
    if !result.is_object() {
        return Ok(None);
    }
    Ok(result.get_truthy("id").or_else(|| result.obj_or_empty("attachment").get("id")).cloned())
}

/// Python's `f"{n:,}"`.
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// `_upload_bytes`: one attachment upload; the new id.
pub fn upload_bytes(c: &dyn Api, name: &str, raw: &[u8], mime: &str) -> Result<i64> {
    let body = json!({
        "content": data_uri(mime, raw),
        "content_file_name": name,
        "content_file_size": raw.len(),
        "content_content_type": mime,
    });
    eprint!("  uploading {name}... ");
    let result = c.int_post("attachments", &body)?;
    let att_id = result.get("id").filter(|v| result.is_object() && truthy(v)).cloned();
    let Some(att_id) = att_id else {
        return Err(api0(format!("attachment upload returned no id: {}", py_repr(&result))));
    };
    let srv_size = result.get_truthy("content_file_size").or_else(|| result.get("size")).cloned().unwrap_or(Value::Null);
    let srv_name = result.get_truthy("content_file_name").or_else(|| result.get("name")).cloned().unwrap_or(Value::Null);
    let n = raw.len();
    if srv_size == json!(n) && srv_name == json!(name) {
        eprintln!("ok ({} B)", thousands(n));
    } else if srv_name != json!(name) {
        eprintln!("ok ({} B, server name \"{}\" differs)", thousands(n), py_str(&srv_name));
    } else {
        eprintln!("mismatch (local {} B, server {} B)", thousands(n), py_str(&srv_size));
    }
    py_int(&att_id).ok_or_else(|| Error::value(format!("invalid literal for int(): {}", py_repr(&att_id))))
}

fn guess_mime(path: &str) -> Option<String> {
    mime_guess::from_path(path).first_raw().map(str::to_string)
}

/// `upload_file`: upload a local file; its attachment id.
pub fn upload_file(c: &dyn Api, path: &str) -> Result<i64> {
    let raw = std::fs::read(path).map_err(|e| Error::other(e.to_string()))?;
    let mime = guess_mime(path).unwrap_or_else(|| "application/octet-stream".into());
    let name = path.rsplit('/').next().unwrap_or(path);
    upload_bytes(c, name, &raw, &mime)
}

/// Same-name attachment handling for uploads.
#[derive(Debug, Clone, Default)]
pub struct DuplicateOpts {
    /// `prompt` | `skip` | `replace` | `append` (default `prompt`).
    pub duplicate: Option<String>,
    pub backup_replaced: Option<bool>,
    pub backup_name: Option<String>,
}

impl DuplicateOpts {
    fn mode(&self) -> Result<String> {
        let mode = casefold(self.duplicate.as_deref().unwrap_or("prompt"));
        if !["prompt", "skip", "replace", "append"].contains(&mode.as_str()) {
            return Err(api0("--duplicate must be one of: prompt, skip, replace, append"));
        }
        Ok(mode)
    }
}

fn file_name(path: &str) -> String {
    Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The shared same-name loop of `attach_files_to_change` and
/// `update_planning_field`: `(ids, skipped, changed)`.
fn merge_uploads(
    c: &dyn Api,
    attachments: &[Value],
    mut ids: Vec<i64>,
    paths: &[String],
    mode: &str,
    opts: &DuplicateOpts,
    prompter: &dyn Prompter,
) -> Result<(Vec<i64>, Vec<String>, bool)> {
    let mut changed = false;
    let mut skipped = Vec::new();
    for path in paths {
        let name = file_name(path);
        let matches: Vec<&Value> = attachments.iter().filter(|a| attachment_name(a) == name).collect();
        let mut action = mode.to_string();
        if !matches.is_empty() && action == "prompt" {
            action = prompt_duplicate_action(&name, matches.len(), prompter)?;
        }
        if !matches.is_empty() && action == "skip" {
            eprintln!("  skipped {name} (already attached)");
            skipped.push(name);
            continue;
        }
        if !matches.is_empty() && action == "replace" {
            let replace: HashSet<Option<i64>> = matches.iter().map(|a| attachment_id(a)).collect();
            ids.retain(|i| !replace.contains(&Some(*i)));
            if should_backup_replaced(&name, opts.backup_replaced, prompter) {
                for (i, att) in matches.iter().enumerate() {
                    let bname = choose_backup_name(&name, opts.backup_name.as_deref(), i, opts.backup_replaced.is_none(), prompter);
                    ids.push(backup_attachment(c, att, &bname)?);
                }
            }
        }
        ids.push(upload_file(c, path)?);
        changed = true;
    }
    Ok((ids, skipped, changed))
}

/// `attach_files_to_change`: upload files onto the change's main Attachments.
pub fn attach_files_to_change(c: &dyn Api, change_id: i64, file_paths: &[String], opts: &DuplicateOpts, prompter: &dyn Prompter) -> Result<Value> {
    let data = c.int_get(&format!("changes/{change_id}"), None)?;
    let change = unwrap_key(data, "change");
    let attachments = change.list_or_empty("attachments").to_vec();
    let ids: Vec<i64> = attachments.iter().filter_map(attachment_id).collect();
    let mode = opts.mode()?;
    let (ids, skipped, changed) = merge_uploads(c, &attachments, ids, file_paths, &mode, opts, prompter)?;
    if !changed {
        return Ok(json!({"_fsv_noop": true, "skipped": skipped, "attachments": ids}));
    }
    c.int_put(&format!("changes/{change_id}"), &json!({"attachments": ids}))?;
    Ok(json!({"attachments": ids, "skipped": skipped}))
}

/// `set_due_by`: PATCH (a separate endpoint from the PUT update).
pub fn set_due_by(c: &dyn Api, change_id: i64, due_by: &str) -> Result<Value> {
    c.int_patch(&format!("changes/{change_id}"), &json!({"due_by": due_by}))
}

/// `get_change_for_edit`: flat, read-only fields stripped.
pub fn get_change_for_edit(c: &dyn Api, change_id: i64) -> Result<Map<String, Value>> {
    let data = c.int_get(&format!("changes/{change_id}"), None)?;
    let mut change = obj(unwrap_key(data, "change"));
    flatten_custom(&mut change);
    change.retain(|k, _| !READ_ONLY_FIELDS.contains(&k.as_str()));
    Ok(change)
}

/// Core vs `custom_fields` split of `submit_change` / `update_change`.
fn split_core(body: &Map<String, Value>, schema_fields: &[Value], extra_core: &[&str]) -> Map<String, Value> {
    let mut core = Map::new();
    let mut custom = Map::new();
    for (k, v) in body {
        let is_default = schema_fields
            .iter()
            .rev()
            .find(|f| f.get("name").and_then(Value::as_str) == Some(k.as_str()))
            .is_some_and(|f| f.get("default_field").is_some_and(truthy));
        let v = if is_id_value_pair(v) { v["id"].clone() } else { v.clone() };
        if CHANGE_CORE_NAMES.contains(&k.as_str()) || extra_core.contains(&k.as_str()) || is_default {
            core.insert(k.clone(), v);
        } else {
            custom.insert(k.clone(), v);
        }
    }
    if !custom.is_empty() {
        core.insert("custom_fields".into(), Value::Object(custom));
    }
    core
}

/// `_api_failure`: a field-error summary for a rejected write.
pub fn api_failure(action: &str, e: Error) -> Error {
    let Some(api) = e.as_api() else { return e };
    let Some(body) = api.body.as_object() else { return e };
    if let Some(errors) = body.get("errors").and_then(Value::as_array) {
        let msgs: Vec<String> = errors
            .iter()
            .map(|err| {
                let g = |k: &str| err.get(k).map(py_str).unwrap_or_else(|| "?".into());
                format!("{}: {}", g("field"), g("message"))
            })
            .collect();
        return Error::other(format!("{action} failed:\n  {}", msgs.join("\n  ")));
    }
    let desc = body.get("description").map(py_str).unwrap_or_else(|| py_str(&api.body));
    Error::other(format!("{action} failed: {desc}"))
}

/// `update_change`: PUT with custom fields nested.
pub fn update_change(c: &dyn Api, change_id: i64, body: &Map<String, Value>, schema_fields: &[Value]) -> Result<Value> {
    let core = split_core(body, schema_fields, &["attachments"]);
    match c.int_put(&format!("changes/{change_id}"), &Value::Object(core)) {
        Ok(data) => Ok(unwrap_key(data, "change")),
        Err(e) => Err(api_failure("update", e)),
    }
}

/// `submit_change`: POST a new change.
pub fn submit_change(c: &dyn Api, body: &Map<String, Value>, schema_fields: &[Value]) -> Result<Value> {
    if body.contains_key("br_validation_excludes") {
        return Err(Error::value("br_validation_excludes is computed by fsv from the form rules; remove it from the body"));
    }
    let core = split_core(body, schema_fields, &[]);
    match c.int_post("changes", &Value::Object(core)) {
        Ok(data) => Ok(unwrap_key(data, "change")),
        Err(e) => Err(api_failure("create", e)),
    }
}

/// `post_change_body`: POST an engine-built change body.
pub fn post_change_body(c: &dyn Api, body: &Value) -> Result<Value> {
    let data = c.int_post("changes", body).map_err(|e| api_failure("create", e))?;
    Ok(unwrap_key(data, "change"))
}

/// `put_change_body`: PUT an engine-built partial change body.
pub fn put_change_body(c: &dyn Api, change_id: i64, body: &Value) -> Result<Value> {
    let data = c.int_put(&format!("changes/{change_id}"), body).map_err(|e| api_failure("update", e))?;
    Ok(unwrap_key(data, "change"))
}

// ---------------------------------------------------------------------------
// planning fields

fn strip_planning_prefixes(value: &Value) -> String {
    let mut text = casefold(&s(Some(value)));
    for prefix in ["planning-field-", "cf_"] {
        if let Some(rest) = text.strip_prefix(prefix) {
            text = rest.to_string();
        }
    }
    text
}

/// `_planning_key`.
pub fn planning_key(value: &Value) -> String {
    strip_planning_prefixes(value).chars().filter(|c| c.is_alphanumeric()).collect()
}

/// `_singular_key`: alphanumeric words with a trailing `s` dropped.
pub fn singular_key(value: &Value) -> String {
    let text = strip_planning_prefixes(value);
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    let flush = |current: &mut String, words: &mut Vec<String>| {
        if !current.is_empty() {
            let w = std::mem::take(current);
            words.push(w.strip_suffix('s').map(str::to_string).unwrap_or(w));
        }
    };
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            current.push(ch);
        } else {
            flush(&mut current, &mut words);
        }
    }
    flush(&mut current, &mut words);
    words.concat()
}

/// `_is_planning_field`.
pub fn is_planning_field(f: &Value) -> bool {
    is_planning_type(&s(f.get("field_type")))
}

/// `_norm_label`.
pub fn norm_label(value: Option<&Value>) -> String {
    casefold(&split_ws(&html_unescape(&s(value))).collect::<Vec<_>>().join(" "))
}

/// The account's planning-field definitions, in metadata order.
pub fn planning_definitions(schema_fields: &[Value]) -> Vec<Value> {
    schema_fields.iter().filter(|f| f.is_object() && is_planning_field(f)).cloned().collect()
}

/// The key `/changes/<id>/planning-fields/<key>` takes (its name, else its id).
pub fn planning_server_key(defn: &Value) -> String {
    py_str(defn.get_truthy("name").or_else(|| defn.get("id")).unwrap_or(&Value::Null))
}

/// `resolve_planning_definition`: exact identity or unique exact label, then
/// a unique alias.
pub fn resolve_planning_definition(value: &str, schema_fields: &[Value]) -> Result<Value> {
    let raw = fsv_core::pystr::strip(value).to_string();
    let defs = planning_definitions(schema_fields);
    let ambiguous = || api0(format!("ambiguous planning field: {}; select an exact identity", py_repr_str(value)));
    let identities: Vec<&Value> =
        defs.iter().filter(|f| !raw.is_empty() && (raw == s(f.get("name")) || raw == s(f.get("id")))).collect();
    let raw_label = norm_label(Some(&json!(raw)));
    let labels: Vec<&Value> = defs.iter().filter(|f| !raw.is_empty() && norm_label(f.get("label")) == raw_label).collect();
    let matches = if identities.is_empty() { labels } else { identities };
    if matches.len() > 1 {
        return Err(ambiguous());
    }
    if let Some(m) = matches.first() {
        return Ok((*m).clone());
    }
    let mut aliases = vec![raw.clone()];
    if let Some(rest) = raw.strip_prefix("planning-field-") {
        aliases.push(rest.to_string());
    }
    let cf: Vec<String> = aliases.iter().filter_map(|a| a.strip_prefix("cf_").map(str::to_string)).collect();
    aliases.extend(cf);
    let mut keys: HashSet<String> = aliases.iter().flat_map(|a| [planning_key(&json!(a)), singular_key(&json!(a))]).collect();
    keys.remove("");
    let mut found: Vec<&Value> = Vec::new();
    for f in &defs {
        let name = py_str(f.get("name").unwrap_or(&Value::Null));
        let mut candidates: Vec<Value> =
            ["name", "id", "label", "export_label", "helptext", "description"].iter().filter_map(|k| f.get_truthy(k).cloned()).collect();
        candidates.extend([json!(format!("planning-field-{name}")), json!(format!("planning-field-cf_{name}")), json!(format!("cf_{name}"))]);
        let ck: HashSet<String> = candidates.iter().flat_map(|c| [planning_key(c), singular_key(c)]).collect();
        if !keys.is_disjoint(&ck) {
            found.push(f);
        }
    }
    if found.len() > 1 {
        return Err(ambiguous());
    }
    if let Some(m) = found.first() {
        return Ok((*m).clone());
    }
    let known: Vec<String> =
        defs.iter().take(12).map(|f| py_str(f.get_truthy("label").or_else(|| f.get("name")).unwrap_or(&Value::Null))).collect();
    Err(api0(format!("planning field not found: {}; choices: {}", py_repr_str(value), known.join(", "))))
}

/// `resolve_planning_field`: the current server key.
pub fn resolve_planning_field(value: &str, schema_fields: &[Value]) -> Result<String> {
    Ok(planning_server_key(&resolve_planning_definition(value, schema_fields)?))
}

fn id_conflicts(row: &Value, fid: &Value) -> bool {
    ["field_id", "planning_field_id"].iter().any(|k| row.get(*k).is_some_and(|v| !v.is_null() && py_str(v) != py_str(fid)))
}

/// `match_planning_row`: unique identity, then unique exact label; never position.
pub fn match_planning_row(defn: &Value, rows: &[Value]) -> Result<Option<Value>> {
    let rows: Vec<&Value> = rows.iter().filter(|r| r.is_object()).collect();
    let name = defn.get("name").cloned().unwrap_or(Value::Null);
    let fid = defn.get("id").cloned().unwrap_or(Value::Null);
    let label = norm_label(defn.get("label"));
    let mut hits: Vec<&Value> = if truthy(&name) { rows.iter().copied().filter(|r| s(r.get("name")) == py_str(&name)).collect() } else { vec![] };
    if hits.is_empty() && !fid.is_null() {
        hits = rows
            .iter()
            .copied()
            .filter(|r| ["field_id", "planning_field_id", "id"].iter().any(|k| r.get(*k).is_some_and(|v| !v.is_null() && py_str(v) == py_str(&fid))))
            .collect();
    }
    if hits.is_empty() && !label.is_empty() {
        hits = rows.iter().copied().filter(|r| norm_label(r.get("label")) == label && !(!fid.is_null() && id_conflicts(r, &fid))).collect();
    }
    let who = || {
        if !label.is_empty() {
            py_repr_str(&label)
        } else if truthy(&name) {
            py_repr(&name)
        } else {
            py_repr(&fid)
        }
    };
    if hits.len() > 1 {
        return Err(api0(format!("ambiguous planning rows for {}", who())));
    }
    if let Some(h) = hits.first() {
        if !fid.is_null() && id_conflicts(h, &fid) {
            return Err(api0(format!("conflicting planning row identity for {}", who())));
        }
    }
    Ok(hits.first().map(|h| (*h).clone()))
}

/// `_planning_description`: the first nonempty text of a planning row.
pub fn planning_description(field: Option<&Value>) -> String {
    let Some(field) = field.filter(|f| truthy(f)) else { return String::new() };
    for key in ["description", "description_text", "body", "content"] {
        if let Some(v) = field.get(key).and_then(Value::as_str) {
            let t = fsv_core::pystr::strip(v);
            if !t.is_empty() {
                return t.to_string();
            }
        }
    }
    String::new()
}

/// `_default_planning_description`: the file names.
pub fn default_planning_description(paths: &[String]) -> String {
    paths.iter().map(|p| file_name(p)).collect::<Vec<_>>().join(", ")
}

/// `_attachment_name`.
pub fn attachment_name(att: &Value) -> String {
    s(att.get_truthy("name").or_else(|| att.get("content_file_name")))
}

/// `_attachment_id`.
pub fn attachment_id(att: &Value) -> Option<i64> {
    att.get("id").filter(|v| !v.is_null()).and_then(py_int)
}

/// `_prompt_duplicate_action`.
pub fn prompt_duplicate_action(name: &str, count: usize, prompter: &dyn Prompter) -> Result<String> {
    if !prompter.interactive() {
        return Err(api0(format!("attachment already exists: {name}; use --duplicate skip|replace|append")));
    }
    let answer = casefold(fsv_core::pystr::strip(&prompter.ask(&format!("  duplicate {name} ({count} existing). Choose [s]kip, [r]eplace, [a]ppend: "))));
    Ok(match answer.as_str() {
        "r" | "replace" | "overwrite" | "o" => "replace",
        "a" | "append" => "append",
        _ => "skip",
    }
    .into())
}

fn stem_suffix(name: &str) -> (String, String) {
    let p = Path::new(name);
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let suffix = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (stem, suffix)
}

/// `_backup_name`: the given name (numbered after the first) or a timestamped one.
pub fn backup_name(original: &str, backup_name: Option<&str>, index: usize) -> String {
    if let Some(b) = backup_name.filter(|b| !b.is_empty()) {
        if index == 0 {
            return b.to_string();
        }
        let (stem, suffix) = stem_suffix(b);
        return format!("{stem}-{index}{suffix}");
    }
    let (stem, suffix) = stem_suffix(original);
    let stamp = jiff::Zoned::now().strftime("%Y%m%d-%H%M%S").to_string();
    let tag = if index == 0 { format!(".backup-{stamp}") } else { format!(".backup-{stamp}-{index}") };
    format!("{stem}{tag}{suffix}")
}

/// `_choose_backup_name`: offers the default when prompting is possible.
pub fn choose_backup_name(original: &str, backup: Option<&str>, index: usize, prompt: bool, prompter: &dyn Prompter) -> String {
    let default = backup_name(original, backup, index);
    if backup.is_some_and(|b| !b.is_empty()) || index > 0 || !prompt || !prompter.interactive() {
        return default;
    }
    let answer = prompter.ask(&format!("  backup name [{default}]: "));
    let answer = fsv_core::pystr::strip(&answer);
    if answer.is_empty() { default } else { answer.to_string() }
}

/// `_should_backup_replaced`.
pub fn should_backup_replaced(name: &str, backup_replaced: Option<bool>, prompter: &dyn Prompter) -> bool {
    if let Some(b) = backup_replaced {
        return b;
    }
    if !prompter.interactive() {
        return false;
    }
    let answer = casefold(fsv_core::pystr::strip(&prompter.ask(&format!("  backup old {name} before replace? [Y/n]: "))));
    !matches!(answer.as_str(), "n" | "no")
}

/// `_backup_attachment`: re-upload an attachment's bytes under `name`.
pub fn backup_attachment(c: &dyn Api, att: &Value, name: &str) -> Result<i64> {
    let url = attachment_url(att, &c.domain());
    if url.is_empty() {
        return Err(api0(format!("cannot backup attachment with no URL: {}", py_repr(att))));
    }
    let r = c.fetch(&url)?;
    if r.status >= 400 {
        let text: String = String::from_utf8_lossy(&r.body).chars().take(500).collect();
        return Err(ApiError::new(r.status, Value::String(text)).into());
    }
    let mime = att
        .get_truthy("content_type")
        .map(py_str)
        .or_else(|| guess_mime(name))
        .unwrap_or_else(|| "application/octet-stream".into());
    upload_bytes(c, name, &r.body, &mime)
}

/// `update_planning_field`: merge new files with the existing ones; PUT an
/// existing row, POST a new one. `label` finds a row whose name differs.
#[allow(clippy::too_many_arguments)]
pub fn update_planning_field(
    c: &dyn Api,
    change_id: i64,
    field_id: &str,
    description: Option<&str>,
    file_paths: &[String],
    opts: &DuplicateOpts,
    label: Option<&str>,
    prompter: &dyn Prompter,
) -> Result<Value> {
    let data = c.int_get(&format!("changes/{change_id}/planning-fields"), None)?;
    let Some(rows) = data.get("change_planning_fields").and_then(Value::as_array).filter(|_| data.is_object()) else {
        return Err(api0("planning rows unavailable; cannot safely determine the write target"));
    };
    if rows.iter().any(|r| !r.is_object()) {
        return Err(api0("invalid planning row; cannot safely determine the write target"));
    }
    let existing = match_planning_row(&json!({"name": field_id, "label": label}), rows)?;
    if existing.is_none() && rows.iter().any(|r| r.get_truthy("name").is_none() && r.get_truthy("label").is_none()) {
        return Err(api0("unidentified planning row; cannot safely determine the write target"));
    }
    let mut field_id = field_id.to_string();
    if let Some(n) = existing.as_ref().and_then(|e| e.get_truthy("name")) {
        field_id = py_str(n);
    }
    let attachments = existing.as_ref().map(|e| e.list_or_empty("attachments").to_vec()).unwrap_or_default();
    let ids: Vec<i64> = attachments.iter().filter_map(attachment_id).collect();
    let mut description = description.map(str::to_string);
    if description.is_none() && !file_paths.is_empty() && planning_description(existing.as_ref()).is_empty() {
        description = Some(default_planning_description(file_paths));
    }
    let mode = opts.mode()?;
    let (ids, skipped, changed) = merge_uploads(c, &attachments, ids, file_paths, &mode, opts, prompter)?;
    if existing.as_ref().is_some_and(truthy) {
        let mut body = Map::new();
        if let Some(d) = &description {
            body.insert("description".into(), json!(d));
        }
        if changed {
            body.insert("attachments".into(), json!(ids));
        }
        if body.is_empty() {
            return Ok(json!({"_fsv_noop": true, "skipped": skipped, "planning_field": field_id}));
        }
        return c.int_put(&format!("changes/{change_id}/planning-fields/{field_id}"), &Value::Object(body));
    }
    let final_description = description.filter(|d| !d.is_empty()).unwrap_or_else(|| default_planning_description(file_paths));
    let mut body = obj(json!({"description": final_description}));
    if !ids.is_empty() {
        body.insert("attachments".into(), json!(ids));
    }
    c.int_post(&format!("changes/{change_id}/planning-fields?id={field_id}"), &Value::Object(body))
}

/// `download_planning_attachments`: planning attachments (and optionally the
/// descriptions as `.md`) into `out_dir` (default `./CHN-<id>/`).
pub fn download_planning_attachments(
    c: &dyn Api,
    change_id: i64,
    field_names: Option<&[String]>,
    out_dir: Option<&Path>,
    include_description: bool,
) -> Result<Vec<PathBuf>> {
    static UNSAFE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^a-zA-Z0-9_-]+").expect("regex"));
    let io = |e: std::io::Error| Error::other(e.to_string());
    let data = c.int_get(&format!("changes/{change_id}/planning-fields"), None)?;
    let out = match out_dir {
        Some(d) => d.to_path_buf(),
        None => std::env::current_dir().map_err(io)?.join(format!("CHN-{change_id}")),
    };
    std::fs::create_dir_all(&out).map_err(io)?;
    let mut downloaded = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for field in rows_at(&data, "change_planning_fields") {
        let name = field.get_truthy("name").map(py_str);
        let label = fsv_core::pystr::strip(&s(field.get_truthy("label").or(field.get("name")))).to_string();
        let Some(name) = name else { continue };
        if field_names.is_some_and(|names| !names.contains(&name)) {
            continue;
        }
        let description = fsv_core::pystr::strip(&s(field.get("description"))).to_string();
        let attachments = field.list_or_empty("attachments");
        if description.is_empty() && attachments.is_empty() {
            continue;
        }
        let safe = UNSAFE.replace_all(&label, "_").trim_matches('_').to_string();
        if include_description && !description.is_empty() {
            let md = out.join(format!("{}.md", if safe.is_empty() { &name } else { &safe }));
            let md_name = md.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if !seen.contains(&md_name) {
                std::fs::write(&md, &description).map_err(io)?;
                downloaded.push(md);
                seen.insert(md_name);
            }
        }
        for att in attachments {
            let Some(url) = att.get_truthy("attachment_url").or_else(|| att.get_truthy("canonical_url")).map(py_str) else { continue };
            let att_name = att.get("name").map(py_str).unwrap_or_else(|| "attachment".into());
            let final_name = if safe.is_empty() { att_name } else { format!("{safe}_{att_name}") };
            let mut path = out.join(&final_name);
            let mut counter = 1;
            while seen.contains(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()) {
                let fname = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let (stem, suffix) = stem_suffix(&fname);
                path = out.join(format!("{stem}_{counter}{suffix}"));
                counter += 1;
            }
            seen.insert(path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
            let r = c.fetch(&url)?;
            if r.status != 200 {
                continue;
            }
            std::fs::write(&path, &r.body).map_err(io)?;
            downloaded.push(path);
        }
    }
    Ok(downloaded)
}

// ---------------------------------------------------------------------------
// reads and associations

/// `get_change_approvals`: a bare list or the `approvals` envelope.
pub fn get_change_approvals(c: &dyn Api, change_id: i64) -> Result<Vec<Value>> {
    let data = c.int_get(&format!("changes/{change_id}/approvals"), None)?;
    Ok(match data {
        Value::Array(a) => a,
        other => rows_at(&other, "approvals"),
    })
}

/// `get_change_approval_groups`: strictly validated complete approval chains.
pub fn get_change_approval_groups(c: &dyn Api, change_id: i64) -> Result<Vec<Value>> {
    crate::approvals::get_change_approval_groups(c, &json!(change_id))
}

fn not_found(e: &Error) -> bool {
    e.as_api().is_some_and(|a| a.status == 404)
}

/// `get_change_assets`: `associated_assets`, else the per-type CI lists.
pub fn get_change_assets(c: &dyn Api, change_id: i64) -> Result<Vec<Value>> {
    match c.int_get(&format!("changes/{change_id}/assets"), None) {
        Ok(data) => {
            if let Some(m) = data.as_object().filter(|m| m.contains_key("associated_assets")) {
                return Ok(m.get("associated_assets").filter(|v| truthy(v)).and_then(Value::as_array).cloned().unwrap_or_default());
            }
        }
        Err(e) if not_found(&e) => {}
        Err(e) => return Err(e),
    }
    let mut items = Vec::new();
    for atype in ["cis", "services", "softwares"] {
        let params = obj(json!({"page": 1, "per_page": 100, "association_type": atype}));
        match c.int_get(&format!("changes/{change_id}/associated-cis"), Some(params)) {
            Ok(data) => items.extend(rows_at(&data, "data")),
            Err(e) if not_found(&e) => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(items)
}

/// `search_assets_for_change`.
pub fn search_assets_for_change(c: &dyn Api, change_id: i64, query: &str, page: i64, per_page: i64) -> Result<Value> {
    let mut params = obj(json!({"entity": "change", "entity_id": change_id, "include_meta": "true", "page": page, "per_page": per_page}));
    if !query.is_empty() {
        params.insert("search_term".into(), json!(query));
    }
    c.int_get("assets-to-associate", Some(params))
}

/// `associate_assets`: asset display ids onto a change.
pub fn associate_assets(c: &dyn Api, change_id: i64, asset_ids: &[i64]) -> Result<()> {
    c.int_put(&format!("changes/{change_id}/assets/associate"), &json!({"item_ids": asset_ids}))?;
    Ok(())
}

/// `dissociate_assets`: detach by asset display id.
pub fn dissociate_assets(c: &dyn Api, change_id: i64, asset_display_ids: &[i64]) -> Result<()> {
    let current = get_change_assets(c, change_id)?;
    let mut by_display: Vec<(Value, Value)> = Vec::new();
    for a in &current {
        let ci = a.get("config_item").cloned().unwrap_or_else(|| json!({}));
        if let Some(did) = ci.get_truthy("display_id").filter(|_| truthy(&ci)) {
            let id = a.get("id").cloned().unwrap_or(Value::Null);
            match by_display.iter_mut().find(|(k, _)| k == did) {
                Some(slot) => slot.1 = id,
                None => by_display.push((did.clone(), id)),
            }
        }
    }
    for did in asset_display_ids {
        let Some((_, cmdb_id)) = by_display.iter().find(|(k, _)| *k == json!(did)).filter(|(_, v)| !v.is_null()) else {
            return Err(Error::value(format!("asset {did} not found on change #{change_id}")));
        };
        c.int_put(&format!("changes/{change_id}/assets/detach"), &json!({"cmdb_request_id": cmdb_id}))?;
    }
    Ok(())
}

/// `get_change_associations`: paginated rows plus completeness (never a false empty).
pub fn get_change_associations(c: &dyn Api, change_id: i64) -> Result<Value> {
    crate::associations::inventory(c, change_id, crate::associations::KINDS)
}

/// `search_change_tickets`.
pub fn search_change_tickets(c: &dyn Api, query: &str) -> Result<Vec<Value>> {
    let data = c.int_get("changes/tickets/search", Some(obj(json!({"module_type": "change", "q": query}))))?;
    Ok(rows_at(&data, "tickets"))
}

/// `get_change_activities`: follows the `start_token` of `next_page_url`.
pub fn get_change_activities(c: &dyn Api, change_id: i64) -> Result<Vec<Value>> {
    static TOKEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"start_token=(\S+)").expect("regex"));
    let mut all = Vec::new();
    let mut params = Map::new();
    loop {
        let data = c.int_get(&format!("changes/{change_id}/activities"), Some(params.clone()))?;
        let acts = rows_at(&data, "activities");
        let empty = acts.is_empty();
        all.extend(acts);
        let Some(next) = data.get_truthy("next_page_url").map(py_str) else { break };
        if empty {
            break;
        }
        let Some(m) = TOKEN.captures(&next) else { break };
        params = obj(json!({"start_token": &m[1]}));
    }
    Ok(all)
}

/// `associate_ticket`: one reviewed, journaled PUT.
pub fn associate_ticket(c: &dyn Api, journal: &dyn Journal, change_id: &str, ticket_ids: &[Value], opts: crate::associations::SendOpts<'_>) -> Result<Value> {
    let plan = crate::associations::build_plan(change_id, ticket_ids, false, &c.domain())?;
    crate::associations::send(&plan, c, journal, opts)
}

/// `dissociate_ticket`: one reviewed, journaled DELETE.
pub fn dissociate_ticket(c: &dyn Api, journal: &dyn Journal, change_id: &str, ticket_id: &Value, opts: crate::associations::SendOpts<'_>) -> Result<Value> {
    let plan = crate::associations::build_plan(change_id, std::slice::from_ref(ticket_id), true, &c.domain())?;
    crate::associations::send(&plan, c, journal, opts)
}

/// `get_task_for_edit`: read-only fields and nulls stripped.
pub fn get_task_for_edit(c: &dyn Api, change_id: i64, task_id: i64) -> Result<Map<String, Value>> {
    let data = c.int_get(&format!("changes/{change_id}/tasks/{task_id}"), None)?;
    let mut task = obj(unwrap_key(data, "task"));
    task.retain(|k, v| !is_task_read_only(k) && !v.is_null());
    Ok(task)
}

/// `_task_put`: exactly one task write with a fresh CSRF token, never replayed.
pub fn task_put(c: &dyn Api, path: &str, body: &Value) -> Result<Value> {
    let csrf = c.get_csrf()?;
    c.send_once("PUT", path, Body::Json(body.clone()), None, &csrf)
}

/// `delete_task`: the observed internal contract.
pub fn delete_task(c: &dyn Api, change_id: i64, task_id: i64) -> Result<()> {
    task_put(c, &format!("changes/{change_id}/tasks/update"), &json!({"id": task_id.to_string(), "action": "delete"}))?;
    Ok(())
}

/// `update_task`: only the given fields (read-only/null custom fields dropped).
pub fn update_task(c: &dyn Api, change_id: i64, task_id: i64, body: &Map<String, Value>) -> Result<Value> {
    let mut payload = body.clone();
    let cf = payload.remove("custom_fields");
    if let Some(cf) = cf.filter(|v| truthy(v)) {
        let kept: Map<String, Value> = cf
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(k, v)| !TASK_READONLY_CF.contains(&k.as_str()) && !v.is_null())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        payload.insert("custom_fields".into(), Value::Object(kept));
    }
    let data = task_put(c, &format!("changes/{change_id}/tasks/{task_id}"), &Value::Object(payload))?;
    Ok(unwrap_key(data, "task"))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::service::testing::*;

    pub(crate) struct Script {
        pub tty: bool,
        pub answers: RefCell<Vec<String>>,
        pub prompts: RefCell<Vec<String>>,
    }

    impl Script {
        pub fn new(tty: bool, answers: &[&str]) -> Self {
            Script { tty, answers: RefCell::new(answers.iter().rev().map(|s| s.to_string()).collect()), prompts: RefCell::default() }
        }
    }

    impl Prompter for Script {
        fn interactive(&self) -> bool {
            self.tty
        }
        fn ask(&self, prompt: &str) -> String {
            self.prompts.borrow_mut().push(prompt.into());
            self.answers.borrow_mut().pop().unwrap_or_default()
        }
    }

    fn api_body(e: Error) -> Value {
        e.as_api().expect("api error").body.clone()
    }
}
