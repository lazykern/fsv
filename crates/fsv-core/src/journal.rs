//! Append-only JSONL record of every write request (`journal.py`).
//!
//! Location: `$XDG_STATE_HOME/fsv/journals/<profile>/<tenant>/journal.jsonl`
//! where `<profile>` is the sha256 of the resolved config directory and
//! `<tenant>` the sha256 of the domain (`unknown` without one). Entries never
//! contain cookies, headers or bodies: only a SHA-256 of the canonical JSON
//! body (the plan hash).

use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::config;
use crate::error::Result;
use crate::value::truthy;

/// Plan entry events.
pub const PLAN_EVENTS: [&str; 3] = ["started", "done", "failed"];

const PATH_SEGMENTS: &[&str] = &[
    "api",
    "_",
    "v2",
    "itil",
    "changes",
    "tickets",
    "problems",
    "releases",
    "notes",
    "reply",
    "attachments",
    "tasks",
    "assets",
    "associate",
    "detach",
    "planning-fields",
    "catalog",
    "service-items",
    "place-request",
];

fn is_id(s: &str) -> bool {
    static ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d+\n?$").expect("regex"));
    ID.is_match(s)
}

fn is_id_full(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| crate::pystr::decimal_value(c).is_some())
}

/// `Path.resolve()` (non-strict): absolute, symlinks resolved for the existing prefix.
pub fn resolve_path(path: &Path) -> PathBuf {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map(|d| d.join(path)).unwrap_or_else(|_| path.to_path_buf())
    };
    if let Ok(p) = std::fs::canonicalize(&abs) {
        return p;
    }
    let mut existing = abs.clone();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    while !existing.exists() {
        match existing.file_name() {
            Some(n) => tail.push(n.to_owned()),
            None => break,
        }
        if !existing.pop() {
            break;
        }
    }
    let mut out = std::fs::canonicalize(&existing).unwrap_or(existing);
    for part in tail.into_iter().rev() {
        let p = Path::new(&part);
        match p.components().next() {
            Some(Component::ParentDir) => {
                out.pop();
            }
            Some(Component::CurDir) => {}
            _ => out.push(p),
        }
    }
    out
}

fn path_bytes(p: &Path) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        p.as_os_str().as_bytes().to_vec()
    }
    #[cfg(not(unix))]
    {
        p.to_string_lossy().as_bytes().to_vec()
    }
}

fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

/// Journal file for the current profile and tenant.
pub fn journal_path() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| config::home_dir().join(".local").join("state"));
    let profile = sha256_hex(&path_bytes(&resolve_path(&config::config_dir())));
    let domain = config::domain();
    let tenant = if domain.is_empty() { "unknown".to_string() } else { sha256_hex(domain.as_bytes()) };
    base.join("fsv").join("journals").join(profile).join(tenant).join("journal.jsonl")
}

/// sha256 of the body as canonical JSON (sorted keys, no whitespace); `None` without a body.
pub fn plan_hash(body: Option<&Value>) -> Option<String> {
    body.map(|b| sha256_hex(fsv_py::canonical(b).as_bytes()))
}

/// URL path split into segments; unknown non-numeric segments become `[redacted]`.
pub fn split_path(url: &str) -> Vec<String> {
    static HOST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^https?://[^/]+").expect("regex"));
    let stripped = HOST.replace(url, "");
    let path = stripped.split('?').next().unwrap_or("").split('#').next().unwrap_or("");
    path.split('/')
        .filter(|p| !p.is_empty())
        .map(|p| if PATH_SEGMENTS.contains(&p) || is_id_full(p) { p.to_string() } else { "[redacted]".to_string() })
        .collect()
}

/// `(kind, target id, normalized path)` for a request.
pub fn classify(method: &str, segments: &[String]) -> (String, Option<String>, String) {
    let mut segs: &[String] = segments;
    while let Some(first) = segs.first() {
        if matches!(first.as_str(), "api" | "_" | "v2" | "itil") {
            segs = &segs[1..];
        } else {
            break;
        }
    }
    let last = segs.last().map(String::as_str).unwrap_or("");
    let resource = segs.iter().find(|s| !is_id(s)).map(String::as_str).unwrap_or("");
    let mut target = None;
    for (i, s) in segs.iter().enumerate() {
        if is_id(s) {
            target = Some(if i > 0 { format!("{}/{s}", segs[i - 1]) } else { s.clone() });
            break;
        }
    }
    let upper = method.to_uppercase();
    let verb = match upper.as_str() {
        "POST" => "create".to_string(),
        "PUT" | "PATCH" => "update".to_string(),
        "DELETE" => "delete".to_string(),
        _ => method.to_lowercase(),
    };
    let kind = match (upper.as_str(), last) {
        ("POST", "notes") => "note".to_string(),
        ("POST", "reply") => "reply".to_string(),
        ("POST", "attachments") => "upload".to_string(),
        _ if !resource.is_empty() => format!("{verb}:{resource}"),
        _ => verb,
    };
    (kind, target, format!("/{}", segments.join("/")))
}

