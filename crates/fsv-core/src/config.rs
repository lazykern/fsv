//! Paths, tenant domain and `config.json` (`config.py`).
//!
//! Paths come from `FSV_CONFIG_DIR` / `FSV_CACHE_DIR`, else `~/.config/fsv`
//! and `~/.cache/fsv` on every platform (not the platform config dir), so a
//! user's existing Python-written state is found. Paths and the domain live
//! in a process-wide lock; [`reload`] re-reads both from the environment.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

use crate::error::{Error, Result};
use crate::pystr;

/// All configured file locations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub config_file: PathBuf,
    pub session_file: PathBuf,
    pub schema_dir: PathBuf,
    pub filters_dir: PathBuf,
}

impl Paths {
    /// Paths derived from the current environment.
    pub fn from_env() -> Self {
        let home = home_dir();
        let config_dir = env_path("FSV_CONFIG_DIR").unwrap_or_else(|| home.join(".config").join("fsv"));
        let cache_dir = env_path("FSV_CACHE_DIR").unwrap_or_else(|| home.join(".cache").join("fsv"));
        Paths {
            config_file: config_dir.join("config.json"),
            session_file: config_dir.join("session.json"),
            schema_dir: cache_dir.join("schema"),
            filters_dir: cache_dir.join("filters"),
            config_dir,
            cache_dir,
        }
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// `Path.home()`.
pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

struct State {
    paths: Paths,
    domain: String,
}

static STATE: LazyLock<RwLock<State>> = LazyLock::new(|| {
    let paths = Paths::from_env();
    let domain = configured_domain_at(&paths.config_file);
    RwLock::new(State { paths, domain })
});

fn read_state<T>(f: impl FnOnce(&State) -> T) -> T {
    match STATE.read() {
        Ok(g) => f(&g),
        Err(p) => f(&p.into_inner()),
    }
}

fn write_state(f: impl FnOnce(&mut State)) {
    match STATE.write() {
        Ok(mut g) => f(&mut g),
        Err(p) => f(&mut p.into_inner()),
    }
}

/// Current paths.
pub fn paths() -> Paths {
    read_state(|s| s.paths.clone())
}

pub fn config_dir() -> PathBuf {
    read_state(|s| s.paths.config_dir.clone())
}

pub fn cache_dir() -> PathBuf {
    read_state(|s| s.paths.cache_dir.clone())
}

pub fn config_file() -> PathBuf {
    read_state(|s| s.paths.config_file.clone())
}

pub fn session_file() -> PathBuf {
    read_state(|s| s.paths.session_file.clone())
}

pub fn schema_dir() -> PathBuf {
    read_state(|s| s.paths.schema_dir.clone())
}

pub fn filters_dir() -> PathBuf {
    read_state(|s| s.paths.filters_dir.clone())
}

/// Configured tenant host (`config.DOMAIN`), `""` when none.
pub fn domain() -> String {
    read_state(|s| s.domain.clone())
}

/// `https://<domain>/api/v2` (`""` without a domain).
pub fn api_v2() -> String {
    urls(&domain()).0
}

/// `https://<domain>/api/_` (`""` without a domain).
pub fn api_int() -> String {
    urls(&domain()).1
}

/// `https://<domain>/` (`""` without a domain).
pub fn login_url() -> String {
    urls(&domain()).2
}

fn urls(domain: &str) -> (String, String, String) {
    if domain.is_empty() {
        return (String::new(), String::new(), String::new());
    }
    (format!("https://{domain}/api/v2"), format!("https://{domain}/api/_"), format!("https://{domain}/"))
}

/// Re-read paths from env and the domain from the config file.
pub fn reload() {
    let paths = Paths::from_env();
    let domain = configured_domain_at(&paths.config_file);
    write_state(|s| {
        s.paths = paths;
        s.domain = domain;
    });
}

/// Human-friendly path (`~/` for home) for messages.
pub fn display_path(path: &Path) -> String {
    display_path_with_home(path, &home_dir())
}

/// [`display_path`] against an explicit home directory.
pub fn display_path_with_home(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rel) => {
            let rel = rel.to_string_lossy();
            if rel.is_empty() { "~/.".into() } else { format!("~/{rel}") }
        }
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

/// Write `text` to `path` atomically (temp file in the same dir + rename) with `mode`.
pub fn atomic_write_text(path: &Path, text: &str, mode: u32) -> Result<()> {
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    std::fs::create_dir_all(&parent)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut tmp = tempfile::Builder::new().prefix(&format!(".{name}.")).suffix(".tmp").tempfile_in(&parent)?;
    tmp.write_all(text.as_bytes())?;
    tmp.flush()?;
    set_mode(tmp.path(), mode)?;
    tmp.persist(path).map_err(|e| Error::Io(e.error))?;
    Ok(())
}

/// Default mode for private files.
pub const PRIVATE_MODE: u32 = 0o600;

#[cfg(unix)]
pub(crate) fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
pub(crate) fn set_mode(_path: &Path, _mode: u32) -> std::io::Result<()> {
    Ok(())
}

/// Create the config (mode 0700), schema and filters directories.
pub fn ensure_dirs() -> Result<()> {
    let p = paths();
    std::fs::create_dir_all(&p.config_dir)?;
    set_mode(&p.config_dir, 0o700)?;
    std::fs::create_dir_all(&p.schema_dir)?;
    std::fs::create_dir_all(&p.filters_dir)?;
    Ok(())
}

fn now_secs() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// Read `config.json`; a corrupt file is moved aside (never silently overwritten).
pub fn load_config() -> Map<String, Value> {
    load_config_at(&config_file())
}

fn load_config_at(file: &Path) -> Map<String, Value> {
    if !file.exists() {
        return Map::new();
    }
    let parsed = std::fs::read(file)
        .map_err(|e| e.to_string())
        .and_then(|b| String::from_utf8(b).map_err(|e| e.to_string()))
        .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()))
        .and_then(|v| match v {
            Value::Object(m) => Ok(m),
            _ => Err("config root is not an object".to_string()),
        });
    match parsed {
        Ok(m) => m,
        Err(e) => {
            let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let backup = file.with_file_name(format!("{name}.corrupt-{}", now_secs() as i64));
            let where_ = match std::fs::rename(file, &backup) {
                Ok(()) => format!("backed up to {}", backup.display()),
                Err(_) => "could not be backed up".to_string(),
            };
            eprintln!("warning: {} is unreadable ({e}); {where_}; starting with empty config", file.display());
            Map::new()
        }
    }
}

