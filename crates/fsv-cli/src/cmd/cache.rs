//! `fsv cache status|refresh|clear` and the cache warmers.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use fsv_core::resource::{REGISTRY, Resource};
use fsv_core::term::{Column, Justify, Table, out};
use fsv_core::{Client, Ctx, Result, cache, config};

use crate::generated::CacheClearArgs;
use crate::pending;
use crate::util::{CmdResult, usage};

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

fn mtime(p: &Path) -> f64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0.0, |d| d.as_secs_f64())
}

fn first_doc(candidates: Vec<PathBuf>) -> Option<(PathBuf, Map<String, Value>, bool)> {
    for p in candidates {
        if let (Some(doc), stale) = cache::load(&p) {
            return Some((p, doc, stale));
        }
    }
    None
}

fn ttl_text(kind: &str) -> String {
    let ttl = cache::ttl_for(kind) as f64;
    if ttl >= 3600.0 {
        format!("{:.0}h", ttl / 3600.0)
    } else {
        format!("{:.0}m", ttl / 60.0)
    }
}

fn status_row(
    name: &str,
    label: &str,
    kind: &str,
    found: Option<(PathBuf, Map<String, Value>, bool)>,
) -> Option<[String; 5]> {
    let (path, doc, stale) = found?;
    let saved = doc
        .get("saved_at")
        .and_then(Value::as_f64)
        .filter(|s| *s != 0.0)
        .unwrap_or_else(|| mtime(&path));
    let age = (now() - saved) / 3600.0;
    Some([
        name.replace(".json", ""),
        label.to_string(),
        format!("{age:.1}h"),
        if stale { "stale" } else { "ok" }.to_string(),
        ttl_text(kind),
    ])
}

/// `fsv cache status`.
pub fn status() -> CmdResult {
    let mut rows = Vec::new();
    for res in REGISTRY {
        let pairs = [
            (
                "schema",
                "schema",
                config::schema_cache_candidates(res.name, None),
            ),
            (
                "filters",
                "views",
                config::filters_cache_candidates(res.name, None),
            ),
        ];
        for (kind, label, candidates) in pairs {
            rows.extend(status_row(res.name, label, kind, first_doc(candidates)));
        }
    }
    rows.extend(status_row(
        "groups",
        "groups",
        "groups",
        first_doc(config::groups_cache_candidates(None)),
    ));
    let mut t = Table::new().title("cache status");
    for c in ["cache", "kind", "age", "state", "ttl"] {
        t.add_column(Column::new(c));
    }
    for r in rows {
        t.add_row(r);
    }
    out().print_table(&t);
    Ok(())
}

fn new_client() -> Result<Arc<Client>> {
    Ok(Arc::new(Client::new(Arc::new(Ctx::from_process()))?))
}

fn first_filters_list(data: &Value) -> Value {
    data.as_object()
        .and_then(|m| m.iter().find(|(k, _)| k.ends_with("_filters")))
        .map_or_else(|| json!([]), |(_, v)| v.clone())
}

fn fetch_filters(res: &Resource) -> Result<(String, usize)> {
    let c = new_client()?;
    let data = c.int_get(res.filters_path.unwrap_or(""), None)?;
    let filters = first_filters_list(&data);
    let n = filters.as_array().map_or(0, Vec::len);
    cache::save(
        &config::filters_cache_path(res.name, None),
        "filters",
        &json!({"filters": filters}),
        "",
        0,
    )?;
    Ok((format!("{} views", res.name), n))
}

fn fetch_groups() -> Result<(String, usize)> {
    let c = new_client()?;
    let data = c.int_get("bootstrap/agents_groups", None)?;
    let groups = data.get("groups").cloned().unwrap_or_else(|| json!([]));
    let n = groups.as_array().map_or(0, Vec::len);
    cache::save(
        &config::groups_cache_path(None),
        "groups",
        &json!({"groups": groups}),
        "",
        0,
    )?;
    Ok(("groups".into(), n))
}

type Task = Box<dyn FnOnce() -> Result<(String, usize)> + Send>;

fn tasks() -> Vec<Task> {
    let mut out: Vec<Task> = Vec::new();
    for res in REGISTRY {
        out.push(Box::new(move || {
            new_client().and_then(|c| pending::refresh_schema(res, &c))
        }));
        if res.filters_path.is_some() {
            out.push(Box::new(move || fetch_filters(res)));
        }
    }
    out.push(Box::new(fetch_groups));
    out
}

/// `_refresh_cache(verbose=True, blocking=True)`; rows in completion order.
pub fn refresh() -> CmdResult {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::scope(|s| {
        for task in tasks() {
            let tx = tx.clone();
            s.spawn(move || {
                let _ = tx.send(task());
            });
        }
        drop(tx);
    });
    let mut t = Table::new().title("cache refresh");
    t.add_column(Column::new("resource").style("bold"));
    t.add_column(Column::new("count").justify(Justify::Right));
    for (name, count) in rx.into_iter().flatten() {
        t.add_row([name, count.to_string()]);
    }
    out().print_table(&t);
    Ok(())
}

/// `_refresh_cache(blocking=False)`: detached warmers.
pub fn refresh_background() {
    for task in tasks() {
        std::thread::spawn(move || {
            let _ = task();
        });
    }
}

/// `fsv cache clear TARGET`.
pub fn clear(a: CacheClearArgs) -> CmdResult {
    let target = a.target.unwrap_or_default();
    if !matches!(
        target.as_str(),
        "schema" | "views" | "filters" | "groups" | "all"
    ) {
        return Err(usage("choose schema|views|groups|all"));
    }
    if matches!(target.as_str(), "schema" | "all") {
        let dir = config::schema_dir();
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
            out().print("schema cache cleared");
        }
    }
    if matches!(target.as_str(), "views" | "filters" | "all") {
        let dir = config::cache_dir().join("filters");
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
            out().print("views cache cleared");
        }
    }
    if matches!(target.as_str(), "groups" | "all") {
        let mut removed = false;
        for p in config::groups_cache_candidates(None) {
            if p.exists() {
                std::fs::remove_file(&p)?;
                removed = true;
            }
        }
        if removed {
            out().print("groups cache cleared");
        }
    }
    Ok(())
}
