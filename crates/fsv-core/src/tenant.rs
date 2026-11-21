//! Make the target tenant impossible to miss (`tenant.py`).
//!
//! fsv's config dir decides which Freshservice tenant a command talks to, and
//! the default dir points at whichever tenant `fsv auth login` configured
//! last. One hook, [`enter_command`], runs for every CLI command:
//!
//! - tenant commands print `fsv: tenant <domain> (config <dir>)` once on
//!   stderr (`--quiet` / `FSV_QUIET=1` suppress it; local commands print
//!   nothing);
//! - `--tenant DOMAIN` must equal the configured domain when given;
//! - `--yes` (a real write) needs the tenant confirmed: `--tenant DOMAIN`,
//!   `FSV_ALLOW_TENANT=DOMAIN` or `fsv config set tenant.confirmed DOMAIN`;
//! - `--expect-plan` reviewed on another tenant is refused by name.
//!
//! Plan hashes are bound to the tenant ([`bind_plan`]), and write-capable
//! commands add `tenant`/`config_dir` to their JSON objects ([`annotate`]).

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::{Map, Value};

use crate::config;
use crate::journal::sha256_hex;
use crate::pystr;

/// Command paths (prefixes) that never talk to a tenant: no banner, no checks.
pub const LOCAL_COMMANDS: [&str; 9] = [
    "help",
    "setup",
    "auth login",
    "auth logout",
    "completion",
    "config",
    "cache status",
    "cache clear",
    "journal",
];

/// `--yes` commands that never reach a tenant (local state only).
pub const LOCAL_YES_COMMANDS: [&str; 1] = ["auth logout"];

const TRUE_WORDS: [&str; 4] = ["1", "true", "yes", "on"];

#[derive(Default)]
struct State {
    depth: usize,
    write: bool,
    new: Vec<String>,
    raw_of: HashMap<String, String>,
    tenant_of: HashMap<String, String>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&mut State) -> T) -> T {
    let mut g = STATE.lock().unwrap_or_else(|p| p.into_inner());
    f(g.get_or_insert_with(State::default))
}

/// Tests: forget every bound hash and the per-command state (Python's
/// conftest resets `fsv.tenant` between tests).
#[doc(hidden)]
pub fn reset() {
    *STATE.lock().unwrap_or_else(|p| p.into_inner()) = None;
}

/// What [`enter_command`] needs to know about the command being run.
#[derive(Clone, Debug, Default)]
pub struct CommandInfo {
    /// Command path without the program name, e.g. `"tickets spam"`.
    pub path: String,
    /// The command defines `--yes`, `--dry-run` or `--expect-plan`.
    pub write_capable: bool,
    /// `--yes` was given.
    pub yes: bool,
    /// `--dry-run` was given.
    pub dry_run: bool,
    /// The `--expect-plan` value, when given.
    pub expect_plan: Option<String>,
    /// The root `--tenant DOMAIN`, when given.
    pub tenant: Option<String>,
    /// The root `--quiet`.
    pub quiet: bool,
}

/// A command refused because the tenant is wrong or unconfirmed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantRefusal(pub String);

fn norm(value: Option<&str>) -> String {
    let raw = pystr::strip(value.unwrap_or(""));
    config::normalize_domain(raw, false).unwrap_or_else(|_| raw.to_lowercase())
}

/// `config.display_path(config.CONFIG_DIR)`.
pub fn config_dir() -> String {
    config::display_path(&config::config_dir())
}

fn shown_domain() -> String {
    let d = config::domain();
    if d.is_empty() {
        "(none configured)".to_string()
    } else {
        d
    }
}

pub fn banner() -> String {
    format!("fsv: tenant {} (config {})", shown_domain(), config_dir())
}

pub fn is_local(path: &str) -> bool {
    LOCAL_COMMANDS
        .iter()
        .any(|p| path == *p || path.starts_with(&format!("{p} ")))
}

pub fn quiet(flag: bool) -> bool {
    flag || TRUE_WORDS.contains(
        &pystr::strip(&std::env::var("FSV_QUIET").unwrap_or_default())
            .to_lowercase()
            .as_str(),
    )
}

/// `tenant.confirmed` from this config dir (`""` when unset).
pub fn confirmed_tenant() -> String {
    let cfg = config::load_config();
    match cfg.get("tenant") {
        Some(Value::Object(m)) => norm(m.get("confirmed").and_then(Value::as_str)),
        _ => String::new(),
    }
}

