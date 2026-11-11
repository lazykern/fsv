//! The TUI's local files: attachment downloads (`cli_sr._download`), the
//! download inventory rows (`cli_sr._attachment_rows`), filter history
//! (`app._load_filter_history` / `_save_filter_history`) and the cached
//! agent groups (`completion._cached_groups`).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use fsv_core::config::Paths;
use fsv_py::{py_str, truthy};

/// Python keeps the newest 100 filters.
pub const HISTORY_MAX: usize = 100;

pub use fsv_api::downloads::{attachment_rows, download, sourced};

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
