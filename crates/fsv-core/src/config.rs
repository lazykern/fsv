//! Paths, tenant domain and `config.json` (`config.py`).
//!
//! Paths come from `FSV_CONFIG_DIR` / `FSV_CACHE_DIR`, else `~/.config/fsv`
//! and `~/.cache/fsv` on every platform (not the platform config dir), so a
//! user's existing Python-written state is found. Paths and the domain live
//! in a process-wide lock; [`reload`] re-reads both from the environment.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
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
    /// `refresh_paths`: paths derived from the current environment.
    pub fn from_env() -> Self {
        let home = home_dir();
        let config_dir =
            env_path("FSV_CONFIG_DIR").unwrap_or_else(|| home.join(".config").join("fsv"));
        let cache_dir =
            env_path("FSV_CACHE_DIR").unwrap_or_else(|| home.join(".cache").join("fsv"));
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
    std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// `Path.home()`.
pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

struct State {
    paths: Paths,
    domain: String,
}

static STATE: RwLock<Option<State>> = RwLock::new(None);

fn from_env() -> State {
    let paths = Paths::from_env();
    let domain = configured_domain_at(&paths.config_file);
    State { paths, domain }
}

fn read_state<T>(f: impl FnOnce(&State) -> T) -> T {
    {
        let g = STATE.read().unwrap_or_else(|p| p.into_inner());
        if let Some(state) = g.as_ref() {
            return f(state);
        }
    }
    let mut g = STATE.write().unwrap_or_else(|p| p.into_inner());
    f(g.get_or_insert_with(from_env))
}

fn write_state(f: impl FnOnce(&mut State)) {
    let mut g = STATE.write().unwrap_or_else(|p| p.into_inner());
    f(g.get_or_insert_with(from_env))
}

/// Tests: use `paths` and `domain` for the process-wide config without
/// reading the environment or any config file (the user's real config is
/// never touched, even on first use).
#[doc(hidden)]
pub fn use_for_tests(paths: Paths, domain: &str) {
    *TEST_STATE_HOME.write().unwrap_or_else(|p| p.into_inner()) =
        Some(paths.config_dir.join("state"));
    let mut g = STATE.write().unwrap_or_else(|p| p.into_inner());
    *g = Some(State {
        paths,
        domain: domain.to_string(),
    });
}

static TEST_STATE_HOME: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);

/// The state directory [`use_for_tests`] set, so test journals never land
/// under the user's real `$XDG_STATE_HOME`.
#[doc(hidden)]
pub fn test_state_home() -> Option<PathBuf> {
    TEST_STATE_HOME.read().unwrap_or_else(|p| p.into_inner()).clone()
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
    (
        format!("https://{domain}/api/v2"),
        format!("https://{domain}/api/_"),
        format!("https://{domain}/"),
    )
}

/// Re-read paths from env and the domain from the config file.
pub fn reload() {
    let fresh = from_env();
    write_state(|s| *s = fresh);
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
            if rel.is_empty() {
                "~/.".into()
            } else {
                format!("~/{rel}")
            }
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
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut tmp = tempfile::Builder::new()
        .prefix(&format!(".{name}."))
        .suffix(".tmp")
        .tempfile_in(&parent)?;
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
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
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
            let name = file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let backup = file.with_file_name(format!("{name}.corrupt-{}", now_secs() as i64));
            let where_ = match std::fs::rename(file, &backup) {
                Ok(()) => format!("backed up to {}", backup.display()),
                Err(_) => "could not be backed up".to_string(),
            };
            eprintln!(
                "warning: {} is unreadable ({e}); {where_}; starting with empty config",
                file.display()
            );
            Map::new()
        }
    }
}

/// Write `config.json` (indent=2, sorted keys, trailing newline, mode 0600).
pub fn save_config(data: &Map<String, Value>) -> Result<()> {
    ensure_dirs()?;
    let text = fsv_py::Dumps::default()
        .indent(2)
        .sort_keys(true)
        .dump(&Value::Object(data.clone()));
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
    let full = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    let parsed = pystr::urlsplit(&full).map_err(Error::Value)?;
    let netloc = if parsed.netloc.is_empty() {
        pystr::urlparse_path(&parsed.path)
            .split('/')
            .next()
            .unwrap_or("")
            .to_string()
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
        eprintln!(
            "warning: {host} is not a *.freshservice.com domain; session cookies will be sent to it"
        );
    }
    Ok(host.to_string())
}