/// How the configured tenant is confirmed for writes, or `None`.
pub fn confirmation(tenant_opt: Option<&str>) -> Option<&'static str> {
    let domain = config::domain();
    if domain.is_empty() {
        return None;
    }
    if tenant_opt.is_some_and(|t| !t.is_empty() && norm(Some(t)) == domain) {
        return Some("--tenant");
    }
    if norm(std::env::var("FSV_ALLOW_TENANT").ok().as_deref()) == domain {
        return Some("FSV_ALLOW_TENANT");
    }
    if confirmed_tenant() == domain {
        return Some("tenant.confirmed");
    }
    None
}

pub fn refusal(_tenant_opt: Option<&str>) -> String {
    let domain = shown_domain();
    let mut why = Vec::new();
    let env = std::env::var("FSV_ALLOW_TENANT").unwrap_or_default();
    if !env.is_empty() && norm(Some(&env)) != domain {
        why.push(format!(
            "FSV_ALLOW_TENANT={} does not match",
            norm(Some(&env))
        ));
    }
    let stale = confirmed_tenant();
    if !stale.is_empty() && stale != domain {
        why.push(format!("tenant.confirmed={stale} does not match"));
    }
    let reason = if why.is_empty() {
        String::new()
    } else {
        format!(" ({})", why.join("; "))
    };
    format!(
        "refusing to write: tenant {domain} (config {}) is not confirmed{reason}. Confirm it with --tenant {domain}, FSV_ALLOW_TENANT={domain}, or once with `fsv config set tenant.confirmed {domain}`. Wrong tenant? Set FSV_CONFIG_DIR to the intended tenant's config dir",
        config_dir()
    )
}

/// Refuse when this command must not run.
pub fn check(info: &CommandInfo) -> Result<(), TenantRefusal> {
    if is_local(&info.path) {
        return Ok(());
    }
    let domain = config::domain();
    if let Some(t) = info.tenant.as_deref().filter(|t| !t.is_empty())
        && norm(Some(t)) != domain
    {
        return Err(TenantRefusal(format!(
            "--tenant {} does not match the configured tenant {} (config {}); nothing was sent. Set FSV_CONFIG_DIR to the intended tenant's config dir",
            norm(Some(t)),
            shown_domain(),
            config_dir()
        )));
    }
    if let Some(expected) = info.expect_plan.as_deref().filter(|e| !e.is_empty())
        && let Some(other) = reviewed_elsewhere(expected)
    {
        let shown: String = pystr::strip(expected).chars().take(12).collect();
        return Err(TenantRefusal(format!(
            "plan {shown} was reviewed on tenant {other}, but this command targets {} (config {}); nothing was sent. Review again on this tenant, or set FSV_CONFIG_DIR to {other}'s config dir",
            shown_domain(),
            config_dir()
        )));
    }
    if info.yes
        && !info.dry_run
        && !LOCAL_YES_COMMANDS.contains(&info.path.as_str())
        && confirmation(info.tenant.as_deref()).is_none()
    {
        return Err(TenantRefusal(refusal(info.tenant.as_deref())));
    }
    Ok(())
}

/// The shared startup hook: banner, tenant assertion, write confirmation.
/// `Err` carries the refusal to print (exit 2). Nested calls (a command that
/// runs another command) only count the depth.
pub fn enter_command(info: &CommandInfo) -> Result<(), TenantRefusal> {
    let outer = with(|s| {
        s.depth += 1;
        s.depth == 1
    });
    if !outer {
        return Ok(());
    }
    let local = is_local(&info.path);
    with(|s| {
        s.write = info.write_capable && !local;
        s.new.clear();
    });
    if local {
        return Ok(());
    }
    if !quiet(info.quiet) {
        eprintln!("{}", banner());
    }
    check(info)
}

/// Pairs with [`enter_command`]; runs on every exit path.
pub fn leave_command() {
    let (write, new) = with(|s| {
        s.depth = s.depth.saturating_sub(1);
        if s.depth > 0 {
            return (false, Vec::new());
        }
        let out = (s.write, std::mem::take(&mut s.new));
        s.write = false;
        out
    });
    if write && !new.is_empty() {
        remember(&new);
    }
}

// --------------------------------------------------------------------------
// JSON annotation

/// Add `tenant`/`config_dir` to a write command's JSON object (`setdefault`).
pub fn annotate(obj: &Value) -> Value {
    if !with(|s| s.write) {
        return obj.clone();
    }
    let Value::Object(map) = obj else {
        return obj.clone();
    };
    let mut out: Map<String, Value> = map.clone();
    out.entry("tenant")
        .or_insert_with(|| Value::String(config::domain()));
    out.entry("config_dir")
        .or_insert_with(|| Value::String(config::config_dir().to_string_lossy().into_owned()));
    Value::Object(out)
}

