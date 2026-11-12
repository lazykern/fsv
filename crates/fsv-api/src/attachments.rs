//! Hash-bound local files, single-attempt uploads, and owning-write
//! readback (`attachments.py`).
//!
//! No multipart uploads, guessed upload limits, attachment GET, or
//! automatic cleanup.
//!
//! Size limit: the tenant publishes it in the agent app page as
//! `window.MAX_ATTACHMENT_LIMIT = <n>;`. The web UI divides file sizes by
//! 1048576 and compares the total of all files in one form with it, so `n` is
//! MiB per submit. Explicit byte keys in form metadata, when present, take
//! precedence. When the tenant publishes neither, the review says so and a
//! send needs an explicit `--max-bytes` (never a built-in default).

use std::path::PathBuf;
use std::sync::LazyLock;

use serde_json::{Map, Value, json};

use fsv_core::client::Body;
use fsv_core::error::{Error, Result};
use fsv_core::journal::sha256_hex;
use fsv_py::py_str;

use crate::conversations::{self, ConversationArgs, MailArgs};
use crate::formmeta::MetaStore;
use crate::notes::{BodyFormat, returned_note};
use crate::readback::{same_text, submitted_value_matches};
use crate::reviewed_write::{WritePlan, execute, refused};
use crate::service::{Api, Journal, last_event, load_conversations, obj, plan_hash, snapshot_hash};
use crate::watchers::positive_id;

const SIZE_KEYS: [&str; 2] = ["max_attachment_size_bytes", "max_file_size_bytes"];
const NESTED_KEYS: [&str; 10] = [
    "attachment_settings",
    "attachments",
    "settings",
    "options",
    "field_options",
    "ticket_fields",
    "ticket_form_fields",
    "problem_fields",
    "problem_form_fields",
    "fields",
];
pub const MIB: i64 = 1024 * 1024;
pub const NOT_PUBLISHED: &str = "not published by the tenant";
pub const PAGE_SOURCE: &str = "tenant agent page MAX_ATTACHMENT_LIMIT (MiB, total per submit)";
pub const METADATA_SOURCE: &str = "tenant form metadata (bytes)";
pub const OVERRIDE_SOURCE: &str = "--max-bytes override";
pub const PAGE_PATH: &str = "/a/dashboard";
static PAGE_LIMIT: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\bMAX_ATTACHMENT_LIMIT\s*=\s*([^;\s]+)\s*;").unwrap());
const UPLOAD: &str = "POST /api/_/attachments, JSON base64 data URI, once per file; returned IDs become owning payload attachments";
const WARNING: &str = "Uploads and owning write are not atomic. A failed/unknown step may leave orphan uploads. Inspect journal and owner; never replay the transaction.";

