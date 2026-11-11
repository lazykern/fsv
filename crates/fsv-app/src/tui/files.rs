//! The TUI's local files: attachment downloads (`cli_sr._download`), the
//! download inventory rows (`cli_sr._attachment_rows`), filter history
//! (`app._load_filter_history` / `_save_filter_history`) and the cached
//! agent groups (`completion._cached_groups`).

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use fsv_api::catalog::{domain_url, is_file_value, safe_target};
use fsv_api::create::attachment_url;
use fsv_api::service::Api;
use fsv_core::config::Paths;
use fsv_py::{py_str, truthy};

/// Python keeps the newest 100 filters.
pub const HISTORY_MAX: usize = 100;

fn objects(v: Option<&Value>) -> impl Iterator<Item = &Map<String, Value>> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
}

fn with_source(row: &Map<String, Value>, source: &str) -> Value {
    let mut row = row.clone();
    row.insert("source".into(), json!(source));
    Value::Object(row)
}

/// `_attachment_rows(record, items, [{} ...])`: record attachments, then
/// files in requested-item file fields tagged with the field name
/// (deduplicated by id).
pub fn attachment_rows(record: &Value, items: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut by_id: HashMap<String, usize> = HashMap::new();
    for a in objects(record.get("attachments")) {
        out.push(with_source(a, "request"));
        if let Some(id) = a.get("id").filter(|v| !v.is_null()) {
            by_id.insert(py_str(id), out.len() - 1);
        }
    }
    for it in items {
        let Some(fields) = it.get("custom_fields").and_then(Value::as_object) else {
            continue;
        };
        for (name, v) in fields {
            if !is_file_value(v) {
                continue;
            }
            for x in v.as_array().into_iter().flatten() {
                let key = match x {
                    Value::Object(m) => py_str(m.get("id").unwrap_or(&Value::Null)),
                    other => py_str(other),
                };
                if let Some(&i) = by_id.get(&key) {
                    out[i]["source"] = json!(name);
                    continue;
                }
                let row = match x {
                    Value::Object(m) => with_source(m, name),
                    other => {
                        json!({"id": other, "name": format!("attachment-{}", py_str(other)), "source": name})
                    }
                };
                out.push(row);
                by_id.insert(key, out.len() - 1);
            }
        }
    }
    out
}

/// Rows of a `{"attachments": [...]}` list tagged with `source`.
pub fn sourced(list: Option<&Value>, source: &str) -> Vec<Value> {
    objects(list).map(|a| with_source(a, source)).collect()
}

fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(bytes)
}

/// `cli_sr._download(c, att, directory, force=False)`: never overwrites;
/// the result row's `status` is `downloaded`, `exists`, `rejected` or `error`.
pub fn download(c: &dyn Api, att: &Value, directory: &str) -> fsv_core::Result<Value> {
    let name = match att.get("name").filter(|v| truthy(v)) {
        Some(n) => py_str(n),
        None => format!(
            "attachment-{}",
            att.get("id").map(py_str).unwrap_or_default()
        ),
    };
    let mut row = Map::new();
    row.insert("id".into(), att.get("id").cloned().unwrap_or(Value::Null));
    row.insert("name".into(), json!(name));
    row.insert(
        "source".into(),
        att.get("source").cloned().unwrap_or(Value::Null),
    );
    let finish = |mut row: Map<String, Value>, extra: Value| {
        if let Value::Object(m) = extra {
            row.extend(m);
        }
        Ok(Value::Object(row))
    };
    let target = match safe_target(directory, &name) {
        Ok(t) => t,
        Err(e) => return finish(row, json!({"status": "rejected", "error": e.to_string()})),
    };
    let domain = c.domain();
    let mut url = attachment_url(att, &domain);
    if url.is_empty()
        && let Some(id) = att.get("id").filter(|v| !v.is_null())
    {
        url = domain_url(
            &format!("/helpdesk/attachments/{}?download=true", py_str(id)),
            &domain,
        );
    }
    if url.is_empty() {
        return finish(row, json!({"status": "error", "error": "no download URL"}));
    }
    let shown = target.to_string_lossy().into_owned();
    if target.exists() {
        return finish(row, json!({"path": shown, "status": "exists"}));
    }
    let r = c.fetch(&url)?;
    if r.status >= 400 {
        return finish(
            row,
            json!({"status": "error", "error": format!("HTTP {}", r.status)}),
        );
    }
    std::fs::create_dir_all(expanduser(directory))?;
    match write_new(&target, &r.body) {
        Ok(()) => finish(
            row,
            json!({"path": shown, "size": r.body.len(), "status": "downloaded"}),
        ),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            finish(row, json!({"path": shown, "status": "exists"}))
        }
        Err(e) => Err(e.into()),
    }
}

fn expanduser(p: &str) -> PathBuf {
    match (p.strip_prefix('~'), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with('/') => {
            PathBuf::from(home).join(rest.trim_start_matches('/'))
        }
        _ => PathBuf::from(p),
    }
}

/// `filters_cache_path("tui_history")` under `paths`.
pub fn history_path(paths: &Paths, domain: &str) -> PathBuf {
    let key = fsv_core::config::cache_domain_key(Some(domain));
    paths.filters_dir.join(format!("{key}--tui_history.json"))
}

/// `groups_cache_path()` under `paths`.
pub fn groups_path(paths: &Paths, domain: &str) -> PathBuf {
    let key = fsv_core::config::cache_domain_key(Some(domain));
    paths.cache_dir.join(format!("groups--{key}.json"))
}

/// `json.loads(path.read_text()).get("history", [])[:100]`; anything
/// unreadable is no history.
pub fn load_history(path: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(m)) => match m.get("history") {
            Some(Value::Array(a)) => a
                .iter()
                .take(HISTORY_MAX)
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    other => py_str(other),
                })
                .collect(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// `ensure_dirs()` then `path.write_text(json.dumps({"history": h[:100]}))`;
/// failures are ignored like Python's.
pub fn save_history(paths: &Paths, path: &Path, history: &[String]) {
    let ensure = || -> std::io::Result<()> {
        std::fs::create_dir_all(&paths.config_dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&paths.config_dir, std::fs::Permissions::from_mode(0o700))?;
        }
        std::fs::create_dir_all(&paths.schema_dir)?;
        std::fs::create_dir_all(&paths.filters_dir)
    };
    if ensure().is_err() {
        return;
    }
    let kept: Vec<&String> = history.iter().take(HISTORY_MAX).collect();
    let text = fsv_py::Dumps::default().dump(&json!({ "history": kept }));
    let _ = std::fs::write(path, text);
}

/// `completion._cached_groups()` names (groups without a name skipped; a
/// malformed list is no groups, like Python's swallowed error).
pub fn cached_group_names(path: &Path) -> Vec<String> {
    let (Some(doc), _) = fsv_core::cache::load(path) else {
        return Vec::new();
    };
    let data = doc.get("data").filter(|v| truthy(v));
    let groups = match data {
        Some(Value::Object(m)) => m.get("groups").cloned(),
        Some(other) => Some(other.clone()),
        None => doc.get("groups").cloned(),
    };
    let groups = match groups {
        Some(Value::Array(a)) => a,
        _ => match doc.get("groups") {
            Some(Value::Array(a)) => a.clone(),
            _ => return Vec::new(),
        },
    };
    if groups.iter().any(|g| !g.is_object()) {
        return Vec::new();
    }
    groups
        .iter()
        .filter_map(|g| g.get("name").map(py_str))
        .collect()
}