// --------------------------------------------------------------------------
// tenant-bound plan hashes

/// Bind a tenant-free plan hash to the configured domain.
pub fn bind_plan(raw: &str) -> String {
    bind_plan_for(raw, &config::domain())
}

/// Bind a tenant-free plan hash to *domain*.
pub fn bind_plan_for(raw: &str, domain: &str) -> String {
    let bound = sha256_hex(format!("fsv-plan-tenant\0{domain}\0{raw}").as_bytes());
    with(|s| {
        s.raw_of.insert(bound.clone(), raw.to_string());
        s.tenant_of.insert(bound.clone(), domain.to_string());
        if s.write {
            s.new.push(bound.clone());
        }
    });
    bound
}

/// Replace bound plan hashes nested in *value* by their tenant-free form, so
/// composed hashes (a batch of plan hashes) bind the tenant exactly once and
/// stay comparable with journals written before the binding.
pub fn unbind(value: &Value) -> Value {
    match value {
        Value::String(s) => match with(|st| st.raw_of.get(s).cloned()) {
            Some(raw) => Value::String(raw),
            None => value.clone(),
        },
        Value::Object(m) => Value::Object(m.iter().map(|(k, v)| (k.clone(), unbind(v))).collect()),
        Value::Array(a) => Value::Array(a.iter().map(unbind).collect()),
        other => other.clone(),
    }
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A journal plan hash written before tenant binding -> its bound form.
pub fn bind_legacy(value: &Value) -> Value {
    match value {
        Value::String(s) if is_hex64(s) => Value::String(bind_plan(s)),
        other => other.clone(),
    }
}

fn registry() -> PathBuf {
    let base = config::test_state_home().unwrap_or_else(|| {
        std::env::var_os("XDG_STATE_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| config::home_dir().join(".local").join("state"))
    });
    base.join("fsv").join("plan-tenants.jsonl")
}

/// Record which tenant each reviewed plan hash belongs to (hashes and
/// domains only). Never fails.
fn remember(hashes: &[String]) {
    let _ = try_remember(hashes);
}

fn try_remember(hashes: &[String]) -> std::io::Result<()> {
    let p = registry();
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut seen: Vec<&String> = Vec::new();
    for h in hashes {
        if !seen.contains(&h) {
            seen.push(h);
        }
    }
    let lines: Vec<String> = seen
        .iter()
        .map(|h| {
            let tenant = with(|s| s.tenant_of.get(*h).cloned()).unwrap_or_default();
            let mut m = Map::new();
            m.insert("plan".into(), Value::String((*h).clone()));
            m.insert("tenant".into(), Value::String(tenant));
            fsv_py::Dumps::default().dump(&Value::Object(m))
        })
        .collect();
    if std::fs::metadata(&p).is_ok_and(|m| m.len() > 2_000_000) {
        let text = std::fs::read_to_string(&p)?;
        let all: Vec<&str> = text.lines().collect();
        let keep = &all[all.len().saturating_sub(5000)..];
        let _ = config::atomic_write_text(&p, &(keep.join("\n") + "\n"), config::PRIVATE_MODE);
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&p)?;
    f.write_all((lines.join("\n") + "\n").as_bytes())
}

/// The other tenant a reviewed plan hash (or 12+ character prefix) belongs
/// to, when it is known only for another tenant; else `None`.
pub fn reviewed_elsewhere(expected: &str) -> Option<String> {
    let e = pystr::strip(expected).to_lowercase();
    let e = e.strip_prefix("sha256:").unwrap_or(&e).to_string();
    if e.chars().count() < 12 {
        return None;
    }
    let mut seen: HashMap<String, String> = with(|s| {
        s.tenant_of
            .iter()
            .filter(|(h, _)| h.starts_with(&e))
            .map(|(h, d)| (h.clone(), d.clone()))
            .collect()
    });
    match std::fs::read_to_string(registry()) {
        Ok(text) => {
            for line in text.lines() {
                let Ok(Value::Object(row)) = serde_json::from_str::<Value>(line) else {
                    continue;
                };
                let h = row
                    .get("plan")
                    .filter(|v| crate::value::truthy(v))
                    .map(fsv_py::py_str)
                    .unwrap_or_default();
                if h.starts_with(&e) {
                    let d = row
                        .get("tenant")
                        .filter(|v| crate::value::truthy(v))
                        .map(fsv_py::py_str)
                        .unwrap_or_default();
                    seen.entry(h).or_insert(d);
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return None,
    }
    let domains: std::collections::BTreeSet<String> = seen.into_values().collect();
    if domains.is_empty() || domains.contains(&config::domain()) {
        return None;
    }
    domains.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testenv::{self, isolated};
    use serde_json::json;

    const ACME: &str = "acme.example.com";
    const GLOBEX: &str = "globex.example.com";

    fn raw_of(body: &Value) -> String {
        sha256_hex(fsv_py::canonical(body).as_bytes())
    }

    fn write_info(path: &str) -> CommandInfo {
        CommandInfo {
            path: path.into(),
            write_capable: true,
            ..CommandInfo::default()
        }
    }

    #[test]
    fn bind_formula_and_unbind() {
        let _env = isolated().with_domain(ACME);
        reset();
        let raw = sha256_hex(b"x");
        let bound = bind_plan(&raw);
        assert_eq!(
            bound,
            sha256_hex(format!("fsv-plan-tenant\0{ACME}\0{raw}").as_bytes())
        );
        assert_eq!(
            unbind(&json!([bound, {"k": bound}])),
            json!([raw, {"k": raw}])
        );
        assert_eq!(unbind(&json!("x")), json!("x"));
        assert_eq!(bind_legacy(&json!("short")), json!("short"));
        assert_eq!(bind_legacy(&json!(raw)), json!(bound));
    }

    #[test]
    fn locality_and_quiet() {
        assert!(is_local("config set") && is_local("journal tail") && is_local("completion show"));
        assert!(is_local("auth login") && !is_local("auth status") && !is_local("cache refresh"));
        assert!(is_local("help") && is_local("setup") && is_local("cache status"));
        assert!(!is_local("configs") && !is_local("tickets spam"));
    }

    /// `test_plan_hash_input_includes_the_tenant`.
    #[test]
    fn plan_hash_input_includes_the_tenant() {
        let env = isolated().with_domain(ACME);
        reset();
        let body = json!({"status": 4});
        let on_acme = crate::journal::plan_hash(Some(&body)).unwrap();
        let env = {
            drop(env);
            isolated().with_domain(GLOBEX)
        };
        assert_ne!(crate::journal::plan_hash(Some(&body)).unwrap(), on_acme);
        assert_eq!(on_acme, bind_plan_for(&raw_of(&body), ACME));
        drop(env);
    }

    /// `test_legacy_journal_hashes_still_guard_replays`: a composed hash
    /// written before binding is rebound when the journal is read.
    #[test]
    fn legacy_journal_hashes_still_guard_replays() {
        let _env = isolated().with_domain(ACME);
        reset();
        let child = json!({"id": 1});
        let legacy_batch = raw_of(&json!([raw_of(&child)]));
        let path = crate::journal::journal_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            format!(
                "{}\n",
                json!({"kind": "plan:x", "event": "started", "plan": legacy_batch})
            ),
        )
        .unwrap();
        let batch =
            crate::journal::plan_hash(Some(&json!([
                crate::journal::plan_hash(Some(&child)).unwrap()
            ])))
            .unwrap();
        let plans = |rows: Vec<Value>| -> Vec<String> {
            rows.iter()
                .map(|r| r["plan"].as_str().unwrap().to_string())
                .collect()
        };
        assert_eq!(plans(crate::journal::read(None)), std::slice::from_ref(&batch));
        crate::journal::record_plan("done", &batch, "x", crate::journal::PlanEvent::default());
        assert_eq!(
            plans(crate::journal::read(None)),
            [batch.clone(), batch.clone()]
        );
        let rows = crate::journal::read(None);
        assert_eq!(rows[1]["tenant_bound"], json!(true));
        assert!(rows[0].get("tenant_bound").is_none());
    }

    /// `test_confirmation_goes_stale_when_the_configured_tenant_changes`.
    #[test]
    fn confirmation_goes_stale_when_the_configured_tenant_changes() {
        let env = isolated().with_domain(ACME);
        testenv::unset("FSV_ALLOW_TENANT");
        let mut cfg = config::load_config();
        cfg.insert("tenant".into(), json!({"confirmed": ACME}));
        config::save_config(&cfg).unwrap();
        assert_eq!(confirmation(None), Some("tenant.confirmed"));
        assert_eq!(confirmation(Some(&ACME.to_uppercase())), Some("--tenant"));
        std::fs::write(
            env.config_dir().join("config.json"),
            json!({"domain": GLOBEX, "tenant": {"confirmed": ACME}}).to_string(),
        )
        .unwrap();
        config::reload();
        assert_eq!(confirmation(None), None);
        assert!(refusal(None).contains(&format!("tenant.confirmed={ACME} does not match")));
        testenv::set("FSV_ALLOW_TENANT", GLOBEX.to_uppercase());
        assert_eq!(confirmation(None), Some("FSV_ALLOW_TENANT"));
        testenv::set("FSV_ALLOW_TENANT", ACME);
        assert!(refusal(None).contains(&format!("FSV_ALLOW_TENANT={ACME} does not match")));
        testenv::unset("FSV_ALLOW_TENANT");
    }

    #[test]
    fn check_orders_mismatch_then_foreign_plan_then_confirmation() {
        let _env = isolated().with_domain(ACME);
        reset();
        testenv::unset("FSV_ALLOW_TENANT");
        let mut info = write_info("tickets spam");
        info.yes = true;
        let err = |i: &CommandInfo| check(i).unwrap_err().0;
        assert!(err(&info).starts_with(&format!("refusing to write: tenant {ACME}")));
        info.dry_run = true;
        assert!(check(&info).is_ok());
        info.dry_run = false;
        info.tenant = Some(ACME.to_uppercase());
        assert!(check(&info).is_ok());
        info.tenant = Some(GLOBEX.into());
        assert!(err(&info).starts_with(&format!(
            "--tenant {GLOBEX} does not match the configured tenant {ACME}"
        )));
        let mut local = info.clone();
        local.path = "auth logout".into();
        assert!(check(&local).is_ok());
        let mut auth = write_info("auth status");
        auth.yes = true;
        assert!(check(&auth).is_err());
    }

    /// `test_cross_tenant_refusal_survives_a_new_process` and the named
    /// refusal of a plan reviewed elsewhere.
    #[test]
    fn plan_reviewed_on_another_tenant_is_refused_by_name() {
        let env = isolated().with_domain(ACME);
        reset();
        testenv::set("FSV_QUIET", "1");
        assert!(enter_command(&write_info("tickets spam")).is_ok());
        let digest = bind_plan(&raw_of(&json!({"status": 4})));
        leave_command();
        reset();
        std::fs::write(
            env.config_dir().join("config.json"),
            json!({"domain": GLOBEX}).to_string(),
        )
        .unwrap();
        config::reload();
        assert_eq!(reviewed_elsewhere(&digest).as_deref(), Some(ACME));
        assert_eq!(reviewed_elsewhere(&digest[..12]).as_deref(), Some(ACME));
        assert_eq!(reviewed_elsewhere(&digest[..11]), None);
        let mut info = write_info("tickets spam");
        info.expect_plan = Some(digest.clone());
        info.tenant = Some(GLOBEX.into());
        info.yes = true;
        let msg = check(&info).unwrap_err().0;
        assert!(
            msg.contains(&format!(
                "plan {} was reviewed on tenant {ACME}",
                &digest[..12]
            )),
            "{msg}"
        );
        std::fs::write(
            env.config_dir().join("config.json"),
            json!({"domain": ACME}).to_string(),
        )
        .unwrap();
        config::reload();
        assert_eq!(reviewed_elsewhere(&digest), None);
        testenv::unset("FSV_QUIET");
    }

    /// `test_read_json_is_not_annotated` and the write-mode annotation.
    #[test]
    fn json_is_annotated_only_inside_write_commands() {
        let _env = isolated().with_domain(ACME);
        reset();
        testenv::set("FSV_QUIET", "1");
        assert_eq!(annotate(&json!({"id": 1})), json!({"id": 1}));
        assert!(enter_command(&write_info("tickets spam")).is_ok());
        let out = annotate(&json!({"id": 1, "tenant": "kept"}));
        assert_eq!(out["tenant"], json!("kept"));
        assert_eq!(
            out["config_dir"],
            json!(config::config_dir().to_string_lossy())
        );
        assert_eq!(annotate(&json!([1])), json!([1]));
        leave_command();
        assert_eq!(annotate(&json!({"id": 1})), json!({"id": 1}));
        let mut local = write_info("config show");
        local.write_capable = true;
        assert!(enter_command(&local).is_ok());
        assert_eq!(annotate(&json!({"id": 1})), json!({"id": 1}));
        leave_command();
        testenv::unset("FSV_QUIET");
    }
}
