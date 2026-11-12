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
pub const CLONE_SKIP_FIELDS: &[&str] = &[
    "stats",
    "planning_field_definitions",
    "main_attachments",
    "attachments",
];

pub const READ_ONLY_FIELDS: &[&str] = &[
    "id",
    "display_id",
    "human_display_id",
    "change_id",
    "workspace_id",
    "import_id",
    "project_id",
    "wf_event_id",
    "created_at",
    "updated_at",
    "closed_at",
    "due_by",
    "nr_due_by",
    "first_response_time",
    "ola_stopped_at",
    "status_name",
    "priority_name",
    "impact_name",
    "risk_name",
    "approval_status",
    "approval_type",
    "summary_status",
    "attached_risk_survey_response_status",
    "resolution_time_taken",
    "total_time_spent",
    "timer_active_agents",
    "task_due_status",
    "is_editable",
    "status_ola_timer",
    "state_flow_id",
    "state_traversal",
    "cab_meeting_ids",
    "trashed",
    "deleted",
    "is_escalated",
    "escalation_level",
    "manual_dueby",
    "planned",
    "requester",
    "department",
    "agent",
    "group",
    "attachments",
    "cloud_files",
    "shared_attachment_ids",
    "assoc_assets",
    "assoc_asset_ids",
    "assoc_change_ids",
    "assoc_release_id",
    "blackout_window",
    "source",
    "business_calendar_id",
    "sla_policy_id",
    "cc_email",
    "email_config_id",
    "group_name",
    "owner_name",
    "description_text",
    "problem_id",
    "release_id",
    "associated_cis_and_services_ids",
    "sub_category",
    "item_category",
    "initiated_by",
    "stack_rank",
    "taskable_display_id",
    "taskable_type",
    "user_id",
    "force_authenticated",
    "tasks_dependency_type",
    "notify_before",
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
pub const TASK_DATE_KEYS: &[&str] = &[
    "due_date",
    "planned_start_date",
    "planned_end_date",
    "start_date",
    "end_date",
];
pub const TASK_READONLY_CF: &[&str] = &["team_name"];

const CHANGE_CORE_NAMES: &[&str] = &[
    "requester_id",
    "email",
    "group_id",
    "agent_id",
    "change_type",
    "priority",
    "impact",
    "risk",
    "status",
    "category",
    "subject",
    "description",
    "planned_start_date",
    "planned_end_date",
    "planned_effort",
    "department_id",
    "change_window_id",
];

/// `APIError(0, message)`.
fn api0(msg: impl Into<String>) -> Error {
    ApiError::new(0, Value::String(msg.into())).into()
}

fn usage(msg: impl Into<String>) -> Error {
    Error::usage(msg)
}

fn s(v: Option<&Value>) -> String {
    v.filter(|x| truthy(x)).map(py_str).unwrap_or_default()
}

fn is_id_value_pair(v: &Value) -> bool {
    v.as_object()
        .is_some_and(|m| m.len() == 2 && m.contains_key("id") && m.contains_key("value"))
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
        let Some(name) = f.get_truthy("name").map(py_str) else {
            continue;
        };
        let ftype = s(f.get("field_type"));
        if SKIP_TYPES.contains(&ftype.as_str()) || is_content_type(&ftype) {
            continue;
        }
        let required = f.get("required").is_some_and(truthy);
        if let Some((_, key, placeholder)) =
            TEMPLATE_FK_PLACEHOLDERS.iter().find(|(n, _, _)| *n == name)
        {
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
    c.get("value")
        .or_else(|| c.get("name"))
        .or_else(|| c.get("id"))
        .cloned()
        .unwrap_or(Value::Null)
}

/// `_field_default`: a placeholder for a schema field.
pub fn field_default(f: &Value) -> Value {
    let ftype = s(f.get("field_type"));
    let choices = f.list_or_empty("choices");
    let boolish = |c: &Value| {
        c.get("id").is_some_and(Value::is_boolean)
            || c.get("id")
                .is_some_and(|v| *v == json!(1) || *v == json!(0))
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
        let examples: Vec<String> = choices
            .iter()
            .take(2)
            .map(|c| py_str(&choice_example(c)))
            .collect();
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
    if let Some(name) = f.get_truthy("name")
        && py_str(name).ends_with("_id")
    {
        return json!("<ID>");
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
    if let Some(cf) = change.shift_remove("custom_fields")
        && let Value::Object(cf) = cf
    {
        for (k, v) in cf {
            let v = if is_id_value_pair(&v) {
                v["id"].clone()
            } else {
                v
            };
            change.insert(k, v);
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
        if READ_ONLY_FIELDS.contains(&k.as_str())
            || CLONE_SKIP_FIELDS.contains(&k.as_str())
            || k.ends_with("planning_fields")
        {
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
pub fn task_clone_body(
    task: &Value,
    keep_owners: bool,
    now: Option<jiff::Timestamp>,
    mut dropped: Option<&mut Vec<String>>,
) -> Map<String, Value> {
    let now = now.unwrap_or_else(jiff::Timestamp::now);
    let mut out = Map::new();
    let title = task
        .get_truthy("title")
        .or_else(|| task.get("id"))
        .cloned()
        .unwrap_or(Value::Null);
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
        if TASK_DATE_KEYS.contains(&k.as_str())
            && let Some(when) = parse_when(v)
            && when < now
        {
            if let Some(d) = dropped.as_deref_mut() {
                d.push(format!("task {}: past {k} not copied", py_repr(&title)));
            }
            continue;
        }
        out.insert(k.clone(), v.clone());
    }
    out.insert("status".into(), json!(1));
    if let Some(Value::Object(cf)) = out.get("custom_fields") {
        let cf: Map<String, Value> = cf
            .iter()
            .filter(|(_, v)| !v.is_null())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if cf.is_empty() {
            out.shift_remove("custom_fields");
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
        let result = c.int_post(
            &format!("changes/{new_change_id}/tasks"),
            &json!({"change_task": body}),
        )?;
        created.push(unwrap_key(result, "task"));
    }
    Ok(created)
}

/// `clone_side_effects`: read-only counts of what the clone flags would copy.
pub fn clone_side_effects(c: &dyn Api, source_change_id: i64) -> Result<Map<String, Value>> {
    let tasks = rows_at(
        &c.int_get(&format!("changes/{source_change_id}/tasks"), None)?,
        "tasks",
    );
    let planning = rows_at(
        &c.int_get(&format!("changes/{source_change_id}/planning-fields"), None)?,
        "change_planning_fields",
    );
    let with_owner = tasks
        .iter()
        .filter(|t| {
            TASK_OWNER_KEYS
                .iter()
                .any(|k| t.get(*k).is_some_and(truthy))
        })
        .count();
    let assets = get_change_assets(c, source_change_id)?.len();
    let filled = planning
        .iter()
        .filter(|f| {
            !fsv_core::pystr::strip(&s(f.get("description"))).is_empty()
                || f.get("attachments").is_some_and(truthy)
        })
        .count();
    let atts: usize = planning
        .iter()
        .map(|f| f.list_or_empty("attachments").len())
        .sum();
    Ok(obj(json!({
        "tasks": tasks.len(), "tasks_with_owner": with_owner, "assets": assets,
        "planning_fields": filled, "planning_attachments": atts,
    })))
}

/// `clone_assets`: the source's assets associated on the new change.
pub fn clone_assets(c: &dyn Api, source_change_id: i64, new_change_id: i64) -> Result<Vec<i64>> {
    let display = |e: &Value| {
        e.get_truthy("display_id")
            .or_else(|| {
                e.obj_or_empty("config_item")
                    .get("display_id")
                    .filter(|v| truthy(v))
            })
            .cloned()
    };
    let ids: Vec<i64> = get_change_assets(c, source_change_id)?
        .iter()
        .filter_map(display)
        .map(|v| py_int(&v).unwrap_or(0))
        .collect();
    if ids.is_empty() {
        return Ok(ids);
    }
    c.int_put(
        &format!("changes/{new_change_id}/assets/associate"),
        &json!({"item_ids": ids}),
    )?;
    Ok(ids)
}

/// `clone_planning_fields`: text and re-uploaded attachments; returns names.
pub fn clone_planning_fields(
    c: &dyn Api,
    source_change_id: i64,
    new_change_id: i64,
) -> Result<Vec<String>> {
    let data = c.int_get(&format!("changes/{source_change_id}/planning-fields"), None)?;
    let mut cloned = Vec::new();
    for field in rows_at(&data, "change_planning_fields") {
        let Some(name) = field.get_truthy("name").map(py_str) else {
            continue;
        };
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
        c.int_post(
            &format!("changes/{new_change_id}/planning-fields?id={name}"),
            &Value::Object(body),
        )?;
        cloned.push(name);
    }
    Ok(cloned)
}

// ---------------------------------------------------------------------------
// attachments

/// `_attachment_url`: the download URL (helpdesk links get `?download=true`).
pub fn attachment_url(att: &Value, domain: &str) -> String {
    let mut url = s(att
        .get_truthy("attachment_url")
        .or_else(|| att.get("canonical_url")));
    if url.is_empty()
        && let Some(p) = att.get_truthy("canonical_path")
    {
        url = format!("https://{domain}{}", py_str(p));
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
    if cleaned.is_empty() {
        "attachment".into()
    } else {
        cleaned.to_string()
    }
}

/// The filename of a `Content-Disposition` header (`Message.get_filename`).
fn disposition_filename(cd: &str) -> Option<String> {
    let mut plain = None;
    for part in cd.split(';').skip(1) {
        let Some((k, v)) = part.split_once('=') else {
            continue;
        };
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
    let fallback = format!(
        "attachment-{}",
        att.get("id").map(py_str).unwrap_or_default()
    );
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
    if att.get_truthy("name").is_none()
        && cd.contains("filename=")
        && let Some(filename) = disposition_filename(&cd)
    {
        name = safe_filename(&filename);
        path = out_dir.join(&name);
        if path.exists() && !force {
            return Ok(skipped_result(&name, &path));
        }
    }
    std::fs::write(&path, &r.body).map_err(|e| Error::other(e.to_string()))?;
    Ok(
        json!({"name": name, "path": path.to_string_lossy(), "size": r.body.len(), "skipped": false}),
    )
}

fn now_epoch() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn data_uri(mime: &str, raw: &[u8]) -> String {
    format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(raw)
    )
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
    let mime_full = att
        .get("content_type")
        .map(py_str)
        .unwrap_or_else(|| "application/octet-stream".into());
    let mime = fsv_core::pystr::strip(mime_full.split(';').next().unwrap_or("")).to_string();
    let name = att
        .get("name")
        .cloned()
        .unwrap_or_else(|| json!("attachment"));
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
    Ok(result
        .get_truthy("id")
        .or_else(|| result.obj_or_empty("attachment").get("id"))
        .cloned())
}

/// Python's `f"{n:,}"`.
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
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
    let att_id = result
        .get("id")
        .filter(|v| result.is_object() && truthy(v))
        .cloned();
    let Some(att_id) = att_id else {
        return Err(api0(format!(
            "attachment upload returned no id: {}",
            py_repr(&result)
        )));
    };
    eprintln!("{}", upload_receipt(name, raw.len(), &result));
    py_int(&att_id)
        .ok_or_else(|| Error::value(format!("invalid literal for int(): {}", py_repr(&att_id))))
}

/// The line `_upload_bytes` prints after `uploading NAME... `: the server's
/// echoed name and size checked against what was sent.
fn upload_receipt(name: &str, n: usize, result: &Value) -> String {
    let srv_size = result
        .get_truthy("content_file_size")
        .or_else(|| result.get("size"))
        .cloned()
        .unwrap_or(Value::Null);
    let srv_name = result
        .get_truthy("content_file_name")
        .or_else(|| result.get("name"))
        .cloned()
        .unwrap_or(Value::Null);
    if srv_size == json!(n) && srv_name == json!(name) {
        format!("ok ({} B)", thousands(n))
    } else if srv_name != json!(name) {
        format!(
            "ok ({} B, server name \"{}\" differs)",
            thousands(n),
            py_str(&srv_name)
        )
    } else {
        format!(
            "mismatch (local {} B, server {} B)",
            thousands(n),
            py_str(&srv_size)
        )
    }
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
            return Err(usage(
                "--duplicate must be one of: prompt, skip, replace, append",
            ));
        }
        Ok(mode)
    }
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
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
        let matches: Vec<&Value> = attachments
            .iter()
            .filter(|a| attachment_name(a) == name)
            .collect();
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
                    let bname = choose_backup_name(
                        &name,
                        opts.backup_name.as_deref(),
                        i,
                        opts.backup_replaced.is_none(),
                        prompter,
                    );
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
pub fn attach_files_to_change(
    c: &dyn Api,
    change_id: i64,
    file_paths: &[String],
    opts: &DuplicateOpts,
    prompter: &dyn Prompter,
) -> Result<Value> {
    let data = c.int_get(&format!("changes/{change_id}"), None)?;
    let change = unwrap_key(data, "change");
    let attachments = change.list_or_empty("attachments").to_vec();
    let ids: Vec<i64> = attachments.iter().filter_map(attachment_id).collect();
    let mode = opts.mode()?;
    let (ids, skipped, changed) =
        merge_uploads(c, &attachments, ids, file_paths, &mode, opts, prompter)?;
    if !changed {
        return Ok(json!({"_fsv_noop": true, "skipped": skipped, "attachments": ids}));
    }
    c.int_put(
        &format!("changes/{change_id}"),
        &json!({"attachments": ids}),
    )?;
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
fn split_core(
    body: &Map<String, Value>,
    schema_fields: &[Value],
    extra_core: &[&str],
) -> Map<String, Value> {
    let mut core = Map::new();
    let mut custom = Map::new();
    for (k, v) in body {
        let is_default = schema_fields
            .iter()
            .rev()
            .find(|f| f.get("name").and_then(Value::as_str) == Some(k.as_str()))
            .is_some_and(|f| f.get("default_field").is_some_and(truthy));
        let v = if is_id_value_pair(v) {
            v["id"].clone()
        } else {
            v.clone()
        };
        if CHANGE_CORE_NAMES.contains(&k.as_str()) || extra_core.contains(&k.as_str()) || is_default
        {
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
    let Some(body) = api.body.as_object() else {
        return e;
    };
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
    let desc = body
        .get("description")
        .map(py_str)
        .unwrap_or_else(|| py_str(&api.body));
    Error::other(format!("{action} failed: {desc}"))
}

/// `update_change`: PUT with custom fields nested.
pub fn update_change(
    c: &dyn Api,
    change_id: i64,
    body: &Map<String, Value>,
    schema_fields: &[Value],
) -> Result<Value> {
    let core = split_core(body, schema_fields, &["attachments"]);
    match c.int_put(&format!("changes/{change_id}"), &Value::Object(core)) {
        Ok(data) => Ok(unwrap_key(data, "change")),
        Err(e) => Err(api_failure("update", e)),
    }
}

/// `submit_change`: POST a new change.
pub fn submit_change(
    c: &dyn Api,
    body: &Map<String, Value>,
    schema_fields: &[Value],
) -> Result<Value> {
    if body.contains_key("br_validation_excludes") {
        return Err(Error::value(
            "br_validation_excludes is computed by fsv from the form rules; remove it from the body",
        ));
    }
    let core = split_core(body, schema_fields, &[]);
    match c.int_post("changes", &Value::Object(core)) {
        Ok(data) => Ok(unwrap_key(data, "change")),
        Err(e) => Err(api_failure("create", e)),
    }
}

/// `post_change_body`: POST an engine-built change body.
pub fn post_change_body(c: &dyn Api, body: &Value) -> Result<Value> {
    let data = c
        .int_post("changes", body)
        .map_err(|e| api_failure("create", e))?;
    Ok(unwrap_key(data, "change"))
}

/// `put_change_body`: PUT an engine-built partial change body.
pub fn put_change_body(c: &dyn Api, change_id: i64, body: &Value) -> Result<Value> {
    let data = c
        .int_put(&format!("changes/{change_id}"), body)
        .map_err(|e| api_failure("update", e))?;
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
    strip_planning_prefixes(value)
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
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
    casefold(
        &split_ws(&html_unescape(&s(value)))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// The account's planning-field definitions, in metadata order.
pub fn planning_definitions(schema_fields: &[Value]) -> Vec<Value> {
    schema_fields
        .iter()
        .filter(|f| f.is_object() && is_planning_field(f))
        .cloned()
        .collect()
}

/// The key `/changes/<id>/planning-fields/<key>` takes (its name, else its id).
pub fn planning_server_key(defn: &Value) -> String {
    py_str(
        defn.get_truthy("name")
            .or_else(|| defn.get("id"))
            .unwrap_or(&Value::Null),
    )
}

/// `resolve_planning_definition`: exact identity or unique exact label, then
/// a unique alias.
pub fn resolve_planning_definition(value: &str, schema_fields: &[Value]) -> Result<Value> {
    let raw = fsv_core::pystr::strip(value).to_string();
    let defs = planning_definitions(schema_fields);
    let ambiguous = || {
        usage(format!(
            "ambiguous planning field: {}; select an exact identity",
            py_repr_str(value)
        ))
    };
    let identities: Vec<&Value> = defs
        .iter()
        .filter(|f| !raw.is_empty() && (raw == s(f.get("name")) || raw == s(f.get("id"))))
        .collect();
    let raw_label = norm_label(Some(&json!(raw)));
    let labels: Vec<&Value> = defs
        .iter()
        .filter(|f| !raw.is_empty() && norm_label(f.get("label")) == raw_label)
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
        return Ok((*m).clone());
    }
    let mut aliases = vec![raw.clone()];
    if let Some(rest) = raw.strip_prefix("planning-field-") {
        aliases.push(rest.to_string());
    }
    let cf: Vec<String> = aliases
        .iter()
        .filter_map(|a| a.strip_prefix("cf_").map(str::to_string))
        .collect();
    aliases.extend(cf);
    let mut keys: HashSet<String> = aliases
        .iter()
        .flat_map(|a| [planning_key(&json!(a)), singular_key(&json!(a))])
        .collect();
    keys.remove("");
    let mut found: Vec<&Value> = Vec::new();
    for f in &defs {
        let name = py_str(f.get("name").unwrap_or(&Value::Null));
        let mut candidates: Vec<Value> = [
            "name",
            "id",
            "label",
            "export_label",
            "helptext",
            "description",
        ]
        .iter()
        .filter_map(|k| f.get_truthy(k).cloned())
        .collect();
        candidates.extend([
            json!(format!("planning-field-{name}")),
            json!(format!("planning-field-cf_{name}")),
            json!(format!("cf_{name}")),
        ]);
        let ck: HashSet<String> = candidates
            .iter()
            .flat_map(|c| [planning_key(c), singular_key(c)])
            .collect();
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
    let known: Vec<String> = defs
        .iter()
        .map(|f| {
            py_str(
                f.get_truthy("label")
                    .or_else(|| f.get("name"))
                    .unwrap_or(&Value::Null),
            )
        })
        .collect();
    Err(usage(format!(
        "planning field not found: {}; choices: {}",
        py_repr_str(value),
        known.join(", ")
    )))
}

/// `resolve_planning_field`: the current server key.
pub fn resolve_planning_field(value: &str, schema_fields: &[Value]) -> Result<String> {
    Ok(planning_server_key(&resolve_planning_definition(
        value,
        schema_fields,
    )?))
}

fn id_conflicts(row: &Value, fid: &Value) -> bool {
    ["field_id", "planning_field_id"].iter().any(|k| {
        row.get(*k)
            .is_some_and(|v| !v.is_null() && py_str(v) != py_str(fid))
    })
}

/// `match_planning_row`: unique identity, then unique exact label; never position.
pub fn match_planning_row(defn: &Value, rows: &[Value]) -> Result<Option<Value>> {
    let rows: Vec<&Value> = rows.iter().filter(|r| r.is_object()).collect();
    let name = defn.get("name").cloned().unwrap_or(Value::Null);
    let fid = defn.get("id").cloned().unwrap_or(Value::Null);
    let label = norm_label(defn.get("label"));
    let mut hits: Vec<&Value> = if truthy(&name) {
        rows.iter()
            .copied()
            .filter(|r| s(r.get("name")) == py_str(&name))
            .collect()
    } else {
        vec![]
    };
    if hits.is_empty() && !fid.is_null() {
        hits = rows
            .iter()
            .copied()
            .filter(|r| {
                ["field_id", "planning_field_id", "id"].iter().any(|k| {
                    r.get(*k)
                        .is_some_and(|v| !v.is_null() && py_str(v) == py_str(&fid))
                })
            })
            .collect();
    }
    if hits.is_empty() && !label.is_empty() {
        hits = rows
            .iter()
            .copied()
            .filter(|r| {
                norm_label(r.get("label")) == label && (fid.is_null() || !id_conflicts(r, &fid))
            })
            .collect();
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
    if let Some(h) = hits.first()
        && !fid.is_null()
        && id_conflicts(h, &fid)
    {
        return Err(api0(format!(
            "conflicting planning row identity for {}",
            who()
        )));
    }
    Ok(hits.first().map(|h| (*h).clone()))
}

/// `_planning_description`: the first nonempty text of a planning row.
pub fn planning_description(field: Option<&Value>) -> String {
    let Some(field) = field.filter(|f| truthy(f)) else {
        return String::new();
    };
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
    paths
        .iter()
        .map(|p| file_name(p))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `_attachment_name`.
pub fn attachment_name(att: &Value) -> String {
    s(att
        .get_truthy("name")
        .or_else(|| att.get("content_file_name")))
}

/// `_attachment_id`.
pub fn attachment_id(att: &Value) -> Option<i64> {
    att.get("id").filter(|v| !v.is_null()).and_then(py_int)
}

/// `_prompt_duplicate_action`.
pub fn prompt_duplicate_action(
    name: &str,
    count: usize,
    prompter: &dyn Prompter,
) -> Result<String> {
    if !prompter.interactive() {
        return Err(api0(format!(
            "attachment already exists: {name}; use --duplicate skip|replace|append"
        )));
    }
    let answer = casefold(fsv_core::pystr::strip(&prompter.ask(&format!(
        "  duplicate {name} ({count} existing). Choose [s]kip, [r]eplace, [a]ppend: "
    ))));
    Ok(match answer.as_str() {
        "r" | "replace" | "overwrite" | "o" => "replace",
        "a" | "append" => "append",
        _ => "skip",
    }
    .into())
}

fn stem_suffix(name: &str) -> (String, String) {
    let p = Path::new(name);
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let suffix = p
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
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
    let tag = if index == 0 {
        format!(".backup-{stamp}")
    } else {
        format!(".backup-{stamp}-{index}")
    };
    format!("{stem}{tag}{suffix}")
}

/// `_choose_backup_name`: offers the default when prompting is possible.
pub fn choose_backup_name(
    original: &str,
    backup: Option<&str>,
    index: usize,
    prompt: bool,
    prompter: &dyn Prompter,
) -> String {
    let default = backup_name(original, backup, index);
    if backup.is_some_and(|b| !b.is_empty()) || index > 0 || !prompt || !prompter.interactive() {
        return default;
    }
    let answer = prompter.ask(&format!("  backup name [{default}]: "));
    let answer = fsv_core::pystr::strip(&answer);
    if answer.is_empty() {
        default
    } else {
        answer.to_string()
    }
}

/// `_should_backup_replaced`.
pub fn should_backup_replaced(
    name: &str,
    backup_replaced: Option<bool>,
    prompter: &dyn Prompter,
) -> bool {
    if let Some(b) = backup_replaced {
        return b;
    }
    if !prompter.interactive() {
        return false;
    }
    let answer = casefold(fsv_core::pystr::strip(
        &prompter.ask(&format!("  backup old {name} before replace? [Y/n]: ")),
    ));
    !matches!(answer.as_str(), "n" | "no")
}

/// `_backup_attachment`: re-upload an attachment's bytes under `name`.
pub fn backup_attachment(c: &dyn Api, att: &Value, name: &str) -> Result<i64> {
    let url = attachment_url(att, &c.domain());
    if url.is_empty() {
        return Err(api0(format!(
            "cannot backup attachment with no URL: {}",
            py_repr(att)
        )));
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
    let Some(rows) = data
        .get("change_planning_fields")
        .and_then(Value::as_array)
        .filter(|_| data.is_object())
    else {
        return Err(api0(
            "planning rows unavailable; cannot safely determine the write target",
        ));
    };
    if rows.iter().any(|r| !r.is_object()) {
        return Err(api0(
            "invalid planning row; cannot safely determine the write target",
        ));
    }
    let existing = match_planning_row(&json!({"name": field_id, "label": label}), rows)?;
    if existing.is_none()
        && rows
            .iter()
            .any(|r| r.get_truthy("name").is_none() && r.get_truthy("label").is_none())
    {
        return Err(api0(
            "unidentified planning row; cannot safely determine the write target",
        ));
    }
    let mut field_id = field_id.to_string();
    if let Some(n) = existing.as_ref().and_then(|e| e.get_truthy("name")) {
        field_id = py_str(n);
    }
    let attachments = existing
        .as_ref()
        .map(|e| e.list_or_empty("attachments").to_vec())
        .unwrap_or_default();
    let ids: Vec<i64> = attachments.iter().filter_map(attachment_id).collect();
    let mut description = description.map(str::to_string);
    if description.is_none()
        && !file_paths.is_empty()
        && planning_description(existing.as_ref()).is_empty()
    {
        description = Some(default_planning_description(file_paths));
    }
    let mode = opts.mode()?;
    let (ids, skipped, changed) =
        merge_uploads(c, &attachments, ids, file_paths, &mode, opts, prompter)?;
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
        return c.int_put(
            &format!("changes/{change_id}/planning-fields/{field_id}"),
            &Value::Object(body),
        );
    }
    let final_description = description
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| default_planning_description(file_paths));
    let mut body = obj(json!({"description": final_description}));
    if !ids.is_empty() {
        body.insert("attachments".into(), json!(ids));
    }
    c.int_post(
        &format!("changes/{change_id}/planning-fields?id={field_id}"),
        &Value::Object(body),
    )
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
    static UNSAFE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"[^a-zA-Z0-9_-]+").expect("regex"));
    let io = |e: std::io::Error| Error::other(e.to_string());
    let data = c.int_get(&format!("changes/{change_id}/planning-fields"), None)?;
    let out = match out_dir {
        Some(d) => d.to_path_buf(),
        None => std::env::current_dir()
            .map_err(io)?
            .join(format!("CHN-{change_id}")),
    };
    std::fs::create_dir_all(&out).map_err(io)?;
    let mut downloaded = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for field in rows_at(&data, "change_planning_fields") {
        let name = field.get_truthy("name").map(py_str);
        let label =
            fsv_core::pystr::strip(&s(field.get_truthy("label").or(field.get("name")))).to_string();
        let Some(name) = name else { continue };
        if field_names.is_some_and(|names| !names.contains(&name)) {
            continue;
        }
        let description = fsv_core::pystr::strip(&s(field.get("description"))).to_string();
        let attachments = field.list_or_empty("attachments");
        if description.is_empty() && attachments.is_empty() {
            continue;
        }
        let safe = UNSAFE
            .replace_all(&label, "_")
            .trim_matches('_')
            .to_string();
        if include_description && !description.is_empty() {
            let md = out.join(format!(
                "{}.md",
                if safe.is_empty() { &name } else { &safe }
            ));
            let md_name = md
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if !seen.contains(&md_name) {
                std::fs::write(&md, &description).map_err(io)?;
                downloaded.push(md);
                seen.insert(md_name);
            }
        }
        for att in attachments {
            let Some(url) = att
                .get_truthy("attachment_url")
                .or_else(|| att.get_truthy("canonical_url"))
                .map(py_str)
            else {
                continue;
            };
            let att_name = att
                .get("name")
                .map(py_str)
                .unwrap_or_else(|| "attachment".into());
            let final_name = if safe.is_empty() {
                att_name
            } else {
                format!("{safe}_{att_name}")
            };
            let mut path = out.join(&final_name);
            let mut counter = 1;
            while seen.contains(
                &path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            ) {
                let fname = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let (stem, suffix) = stem_suffix(&fname);
                path = out.join(format!("{stem}_{counter}{suffix}"));
                counter += 1;
            }
            seen.insert(
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            );
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
            if let Some(m) = data
                .as_object()
                .filter(|m| m.contains_key("associated_assets"))
            {
                return Ok(m
                    .get("associated_assets")
                    .filter(|v| truthy(v))
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default());
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
pub fn search_assets_for_change(
    c: &dyn Api,
    change_id: i64,
    query: &str,
    page: i64,
    per_page: i64,
) -> Result<Value> {
    let mut params = obj(
        json!({"entity": "change", "entity_id": change_id, "include_meta": "true", "page": page, "per_page": per_page}),
    );
    if !query.is_empty() {
        params.insert("search_term".into(), json!(query));
    }
    c.int_get("assets-to-associate", Some(params))
}

/// `associate_assets`: asset display ids onto a change.
pub fn associate_assets(c: &dyn Api, change_id: i64, asset_ids: &[i64]) -> Result<()> {
    c.int_put(
        &format!("changes/{change_id}/assets/associate"),
        &json!({"item_ids": asset_ids}),
    )?;
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
        let Some((_, cmdb_id)) = by_display
            .iter()
            .find(|(k, _)| *k == json!(did))
            .filter(|(_, v)| !v.is_null())
        else {
            return Err(Error::value(format!(
                "asset {did} not found on change #{change_id}"
            )));
        };
        c.int_put(
            &format!("changes/{change_id}/assets/detach"),
            &json!({"cmdb_request_id": cmdb_id}),
        )?;
    }
    Ok(())
}

/// `get_change_associations`: paginated rows plus completeness (never a false empty).
pub fn get_change_associations(c: &dyn Api, change_id: i64) -> Result<Value> {
    crate::associations::inventory(c, change_id, crate::associations::KINDS)
}

/// `search_change_tickets`.
pub fn search_change_tickets(c: &dyn Api, query: &str) -> Result<Vec<Value>> {
    let data = c.int_get(
        "changes/tickets/search",
        Some(obj(json!({"module_type": "change", "q": query}))),
    )?;
    Ok(rows_at(&data, "tickets"))
}

/// `get_change_activities`: follows the `start_token` of `next_page_url`.
pub fn get_change_activities(c: &dyn Api, change_id: i64) -> Result<Vec<Value>> {
    static TOKEN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"start_token=(\S+)").expect("regex"));
    let mut all = Vec::new();
    let mut params = Map::new();
    loop {
        let data = c.int_get(
            &format!("changes/{change_id}/activities"),
            Some(params.clone()),
        )?;
        let acts = rows_at(&data, "activities");
        let empty = acts.is_empty();
        all.extend(acts);
        let Some(next) = data.get_truthy("next_page_url").map(py_str) else {
            break;
        };
        if empty {
            break;
        }
        let Some(m) = TOKEN.captures(&next) else {
            break;
        };
        params = obj(json!({"start_token": &m[1]}));
    }
    Ok(all)
}

/// `associate_ticket`: one reviewed, journaled PUT.
pub fn associate_ticket(
    c: &dyn Api,
    journal: &dyn Journal,
    change_id: &str,
    ticket_ids: &[Value],
    opts: crate::associations::SendOpts<'_>,
) -> Result<Value> {
    let plan = crate::associations::build_plan(change_id, ticket_ids, false, &c.domain())?;
    crate::associations::send(&plan, c, journal, opts)
}

/// `dissociate_ticket`: one reviewed, journaled DELETE.
pub fn dissociate_ticket(
    c: &dyn Api,
    journal: &dyn Journal,
    change_id: &str,
    ticket_id: &Value,
    opts: crate::associations::SendOpts<'_>,
) -> Result<Value> {
    let plan = crate::associations::build_plan(
        change_id,
        std::slice::from_ref(ticket_id),
        true,
        &c.domain(),
    )?;
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
    task_put(
        c,
        &format!("changes/{change_id}/tasks/update"),
        &json!({"id": task_id.to_string(), "action": "delete"}),
    )?;
    Ok(())
}

/// `update_task`: only the given fields (read-only/null custom fields dropped).
pub fn update_task(
    c: &dyn Api,
    change_id: i64,
    task_id: i64,
    body: &Map<String, Value>,
) -> Result<Value> {
    let mut payload = body.clone();
    let cf = payload.shift_remove("custom_fields");
    if let Some(cf) = cf.filter(truthy) {
        let kept: Map<String, Value> = cf
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(k, v)| !TASK_READONLY_CF.contains(&k.as_str()) && !v.is_null())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        payload.insert("custom_fields".into(), Value::Object(kept));
    }
    let data = task_put(
        c,
        &format!("changes/{change_id}/tasks/{task_id}"),
        &Value::Object(payload),
    )?;
    Ok(unwrap_key(data, "task"))
}

#[cfg(test)]
#[path = "create_failure_tests.rs"]
mod failure_tests;

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
            Script {
                tty,
                answers: RefCell::new(answers.iter().rev().map(|s| s.to_string()).collect()),
                prompts: RefCell::default(),
            }
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

    use std::sync::LazyLock;

    /// Outputs of `port/golden/oracles/fsv_oracle/o_create.py` (the Python functions).
    static ORACLE: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(include_str!("testdata/create_oracle.json")).unwrap()
    });
    static FILES: LazyLock<String> = LazyLock::new(|| {
        let dir = std::env::temp_dir().join("fsv_api_create_files");
        std::fs::create_dir_all(&dir).unwrap();
        for (n, b) in [
            ("a.txt", &b"hello"[..]),
            ("b.txt", b"world!"),
            ("rollout.txt", b"steps"),
        ] {
            std::fs::write(dir.join(n), b).unwrap();
        }
        dir.to_string_lossy().into_owned()
    });

    fn want(key: &str) -> Value {
        ORACLE[key].clone()
    }

    fn want_err(key: &str) -> String {
        ORACLE[key]["msg"].as_str().unwrap().to_string()
    }

    fn err<T: std::fmt::Debug>(r: Result<T>) -> String {
        r.expect_err("an error").to_string()
    }

    fn file(n: &str) -> String {
        format!("{}/{n}", *FILES)
    }

    fn tmp(sub: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fsv_api_create_{sub}"));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn writes(a: &FakeApi) -> Value {
        json!(a.writes().iter().filter(|c| c.method != "FETCH").map(|c| json!({"method": c.method, "path": c.path, "params": c.params, "body": c.body})).collect::<Vec<_>>())
    }

    /// `[method, path, uploaded file name | body]` per call (the oracle's projection).
    fn trail(a: &FakeApi) -> Value {
        json!(
            a.calls
                .lock()
                .unwrap()
                .iter()
                .map(|c| {
                    let third = if c.method == "POST" && c.path == "attachments" {
                        c.body.as_ref().unwrap()["content_file_name"].clone()
                    } else {
                        json!(c.body)
                    };
                    json!([c.method, c.path, third])
                })
                .collect::<Vec<_>>()
        )
    }

    fn schema() -> Vec<Value> {
        let pri: Vec<Value> = (1..7)
            .map(|i| json!({"id": i, "value": format!("P{i}")}))
            .collect();
        serde_json::from_value(json!([
            {"name": "subject", "field_type": "default_subject", "required": true, "default_field": true},
            {"name": "description", "field_type": "default_description", "required": true, "default_field": true},
            {"name": "requester", "field_type": "default_requester", "required": true, "default_field": true},
            {"name": "agent", "field_type": "default_agent", "required": false, "default_field": true},
            {"name": "status", "field_type": "default_status", "required": true, "default_field": true,
             "choices": [{"id": 1, "value": "Open"}, {"id": 2, "value": "Planning"}]},
            {"name": "priority", "field_type": "default_priority", "required": false, "default_field": true, "choices": pri},
            {"name": "cf_flag", "field_type": "custom_checkbox"},
            {"name": "cf_yes", "field_type": "custom_dropdown", "choices": [{"id": "y", "value": "true"}, {"id": "n", "value": "false"}]},
            {"name": "cf_named", "field_type": "custom_dropdown", "choices": [{"name": "N1"}, {"id": 7}]},
            {"name": "planned_start_date", "field_type": "default_date", "default_field": true},
            {"name": "cf_num", "field_type": "custom_number"},
            {"name": "cf_dec", "field_type": "custom_decimal"},
            {"name": "group", "field_type": "default_group", "required": true},
            {"name": "department", "field_type": "default_department"},
            {"name": "cf_multi", "field_type": "custom_multi_select_dropdown"},
            {"name": "cf_ref_id", "field_type": "custom_text"},
            {"name": "cf_text", "field_type": "custom_text", "required": true},
            {"name": "id", "field_type": "default_id", "required": true},
            {"name": "cf_note", "field_type": "custom_content"},
            {"name": "cf_static", "field_type": "custom_static_rich_text"},
            {"name": "", "field_type": "custom_text"},
            {"name": "reason_for_change", "field_type": "planning_field", "label": "Reason for change", "id": 11},
            {"name": "rollout_plan", "field_type": "planning_field", "label": "Rollout  Plan", "id": 12, "export_label": "Rollout plans"},
            {"name": "backout", "field_type": "default_backout_plan", "label": "Backout &amp; Recovery", "id": 13, "helptext": "Back out steps"},
        ]))
        .unwrap()
    }

    #[test]
    fn templates_and_defaults() {
        let sf = schema();
        let got: Vec<Value> = ["required", "optional", "all"]
            .iter()
            .map(|l| json!(build_template(&sf, l)))
            .collect();
        assert_eq!(json!(got), want("build_template"));
        let mut defaults: Vec<Value> = sf.iter().map(field_default).collect();
        defaults.push(field_default(
            &json!({"field_type": "custom_text", "choices": [{"id": true}, {"id": false}]}),
        ));
        assert_eq!(json!(defaults), want("field_default"));
        let schema = json!({"fields": sf});
        assert_eq!(
            json!([
                change_template(&schema, "required"),
                change_template(&schema, "all") == build_template(&sf, "all")
            ]),
            want("change_template")
        );
        assert_eq!(
            json!([change_schema_fields(&schema) == sf]),
            want("schema_fields")
        );
        assert_eq!(
            json!(change_schema_fields(
                &json!({"data": {"change_fields": [{"name": "x"}]}})
            )),
            want("schema_fields_data")
        );
        assert_eq!(
            json!(change_schema_fields(
                &json!({"fields": [], "change_fields": [{"name": "y"}]})
            )),
            want("schema_fields_flat")
        );
    }

    fn change() -> Value {
        json!({"change": {"id": 5, "display_id": 5, "subject": "S", "status": 3, "attachments": [1], "stats": {}, "updated_at": "x",
                          "attached_risk_survey_response_status": "completed",
                          "reason_planning_fields": [], "requester_id": 9, "planning_field_definitions": [],
                          "custom_fields": {"cf_lookup": {"id": 4, "value": "Four"}, "cf_text": "t", "cf_obj": {"id": 1, "value": 2, "x": 3}}}})
    }

    #[test]
    fn clone_data_and_edit_view() {
        let a = FakeApi::new().ok("GET changes/5", change());
        assert_eq!(json!(change_clone_data(&a, 5).unwrap()), want("clone_data"));
        assert_eq!(json!(get_change_for_edit(&a, 5).unwrap()), want("for_edit"));
    }

    #[test]
    fn parse_when_instants() {
        let vals = [
            json!("2030-01-01T10:00:00Z"),
            json!("2030-01-01T10:00:00+07:00"),
            json!("2030-01-01 10:00:00"),
            json!(""),
            json!("junk"),
            json!(5),
            Value::Null,
        ];
        let w = want("parse_when");
        for (v, w) in vals.iter().zip(w.as_array().unwrap()) {
            let expect = match w.as_str().unwrap() {
                "None" => None,
                t => Some(t.replace(' ', "T").parse::<jiff::Timestamp>().unwrap()),
            };
            assert_eq!(parse_when(v), expect, "{v}");
        }
    }

    fn task() -> Value {
        json!({"id": 70, "title": "Deploy", "status": 3, "owner_id": 4, "agent_id": null, "due_date": "2029-12-01T00:00:00Z", "planned_end_date": "2031-01-01T00:00:00Z",
               "start_date": "junk", "created_at": "x", "stack_rank": 1, "description": "d", "custom_fields": {"a": null, "b": 2}})
    }

    fn task2() -> Value {
        json!({"id": 71, "title": "", "owner": {"id": 1}, "custom_fields": {"a": null}})
    }

    fn now() -> Option<jiff::Timestamp> {
        Some("2030-01-01T00:00:00Z".parse().unwrap())
    }

    #[test]
    fn task_clone_body_and_clone_tasks() {
        let (mut d1, mut d2) = (Vec::new(), Vec::new());
        let got = json!([
            task_clone_body(&task(), false, now(), Some(&mut d1)),
            d1,
            task_clone_body(&task(), true, now(), None),
            task_clone_body(&task2(), false, now(), Some(&mut d2)),
            d2,
        ]);
        assert_eq!(got, want("task_clone_body"));

        let a = FakeApi::new()
            .ok("GET changes/5/tasks", json!({"tasks": [task(), task2()]}))
            .ok("POST changes/6/tasks", json!({"task": {"id": 1}}))
            .ok("POST changes/6/tasks", json!({"id": 2}));
        let mut d = Vec::new();
        let out = clone_tasks(&a, 5, 6, false, now(), Some(&mut d)).unwrap();
        assert_eq!(json!([out, d, writes(&a)]), want("clone_tasks"));
    }

    fn plan_rows() -> Value {
        json!({"change_planning_fields": [
            {"name": "reason_for_change", "label": "Reason", "description": " why ", "attachments": [
                {"id": 1, "name": "r.txt", "attachment_url": "https://acme.freshservice.com/helpdesk/attachments/1", "content_type": "text/plain; charset=utf-8"},
                {"id": 2, "name": "gone.txt", "canonical_url": "https://cdn/x?sig=1"},
                {"id": 3, "name": "nourl.txt"}]},
            {"name": "rollout_plan", "label": "Rollout plan", "description": "", "attachments": []},
            {"name": "backout", "label": "Backout", "description": "back", "attachments": []},
            {"name": "", "description": "orphan"}]})
    }

    fn assets() -> Value {
        json!({"associated_assets": [{"id": 100, "display_id": 7, "config_item": {"display_id": 7}}, {"id": 101, "config_item": {"display_id": 8}}, {"id": 102}]})
    }

    #[test]
    fn clone_side_effects_assets_and_planning() {
        let a = FakeApi::new()
            .ok("GET changes/5/tasks", json!({"tasks": [task(), task2()]}))
            .ok("GET changes/5/planning-fields", plan_rows())
            .ok("GET changes/5/assets", assets());
        assert_eq!(
            json!(clone_side_effects(&a, 5).unwrap()),
            want("side_effects")
        );

        let a = FakeApi::new()
            .ok("GET changes/5/assets", assets())
            .ok("PUT changes/6/assets/associate", json!({}));
        assert_eq!(
            json!([clone_assets(&a, 5, 6).unwrap(), writes(&a)]),
            want("clone_assets")
        );
        let a = FakeApi::new().ok("GET changes/5/assets", json!({"associated_assets": []}));
        assert_eq!(
            json!([clone_assets(&a, 5, 6).unwrap(), writes(&a)]),
            want("clone_assets_none")
        );

        let a = FakeApi::new()
            .ok("GET changes/5/planning-fields", plan_rows())
            .on(
                "FETCH https://acme.freshservice.com/helpdesk/attachments/1?download=true",
                Reply::Raw(200, b"R!".to_vec()),
            )
            .on("FETCH https://cdn/x?sig=1", Reply::Status(403))
            .ok("POST attachments", json!({"attachment": {"id": 900}}))
            .ok(
                "POST changes/6/planning-fields?id=reason_for_change",
                json!({}),
            )
            .ok("POST changes/6/planning-fields?id=backout", json!({}));
        let out = clone_planning_fields(&a, 5, 6).unwrap();
        let calls: Vec<Value> = a
            .calls
            .lock()
            .unwrap()
            .iter()
            .map(|c| {
                let mut body = c.body.clone();
                if let Some(Value::Object(m)) = body.as_mut() {
                    m.shift_remove("content_updated_at");
                }
                json!([c.method, c.path, body])
            })
            .collect();
        let w = want("clone_planning");
        let wcalls: Vec<Value> = w[1]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                json!([
                    c["method"],
                    c["path"],
                    c.get("body").cloned().unwrap_or(Value::Null)
                ])
            })
            .collect();
        assert_eq!(json!([out, calls]), json!([w[0], wcalls]));
    }

    #[test]
    fn attachment_url_and_safe_filename() {
        let atts = [
            json!({"attachment_url": "https://x/helpdesk/attachments/1"}),
            json!({"attachment_url": "https://x/helpdesk/attachments/1?a=1"}),
            json!({"canonical_url": "https://y/z"}),
            json!({"canonical_path": "/helpdesk/attachments/9"}),
            json!({}),
        ];
        assert_eq!(
            json!(
                atts.iter()
                    .map(|a| attachment_url(a, "acme.freshservice.com"))
                    .collect::<Vec<_>>()
            ),
            want("attachment_url")
        );
        assert_eq!(
            json!(
                [" a/b\0c ", "  ", "ok.txt"]
                    .iter()
                    .map(|n| safe_filename(n))
                    .collect::<Vec<_>>()
            ),
            want("safe_filename")
        );
    }

    #[test]
    fn download_attachment_cases() {
        let dir = tmp("dl");
        let a = FakeApi::new().on("FETCH https://x/f1", Reply::Raw(200, b"one".to_vec()));
        let one = json!({"name": "one.txt", "attachment_url": "https://x/f1"});
        let mut out = vec![
            download_attachment(&a, &one, &dir, false).unwrap(),
            download_attachment(&a, &one, &dir, false).unwrap(),
            download_attachment(&a, &one, &dir, true).unwrap(),
            download_attachment(
                &a,
                &json!({"id": 4, "attachment_url": "https://x/f1"}),
                &dir,
                false,
            )
            .unwrap(),
        ];
        for o in &mut out {
            let base = Path::new(o["path"].as_str().unwrap())
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            o["path"] = json!(base);
        }
        let paths: Vec<String> = a
            .calls
            .lock()
            .unwrap()
            .iter()
            .map(|c| c.path.clone())
            .collect();
        assert_eq!(json!([out, paths]), want("download"));
        let bad = FakeApi::new().on("FETCH https://x/bad", Reply::Raw(500, b"err body".to_vec()));
        assert_eq!(
            err(download_attachment(
                &bad,
                &json!({"name": "z.txt", "attachment_url": "https://x/bad"}),
                &dir,
                false
            )),
            want_err("download_bad")
        );
        assert_eq!(
            err(download_attachment(
                &FakeApi::new(),
                &json!({"id": 1}),
                &dir,
                false
            )),
            want_err("download_nourl")
        );
    }

    #[test]
    fn reupload_and_upload() {
        let a = FakeApi::new()
            .on("FETCH https://x/a", Reply::Raw(200, b"abc".to_vec()))
            .ok("POST attachments", json!({"id": 5}))
            .ok("POST attachments", json!("notadict"));
        let out = json!([
            reupload_attachment(
                &a,
                &json!({"attachment_url": "https://x/a", "name": "a.bin"})
            )
            .unwrap(),
            reupload_attachment(&a, &json!({"attachment_url": "https://x/a"})).unwrap(),
            reupload_attachment(&a, &json!({})).unwrap(),
            reupload_attachment(&a, &json!({"attachment_url": "https://x/none"})).unwrap(),
        ]);
        let bodies: Vec<Value> = a
            .writes()
            .iter()
            .filter(|c| c.method == "POST")
            .map(|c| {
                let mut b = c.body.clone().unwrap();
                b.as_object_mut()
                    .unwrap()
                    .shift_remove("content_updated_at");
                b
            })
            .collect();
        assert_eq!(json!([out, bodies]), want("reupload"));

        let a = FakeApi::new().ok(
            "POST attachments",
            json!({"id": "12", "content_file_size": 3, "content_file_name": "n.txt"}),
        );
        assert_eq!(
            json!([
                upload_bytes(&a, "n.txt", b"xyz", "text/plain").unwrap(),
                writes(&a)
            ]),
            want("upload_bytes")
        );
        assert_eq!(
            err(upload_bytes(
                &FakeApi::new().ok("POST attachments", json!({"name": "n.txt"})),
                "n.txt",
                b"xyz",
                "text/plain"
            )),
            want_err("upload_bytes_noid")
        );
        let a = FakeApi::new().ok("POST attachments", json!({"id": 33}));
        assert_eq!(
            json!([upload_file(&a, &file("a.txt")).unwrap(), a.writes()[0].body]),
            want("upload_file")
        );
    }

    fn exist_api() -> FakeApi {
        FakeApi::new()
            .ok("GET changes/5", json!({"change": {"id": 5, "attachments": [{"id": 1, "name": "a.txt", "attachment_url": "https://x/old-a", "content_type": "text/plain"}, {"id": 2, "content_file_name": "c.txt"}, {"name": "noid.txt"}]}}))
            .ok("POST attachments", json!({"id": 50}))
            .ok("POST attachments", json!({"id": 51}))
            .ok("POST attachments", json!({"id": 52}))
            .ok("PUT changes/5", json!({}))
            .on("FETCH https://x/old-a", Reply::Raw(200, b"old".to_vec()))
    }

    fn dup(mode: Option<&str>, backup: Option<bool>, name: Option<&str>) -> DuplicateOpts {
        DuplicateOpts {
            duplicate: mode.map(str::to_string),
            backup_replaced: backup,
            backup_name: name.map(str::to_string),
        }
    }

    #[test]
    fn attach_files_to_change_modes() {
        let (a, b) = (file("a.txt"), file("b.txt"));
        let run = |paths: &[String], opts: DuplicateOpts, script: Script| -> Value {
            let api = exist_api();
            match attach_files_to_change(&api, 5, paths, &opts, &script) {
                Ok(v) => json!([v, trail(&api)]),
                Err(e) => json!({"msg": e.to_string()}),
            }
        };
        let no = || Script::new(false, &[]);
        let one = [a.clone()];
        assert_eq!(
            run(
                &[a.clone(), b.clone()],
                dup(Some("append"), None, None),
                no()
            ),
            want("attach_append")
        );
        assert_eq!(
            run(&one, dup(Some("SKIP"), None, None), no()),
            want("attach_skip")
        );
        assert_eq!(
            run(&one, dup(Some("replace"), Some(false), None), no()),
            want("attach_replace")
        );
        assert_eq!(
            run(
                &one,
                dup(Some("replace"), Some(true), Some("old.txt")),
                no()
            ),
            want("attach_replace_backup")
        );
        assert_eq!(
            run(&one, dup(None, None, None), no())["msg"],
            want("attach_prompt_notty")["msg"]
        );
        assert_eq!(
            run(&one, dup(None, None, None), Script::new(true, &["r", "n"])),
            want("attach_prompt_tty")
        );
        assert_eq!(
            run(
                &one,
                dup(None, None, None),
                Script::new(true, &["overwrite", "", "mine.txt"])
            ),
            want("attach_prompt_tty_backup")
        );
        assert_eq!(
            run(&one, dup(None, None, None), Script::new(true, &["a"])),
            want("attach_prompt_tty_append")
        );
        assert_eq!(
            run(
                &one,
                dup(None, None, None),
                Script::new(true, &["whatever"])
            ),
            want("attach_prompt_tty_skip")
        );
        assert_eq!(
            run(&one, dup(Some("nope"), None, None), no())["msg"],
            want("attach_bad_mode")["msg"]
        );
        let api = FakeApi::new().ok("PATCH changes/5", json!({"ok": 1}));
        assert_eq!(
            json!([set_due_by(&api, 5, "2030-01-01").unwrap(), writes(&api)]),
            want("set_due_by")
        );
    }

    fn body() -> Map<String, Value> {
        obj(
            json!({"subject": "S", "cf_text": "x", "group_id": {"id": 3, "value": "G"}, "planned_start_date": "d", "attachments": [1], "unknown": 1}),
        )
    }

    #[test]
    fn update_and_submit_change() {
        let sf = schema();
        let a = FakeApi::new().ok("PUT changes/5", json!({"change": {"id": 5}}));
        assert_eq!(
            json!([update_change(&a, 5, &body(), &sf).unwrap(), writes(&a)]),
            want("update_change")
        );
        let put = |r: Reply| FakeApi::new().on("PUT changes/5", r);
        assert_eq!(
            err(update_change(
                &put(Reply::Fail(
                    400,
                    json!({"errors": [{"field": "subject", "message": "bad"}, {}]})
                )),
                5,
                &body(),
                &sf
            )),
            want_err("update_change_errs")
        );
        assert_eq!(
            err(update_change(
                &put(Reply::Fail(
                    400,
                    json!({"description": "Validation failed"})
                )),
                5,
                &body(),
                &sf
            )),
            want_err("update_change_desc")
        );
        assert_eq!(
            err(update_change(
                &put(Reply::Fail(400, json!({"code": "x"}))),
                5,
                &body(),
                &sf
            )),
            want_err("update_change_dict")
        );
        assert_eq!(
            err(update_change(
                &put(Reply::Fail(500, json!("boom"))),
                5,
                &body(),
                &sf
            )),
            want_err("update_change_raw")
        );

        let a = FakeApi::new().ok("POST changes", json!({"change": {"id": 9}}));
        assert_eq!(
            json!([submit_change(&a, &body(), &sf).unwrap(), writes(&a)]),
            want("submit_change")
        );
        assert_eq!(
            err(submit_change(
                &FakeApi::new(),
                &obj(json!({"br_validation_excludes": "x"})),
                &sf
            )),
            want_err("submit_change_br")
        );
        let post = |r: Reply| FakeApi::new().on("POST changes", r);
        assert_eq!(
            err(submit_change(
                &post(Reply::Fail(
                    400,
                    json!({"errors": [{"field": "f", "message": "m"}]})
                )),
                &body(),
                &sf
            )),
            want_err("submit_change_errs")
        );
        assert_eq!(
            err(submit_change(
                &post(Reply::Fail(400, json!({"description": "D"}))),
                &body(),
                &sf
            )),
            want_err("submit_change_desc")
        );
    }

    #[test]
    fn api_failure_and_raw_bodies() {
        let got: Vec<Value> = [
            json!({"errors": [{"field": "a", "message": "b"}]}),
            json!({"description": "D"}),
            json!({"x": 1}),
            json!("text"),
        ]
        .into_iter()
        .map(|b| {
            let e = api_failure("create", ApiError::new(400, b).into());
            json!([
                if e.as_api().is_some() {
                    "APIError"
                } else {
                    "RuntimeError"
                },
                e.to_string()
            ])
        })
        .collect();
        assert_eq!(json!(got), want("api_failure"));
        assert_eq!(
            api_body(api_failure("x", ApiError::new(400, json!("text")).into())),
            json!("text")
        );

        let a = FakeApi::new().ok("POST changes", json!({"change": {"id": 9}}));
        assert_eq!(
            json!([post_change_body(&a, &json!({"a": 1})).unwrap(), writes(&a)]),
            want("post_change_body")
        );
        assert_eq!(
            post_change_body(
                &FakeApi::new().ok("POST changes", json!([1, 2])),
                &json!({"a": 1})
            )
            .unwrap(),
            want("post_change_body_list")
        );
        assert_eq!(
            err(post_change_body(
                &FakeApi::new().on(
                    "POST changes",
                    Reply::Fail(400, json!({"description": "D"}))
                ),
                &json!({"a": 1})
            )),
            want_err("post_change_body_err")
        );
        let a = FakeApi::new().ok("PUT changes/5", json!({"id": 5}));
        assert_eq!(
            json!([
                put_change_body(&a, 5, &json!({"a": 1})).unwrap(),
                writes(&a)
            ]),
            want("put_change_body")
        );
        assert_eq!(
            err(put_change_body(
                &FakeApi::new().on(
                    "PUT changes/5",
                    Reply::Fail(422, json!({"errors": [{"field": "a"}]}))
                ),
                5,
                &json!({"a": 1})
            )),
            want_err("put_change_body_err")
        );
    }

    #[test]
    fn planning_keys_and_labels() {
        let keys = [
            json!("Planning-Field-cf_Rollout_Plans"),
            json!("cf_reason"),
            json!("Backout & Recovery"),
            json!(""),
            Value::Null,
            json!(12),
        ];
        assert_eq!(
            json!(keys.iter().map(planning_key).collect::<Vec<_>>()),
            want("planning_key")
        );
        let mut sk: Vec<String> = keys.iter().map(singular_key).collect();
        sk.push(singular_key(&json!("plans-items s")));
        assert_eq!(json!(sk), want("singular_key"));
        let sf = schema();
        assert_eq!(
            json!(
                sf[sf.len() - 4..]
                    .iter()
                    .map(is_planning_field)
                    .collect::<Vec<_>>()
            ),
            want("is_planning_field")
        );
        assert_eq!(
            json!([
                norm_label(Some(&json!("  Backout &amp;  Recovery "))),
                norm_label(None),
                norm_label(Some(&json!(5)))
            ]),
            want("norm_label")
        );
        let mut with_junk = sf.clone();
        with_junk.push(json!("junk"));
        assert_eq!(
            json!(
                planning_definitions(&with_junk)
                    .iter()
                    .map(|f| f["name"].clone())
                    .collect::<Vec<_>>()
            ),
            want("planning_definitions")
        );
        let defs = [
            json!({"name": "a", "id": 1}),
            json!({"id": 2}),
            json!({"name": "", "id": 3}),
        ];
        assert_eq!(
            json!(defs.iter().map(planning_server_key).collect::<Vec<_>>()),
            want("planning_server_key")
        );
    }

    #[test]
    fn resolve_planning() {
        let sf = schema();
        let vals = [
            "reason_for_change",
            "12",
            "rollout plan",
            "backout &amp; recovery",
            "planning-field-cf_rollout_plans",
            "cf_reason_for_change",
            "Back out step",
        ];
        let got: Vec<Value> = vals
            .iter()
            .map(|v| resolve_planning_definition(v, &sf).unwrap()["name"].clone())
            .collect();
        assert_eq!(json!(got), want("resolve_def"));
        assert_eq!(
            err(resolve_planning_definition("zzz", &sf)),
            want_err("resolve_def_missing")
        );
        assert_eq!(
            err(resolve_planning_definition("  ", &sf)),
            want_err("resolve_def_blank")
        );
        let amb: Vec<Value> = serde_json::from_value(json!([{"name": "a", "field_type": "planning_field", "label": "Same"}, {"name": "b", "field_type": "planning_field", "label": "same"},
                                                            {"name": "plan_x", "field_type": "planning_field", "label": "X"}, {"name": "plans_x", "field_type": "planning_field", "label": "Y"}])).unwrap();
        assert_eq!(
            err(resolve_planning_definition("SAME", &amb)),
            want_err("resolve_def_amb_label")
        );
        assert_eq!(
            err(resolve_planning_definition("planx", &amb)),
            want_err("resolve_def_amb_alias")
        );
        let got = json!([
            resolve_planning_field("Rollout plan", &sf).unwrap(),
            resolve_planning_field("13", &[json!({"id": 13, "field_type": "planning_field"})])
                .unwrap()
        ]);
        assert_eq!(got, want("resolve_field"));
    }

    #[test]
    fn match_rows_and_descriptions() {
        let rows: Vec<Value> = serde_json::from_value(json!([{"name": "r1", "label": "Reason", "field_id": 11}, {"name": "r2", "label": "Plan", "planning_field_id": 12},
                                                             {"id": 13, "label": "Backout"}, {"label": "dup"}, {"label": "Dup"}, "junk"])).unwrap();
        let defs = [
            json!({"name": "r1"}),
            json!({"id": 12}),
            json!({"id": "13"}),
            json!({"label": " reason "}),
            json!({"label": "Plan", "id": 99}),
            json!({"name": "zz", "label": "nothing"}),
            json!({}),
        ];
        assert_eq!(
            json!(
                defs.iter()
                    .map(|d| match_planning_row(d, &rows).unwrap())
                    .collect::<Vec<_>>()
            ),
            want("match_row")
        );
        assert_eq!(
            err(match_planning_row(&json!({"label": "DUP"}), &rows)),
            want_err("match_row_amb")
        );
        assert_eq!(
            err(match_planning_row(&json!({"name": "r1", "id": 99}), &rows)),
            want_err("match_row_conflict")
        );

        let descs = [
            None,
            Some(json!({})),
            Some(json!({"description": "  "})),
            Some(json!({"description": 5, "body": " b "})),
            Some(json!({"content": "c"})),
            Some(json!({"description_text": "dt", "content": "c"})),
        ];
        assert_eq!(
            json!(
                descs
                    .iter()
                    .map(|d| planning_description(d.as_ref()))
                    .collect::<Vec<_>>()
            ),
            want("planning_description")
        );
        assert_eq!(
            json!([
                default_planning_description(&[file("a.txt"), "/x/y/z.pdf".into()]),
                default_planning_description(&[])
            ]),
            want("default_planning_description")
        );
        let atts = [
            json!({"name": "n"}),
            json!({"content_file_name": "c"}),
            json!({"name": "", "content_file_name": "c"}),
            json!({}),
        ];
        assert_eq!(
            json!(atts.iter().map(attachment_name).collect::<Vec<_>>()),
            want("attachment_name")
        );
        let ids = [
            json!({"id": 3}),
            json!({"id": "4"}),
            json!({}),
            json!({"id": null}),
        ];
        assert_eq!(
            json!(ids.iter().map(attachment_id).collect::<Vec<_>>()),
            want("attachment_id")
        );
    }

    #[test]
    fn prompts_and_backup_names() {
        let mut got = vec![err(prompt_duplicate_action(
            "a.txt",
            2,
            &Script::new(false, &[]),
        ))];
        for ans in [
            "r",
            "Replace",
            "o",
            "overwrite",
            "a",
            "APPEND",
            "",
            "s",
            "x",
        ] {
            got.push(
                prompt_duplicate_action("a.txt", 2, &Script::new(true, &[&format!(" {ans} ")]))
                    .unwrap(),
            );
        }
        assert_eq!(json!(got), want("prompt_duplicate"));
        let s = Script::new(true, &["x"]);
        prompt_duplicate_action("a.txt", 2, &s).unwrap();
        assert_eq!(
            s.prompts.borrow()[0],
            "  duplicate a.txt (2 existing). Choose [s]kip, [r]eplace, [a]ppend: "
        );

        assert_eq!(
            json!([
                backup_name("a.txt", Some("b.tar.gz"), 0),
                backup_name("a.txt", Some("b.tar.gz"), 2),
                backup_name("a.txt", Some("plain"), 1)
            ]),
            want("backup_name_given")
        );
        let stamp = Regex::new(r"\d{8}-\d{6}").unwrap();
        let got: Vec<String> = [
            backup_name("dir/a.txt", None, 0),
            backup_name("a.txt", None, 3),
            backup_name("noext", None, 0),
        ]
        .iter()
        .map(|n| stamp.replace(n, "STAMP").into_owned())
        .collect();
        let w: Vec<String> = want("backup_name_stamp")
            .as_array()
            .unwrap()
            .iter()
            .map(|n| stamp.replace(n.as_str().unwrap(), "STAMP").into_owned())
            .collect();
        assert_eq!(got, w);

        let got = json!([
            choose_backup_name("a.txt", None, 0, true, &Script::new(true, &["  mine.txt "])),
            choose_backup_name(
                "a.txt",
                Some("given.txt"),
                0,
                true,
                &Script::new(true, &[""])
            ),
            choose_backup_name(
                "a.txt",
                Some("given.txt"),
                1,
                true,
                &Script::new(true, &[""])
            ),
            choose_backup_name(
                "a.txt",
                Some("given.txt"),
                0,
                true,
                &Script::new(false, &[])
            ),
        ]);
        assert_eq!(got, want("choose_backup_name"));

        let mut got = vec![
            should_backup_replaced("a", Some(true), &Script::new(false, &[])),
            should_backup_replaced("a", Some(false), &Script::new(false, &[])),
        ];
        got.push(should_backup_replaced("a", None, &Script::new(false, &[])));
        for ans in ["", "y", " N ", "no", "nope"] {
            got.push(should_backup_replaced(
                "a",
                None,
                &Script::new(true, &[ans]),
            ));
        }
        assert_eq!(json!(got), want("should_backup"));
    }

    #[test]
    fn backup_attachment_cases() {
        let a = FakeApi::new()
            .on("FETCH https://x/o", Reply::Raw(200, b"old".to_vec()))
            .ok("POST attachments", json!({"id": 61}));
        let w = want("backup_attachment");
        assert_eq!(
            json!([
                backup_attachment(&a, &json!({"attachment_url": "https://x/o"}), "old.txt")
                    .unwrap(),
                writes(&a)
            ]),
            json!([w[0], [w[1][1]]])
        );
        let a = FakeApi::new()
            .on("FETCH https://x/o", Reply::Raw(200, b"old".to_vec()))
            .ok("POST attachments", json!({"id": 61}));
        let id = backup_attachment(
            &a,
            &json!({"attachment_url": "https://x/o", "content_type": "image/png"}),
            "old.zzz",
        )
        .unwrap();
        assert_eq!(
            json!([
                id,
                a.writes()
                    .iter()
                    .find(|c| c.method == "POST")
                    .unwrap()
                    .body
                    .as_ref()
                    .unwrap()["content_content_type"]
            ]),
            want("backup_attachment_ct")
        );
        assert_eq!(
            err(backup_attachment(&FakeApi::new(), &json!({"id": 1}), "x")),
            want_err("backup_attachment_nourl")
        );
        assert_eq!(
            err(backup_attachment(
                &FakeApi::new().on("FETCH https://x/o", Reply::Raw(404, b"missing".to_vec())),
                &json!({"attachment_url": "https://x/o"}),
                "x"
            )),
            want_err("backup_attachment_fail")
        );
    }

    fn upf_rows() -> Value {
        json!({"change_planning_fields": [{"name": "rollout_plan", "label": "Rollout plan", "description": "", "attachments": [{"id": 1, "name": "a.txt", "attachment_url": "https://x/old-a"}]},
                                          {"name": "reason_for_change", "label": "Reason", "description": "why", "attachments": []}]})
    }

    fn upf(
        field: &str,
        rows: Value,
        description: Option<&str>,
        paths: &[String],
        opts: DuplicateOpts,
        label: Option<&str>,
    ) -> Value {
        let a = FakeApi::new()
            .ok("GET changes/5/planning-fields", rows)
            .ok("POST attachments", json!({"id": 70}))
            .ok("POST attachments", json!({"id": 71}))
            .on("FETCH https://x/old-a", Reply::Raw(200, b"old".to_vec()))
            .ok(
                "PUT changes/5/planning-fields/rollout_plan",
                json!({"ok": "put"}),
            )
            .ok(
                "PUT changes/5/planning-fields/reason_for_change",
                json!({"ok": "put"}),
            )
            .ok(
                "POST changes/5/planning-fields?id=backout",
                json!({"ok": "post"}),
            );
        match update_planning_field(
            &a,
            5,
            field,
            description,
            paths,
            &opts,
            label,
            &Script::new(false, &[]),
        ) {
            Ok(v) => json!([v, trail(&a)]),
            Err(e) => json!({"msg": e.to_string()}),
        }
    }

    #[test]
    fn update_planning_field_cases() {
        let (a, b, r) = (file("a.txt"), file("b.txt"), file("rollout.txt"));
        let append = || dup(Some("append"), None, None);
        let cases: Vec<(&str, Value)> = vec![
            (
                "upf_append",
                upf(
                    "rollout_plan",
                    upf_rows(),
                    None,
                    &[a.clone(), b.clone()],
                    append(),
                    None,
                ),
            ),
            (
                "upf_replace",
                upf(
                    "Rollout plan",
                    upf_rows(),
                    None,
                    std::slice::from_ref(&a),
                    dup(Some("replace"), Some(true), Some("bk.txt")),
                    Some("Rollout plan"),
                ),
            ),
            (
                "upf_skip_noop",
                upf(
                    "rollout_plan",
                    upf_rows(),
                    None,
                    std::slice::from_ref(&a),
                    dup(Some("skip"), None, None),
                    None,
                ),
            ),
            (
                "upf_desc_only",
                upf(
                    "reason_for_change",
                    upf_rows(),
                    Some("new why"),
                    &[],
                    DuplicateOpts::default(),
                    None,
                ),
            ),
            (
                "upf_desc_kept",
                upf(
                    "reason_for_change",
                    upf_rows(),
                    None,
                    std::slice::from_ref(&b),
                    append(),
                    None,
                ),
            ),
            (
                "upf_new",
                upf(
                    "backout",
                    upf_rows(),
                    None,
                    std::slice::from_ref(&r),
                    append(),
                    None,
                ),
            ),
            (
                "upf_new_desc",
                upf(
                    "backout",
                    upf_rows(),
                    Some("text"),
                    &[],
                    DuplicateOpts::default(),
                    None,
                ),
            ),
            (
                "upf_new_empty",
                upf(
                    "backout",
                    upf_rows(),
                    None,
                    &[],
                    DuplicateOpts::default(),
                    None,
                ),
            ),
            (
                "upf_bad_mode",
                upf(
                    "rollout_plan",
                    upf_rows(),
                    None,
                    std::slice::from_ref(&a),
                    dup(Some("x"), None, None),
                    None,
                ),
            ),
            (
                "upf_unavailable",
                upf(
                    "x",
                    json!({"nope": 1}),
                    None,
                    &[],
                    DuplicateOpts::default(),
                    None,
                ),
            ),
            (
                "upf_unavailable_list",
                upf("x", json!([1]), None, &[], DuplicateOpts::default(), None),
            ),
            (
                "upf_invalid_row",
                upf(
                    "x",
                    json!({"change_planning_fields": [{"name": "a"}, 5]}),
                    None,
                    &[],
                    DuplicateOpts::default(),
                    None,
                ),
            ),
            (
                "upf_unidentified",
                upf(
                    "x",
                    json!({"change_planning_fields": [{"name": "a"}, {"description": "?"}]}),
                    None,
                    &[],
                    DuplicateOpts::default(),
                    None,
                ),
            ),
        ];
        for (key, got) in cases {
            let w = want(key);
            let w = if w.get("__error__").is_some() {
                json!({"msg": w["msg"]})
            } else {
                w
            };
            assert_eq!(got, w, "{key}");
        }
    }

    #[test]
    fn approvals_and_assets_reads() {
        let got = json!([
            get_change_approvals(
                &FakeApi::new().ok("GET changes/5/approvals", json!([{"id": 1}])),
                5
            )
            .unwrap(),
            get_change_approvals(
                &FakeApi::new().ok("GET changes/5/approvals", json!({"approvals": [{"id": 2}]})),
                5
            )
            .unwrap(),
            get_change_approvals(&FakeApi::new().ok("GET changes/5/approvals", json!({})), 5)
                .unwrap(),
        ]);
        assert_eq!(got, want("approvals"));
        assert_eq!(
            json!(
                get_change_approval_groups(
                    &FakeApi::new().ok(
                        "GET changes/5/approval-chains",
                        json!({"approval_chains": []})
                    ),
                    5
                )
                .unwrap()
            ),
            want("approval_groups")
        );
        assert_eq!(
            err(get_change_approval_groups(
                &FakeApi::new().ok("GET changes/5/approval-chains", json!({"x": 1})),
                5
            )),
            want_err("approval_groups_bad")
        );

        assert_eq!(
            json!(
                get_change_assets(&FakeApi::new().ok("GET changes/5/assets", assets()), 5).unwrap()
            ),
            want("assets_direct")
        );
        assert_eq!(
            json!(
                get_change_assets(
                    &FakeApi::new().ok("GET changes/5/assets", json!({"associated_assets": null})),
                    5
                )
                .unwrap()
            ),
            want("assets_null")
        );
        let a = FakeApi::new()
            .ok("GET changes/5/associated-cis", json!({"data": [{"id": 1}]}))
            .on("GET changes/5/associated-cis", Reply::Status(404))
            .ok("GET changes/5/associated-cis", json!({"data": [{"id": 3}]}));
        let items = get_change_assets(&a, 5).unwrap();
        let calls: Vec<Value> = a
            .calls
            .lock()
            .unwrap()
            .iter()
            .map(|c| json!([c.path, c.params]))
            .collect();
        assert_eq!(json!([items, calls]), want("assets_fallback"));
        let a = FakeApi::new()
            .ok("GET changes/5/assets", json!({"x": 1}))
            .ok("GET changes/5/associated-cis", json!({"data": [{"id": 9}]}));
        assert_eq!(
            json!([
                get_change_assets(&a, 5).unwrap(),
                a.calls.lock().unwrap().len()
            ]),
            want("assets_other_shape")
        );
        assert_eq!(
            err(get_change_assets(
                &FakeApi::new().on("GET changes/5/assets", Reply::Status(500)),
                5
            )),
            want_err("assets_500")
        );
        assert_eq!(
            err(get_change_assets(
                &FakeApi::new().on("GET changes/5/associated-cis", Reply::Status(500)),
                5
            )),
            want_err("assets_cis_500")
        );
    }

    #[test]
    fn asset_writes_and_searches() {
        let a = FakeApi::new().ok("GET assets-to-associate", json!({"assets": []}));
        let r1 = search_assets_for_change(&a, 5, "srv", 2, 10).unwrap();
        let r2 = search_assets_for_change(&a, 5, "", 1, 30).unwrap();
        let p = |i: usize| json!(a.calls.lock().unwrap()[i].params);
        assert_eq!(json!([r1, p(0), r2, p(1)]), want("search_assets"));
        let a = FakeApi::new().ok("PUT changes/5/assets/associate", json!({}));
        associate_assets(&a, 5, &[7, 8]).unwrap();
        assert_eq!(json!([null, writes(&a)]), want("associate_assets"));
        let a = FakeApi::new()
            .ok("GET changes/5/assets", assets())
            .ok("PUT changes/5/assets/detach", json!({}));
        dissociate_assets(&a, 5, &[8, 7]).unwrap();
        assert_eq!(json!([null, writes(&a)]), want("dissociate_assets"));
        assert_eq!(
            err(dissociate_assets(
                &FakeApi::new().ok("GET changes/5/assets", assets()),
                5,
                &[99]
            )),
            want_err("dissociate_missing")
        );

        let r = get_change_associations(&FakeApi::new(), 5).unwrap();
        let mut keys: Vec<String> = r.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(json!([r["complete"], keys]), want("associations"));
        let a = FakeApi::new().ok(
            "GET changes/tickets/search",
            json!({"tickets": [{"id": 1}]}),
        );
        assert_eq!(
            json!([
                search_change_tickets(&a, "vpn").unwrap(),
                a.calls.lock().unwrap()[0].params
            ]),
            want("search_tickets")
        );
    }

    #[test]
    fn activities_follow_start_token() {
        let f = FakeApi::new()
            .ok(
                "GET changes/5/activities",
                json!({"activities": [{"id": 1}], "next_page_url": "/x?start_token=abc&y"}),
            )
            .ok(
                "GET changes/5/activities",
                json!({"activities": [{"id": 2}], "next_page_url": "/x?nothing"}),
            );
        let g = FakeApi::new()
            .ok(
                "GET changes/5/activities",
                json!({"activities": [{"id": 1}], "next_page_url": "/x?start_token=T"}),
            )
            .ok(
                "GET changes/5/activities",
                json!({"activities": [], "next_page_url": "/x?start_token=U"}),
            );
        let (a, b) = (
            get_change_activities(&f, 5).unwrap(),
            get_change_activities(&g, 5).unwrap(),
        );
        let params = |x: &FakeApi| {
            json!(
                x.calls
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|c| json!(c.params))
                    .collect::<Vec<_>>()
            )
        };
        assert_eq!(json!([a, params(&f), b, params(&g)]), want("activities"));
    }

    #[test]
    fn ticket_association_wrappers() {
        let j = MemJournal::default();
        let dry = crate::associations::SendOpts {
            reviewed_hash: None,
            read_only: false,
            dry_run: true,
        };
        let ro = crate::associations::SendOpts {
            reviewed_hash: None,
            read_only: true,
            dry_run: false,
        };
        assert_eq!(
            associate_ticket(&FakeApi::new(), &j, "5", &[json!(7)], dry).unwrap(),
            want("associate_ticket_dry")
        );
        assert_eq!(
            err(associate_ticket(&FakeApi::new(), &j, "5", &[json!(7)], ro)),
            want_err("associate_ticket_ro")
        );
        assert_eq!(
            dissociate_ticket(&FakeApi::new(), &j, "5", &json!(7), dry).unwrap(),
            want("dissociate_ticket_dry")
        );
    }

    #[test]
    fn task_edit_writes() {
        let a = FakeApi::new().ok("GET changes/5/tasks/70", json!({"task": task()}));
        assert_eq!(
            json!(get_task_for_edit(&a, 5, 70).unwrap()),
            want("task_for_edit")
        );
        let a = FakeApi::new().ok("PUT changes/5/tasks/70", json!({"ok": 1}));
        assert_eq!(
            json!([
                task_put(&a, "changes/5/tasks/70", &json!({"a": 1})).unwrap(),
                writes(&a)
            ]),
            want("task_put")
        );
        let a = FakeApi::new().ok("PUT changes/5/tasks/update", json!({}));
        delete_task(&a, 5, 70).unwrap();
        assert_eq!(json!([null, writes(&a)]), want("delete_task"));
        let a = FakeApi::new().ok("PUT changes/5/tasks/70", json!({"task": {"id": 70}}));
        let got = update_task(
            &a,
            5,
            70,
            &obj(json!({"title": "T", "custom_fields": {"team_name": "x", "a": null, "b": 1}})),
        )
        .unwrap();
        assert_eq!(json!([got, writes(&a)]), want("update_task"));
        let a = FakeApi::new().ok("PUT changes/5/tasks/70", json!({"id": 70}));
        let got = update_task(&a, 5, 70, &obj(json!({"title": "T", "custom_fields": {}}))).unwrap();
        assert_eq!(json!([got, a.writes()[0].body]), want("update_task_nocf"));
    }

    fn dl_api() -> FakeApi {
        FakeApi::new()
            .ok("GET changes/5/planning-fields", json!({"change_planning_fields": [
                {"name": "reason_for_change", "label": "Reason for change!", "description": " why ", "attachments": [
                    {"name": "a.txt", "attachment_url": "https://x/1"}, {"name": "a.txt", "canonical_url": "https://x/2"}, {"name": "skip.txt"}, {"attachment_url": "https://x/404"}]},
                {"name": "rollout_plan", "label": "", "description": "", "attachments": [{"name": "r.txt", "attachment_url": "https://x/3"}]},
                {"name": "backout", "label": "???", "description": "back", "attachments": []},
                {"name": "empty", "label": "Empty", "description": "", "attachments": []},
                {"label": "nameless", "description": "x"}]}))
            .on("FETCH https://x/1", Reply::Raw(200, b"one".to_vec()))
            .on("FETCH https://x/2", Reply::Raw(200, b"two".to_vec()))
            .on("FETCH https://x/3", Reply::Raw(200, b"three".to_vec()))
            .on("FETCH https://x/404", Reply::Raw(404, vec![]))
    }

    #[test]
    fn download_planning_attachments_cases() {
        let run = |names: Option<&[String]>, desc: bool, sub: &str| {
            let a = dl_api();
            let out = tmp(sub);
            let paths = download_planning_attachments(&a, 5, names, Some(&out), desc).unwrap();
            let files: Vec<Value> = paths
                .iter()
                .map(|p| {
                    json!([
                        p.file_name().unwrap().to_string_lossy(),
                        std::fs::read_to_string(p).unwrap()
                    ])
                })
                .collect();
            let calls: Vec<String> = a
                .calls
                .lock()
                .unwrap()
                .iter()
                .map(|c| c.path.clone())
                .collect();
            json!([files, calls])
        };
        assert_eq!(run(None, true, "pdl_all"), want("planning_download"));
        let names = ["rollout_plan".to_string(), "backout".to_string()];
        assert_eq!(
            run(Some(&names), false, "pdl_names"),
            want("planning_download_names")
        );
    }
}