fn configured_domain_at(file: &Path) -> String {
    match load_config_at(file).get("domain") {
        Some(v) if crate::value::truthy(v) => {
            normalize_domain(&fsv_py::py_str(v), false).unwrap_or_default()
        }
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
        return Err(Error::other(
            "no Freshservice domain; run `fsv auth login --domain acme.freshservice.com`",
        ));
    }
    Ok(d)
}

/// File-name key for per-tenant cache files.
pub fn cache_domain_key(domain: Option<&str>) -> String {
    let given = domain.filter(|d| !d.is_empty()).map(str::to_string);
    let current = self::domain();
    let raw_owned = given
        .or(if current.is_empty() {
            None
        } else {
            Some(current)
        })
        .unwrap_or_else(|| configured_domain_at(&config_file()));
    let raw = pystr::strip(&raw_owned);
    if raw.is_empty() {
        return "default".into();
    }
    normalize_domain(raw, false).unwrap_or_else(|_| raw.to_lowercase().replace('/', "-"))
}

/// Workspace half of schema and form-field cache keys. The
/// `forms.workspace_id` override, else `wsauto`.
pub fn cache_workspace_key() -> String {
    let config = load_config();
    // `str(workspace_id).strip()` when the value is neither null nor `""`.
    // A non-object `forms` is `wsauto` (Python raises AttributeError).
    let raw = config
        .get("forms")
        .and_then(|f| f.get("workspace_id"))
        .filter(|v| !v.is_null() && v.as_str() != Some(""));
    let text = match raw {
        Some(v) => crate::pystr::strip(&fsv_py::py_str(v)).to_string(),
        None => String::new(),
    };
    if !text.is_empty() && text.is_ascii() && text.chars().all(|c| c.is_ascii_alphanumeric()) {
        format!("ws{text}")
    } else {
        "wsauto".into()
    }
}

/// `<domain>--ws<id>`. Older `<domain>--<name>` files are never read.
pub fn cache_scope_key(domain: Option<&str>) -> String {
    format!("{}--{}", cache_domain_key(domain), cache_workspace_key())
}