/// Write `config.json` (indent=2, sorted keys, trailing newline, mode 0600).
pub fn save_config(data: &Map<String, Value>) -> Result<()> {
    ensure_dirs()?;
    let text = fsv_py::Dumps::default().indent(2).sort_keys(true).dump(&Value::Object(data.clone()));
    atomic_write_text(&config_file(), &format!("{text}\n"), PRIVATE_MODE)
}

/// Validate and normalize a tenant host (`https://Acme.freshservice.com/a` → `acme.freshservice.com`).
///
/// With `warn`, a host outside `*.freshservice.com` prints a warning.
pub fn normalize_domain(value: &str, warn: bool) -> Result<String> {
    let raw = pystr::strip(value);
    if raw.is_empty() {
        return Err(Error::value("empty domain"));
    }
    if raw.chars().any(pystr::is_space) {
        return Err(Error::value("domain must not contain whitespace"));
    }
    let full = if raw.contains("://") { raw.to_string() } else { format!("https://{raw}") };
    let parsed = pystr::urlsplit(&full).map_err(Error::Value)?;
    let netloc = if parsed.netloc.is_empty() {
        pystr::urlparse_path(&parsed.path).split('/').next().unwrap_or("").to_string()
    } else {
        parsed.netloc.clone()
    };
    if netloc.contains('@') {
        return Err(Error::value("domain must not contain user info ('@')"));
    }
    let lower = netloc.to_lowercase();
    let host = lower.trim_end_matches('/');
    if host.contains(':') {
        return Err(Error::value("domain must not contain a port"));
    }
    if host.is_empty() || !host.contains('.') || host.starts_with('.') || host.ends_with('.') {
        return Err(Error::value("domain must look like acme.freshservice.com"));
    }
    if warn && !host.ends_with(".freshservice.com") {
        eprintln!("warning: {host} is not a *.freshservice.com domain; session cookies will be sent to it");
    }
    Ok(host.to_string())
}

fn configured_domain_at(file: &Path) -> String {
    match load_config_at(file).get("domain") {
        Some(v) if crate::value::truthy(v) => normalize_domain(&fsv_py::py_str(v), false).unwrap_or_default(),
        _ => String::new(),
    }
}