/// Port of attachments.size_limit: explicitly byte-denominated limits in
/// form metadata; `None` when absent. Invalid or conflicting values block.
pub fn size_limit(metadata: &Value) -> Result<Option<i64>> {
    fn visit(value: &Value, found: &mut Vec<i64>) -> Result<()> {
        match value {
            Value::Object(m) => {
                for (key, item) in m {
                    if SIZE_KEYS.contains(&key.as_str()) {
                        match item.as_i64().filter(|n| item.is_number() && *n > 0) {
                            Some(n) if item.is_i64() || item.is_u64() => found.push(n),
                            _ => return Err(refused("invalid runtime attachment byte limit")),
                        }
                    } else if NESTED_KEYS.contains(&key.as_str()) {
                        visit(item, found)?;
                    }
                }
            }
            Value::Array(a) => {
                for item in a {
                    visit(item, found)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut found = Vec::new();
    visit(metadata, &mut found)?;
    let mut unique = found.clone();
    unique.sort_unstable();
    unique.dedup();
    if unique.len() > 1 {
        return Err(refused(
            "conflicting runtime attachment byte limits; no upload attempted",
        ));
    }
    Ok(found.first().copied())
}

/// `int(Decimal(value) * MIB)` for a `[0-9]+(\.[0-9]+)?` value; `None` when
/// it does not fit an `i64`.
fn mib_bytes(value: &str) -> Option<i64> {
    let (whole, frac) = value.split_once('.').unwrap_or((value, ""));
    let mut digits: Vec<u64> = format!("{whole}{frac}")
        .bytes()
        .map(|b| u64::from(b - b'0'))
        .collect();
    let mut carry = 0;
    for d in digits.iter_mut().rev() {
        let v = *d * MIB as u64 + carry;
        *d = v % 10;
        carry = v / 10;
    }
    let mut head = carry.to_string();
    if head == "0" {
        head.clear();
    }
    digits.truncate(digits.len() - frac.len());
    head.extend(digits.iter().map(|d| char::from(b'0' + *d as u8)));
    let head = head.trim_start_matches('0');
    if head.is_empty() {
        Some(0)
    } else {
        head.parse().ok()
    }
}

/// Port of attachments.page_limit: bytes from the agent page's
/// `MAX_ATTACHMENT_LIMIT` (MiB); `None` if the page publishes none.
pub fn page_limit(c: &dyn Api) -> Result<Option<i64>> {
    let Ok(r) = c.page(PAGE_PATH) else {
        return Err(refused(
            "could not read the tenant's published attachment size limit; no upload attempted",
        ));
    };
    let text = r.text();
    if (300..400).contains(&r.status)
        || matches!(r.status, 401 | 403)
        || (r.status == 200 && fsv_core::client::looks_like_login_html(&text))
    {
        return Err(Error::session(
            "session expired or invalid; run `fsv auth login`",
        ));
    }
    if r.status != 200 {
        return Err(refused(format!(
            "could not read the tenant's published attachment size limit (HTTP {}); no upload attempted",
            r.status
        )));
    }
    let mut values: Vec<&str> = PAGE_LIMIT
        .captures_iter(&text)
        .filter_map(|m| m.get(1).map(|g| g.as_str()))
        .collect();
    values.sort_unstable();
    values.dedup();
    let [value] = values[..] else {
        if values.is_empty() {
            return Ok(None);
        }
        return Err(refused(
            "ambiguous or invalid published attachment size limit; no upload attempted",
        ));
    };
    let (whole, frac) = value
        .split_once('.')
        .map_or((value, None), |(w, f)| (w, Some(f)));
    let numeric = |d: &str| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit());
    let limit = (numeric(whole) && frac.is_none_or(numeric))
        .then(|| mib_bytes(value))
        .flatten()
        .ok_or_else(|| {
            refused("ambiguous or invalid published attachment size limit; no upload attempted")
        })?;
    if limit <= 0 {
        return Err(refused(
            "invalid published attachment size limit; no upload attempted",
        ));
    }
    Ok(Some(limit))
}

/// Port of attachments.Limit: the effective byte limit, where it came from,
/// and what the tenant published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limit {
    pub max_bytes: Option<i64>,
    pub source: String,
    pub published: Option<i64>,
}

/// Port of attachments.resolve_limit: metadata bytes, else the published
/// page limit; an explicit `max_bytes` replaces either.
pub fn resolve_limit(c: &dyn Api, metadata: &Value, max_bytes: Option<i64>) -> Result<Limit> {
    if max_bytes.is_some_and(|n| n <= 0) {
        return Err(refused(
            "--max-bytes must be a positive whole number of bytes",
        ));
    }
    let (published, source) = match size_limit(metadata)? {
        Some(n) => (Some(n), METADATA_SOURCE),
        None => (page_limit(c)?, PAGE_SOURCE),
    };
    Ok(match (max_bytes, published) {
        (Some(n), _) => Limit {
            max_bytes: Some(n),
            source: OVERRIDE_SOURCE.into(),
            published,
        },
        (None, None) => Limit {
            max_bytes: None,
            source: NOT_PUBLISHED.into(),
            published: None,
        },
        (None, Some(n)) => Limit {
            max_bytes: Some(n),
            source: source.into(),
            published: Some(n),
        },
    })
}

/// Port of attachments.metadata: the owner module's form-field metadata.
pub fn metadata(c: &dyn Api, module: &str) -> Result<Value> {
    c.int_get(
        if module == "tickets" {
            "ticket_form_fields"
        } else {
            "problem_form_fields"
        },
        None,
    )
}

/// Port of attachments.LocalFile: one hash-bound local file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalFile {
    pub path: String,
    pub name: String,
    pub size: i64,
    pub mime: String,
    pub sha256: String,
}

impl LocalFile {
    /// Port of attachments.LocalFile.review.
    pub fn review(&self) -> Value {
        json!({"name": self.name, "size": self.size, "content_type": self.mime, "sha256": self.sha256})
    }
}

/// `Path(value).expanduser().resolve()`.
fn resolve(value: &str) -> PathBuf {
    let expanded = if value == "~" || value.starts_with("~/") {
        let home = fsv_core::config::home_dir();
        format!(
            "{}{}",
            home.to_string_lossy().trim_end_matches('/'),
            &value[1..]
        )
    } else {
        value.to_string()
    };
    let path = PathBuf::from(&expanded);
    std::fs::canonicalize(&path).unwrap_or_else(|_| std::path::absolute(&path).unwrap_or(path))
}

fn over(total: i64, limit: Option<i64>) -> bool {
    limit.is_some_and(|l| total > l)
}

/// Port of attachments.local_files: unique non-empty regular files; the
/// total must fit an explicit limit.
pub fn local_files(paths: &[String], limit: Option<i64>) -> Result<Vec<LocalFile>> {
    let mut files = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();
    for value in paths {
        let path = resolve(value);
        if seen.contains(&path) || !path.is_file() {
            return Err(refused("attachments must be unique regular local files"));
        }
        seen.push(path.clone());
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.chars().any(|ch| (ch as u32) < 32) {
            return Err(refused("unsafe attachment filename"));
        }
        let size = std::fs::metadata(&path)
            .map_err(|e| Error::other(e.to_string()))?
            .len() as i64;
        let before: i64 = files.iter().map(|f: &LocalFile| f.size).sum();
        if size == 0 {
            return Err(refused(format!("attachment is empty: {name}")));
        }
        if over(size + before, limit) {
            return Err(refused(format!(
                "attachments exceed the {}-byte size limit; no upload attempted",
                limit.unwrap_or_default()
            )));
        }
        let raw = std::fs::read(&path).map_err(|e| Error::other(e.to_string()))?;
        if raw.is_empty() || over(raw.len() as i64 + before, limit) {
            return Err(refused(
                "attachment changed, is empty or exceeds the size limit; no upload attempted",
            ));
        }
        files.push(LocalFile {
            path: path.to_string_lossy().into_owned(),
            mime: crate::create::guess_mime(&name)
                .unwrap_or_else(|| "application/octet-stream".into()),
            name,
            size: raw.len() as i64,
            sha256: sha256_hex(&raw),
        });
    }
    if files.is_empty() {
        return Err(refused("at least one attachment required"));
    }
    Ok(files)
}

/// Port of attachments.AttachedPlan: the owning write plus the reviewed
/// files and the total byte limit with its source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachedPlan {
    pub base: WritePlan,
    pub files: Vec<LocalFile>,
    pub limit: Option<i64>,
    pub limit_source: String,
    pub published_limit: Option<i64>,
}

