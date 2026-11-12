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
        std::env::current_dir()
            .map(|d| d.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
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

/// Lowercase hex SHA-256 of `data` (`hashlib.sha256(data).hexdigest()`).
pub fn sha256_hex(data: &[u8]) -> String {
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
    let tenant = if domain.is_empty() {
        "unknown".to_string()
    } else {
        sha256_hex(domain.as_bytes())
    };
    base.join("fsv")
        .join("journals")
        .join(profile)
        .join(tenant)
        .join("journal.jsonl")
}

/// sha256 of the body as canonical JSON (sorted keys, no whitespace); `None` without a body.
pub fn plan_hash(body: Option<&Value>) -> Option<String> {
    body.map(|b| sha256_hex(fsv_py::canonical(b).as_bytes()))
}

/// URL path split into segments; unknown non-numeric segments become `[redacted]`.
pub fn split_path(url: &str) -> Vec<String> {
    static HOST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^https?://[^/]+").expect("regex"));
    let stripped = HOST.replace(url, "");
    let path = stripped
        .split('?')
        .next()
        .unwrap_or("")
        .split('#')
        .next()
        .unwrap_or("");
    path.split('/')
        .filter(|p| !p.is_empty())
        .map(|p| {
            if PATH_SEGMENTS.contains(&p) || is_id_full(p) {
                p.to_string()
            } else {
                "[redacted]".to_string()
            }
        })
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
    let resource = segs
        .iter()
        .find(|s| !is_id(s))
        .map(String::as_str)
        .unwrap_or("");
    let mut target = None;
    for (i, s) in segs.iter().enumerate() {
        if is_id(s) {
            target = Some(if i > 0 {
                format!("{}/{s}", segs[i - 1])
            } else {
                s.clone()
            });
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
    static CATALOG: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^catalog-item/\d+$").expect("regex"));
    match value {
        Some(Value::String(s)) if CATALOG.is_match(s) => Value::String(s.clone()),
        other => safe_id(other),
    }
}

fn created_id(data: Option<&Value>) -> Value {
    let Some(Value::Object(map)) = data else {
        return Value::Null;
    };
    let usable =
        |v: Option<&Value>| matches!(v, Some(x) if !x.is_null() && !x.is_object() && !x.is_array());
    if usable(map.get("id")) {
        return safe_id(map.get("id"));
    }
    for v in map.values() {
        if let Value::Object(inner) = v
            && usable(inner.get("id"))
        {
            return safe_id(inner.get("id"));
        }
    }
    Value::Null
}

fn timestamp() -> String {
    jiff::Zoned::now()
        .strftime("%Y-%m-%dT%H:%M:%S%z")
        .to_string()
}

fn epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Port of fsv.journal._command.
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
    let line = fsv_py::Dumps::default()
        .ensure_ascii(false)
        .dump(&Value::Object(entry.clone()))
        + "\n";
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
    entry.insert(
        "plan_hash".into(),
        plan_hash(body).map_or(Value::Null, Value::String),
    );
    entry.insert(
        "status".into(),
        outcome.status.map_or(Value::Null, |s| json!(s)),
    );
    entry.insert("ok".into(), json!(outcome.ok));
    let created = if outcome.ok && upper == "POST" {
        created_id(outcome.response)
    } else {
        Value::Null
    };
    entry.insert("created_id".into(), created);
    if outcome.error {
        entry.insert("error".into(), json!("details omitted"));
    }
    let _ = append(&entry);
}

/// All journal rows (the last `limit` when given and non-zero); bad lines are skipped.
pub fn read(limit: Option<usize>) -> Vec<Value> {
    let Ok(text) = std::fs::read_to_string(journal_path()) else {
        return Vec::new();
    };
    let rows: Vec<Value> = text
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
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
    let _ = try_record_plan(event, plan, action, extra);
}

/// [`record_plan`] that reports a failed append, so a caller can refuse to
/// send when its `started` mark was not persisted.
pub fn try_record_plan(event: &str, plan: &str, action: &str, extra: PlanEvent<'_>) -> Result<()> {
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
        let known = [
            "create",
            "update",
            "attachments",
            "upload",
            "place-request",
            "after",
        ];
        let rows: Vec<Value> = steps
            .iter()
            .map(|s| {
                let name = if known.contains(&s.step.as_str()) {
                    s.step.as_str()
                } else {
                    "other"
                };
                json!({"step": name, "ok": s.ok})
            })
            .collect();
        entry.insert("steps".into(), Value::Array(rows));
    }
    if extra.error {
        entry.insert("error".into(), json!("details omitted"));
    }
    append(&entry)
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
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    recent_plan_rows(&read(None), plan, action, now - hours * 3600.0)
}

/// [`recent_plan`] over already-read rows; `cutoff` is the oldest epoch kept.
pub fn recent_plan_rows(rows: &[Value], plan: &str, action: &str, cutoff: f64) -> Option<Value> {
    let kind = format!("plan:{action}");
    let mut outcome: Vec<(String, &Value)> = Vec::new();
    let mut started: Vec<&Value> = Vec::new();
    for row in rows {
        if row.get("plan").and_then(Value::as_str) != Some(plan)
            || row.get("kind").and_then(Value::as_str) != Some(&kind)
        {
            continue;
        }
        if epoch_of(row) < cutoff {
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
        if r.get("event").and_then(Value::as_str) == Some("done")
            && best.is_none_or(|b| epoch_of(r) > epoch_of(b))
        {
            best = Some(r);
        }
    }
    if let Some(b) = best {
        return Some(b.clone());
    }
    started
        .iter()
        .rev()
        .find(|s| !outcome.iter().any(|(_, r)| epoch_of(r) >= epoch_of(s)))
        .map(|s| (*s).clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_hash_is_order_independent_and_none_for_no_body() {
        assert_eq!(
            plan_hash(Some(&json!({"a": 1, "b": [2]}))),
            plan_hash(Some(&json!({"b": [2], "a": 1})))
        );
        assert_ne!(
            plan_hash(Some(&json!({"a": 1}))),
            plan_hash(Some(&json!({"a": 2})))
        );
        assert_eq!(plan_hash(None), None);
    }

    #[test]
    fn classify_paths() {
        let segs = split_path(
            "https://acme.freshservice.com/api/_/changes/1/planning-fields/private-value?token=S",
        );
        assert_eq!(segs.join("/"), "api/_/changes/1/planning-fields/[redacted]");
        let (kind, target, path) = classify("PUT", &segs);
        assert_eq!(kind, "update:changes");
        assert_eq!(target.as_deref(), Some("changes/1"));
        assert_eq!(path, "/api/_/changes/1/planning-fields/[redacted]");
        let (kind, _, _) = classify("POST", &split_path("https://x/api/_/changes/1/notes"));
        assert_eq!(kind, "note");
    }

    use crate::testenv;

    fn select(env: &testenv::Isolated, profile: &str, domain: &str) -> PathBuf {
        let dir = env.root().join(profile);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("config.json"),
            format!("{{\"domain\": \"{domain}\"}}"),
        )
        .unwrap();
        testenv::set("FSV_CONFIG_DIR", &dir);
        crate::config::reload();
        journal_path()
    }

    fn without_time(row: &Value) -> Value {
        let mut row = row.clone();
        let map = row.as_object_mut().unwrap();
        assert!(map.shift_remove("ts").is_some_and(|t| t.is_string()));
        map.shift_remove("epoch");
        row
    }

    /// `_command()` is click's `ctx.command_path`, `None` outside a command.
    #[test]
    fn command_is_the_running_command_path() {
        let _env = testenv::isolated();
        let saved = crate::runtime::command_path();
        crate::runtime::set_command_path(Some("fsv changes create"));
        assert_eq!(command(), json!("fsv changes create"));
        crate::runtime::set_command_path(None);
        assert_eq!(command(), Value::Null);
        crate::runtime::set_command_path(saved.as_deref());
    }

    #[test]
    fn journal_path_layout() {
        let env = testenv::isolated().with_domain("acme.freshservice.com");
        let profile = sha256_hex(&path_bytes(&resolve_path(&env.config_dir())));
        assert_eq!(
            journal_path(),
            env.root()
                .join("state/fsv/journals")
                .join(profile)
                .join("d0ba64d859ebafab6b3b77efc7d19e8e5784676cd027af2c9819e609dc3c5bd4")
                .join("journal.jsonl")
        );
        std::fs::write(env.config_dir().join("config.json"), "{}").unwrap();
        crate::config::reload();
        assert_eq!(
            journal_path().parent().unwrap().file_name().unwrap(),
            "unknown"
        );
        assert!(!journal_path().exists());
    }

    #[cfg(unix)]
    #[test]
    fn journal_default_state_root_and_resolved_profile() {
        let env = testenv::isolated();
        testenv::unset("XDG_STATE_HOME");
        let real = env.root().join("profile");
        std::fs::create_dir(&real).unwrap();
        let alias = env.root().join("alias");
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        let old_home = std::env::var_os("HOME");
        testenv::set("HOME", env.root());
        testenv::set("FSV_CONFIG_DIR", &real);
        crate::config::reload();
        let expected = journal_path();
        testenv::set("FSV_CONFIG_DIR", &alias);
        crate::config::reload();
        let via_alias = journal_path();
        match old_home {
            Some(h) => testenv::set("HOME", h),
            None => testenv::unset("HOME"),
        }
        assert!(
            expected.starts_with(env.root().join(".local/state/fsv/journals")),
            "{expected:?}"
        );
        assert_eq!(via_alias, expected);
    }

    #[test]
    fn safe_ids_and_targets() {
        for (v, want) in [
            (json!(5), json!(5)),
            (json!("12"), json!("12")),
            (json!("12\n"), Value::Null),
            (json!("abc"), Value::Null),
            (json!(1.5), Value::Null),
            (json!(true), Value::Null),
            (Value::Null, Value::Null),
            (json!(-3), json!(-3)),
            (json!("١٢"), json!("١٢")),
            (json!("-3"), Value::Null),
        ] {
            assert_eq!(safe_id(Some(&v)), want, "{v}");
            assert_eq!(safe_target(Some(&v)), want, "{v}");
        }
        assert_eq!(safe_id(None), Value::Null);
        assert_eq!(
            safe_target(Some(&json!("catalog-item/42"))),
            json!("catalog-item/42")
        );
        assert_eq!(safe_target(Some(&json!("catalog-item/x"))), Value::Null);
        assert_eq!(safe_target(Some(&json!("catalog-item/4\n"))), Value::Null);
    }

    #[test]
    fn created_id_shapes() {
        assert_eq!(created_id(Some(&json!({"id": 5}))), json!(5));
        assert_eq!(
            created_id(Some(&json!({"change": {"id": "7"}}))),
            json!("7")
        );
        assert_eq!(
            created_id(Some(&json!({"id": null, "x": {"id": 9}}))),
            json!(9)
        );
        assert_eq!(
            created_id(Some(&json!({"id": {"a": 1}, "y": {"id": 3}}))),
            json!(3)
        );
        for v in [json!([1]), json!({"id": "PRIVATE"}), json!({})] {
            assert_eq!(created_id(Some(&v)), Value::Null, "{v}");
        }
        assert_eq!(created_id(None), Value::Null);
    }

    #[test]
    fn record_plan_append_read_and_recent_plan() {
        let _env = testenv::isolated().with_domain("acme.freshservice.com");
        assert!(read(None).is_empty());
        record_plan("started", "h1", "create", PlanEvent::default());
        let steps = [
            PlanStep {
                step: "create".into(),
                ok: true,
            },
            PlanStep {
                step: "secret.txt".into(),
                ok: false,
            },
        ];
        record_plan(
            "done",
            "h1",
            "create",
            PlanEvent {
                target: Some(&json!("catalog-item/3")),
                created_id: Some(&json!(77)),
                steps: Some(&steps),
                error: true,
            },
        );
        let rows = read(None);
        assert_eq!(
            rows.iter().map(without_time).collect::<Vec<_>>(),
            [
                json!({"command": null, "kind": "plan:create", "event": "started", "plan": "h1", "target": null, "created_id": null}),
                json!({"command": null, "kind": "plan:create", "event": "done", "plan": "h1", "target": "catalog-item/3",
                       "created_id": 77, "steps": [{"step": "create", "ok": true}, {"step": "other", "ok": false}],
                       "error": "details omitted"}),
            ]
        );
        assert!(rows[0]["epoch"].is_i64());
        assert_eq!(read(Some(1))[0]["event"], "done");
        assert_eq!(read(Some(0)).len(), 2);
        assert_eq!(read(Some(10)).len(), 2);
        assert_eq!(recent_plan("h1", "create", 24.0).unwrap()["event"], "done");
        assert_eq!(recent_plan("h1", "update", 24.0), None);
        assert_eq!(recent_plan("h2", "create", 24.0), None);
        let text = std::fs::read_to_string(journal_path()).unwrap();
        assert!(!text.contains("secret.txt"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(journal_path())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn journal_isolates_profiles_tenants_and_legacy() {
        let env = testenv::isolated();
        let legacy = env.root().join("state/fsv/journal.jsonl");
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, "{\"kind\":\"plan:create\",\"plan\":\"same\",\"event\":\"done\",\"epoch\":9999999999}\n").unwrap();

        let acme_a = select(&env, "a", "acme.freshservice.com");
        assert!(read(None).is_empty());
        assert_eq!(recent_plan("same", "create", 24.0), None);
        record_plan("started", "same", "create", PlanEvent::default());
        record_plan(
            "done",
            "same",
            "create",
            PlanEvent {
                created_id: Some(&json!(1)),
                ..Default::default()
            },
        );
        record(
            "POST",
            "https://acme.freshservice.com/api/v2/tickets",
            Some(&json!({"private": "BODY"})),
            WriteOutcome {
                status: Some(422),
                ok: false,
                response: None,
                error: true,
            },
        );
        assert_eq!(
            recent_plan("same", "create", 24.0).unwrap()["event"],
            "done"
        );
        assert_eq!(read(None).last().unwrap()["ok"], false);

        let acme_b = select(&env, "b", "acme.freshservice.com");
        assert!(acme_b != acme_a && read(None).is_empty());
        record_plan("started", "same", "create", PlanEvent::default());
        record_plan(
            "failed",
            "same",
            "create",
            PlanEvent {
                error: true,
                ..Default::default()
            },
        );
        assert_eq!(recent_plan("same", "create", 24.0), None);

        let globex_a = select(&env, "a", "globex.freshservice.com");
        assert!(globex_a != acme_a && globex_a != acme_b && read(None).is_empty());
        record_plan("started", "same", "create", PlanEvent::default());
        assert_eq!(
            recent_plan("same", "create", 24.0).unwrap()["event"],
            "started"
        );
        assert_eq!(select(&env, "a", "acme.freshservice.com"), acme_a);
        assert_eq!(
            recent_plan("same", "create", 24.0).unwrap()["event"],
            "done"
        );
        assert_eq!(
            std::fs::read_to_string(&legacy)
                .unwrap()
                .matches("\"plan\":\"same\"")
                .count(),
            1
        );
        for p in [acme_a, acme_b, globex_a] {
            let text = std::fs::read_to_string(p).unwrap();
            assert!(!text.contains("COOKIE") && !text.contains("BODY"));
        }
    }

    #[test]
    fn record_write_entry() {
        let _env = testenv::isolated().with_domain("acme.freshservice.com");
        record(
            "POST",
            "https://acme.freshservice.com/api/_/changes",
            Some(&json!({"a": 1})),
            WriteOutcome {
                status: Some(201),
                ok: true,
                response: Some(&json!({"change": {"id": 4}})),
                error: false,
            },
        );
        record(
            "POST",
            "https://acme.freshservice.com/api/_/changes",
            Some(&json!({})),
            WriteOutcome {
                status: Some(200),
                ok: true,
                response: Some(&json!({"id": "PRIVATE"})),
                error: false,
            },
        );
        let rows = read(None);
        assert_eq!(
            without_time(&rows[0]),
            json!({"command": null, "kind": "create:changes", "target": null, "method": "POST", "path": "/api/_/changes",
                   "plan_hash": "015abd7f5cc57a2dd94b7590f04ad8084273905ee33ec5cebeae62276a97f862",
                   "status": 201, "ok": true, "created_id": 4})
        );
        assert_eq!(rows[1]["created_id"], Value::Null);
        assert!(
            !std::fs::read_to_string(journal_path())
                .unwrap()
                .contains("PRIVATE")
        );
    }

    #[test]
    fn append_failure_never_breaks_the_caller() {
        let env = testenv::isolated();
        let blocker = env.root().join("file");
        std::fs::write(&blocker, "x").unwrap();
        testenv::set("XDG_STATE_HOME", &blocker);
        assert!(append(&Map::new()).is_err());
        record_plan("done", "x", "create", PlanEvent::default());
        assert!(try_record_plan("started", "x", "create", PlanEvent::default()).is_err());
        assert!(read(None).is_empty());
    }

    #[test]
    fn recent_plan_rows_prefers_done_and_honours_cutoff() {
        let row = |epoch: f64, event: &str| json!({"epoch": epoch, "kind": "plan:create", "plan": "h", "event": event});
        let rows = vec![row(1.0, "started"), row(2.0, "done"), row(3.0, "started")];
        assert_eq!(
            recent_plan_rows(&rows, "h", "create", 0.0).unwrap()["event"],
            json!("done")
        );
        assert_eq!(
            recent_plan_rows(&rows, "h", "create", 2.5).unwrap()["epoch"],
            json!(3.0)
        );
        assert!(recent_plan_rows(&rows, "h", "update", 0.0).is_none());
    }
}