/// Keep numeric service IDs, never arbitrary server-provided text.
pub fn safe_id(value: Option<&Value>) -> Value {
    match value {
        Some(v @ Value::Number(n)) if !n.is_f64() => v.clone(),
        Some(Value::String(s)) if is_id_full(s) => Value::String(s.clone()),
        _ => Value::Null,
    }
}

fn safe_target(value: Option<&Value>) -> Value {
    static CATALOG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^catalog-item/\d+$").expect("regex"));
    match value {
        Some(Value::String(s)) if CATALOG.is_match(s) => Value::String(s.clone()),
        other => safe_id(other),
    }
}

fn created_id(data: Option<&Value>) -> Value {
    let Some(Value::Object(map)) = data else { return Value::Null };
    let usable = |v: Option<&Value>| matches!(v, Some(x) if !x.is_null() && !x.is_object() && !x.is_array());
    if usable(map.get("id")) {
        return safe_id(map.get("id"));
    }
    for v in map.values() {
        if let Value::Object(inner) = v {
            if usable(inner.get("id")) {
                return safe_id(inner.get("id"));
            }
        }
    }
    Value::Null
}

fn timestamp() -> String {
    jiff::Zoned::now().strftime("%Y-%m-%dT%H:%M:%S%z").to_string()
}

fn epoch() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn command() -> Value {
    crate::runtime::command_path().map_or(Value::Null, Value::String)
}

fn append(entry: &Map<String, Value>) -> Result<()> {
    let p = journal_path();
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&p)?;
    let line = fsv_py::Dumps::default().ensure_ascii(false).dump(&Value::Object(entry.clone())) + "\n";
    f.write_all(line.as_bytes())?;
    Ok(())
}

/// Outcome of one write request for [`record`].
#[derive(Clone, Copy, Debug, Default)]
pub struct WriteOutcome<'a> {
    pub status: Option<u16>,
    pub ok: bool,
    pub response: Option<&'a Value>,
    /// Whether an error occurred (its text is never stored).
    pub error: bool,
}

/// Append one write entry. Never fails: the journal must not break a command.
pub fn record(method: &str, url: &str, body: Option<&Value>, outcome: WriteOutcome<'_>) {
    let (kind, target, path) = classify(method, &split_path(url));
    let upper = method.to_uppercase();
    let mut entry = Map::new();
    entry.insert("ts".into(), json!(timestamp()));
    entry.insert("command".into(), command());
    entry.insert("kind".into(), json!(kind));
    entry.insert("target".into(), target.map_or(Value::Null, Value::String));
    entry.insert("method".into(), json!(upper));
    entry.insert("path".into(), json!(path));
    entry.insert("plan_hash".into(), plan_hash(body).map_or(Value::Null, Value::String));
    entry.insert("status".into(), outcome.status.map_or(Value::Null, |s| json!(s)));
    entry.insert("ok".into(), json!(outcome.ok));
    let created = if outcome.ok && upper == "POST" { created_id(outcome.response) } else { Value::Null };
    entry.insert("created_id".into(), created);
    if outcome.error {
        entry.insert("error".into(), json!("details omitted"));
    }
    let _ = append(&entry);
}

/// All journal rows (the last `limit` when given and non-zero); bad lines are skipped.
pub fn read(limit: Option<usize>) -> Vec<Value> {
    let Ok(text) = std::fs::read_to_string(journal_path()) else { return Vec::new() };
    let rows: Vec<Value> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    match limit {
        Some(n) if n > 0 && n < rows.len() => rows[rows.len() - n..].to_vec(),
        _ => rows,
    }
}

/// One step result for [`record_plan`].
#[derive(Clone, Debug)]
pub struct PlanStep {
    pub step: String,
    pub ok: bool,
}