impl AttachedPlan {
    fn file_reviews(&self) -> Vec<Value> {
        self.files.iter().map(LocalFile::review).collect()
    }

    /// Port of attachments.AttachedPlan.hash.
    pub fn hash(&self) -> String {
        plan_hash(
            &json!({"owner": self.base.hash(), "files": self.file_reviews(), "max_bytes": self.limit,
                          "limit_source": self.limit_source, "published_bytes": self.published_limit}),
        )
    }

    /// Port of attachments.AttachedPlan.review.
    pub fn review(&self) -> Map<String, Value> {
        let mut out = self.base.review();
        out.insert("plan_hash".into(), json!(self.hash()));
        out.insert("files".into(), json!(self.file_reviews()));
        out.insert("max_attachment_size_bytes".into(), json!(self.limit));
        let mut limit = obj(json!({
            "max_bytes": self.limit, "source": self.limit_source,
            "published_bytes": self.published_limit.map_or(json!(NOT_PUBLISHED), |n| json!(n)),
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
        out.insert("upload".into(), json!(UPLOAD));
        out.insert("warning".into(), json!(WARNING));
        out
    }
}

/// Port of attachments.wrap.
pub fn wrap(
    base: WritePlan,
    paths: &[String],
    raw_metadata: &Value,
    c: &dyn Api,
    max_bytes: Option<i64>,
) -> Result<AttachedPlan> {
    let limit = resolve_limit(c, raw_metadata, max_bytes)?;
    Ok(AttachedPlan {
        base,
        files: local_files(paths, limit.max_bytes)?,
        limit: limit.max_bytes,
        limit_source: limit.source,
        published_limit: limit.published,
    })
}

/// Port of attachments.owner_row: the exact owning record.
pub fn owner_row(c: &dyn Api, module: &str, target: i64) -> Result<Map<String, Value>> {
    let key = match module {
        "tickets" => "ticket",
        "problems" => "problem",
        _ => "change",
    };
    let raw = c.int_get(&format!("{module}/{target}"), None)?;
    let row = match raw {
        Value::Object(mut m) => match m.remove(key) {
            Some(Value::Object(inner)) => Some(inner),
            Some(_) => None,
            None => Some(m),
        },
        _ => None,
    };
    match row {
        Some(r) if positive_id(r.get("id").unwrap_or(&Value::Null))? == target => Ok(r),
        _ => Err(refused("exact attachment owner unavailable")),
    }
}

/// The `conversation_plan` keyword arguments.
#[derive(Debug, Clone, Default)]
pub struct ConversationAttach<'a> {
    /// `note` | `reply`.
    pub kind: &'a str,
    pub public: bool,
    pub sender: Option<&'a str>,
    pub html_body: bool,
    pub notify: &'a [String],
    pub max_bytes: Option<i64>,
}

/// Port of attachments.conversation_plan: a ticket note or reply bound to
/// the parent ticket and form metadata.
pub fn conversation_plan(
    c: &dyn Api,
    store: &dyn MetaStore,
    target: &str,
    body: &str,
    paths: &[String],
    args: &ConversationAttach<'_>,
) -> Result<AttachedPlan> {
    if args.kind == "reply" && args.sender.is_none_or(str::is_empty) {
        return Err(refused(
            "attached reply requires explicit configured --sender",
        ));
    }
    let plan = conversations::build_plan_mail(
        c,
        store,
        target,
        body,
        &ConversationArgs {
            kind: args.kind,
            public: args.public,
            notify: args.notify,
            format: if args.html_body {
                BodyFormat::Html
            } else {
                BodyFormat::Plain
            },
            ..ConversationArgs::default()
        },
        &MailArgs {
            sender: args.sender,
            ..MailArgs::default()
        },
    )?;
    let parent = owner_row(c, "tickets", plan.ticket_id)?;
    let raw = metadata(c, "tickets")?;
    let base = WritePlan {
        action: format!("ticket_{}_attachments", args.kind),
        target: plan.ticket_id,
        tenant: c.domain(),
        method: "POST".into(),
        path: plan.path(),
        payload_json: plan.payload_json.clone(),
        context_hash: snapshot_hash(&json!({"parent": parent, "metadata": raw}))?,
    };
    wrap(base, paths, &raw, c, args.max_bytes)
}

/// Port of attachments.problem_note_plan: a problem note bound to the base
/// note plan and form metadata.
pub fn problem_note_plan(
    c: &dyn Api,
    target: &str,
    body: &str,
    paths: &[String],
    max_bytes: Option<i64>,
) -> Result<AttachedPlan> {
    let plan = crate::problem_workflows::child_plan(c, target, "note", body)?;
    let raw = metadata(c, "problems")?;
    let base = WritePlan {
        action: "problem_note_attachments".into(),
        target: plan.plan.target,
        tenant: c.domain(),
        method: plan.plan.method.clone(),
        path: plan.plan.path.clone(),
        payload_json: plan.plan.payload_json.clone(),
        context_hash: snapshot_hash(&json!({"base": plan.hash(), "metadata": raw}))?,
    };
    wrap(base, paths, &raw, c, max_bytes)
}

/// Port of attachments._rows_attachments: a well-formed attachment inventory.
fn rows_attachments(row: Option<&Map<String, Value>>) -> Result<Vec<Value>> {
    let Some(rows) = row
        .and_then(|r| r.get("attachments"))
        .and_then(Value::as_array)
    else {
        return Err(refused("owner attachment inventory unavailable"));
    };
    let ids = rows
        .iter()
        .filter(|r| r.is_object())
        .map(|r| positive_id(r.get("id").unwrap_or(&Value::Null)))
        .collect::<Result<Vec<_>>>()?;
    let unique: std::collections::HashSet<&i64> = ids.iter().collect();
    if ids.len() != rows.len() || unique.len() != ids.len() {
        return Err(refused("invalid attachment inventory"));
    }
    Ok(rows.clone())
}

fn ids_of(rows: &[Value]) -> Result<std::collections::BTreeSet<i64>> {
    rows.iter().map(|r| positive_id(&r["id"])).collect()
}

const IGNORED: [&str; 5] = [
    "conversation_from_summary",
    "full_text",
    "translate_reply",
    "freddy_message_id",
    "br_validation_excludes",
];

fn exact_row(rows: &[Value], id: i64) -> Option<Map<String, Value>> {
    let id = id.to_string();
    let matches: Vec<&Value> = rows
        .iter()
        .filter(|r| py_str(r.get("id").unwrap_or(&Value::Null)) == id)
        .collect();
    match matches[..] {
        [Value::Object(m)] => Some(m.clone()),
        _ => None,
    }
}

/// `response.get(key, response)` for a dict response, else `{}`.
fn envelope<'a>(response: &'a Value, key: &str, empty: &'a Value) -> &'a Value {
    match response {
        Value::Object(m) => m.get(key).unwrap_or(response),
        _ => empty,
    }
}

/// Port of attachments.readback: the owning record carries the uploaded
/// files and every submitted business value.
pub fn readback(
    c: &dyn Api,
    plan: &AttachedPlan,
    response: &Value,
    ids: &[i64],
) -> Result<Map<String, Value>> {
    let base = &plan.base;
    let empty = json!({});
    let id_of = |raw: &Value| {
        if !raw.is_object() {
            return Err(Error::other("owner response is not an object"));
        }
        positive_id(raw.get("id").unwrap_or(&Value::Null))
    };
    let (returned_id, row) = if base.path == "tickets" || base.path == "changes" {
        let key = if base.path == "tickets" {
            "ticket"
        } else {
            "change"
        };
        let returned_id = id_of(envelope(response, key, &empty))?;
        (returned_id, Some(owner_row(c, &base.path, returned_id)?))
    } else if base.path.starts_with("problems/") {
        let returned_id = id_of(envelope(response, "note", &empty))?;
        let rows = crate::problem_workflows::inventory(c, base.target, "notes")?;
        (returned_id, exact_row(&rows, returned_id))
    } else {
        let raw = returned_note(response);
        let returned_id = positive_id(raw.and_then(|r| r.get("id")).unwrap_or(&Value::Null))?;
        let pages = load_conversations(c, base.target)?;
        if pages.missing.is_some() || pages.meta.get("partial").is_some_and(fsv_py::truthy) {
            return Err(refused("complete conversation readback unavailable"));
        }
        (returned_id, exact_row(&pages.rows, returned_id))
    };
    let attachments = rows_attachments(row.as_ref())?;
    let observed: std::collections::BTreeMap<i64, &Value> = attachments
        .iter()
        .map(|r| Ok((positive_id(&r["id"])?, r)))
        .collect::<Result<_>>()?;
    let wanted: std::collections::BTreeSet<i64> = ids.iter().copied().collect();
    let files_match = observed
        .keys()
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
        == wanted
        && ids.iter().zip(&plan.files).all(|(i, f)| {
            observed[i].get("name") == Some(&json!(f.name))
                && submitted_value_matches(
                    observed[i].get("size").unwrap_or(&Value::Null),
                    &json!(f.size),
                )
        });
    let mut fields = Map::new();
    for (key, expected) in base.payload().as_object().into_iter().flatten() {
        if IGNORED.contains(&key.as_str()) {
            continue;
        }
        let actual_key = match &row {
            Some(r) if key == "notify_emails" && !r.contains_key(key) => "to_emails",
            _ => key.as_str(),
        };
        let actual = row.as_ref().and_then(|r| r.get(actual_key));
        let status = match actual {
            _ if row.as_ref().is_none_or(|r| !r.contains_key(actual_key)) => "unavailable",
            Some(a) => {
                let matched = if key == "body" || key == "description" {
                    same_text(a, expected)
                } else {
                    submitted_value_matches(a, expected)
                };
                if matched { "matched" } else { "mismatched" }
            }
            None => "unavailable",
        };
        fields.insert(key.clone(), json!(status));
    }
    let verified = files_match && fields.values().all(|v| v == "matched");
    Ok(obj(json!({
        "owner_id": returned_id, "attachments": ids, "attachments_verified": files_match,
        "fields": fields, "verified": verified,
    })))
}

fn upload_body(file: &LocalFile, raw: &[u8]) -> Value {
    json!({
        "content": crate::create::data_uri(&file.mime, raw),
        "content_file_name": file.name,
        "content_file_size": file.size,
        "content_content_type": file.mime,
    })
}

fn dumps(v: &Value) -> Vec<u8> {
    fsv_py::Dumps::default().dump(v).into_bytes()
}

/// The upload and owning write; any failure is the caller's ambiguous refusal.
fn transact(
    plan: &AttachedPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    hash: &str,
    csrf: &str,
    uploaded: &mut Vec<i64>,
) -> Result<Value> {
    let action = &plan.base.action;
    let upload_action = format!("{action}_upload");
    let target = json!(plan.base.target);
    for file in &plan.files {
        let raw = std::fs::read(&file.path).map_err(|e| Error::other(e.to_string()))?;
        if raw.len() as i64 != file.size || sha256_hex(&raw) != file.sha256 {
            return Err(refused("attachment changed after review"));
        }
        let response = c.send_once(
            "POST",
            "attachments",
            Body::Raw(dumps(&upload_body(file, &raw))),
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
            Some(&target),
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
    let Value::Object(mut body) = plan.base.payload() else {
        return Err(Error::other("owner payload is not an object"));
    };
    body.insert("attachments".into(), json!(uploaded));
    c.send_once(
        &plan.base.method,
        &plan.base.path,
        Body::Raw(dumps(&Value::Object(body))),
        None,
        csrf,
    )
}

/// Port of attachments.send: guards, one upload per file, one owning write,
/// then the owning-record readback. Nothing is ever replayed.
pub fn send(
    plan: &AttachedPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
    rebuild: &dyn Fn() -> Result<AttachedPlan>,
) -> Result<Value> {
    let Some(reviewed) = reviewed_hash else {
        return Ok(json!({"status": "cancelled", "verified": false}));
    };
    let hash = plan.hash();
    if plan.base.tenant.is_empty() || plan.base.tenant != c.domain() || reviewed != hash {
        return Err(refused("tenant or full attachment review hash changed"));
    }
    if plan.limit.is_none() {
        return Err(refused(
            "the tenant publishes no attachment size limit; review again with --max-bytes N; no upload attempted",
        ));
    }
    let action = &plan.base.action;
    if matches!(
        last_event(journal, action, &hash).as_deref(),
        Some("started" | "done")
    ) {
        return Err(refused(
            "attachment transaction was sent or outcome unknown; do not replay",
        ));
    }
    let paths: Vec<String> = plan.files.iter().map(|f| f.path.clone()).collect();
    if rebuild()?.hash() != hash || local_files(&paths, plan.limit)? != plan.files {
        return Err(refused(
            "owner, metadata or attachment bytes changed; review again",
        ));
    }
    let csrf = c.get_csrf()?;
    let target = json!(plan.base.target);
    journal.record_plan("started", &hash, action, Some(&target), None);
    if last_event(journal, action, &hash).as_deref() != Some("started") {
        return Err(refused(
            "could not persist attachment journal; no upload attempted",
        ));
    }
    let mut uploaded = Vec::new();
    let response = match transact(plan, c, journal, &hash, &csrf, &mut uploaded) {
        Ok(r) => r,
        Err(_) => {
            return Err(refused(format!(
                "attachment transaction failed or outcome unknown; uploaded IDs: {}; inspect owner and journal, do not replay",
                fsv_py::Dumps::default().dump(&json!(uploaded))
            )));
        }
    };
    journal.record_plan("done", &hash, action, Some(&target), None);
    let mut result = obj(
        json!({"status": "submitted", "plan_hash": hash, "uploaded_ids": uploaded,
                                "verified": false}),
    );
    if let Ok(extra) = readback(c, plan, &response, &uploaded) {
        result.extend(extra);
    }
    if result.get("verified") != Some(&json!(true)) {
        result.insert(
            "warning".into(),
            json!("Independent owning-record readback not fully verified; do not replay."),
        );
    }
    Ok(Value::Object(result))
}

/// `tickets/ID`, `problems/ID` or `changes/ID`.
fn parse_owner(owner: &str) -> Option<(&str, i64)> {
    let (module, id) = owner.split_once('/')?;
    let ok = matches!(module, "tickets" | "problems" | "changes")
        && id.bytes().all(|b| b.is_ascii_digit())
        && id.bytes().next().is_some_and(|b| b != b'0');
    ok.then(|| id.parse().ok().map(|n| (module, n))).flatten()
}

/// Port of attachments.deletion_owner: the exact record, child note or
/// planning section that owns an attachment.
pub fn deletion_owner(
    c: &dyn Api,
    owner: &str,
    child: Option<&Value>,
    section: Option<&str>,
) -> Result<Map<String, Value>> {
    let Some((module, target)) =
        parse_owner(owner).filter(|_| !(child.is_some() && section.is_some()))
    else {
        return Err(Error::value(
            "owner must be tickets/ID, problems/ID or changes/ID; choose at most one child or section",
        ));
    };
    let parent = owner_row(c, module, target)?;
    if let Some(section) = section {
        let rows = crate::planning_text::inventory(c, module, target)?;
        let matches: Vec<&Value> = rows
            .iter()
            .filter(|r| r.get("name") == Some(&json!(section)))
            .collect();
        let [Value::Object(m)] = matches[..] else {
            return Err(refused("exact planning owner unavailable"));
        };
        return Ok(m.clone());
    }
    let Some(child) = child else {
        return Ok(parent);
    };
    let child = positive_id(child)?;
    let rows = match module {
        "tickets" => {
            let pages = load_conversations(c, target)?;
            if pages.missing.is_some() || pages.meta.get("partial").is_some_and(fsv_py::truthy) {
                return Err(refused("complete conversation inventory unavailable"));
            }
            pages.rows
        }
        "problems" => crate::problem_workflows::inventory(c, target, "notes")?,
        _ => return Err(refused("change note attachment deletion not proven")),
    };
    exact_row(&rows, child).ok_or_else(|| refused("exact child attachment owner unavailable"))
}

/// Port of attachments.delete_plan: a bodyless DELETE of an attachment
/// present on its exact owner.
pub fn delete_plan(
    c: &dyn Api,
    ident: &Value,
    owner: &str,
    child: Option<&Value>,
    section: Option<&str>,
) -> Result<WritePlan> {
    let ident = positive_id(ident)?;
    let row = deletion_owner(c, owner, child, section)?;
    if !ids_of(&rows_attachments(Some(&row))?)?.contains(&ident) {
        return Err(refused(
            "attachment not present on exact owner; orphan deletion cannot be verified",
        ));
    }
    Ok(WritePlan {
        action: "attachment_delete".into(),
        target: ident,
        tenant: c.domain(),
        method: "DELETE".into(),
        path: format!("attachments/{ident}"),
        payload_json: "null".into(),
        context_hash: snapshot_hash(
            &json!({"owner": owner, "child": child, "section": section, "row": row}),
        )?,
    })
}

/// Port of attachments.delete: one DELETE, verified by the owner's
/// remaining attachment IDs.
pub fn delete(
    plan: &WritePlan,
    c: &dyn Api,
    journal: &dyn Journal,
    owner: &str,
    reviewed_hash: Option<&str>,
    child: Option<&Value>,
    section: Option<&str>,
) -> Result<Value> {
    let inventory = || -> Result<std::collections::BTreeSet<i64>> {
        ids_of(&rows_attachments(Some(&deletion_owner(
            c, owner, child, section,
        )?))?)
    };
    let mut expected = inventory()?;
    expected.remove(&plan.target);
    let rebuild = || delete_plan(c, &json!(plan.target), owner, child, section);
    let verify = |_: &Value| -> Result<Map<String, Value>> {
        Ok(obj(
            json!({"attachment_id": plan.target, "verified": inventory()? == expected}),
        ))
    };
    execute(plan, c, journal, reviewed_hash, &rebuild, &verify)
}

#[cfg(test)]
mod tests;