/// Validate, persist and activate a new tenant domain.
pub fn set_domain(value: &str) -> Result<String> {
    let domain = normalize_domain(value, true)?;
    let mut cfg = load_config();
    cfg.insert("domain".into(), Value::String(domain.clone()));
    save_config(&cfg)?;
    write_state(|s| s.domain = domain.clone());
    Ok(domain)
}

/// The configured domain, or the "run `fsv auth login`" error.
pub fn require_domain() -> Result<String> {
    let d = domain();
    if d.is_empty() {
        return Err(Error::other("no Freshservice domain; run `fsv auth login --domain acme.freshservice.com`"));
    }
    Ok(d)
}

/// File-name key for per-tenant cache files.
pub fn cache_domain_key(domain: Option<&str>) -> String {
    let given = domain.filter(|d| !d.is_empty()).map(str::to_string);
    let current = self::domain();
    let raw_owned = given
        .or_else(|| if current.is_empty() { None } else { Some(current) })
        .unwrap_or_else(|| configured_domain_at(&config_file()));
    let raw = pystr::strip(&raw_owned);
    if raw.is_empty() {
        return "default".into();
    }
    normalize_domain(raw, false).unwrap_or_else(|_| raw.to_lowercase().replace('/', "-"))
}

pub fn schema_cache_path(name: &str, domain: Option<&str>) -> PathBuf {
    schema_dir().join(format!("{}--{name}.json", cache_domain_key(domain)))
}

pub fn schema_cache_candidates(name: &str, domain: Option<&str>) -> Vec<PathBuf> {
    vec![schema_cache_path(name, domain)]
}

pub fn filters_cache_path(name: &str, domain: Option<&str>) -> PathBuf {
    filters_dir().join(format!("{}--{name}.json", cache_domain_key(domain)))
}

pub fn filters_cache_candidates(name: &str, domain: Option<&str>) -> Vec<PathBuf> {
    vec![filters_cache_path(name, domain)]
}

pub fn groups_cache_path(domain: Option<&str>) -> PathBuf {
    cache_dir().join(format!("groups--{}.json", cache_domain_key(domain)))
}

pub fn groups_cache_candidates(domain: Option<&str>) -> Vec<PathBuf> {
    vec![groups_cache_path(domain)]
}

/// Saved default `--where` expressions for a resource.
pub fn get_default_where(res_name: &str) -> Vec<String> {
    let cfg = load_config();
    let items = cfg
        .get("defaults")
        .filter(|v| crate::value::truthy(v))
        .and_then(|d| d.get(res_name))
        .and_then(|r| r.get("where"))
        .filter(|v| crate::value::truthy(v));
    match items {
        Some(Value::Array(a)) => a.iter().map(fsv_py::py_str).collect(),
        Some(Value::String(s)) => s.chars().map(String::from).collect(),
        Some(Value::Object(o)) => o.keys().cloned().collect(),
        _ => Vec::new(),
    }
}

pub fn set_default_where(res_name: &str, exprs: &[String]) -> Result<()> {
    let mut cfg = load_config();
    let defaults = cfg.entry("defaults").or_insert_with(|| Value::Object(Map::new()));
    if !defaults.is_object() {
        *defaults = Value::Object(Map::new());
    }
    if let Value::Object(d) = defaults {
        let res = d.entry(res_name).or_insert_with(|| Value::Object(Map::new()));
        if !res.is_object() {
            *res = Value::Object(Map::new());
        }
        if let Value::Object(r) = res {
            r.insert("where".into(), Value::Array(exprs.iter().cloned().map(Value::String).collect()));
        }
    }
    save_config(&cfg)
}

pub fn clear_default_where(res_name: &str) -> Result<()> {
    let mut cfg = load_config();
    if let Some(Value::Object(d)) = cfg.get_mut("defaults") {
        d.shift_remove(res_name);
    }
    save_config(&cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_domain_rejects() {
        for bad in [
            "user@acme.freshservice.com",
            "https://u:p@acme.freshservice.com",
            "acme.freshservice.com:8443",
            "acme .freshservice.com",
            "acme",
            ".com",
        ] {
            assert!(normalize_domain(bad, false).is_err(), "{bad}");
        }
        assert_eq!(normalize_domain("https://ACME.freshservice.com/a/dashboard", false).unwrap(), "acme.freshservice.com");
    }

    #[test]
    fn display_path_home() {
        let home = Path::new("/h/me");
        assert_eq!(display_path_with_home(Path::new("/h/me/x"), home), "~/x");
        assert_eq!(display_path_with_home(Path::new("/tmp/x"), home), "/tmp/x");
    }
}