/// Optional fields of a plan event.
#[derive(Clone, Debug, Default)]
pub struct PlanEvent<'a> {
    pub target: Option<&'a Value>,
    pub created_id: Option<&'a Value>,
    pub steps: Option<&'a [PlanStep]>,
    /// Whether the plan failed with an error (its text is never stored).
    pub error: bool,
}

/// Append a plan event (`started`, `done`, `failed`). Never fails.
pub fn record_plan(event: &str, plan: &str, action: &str, extra: PlanEvent<'_>) {
    let mut entry = Map::new();
    entry.insert("ts".into(), json!(timestamp()));
    entry.insert("epoch".into(), json!(epoch()));
    entry.insert("command".into(), command());
    entry.insert("kind".into(), json!(format!("plan:{action}")));
    entry.insert("event".into(), json!(event));
    entry.insert("plan".into(), json!(plan));
    entry.insert("target".into(), safe_target(extra.target));
    entry.insert("created_id".into(), safe_id(extra.created_id));
    if let Some(steps) = extra.steps {
        // Filenames and exception messages can contain private data.
        let known = ["create", "update", "attachments", "upload", "place-request", "after"];
        let rows: Vec<Value> = steps
            .iter()
            .map(|s| {
                let name = if known.contains(&s.step.as_str()) { s.step.as_str() } else { "other" };
                json!({"step": name, "ok": s.ok})
            })
            .collect();
        entry.insert("steps".into(), Value::Array(rows));
    }
    if extra.error {
        entry.insert("error".into(), json!("details omitted"));
    }
    let _ = append(&entry);
}

fn epoch_of(row: &Value) -> f64 {
    match row.get("epoch") {
        Some(v) if truthy(v) => v.as_f64().unwrap_or(0.0),
        _ => 0.0,
    }
}

/// Latest entry showing this plan was (or may have been) sent within `hours`:
/// a `done` entry, or a `started` one with no outcome. `failed` plans can be retried.
pub fn recent_plan(plan: &str, action: &str, hours: f64) -> Option<Value> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    let cutoff = now - hours * 3600.0;
    let kind = format!("plan:{action}");
    let mut outcome: Vec<(String, Value)> = Vec::new();
    let mut started: Vec<Value> = Vec::new();
    for row in read(None) {
        if row.get("plan").and_then(Value::as_str) != Some(plan) || row.get("kind").and_then(Value::as_str) != Some(&kind)
        {
            continue;
        }
        if epoch_of(&row) < cutoff {
            continue;
        }
        if row.get("event").and_then(Value::as_str) == Some("started") {
            started.push(row);
        } else {
            let key = fsv_py::py_str(row.get("epoch").unwrap_or(&Value::Null));
            match outcome.iter_mut().find(|(k, _)| *k == key) {
                Some(slot) => slot.1 = row,
                None => outcome.push((key, row)),
            }
        }
    }
    let mut best: Option<&Value> = None;
    for (_, r) in &outcome {
        if r.get("event").and_then(Value::as_str) == Some("done") && best.is_none_or(|b| epoch_of(r) > epoch_of(b)) {
            best = Some(r);
        }
    }
    if let Some(b) = best {
        return Some(b.clone());
    }
    for s in started.iter().rev() {
        if !outcome.iter().any(|(_, r)| epoch_of(r) >= epoch_of(s)) {
            return Some(s.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_hash_is_order_independent_and_none_for_no_body() {
        assert_eq!(plan_hash(Some(&json!({"a": 1, "b": [2]}))), plan_hash(Some(&json!({"b": [2], "a": 1}))));
        assert_ne!(plan_hash(Some(&json!({"a": 1}))), plan_hash(Some(&json!({"a": 2}))));
        assert_eq!(plan_hash(None), None);
    }

    #[test]
    fn classify_paths() {
        let segs = split_path("https://acme.freshservice.com/api/_/changes/1/planning-fields/private-value?token=S");
        assert_eq!(segs.join("/"), "api/_/changes/1/planning-fields/[redacted]");
        let (kind, target, path) = classify("PUT", &segs);
        assert_eq!(kind, "update:changes");
        assert_eq!(target.as_deref(), Some("changes/1"));
        assert_eq!(path, "/api/_/changes/1/planning-fields/[redacted]");
        let (kind, _, _) = classify("POST", &split_path("https://x/api/_/changes/1/notes"));
        assert_eq!(kind, "note");
    }
}
