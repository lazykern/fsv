//! Stale-while-revalidate cache files (`cache.py`).
//!
//! A hit is used even when stale (callers refresh in the background), a miss
//! blocks. Files hold a metadata envelope around `data`:
//! `{_kind, saved_at, checked_at, changed_at, hash, etag, latency_ms, fetch_count, data}`.

use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use crate::config::{PRIVATE_MODE, atomic_write_text};
use crate::error::Result;

/// Built-in TTLs in seconds by cache kind.
pub const TTL: &[(&str, u64)] = &[
    // field definitions rarely change; expensive payloads
    ("schema", 7 * 86400),
    // saved-filter list may change daily; cheap payloads
    ("filters", 900),
    // org groups; server supports ETag
    ("groups", 6 * 3600),
    // requester/agent prefix matching
    ("autocomplete", 1800),
    // lookup field completions
    ("lookup", 300),
];

/// TTL for a kind (default one hour).
pub fn ttl_for(kind: &str) -> u64 {
    TTL.iter().find(|(k, _)| *k == kind).map_or(3600, |(_, t)| *t)
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn dump(doc: &Value) -> String {
    fsv_py::Dumps::default().indent(2).ensure_ascii(false).dump(doc)
}

fn read_doc(path: &Path) -> Option<Map<String, Value>> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<Value>(&text).ok()? {
        Value::Object(m) => Some(m),
        _ => None,
    }
}

/// Read a cache file: `(envelope, is_stale)`; `(None, true)` when missing,
/// unreadable or not an object (e.g. a bare list from an older version).
pub fn load(path: &Path) -> (Option<Map<String, Value>>, bool) {
    let Some(doc) = read_doc(path) else { return (None, true) };
    let mut saved = match doc.get("saved_at") {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        _ => 0.0,
    };
    if saved == 0.0 && !doc.contains_key("saved_at") {
        saved = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0.0, |d| d.as_secs_f64());
    }
    let ttl = match doc.get("_kind") {
        Some(Value::String(k)) => ttl_for(k),
        None => ttl_for(""),
        Some(_) => 3600,
    };
    let stale = (now() - saved) > ttl as f64;
    (Some(doc), stale)
}

/// Atomically persist `data` under a fresh envelope.
pub fn save(path: &Path, kind: &str, data: &Value, etag: &str, latency_ms: u64) -> Result<()> {
    let t = now();
    let doc = json!({
        "_kind": kind,
        "saved_at": t,
        "checked_at": t,
        "changed_at": t,
        "hash": "",
        "etag": etag,
        "latency_ms": latency_ms,
        "fetch_count": 1,
        "data": data,
    });
    atomic_write_text(path, &dump(&doc), PRIVATE_MODE)
}

/// Update envelope metadata without touching `data`. Silent on unreadable files.
pub fn update_meta(path: &Path, checked_at: bool, fetch_inc: bool, etag: Option<&str>) -> Result<()> {
    let Some(mut doc) = read_doc(path) else { return Ok(()) };
    if checked_at {
        doc.insert("checked_at".into(), json!(now()));
    }
    if fetch_inc {
        let next = match doc.get("fetch_count") {
            None => json!(2),
            Some(v) => match crate::value::as_int(v) {
                Some(i) => json!(i + 1),
                None => json!(v.as_f64().unwrap_or(1.0) + 1.0),
            },
        };
        doc.insert("fetch_count".into(), next);
    }
    if let Some(e) = etag {
        doc.insert("etag".into(), json!(e));
    }
    atomic_write_text(path, &dump(&Value::Object(doc)), PRIVATE_MODE)
}

/// Force the next check to re-fetch (`saved_at` = 1.0; a missing value would
/// fall back to the file mtime and look fresh).
pub fn mark_stale(path: &Path) -> Result<()> {
    let Some(mut doc) = read_doc(path) else { return Ok(()) };
    doc.insert("saved_at".into(), json!(1.0));
    atomic_write_text(path, &dump(&Value::Object(doc)), PRIVATE_MODE)
}

/// Default lock file: `<path>.lock` (suffix appended).
pub fn lock_path_for(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".lock");
    PathBuf::from(s)
}

/// Refresh the cache on a background thread. A lock file younger than 30s
/// means another refresh is in flight, so this one does nothing. Errors are
/// swallowed. The returned handle may be dropped (the thread is detached).
pub fn refresh_async<F>(path: &Path, kind: &str, fetcher: F, lock_path: Option<&Path>) -> JoinHandle<()>
where
    F: FnOnce() -> Result<Value> + Send + 'static,
{
    let path = path.to_path_buf();
    let kind = kind.to_string();
    let lock = lock_path.map_or_else(|| lock_path_for(&path), Path::to_path_buf);
    std::thread::spawn(move || {
        if let Ok(meta) = std::fs::metadata(&lock) {
            let age = meta.modified().ok().and_then(|m| SystemTime::now().duration_since(m).ok());
            if age.is_none_or(|a| a.as_secs_f64() < 30.0) {
                return;
            }
        }
        let run = || -> Result<()> {
            if let Some(parent) = lock.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&lock, "")?;
            let t0 = Instant::now();
            let data = fetcher()?;
            let latency = t0.elapsed().as_millis() as u64;
            save(&path, &kind, &data, "", latency)
        };
        let _ = run();
        let _ = std::fs::remove_file(&lock);
    })
}