pub fn schema_cache_path(name: &str, domain: Option<&str>) -> PathBuf {
    schema_dir().join(format!("{}--{name}.json", cache_scope_key(domain)))
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
    let defaults = cfg
        .entry("defaults")
        .or_insert_with(|| Value::Object(Map::new()));
    if !defaults.is_object() {
        *defaults = Value::Object(Map::new());
    }
    if let Value::Object(d) = defaults {
        let res = d
            .entry(res_name)
            .or_insert_with(|| Value::Object(Map::new()));
        if !res.is_object() {
            *res = Value::Object(Map::new());
        }
        if let Value::Object(r) = res {
            r.insert(
                "where".into(),
                Value::Array(exprs.iter().cloned().map(Value::String).collect()),
            );
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
    use crate::testenv::{self, isolated};
    use serde_json::json;

    fn rel(p: &Path, env: &testenv::Isolated) -> String {
        format!("<D>/{}", p.strip_prefix(env.root()).unwrap().display())
    }

    #[test]
    fn refresh_paths_follow_env() {
        let env = isolated();
        let p = Paths::from_env();
        assert_eq!(p.config_dir, env.config_dir());
        assert_eq!(p.session_file, env.config_dir().join("session.json"));
        assert_eq!(p.config_file, env.config_dir().join("config.json"));
        assert_eq!(p.schema_dir, env.cache_dir().join("schema"));
        assert_eq!(p.filters_dir, env.cache_dir().join("filters"));
        testenv::set("FSV_CONFIG_DIR", env.root().join("c2"));
        assert_eq!(
            session_file(),
            env.config_dir().join("session.json"),
            "globals change only on reload"
        );
        reload();
        assert_eq!(session_file(), env.root().join("c2").join("session.json"));
        assert_eq!(paths(), Paths::from_env());
    }

    #[test]
    fn display_path_uses_tilde_for_home() {
        assert_eq!(display_path(&home_dir().join("x")), "~/x");
        let elsewhere = Path::new("/nonexistent-root/x");
        assert_eq!(display_path(elsewhere), "/nonexistent-root/x");
    }

    #[test]
    fn urls_for_domain() {
        assert_eq!(urls(""), (String::new(), String::new(), String::new()));
        assert_eq!(
            urls("acme.freshservice.com"),
            (
                "https://acme.freshservice.com/api/v2".to_string(),
                "https://acme.freshservice.com/api/_".to_string(),
                "https://acme.freshservice.com/".to_string()
            )
        );
    }

    #[test]
    fn atomic_write_text_sets_mode_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t").join("a.txt");
        atomic_write_text(&p, "hi", 0o640).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "hi");
        let names: Vec<_> = std::fs::read_dir(p.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["a.txt"]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
                0o640
            );
        }
    }

    #[test]
    fn save_and_load_config() {
        let env = isolated();
        assert_eq!(load_config(), Map::new());
        let data = json!({"z": 1, "domain": "acme.freshservice.com", "a": {"y": [1, 2], "b": "é"}});
        save_config(data.as_object().unwrap()).unwrap();
        assert_eq!(
            std::fs::read_to_string(config_file()).unwrap(),
            "{\n  \"a\": {\n    \"b\": \"\\u00e9\",\n    \"y\": [\n      1,\n      2\n    ]\n  },\n  \"domain\": \"acme.freshservice.com\",\n  \"z\": 1\n}\n"
        );
        assert_eq!(Value::Object(load_config()), data);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&env.config_dir()), 0o700);
            assert_eq!(mode(&config_file()), 0o600);
        }
        let mut cache: Vec<_> = std::fs::read_dir(env.cache_dir())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        cache.sort();
        assert_eq!(
            cache,
            ["filters", "schema"],
            "ensure_dirs creates both cache dirs"
        );
    }

    #[test]
    fn ensure_dirs_creates_private_config_dir() {
        let env = isolated();
        ensure_dirs().unwrap();
        assert!(env.config_dir().is_dir() && schema_dir().is_dir() && filters_dir().is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(env.config_dir())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
    }

    #[test]
    fn corrupt_config_is_backed_up() {
        let env = isolated();
        ensure_dirs().unwrap();
        for bad in ["{not json", "[1]"] {
            std::fs::write(config_file(), bad).unwrap();
            assert_eq!(load_config(), Map::new());
            let backups: Vec<_> = std::fs::read_dir(env.config_dir())
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with("config.json.corrupt-"))
                .collect();
            assert_eq!(backups.len(), 1, "{bad}");
            assert_eq!(
                std::fs::read_to_string(env.config_dir().join(&backups[0])).unwrap(),
                bad
            );
            assert!(!config_file().exists());
            std::fs::remove_file(env.config_dir().join(&backups[0])).unwrap();
        }
    }

    #[test]
    fn domain_reload_set_and_require() {
        let _env = isolated();
        assert_eq!(
            (domain(), api_v2(), api_int(), login_url()),
            Default::default()
        );
        assert_eq!(
            require_domain().unwrap_err().to_string(),
            "no Freshservice domain; run `fsv auth login --domain acme.freshservice.com`"
        );
        save_config(
            json!({"domain": "acme.freshservice.com"})
                .as_object()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(domain(), "", "save_config does not reload");
        reload();
        assert_eq!(domain(), "acme.freshservice.com");
        assert_eq!(api_v2(), "https://acme.freshservice.com/api/v2");
        assert_eq!(api_int(), "https://acme.freshservice.com/api/_");
        assert_eq!(login_url(), "https://acme.freshservice.com/");
        assert_eq!(require_domain().unwrap(), "acme.freshservice.com");

        assert_eq!(
            set_domain("https://Globex.freshservice.com/a/x").unwrap(),
            "globex.freshservice.com"
        );
        assert_eq!(domain(), "globex.freshservice.com");
        assert_eq!(api_v2(), "https://globex.freshservice.com/api/v2");
        assert_eq!(load_config()["domain"], "globex.freshservice.com");
        assert!(set_domain("acme").is_err());
        assert_eq!(domain(), "globex.freshservice.com");

        for bad in [json!("not a domain"), json!(5)] {
            save_config(json!({"domain": bad}).as_object().unwrap()).unwrap();
            reload();
            assert_eq!(
                (domain(), api_v2()),
                (String::new(), String::new()),
                "{bad}"
            );
        }
    }

    #[test]
    fn configured_domain_at_reads_file() {
        let env = isolated();
        let file = env.root().join("c.json");
        assert_eq!(configured_domain_at(&file), "");
        std::fs::write(&file, r#"{"domain": "ACME.freshservice.com"}"#).unwrap();
        assert_eq!(configured_domain_at(&file), "acme.freshservice.com");
        std::fs::write(&file, r#"{"domain": ""}"#).unwrap();
        assert_eq!(configured_domain_at(&file), "");
    }

    #[test]
    fn cache_keys_and_paths() {
        let env = isolated();
        assert_eq!(cache_domain_key(None), "default");
        assert_eq!(
            cache_domain_key(Some("ACME.Freshservice.com")),
            "acme.freshservice.com"
        );
        assert_eq!(cache_domain_key(Some("bad host/x")), "bad host-x");
        assert_eq!(cache_domain_key(Some("  ")), "default");
        for (id, want) in [
            (json!("12"), "ws12"),
            (json!(" 12 "), "ws12"),
            (json!(12), "ws12"),
            (json!(0), "ws0"),
            (json!(true), "wsTrue"),
            (json!(false), "wsFalse"),
            (json!(1.5), "wsauto"),
            (json!(""), "wsauto"),
            (json!("  "), "wsauto"),
            (json!(null), "wsauto"),
        ] {
            let mut cfg = json!({"forms": {"workspace_id": id}});
            save_config(cfg.as_object_mut().unwrap()).unwrap();
            assert_eq!(cache_workspace_key(), want);
        }
        let env = env.with_domain("acme.freshservice.com");
        assert_eq!(cache_domain_key(None), "acme.freshservice.com");
        assert_eq!(
            rel(&schema_cache_path("changes", None), &env),
            "<D>/cache/schema/acme.freshservice.com--wsauto--changes.json"
        );
        assert_eq!(
            schema_cache_candidates("changes", Some("globex.freshservice.com"))
                .iter()
                .map(|p| rel(p, &env))
                .collect::<Vec<_>>(),
            ["<D>/cache/schema/globex.freshservice.com--wsauto--changes.json"]
        );
        assert_eq!(
            rel(&filters_cache_path("tickets", None), &env),
            "<D>/cache/filters/acme.freshservice.com--tickets.json"
        );
        assert_eq!(
            filters_cache_candidates("tickets", None)
                .iter()
                .map(|p| rel(p, &env))
                .collect::<Vec<_>>(),
            ["<D>/cache/filters/acme.freshservice.com--tickets.json"]
        );
        assert_eq!(
            rel(&groups_cache_path(None), &env),
            "<D>/cache/groups--acme.freshservice.com.json"
        );
        assert_eq!(
            groups_cache_candidates(Some("x.y"))
                .iter()
                .map(|p| rel(p, &env))
                .collect::<Vec<_>>(),
            ["<D>/cache/groups--x.y.json"]
        );
    }

    #[test]
    fn default_where_round_trip() {
        let _env = isolated();
        assert!(get_default_where("changes").is_empty());
        set_default_where("changes", &["status=open".into(), "priority>2".into()]).unwrap();
        assert_eq!(get_default_where("changes"), ["status=open", "priority>2"]);
        assert_eq!(
            Value::Object(load_config())["defaults"],
            json!({"changes": {"where": ["status=open", "priority>2"]}})
        );
        set_default_where("tickets", &["a=b".into()]).unwrap();
        clear_default_where("changes").unwrap();
        assert_eq!(
            load_config()["defaults"],
            json!({"tickets": {"where": ["a=b"]}})
        );
        clear_default_where("nope").unwrap();
        assert!(get_default_where("nope").is_empty());
        save_config(
            json!({"defaults": {"problems": {"where": "ab"}}})
                .as_object()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            get_default_where("problems"),
            ["a", "b"],
            "list() of a string"
        );
    }

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
        assert_eq!(
            normalize_domain("https://ACME.freshservice.com/a/dashboard", false).unwrap(),
            "acme.freshservice.com"
        );
    }

    #[test]
    fn display_path_home() {
        let home = Path::new("/h/me");
        assert_eq!(display_path_with_home(Path::new("/h/me/x"), home), "~/x");
        assert_eq!(display_path_with_home(Path::new("/tmp/x"), home), "/tmp/x");
    }
}
