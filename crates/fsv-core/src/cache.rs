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
    TTL.iter()
        .find(|(k, _)| *k == kind)
        .map_or(3600, |(_, t)| *t)
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn dump(doc: &Value) -> String {
    fsv_py::Dumps::default()
        .indent(2)
        .ensure_ascii(false)
        .dump(doc)
}

/// Unique temp file + rename, mode 0600.
fn atomic_write(path: &Path, doc: &Value) -> Result<()> {
    atomic_write_text(path, &dump(doc), PRIVATE_MODE)
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
    let Some(doc) = read_doc(path) else {
        return (None, true);
    };
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
    atomic_write(path, &doc)
}

/// Update envelope metadata without touching `data`. Silent on unreadable files.
pub fn update_meta(
    path: &Path,
    checked_at: bool,
    fetch_inc: bool,
    etag: Option<&str>,
) -> Result<()> {
    let Some(mut doc) = read_doc(path) else {
        return Ok(());
    };
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
    atomic_write(path, &Value::Object(doc))
}

/// Force the next check to re-fetch (`saved_at` = 1.0; a missing value would
/// fall back to the file mtime and look fresh).
pub fn mark_stale(path: &Path) -> Result<()> {
    let Some(mut doc) = read_doc(path) else {
        return Ok(());
    };
    doc.insert("saved_at".into(), json!(1.0));
    atomic_write(path, &Value::Object(doc))
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
pub fn refresh_async<F>(
    path: &Path,
    kind: &str,
    fetcher: F,
    lock_path: Option<&Path>,
) -> JoinHandle<()>
where
    F: FnOnce() -> Result<Value> + Send + 'static,
{
    let path = path.to_path_buf();
    let kind = kind.to_string();
    let lock = lock_path.map_or_else(|| lock_path_for(&path), Path::to_path_buf);
    std::thread::spawn(move || {
        if let Ok(meta) = std::fs::metadata(&lock) {
            let age = meta
                .modified()
                .ok()
                .and_then(|m| SystemTime::now().duration_since(m).ok());
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn write(path: &Path, v: &Value) {
        std::fs::write(path, v.to_string()).unwrap();
    }

    fn doc(path: &Path) -> Map<String, Value> {
        load(path).0.unwrap()
    }

    #[test]
    fn ttl_for_kinds() {
        let got: Vec<u64> = [
            "schema",
            "filters",
            "groups",
            "autocomplete",
            "lookup",
            "nope",
            "",
        ]
        .map(ttl_for)
        .to_vec();
        assert_eq!(got, vec![604800, 900, 21600, 1800, 300, 3600, 3600]);
    }

    #[test]
    fn save_writes_envelope_and_load_reports_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub").join("c.json");
        assert_eq!(load(&p), (None, true));
        save(&p, "filters", &json!({"a": [1]}), "E", 12).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(
            text.starts_with("{\n  \"_kind\": \"filters\",\n  \"saved_at\": "),
            "{text}"
        );
        let (d, stale) = load(&p);
        let d = d.unwrap();
        let keys: std::collections::BTreeSet<&str> = d.keys().map(String::as_str).collect();
        assert_eq!(
            keys.into_iter().collect::<Vec<_>>(),
            [
                "_kind",
                "changed_at",
                "checked_at",
                "data",
                "etag",
                "fetch_count",
                "hash",
                "latency_ms",
                "saved_at"
            ]
        );
        assert_eq!(d["_kind"], "filters");
        assert_eq!(d["hash"], "");
        assert_eq!(d["etag"], "E");
        assert_eq!(d["latency_ms"], 12);
        assert_eq!(d["fetch_count"], 1);
        assert_eq!(d["data"], json!({"a": [1]}));
        assert!(d["saved_at"] == d["checked_at"] && d["checked_at"] == d["changed_at"]);
        assert!(!stale);
        let names: Vec<_> = std::fs::read_dir(p.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["c.json"], "no temp leftovers");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn load_staleness_rules() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.json");
        let old = now() - 1000.0;
        write(&p, &json!({"_kind": "filters", "saved_at": old}));
        assert!(load(&p).1, "older than the 900s filters TTL");
        write(&p, &json!({"_kind": "schema", "saved_at": old}));
        assert!(!load(&p).1);
        write(&p, &json!({"_kind": "schema", "saved_at": "bad"}));
        assert!(load(&p).1, "a non-numeric saved_at counts as 0");
        write(&p, &json!({"_kind": "schema", "saved_at": true}));
        assert!(load(&p).1, "a bool saved_at counts as 0");
        write(&p, &json!({"_kind": "schema"}));
        assert!(!load(&p).1, "no saved_at falls back to the file mtime");
        write(&p, &json!([1, 2, 3]));
        assert_eq!(load(&p), (None, true));
        std::fs::write(&p, "{bad").unwrap();
        assert_eq!(load(&p), (None, true));
    }

    #[test]
    fn update_meta_and_mark_stale() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.json");
        save(&p, "schema", &json!([{"name": "x"}]), "", 0).unwrap();
        update_meta(&p, false, true, Some("F")).unwrap();
        let d = doc(&p);
        assert_eq!(
            (d["fetch_count"].clone(), d["etag"].clone()),
            (json!(2), json!("F"))
        );
        let mut d = doc(&p);
        d.shift_remove("fetch_count");
        write(&p, &Value::Object(d));
        update_meta(&p, false, true, None).unwrap();
        assert_eq!(doc(&p)["fetch_count"], 2, "a missing count defaults to 1");
        let before = doc(&p)["checked_at"].as_f64().unwrap();
        std::thread::sleep(Duration::from_millis(5));
        update_meta(&p, true, false, None).unwrap();
        assert!(doc(&p)["checked_at"].as_f64().unwrap() > before);
        assert_eq!(doc(&p)["data"], json!([{"name": "x"}]));

        assert!(!load(&p).1);
        mark_stale(&p).unwrap();
        let (d, stale) = load(&p);
        assert_eq!(d.unwrap()["saved_at"], json!(1.0));
        assert!(stale);
    }

    #[test]
    fn atomic_write_is_private_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("d").join("f.json");
        atomic_write(&p, &json!({"a": "é"})).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{\n  \"a\": \"é\"\n}");
        let names: Vec<_> = std::fs::read_dir(p.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["f.json"]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn unreadable_files_are_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.json");
        std::fs::write(&p, "[1, 2, 3]").unwrap();
        update_meta(&p, true, true, Some("x")).unwrap();
        mark_stale(&p).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "[1, 2, 3]");
        std::fs::write(&p, "{bad").unwrap();
        update_meta(&p, true, false, None).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{bad");
        let missing = dir.path().join("none.json");
        update_meta(&missing, true, false, None).unwrap();
        mark_stale(&missing).unwrap();
        assert!(!missing.exists());
    }

    #[test]
    fn refresh_async_saves_and_removes_lock() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("g.json");
        assert_eq!(lock_path_for(&p), dir.path().join("g.json.lock"));
        refresh_async(&p, "groups", || Ok(json!(["g1"])), None)
            .join()
            .unwrap();
        let d = doc(&p);
        assert_eq!(
            (d["_kind"].clone(), d["data"].clone(), d["etag"].clone()),
            (json!("groups"), json!(["g1"]), json!(""))
        );
        assert!(!lock_path_for(&p).exists());
    }

    #[test]
    fn refresh_async_skips_when_recently_locked_and_swallows_errors() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("g.json");
        let lock = dir.path().join("locks").join("g.lock");
        std::fs::create_dir_all(lock.parent().unwrap()).unwrap();
        std::fs::write(&lock, "").unwrap();
        refresh_async(&p, "groups", || Ok(json!(1)), Some(&lock))
            .join()
            .unwrap();
        assert!(
            !p.exists(),
            "a fresh lock means another refresh is in flight"
        );
        assert!(lock.exists(), "someone else's lock is not removed");

        std::fs::remove_file(&lock).unwrap();
        refresh_async(
            &p,
            "groups",
            || Err(crate::Error::other("boom")),
            Some(&lock),
        )
        .join()
        .unwrap();
        assert!(!p.exists());
        assert!(!lock.exists(), "lock removed after a failed fetch");
    }
}
