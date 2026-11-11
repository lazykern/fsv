//! Drive the debug `fsv` binary against canned HTTP routes (`FSV_TEST_ROUTES`).
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

pub const DOMAIN: &str = "acme.freshservice.com";

pub struct Run {
    pub code: i32,
    pub out: String,
    pub err: String,
    pub requests: Vec<Value>,
}

impl Run {
    /// `(method, path)` of every request after the first (the client's probe, if any).
    pub fn calls(&self) -> Vec<(String, String)> {
        self.requests
            .iter()
            .map(|r| {
                let url = r["url"].as_str().unwrap_or_default();
                let path = url.split_once(DOMAIN).map_or(url, |(_, p)| p).to_string();
                (r["method"].as_str().unwrap_or_default().to_string(), path)
            })
            .collect()
    }

    /// Bodies of non-GET requests.
    pub fn writes(&self) -> Vec<(String, String, Value)> {
        self.calls()
            .into_iter()
            .zip(&self.requests)
            .filter(|((m, _), _)| m != "GET" && m != "HEAD")
            .map(|((m, p), r)| (m, p, r["body"].clone()))
            .collect()
    }

    /// `METHOD path` of non-GET requests, without the query.
    pub fn write_paths(&self) -> Vec<String> {
        self.calls()
            .into_iter()
            .filter(|(m, _)| m != "GET" && m != "HEAD")
            .map(|(m, p)| format!("{m} {}", p.split('?').next().unwrap_or_default()))
            .collect()
    }

    pub fn json(&self) -> Value {
        serde_json::from_str(&self.out).unwrap_or_else(|e| panic!("{e}: {}", self.out))
    }
}

pub struct Env {
    pub dir: tempfile::TempDir,
    pub routes: Vec<Value>,
    pub stdin: Option<String>,
    pub cwd: Option<PathBuf>,
    pub session: bool,
    pub vars: Vec<(String, String)>,
}

/// Routes from `tests/fixtures/routes/NAME.json`.
pub fn fixture(name: &str) -> Vec<Value> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/routes")
        .join(format!("{name}.json"));
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
}

/// Run `args` against a route fixture in a fresh home (cwd = `HOME/work`).
pub fn run_fixture(name: &str, args: &[&str], stdin: Option<&str>) -> Run {
    run_steps(name, &[], args, stdin)
}

/// `run_fixture` after running each of `pre` in the same home with empty
/// stdin; only the last run is returned.
pub fn run_steps(name: &str, pre: &[&[&str]], args: &[&str], stdin: Option<&str>) -> Run {
    run_with(name, pre, args, stdin, Opts::default())
}

/// Extra process settings for [`run_with`].
#[derive(Default)]
pub struct Opts<'a> {
    /// `FSV_TEST_TTY=1`: stdin counts as a terminal (prompts read the pipe).
    pub tty: bool,
    pub env: &'a [(&'a str, &'a str)],
}

/// [`run_steps`] with [`Opts`].
pub fn run_with(
    name: &str,
    pre: &[&[&str]],
    args: &[&str],
    stdin: Option<&str>,
    opts: Opts<'_>,
) -> Run {
    let mut env = Env::new(fixture(name));
    let work = env.home().join("work");
    std::fs::create_dir_all(&work).unwrap();
    env.cwd = Some(work);
    if opts.tty {
        env.vars.push(("FSV_TEST_TTY".into(), "1".into()));
    }
    for (k, v) in opts.env {
        env.vars.push((k.to_string(), v.to_string()));
    }
    env.stdin = Some(String::new());
    for step in pre {
        env.run(step);
        env.session = false;
    }
    env.stdin = stdin.map(str::to_string);
    env.run(args)
}

/// A GET route answering JSON.
pub fn get(path: &str, body: Value) -> Value {
    json!({"method": "GET", "path": format!("/api/_/{path}"), "json": body})
}

/// A route with any method and full path.
pub fn route(method: &str, path: &str, status: u16, body: Value) -> Value {
    json!({"method": method, "path": path, "status": status, "json": body})
}

impl Env {
    pub fn new(routes: Vec<Value>) -> Env {
        Env {
            dir: tempfile::tempdir().unwrap(),
            routes,
            stdin: None,
            cwd: None,
            session: true,
            vars: Vec::new(),
        }
    }

    pub fn home(&self) -> &Path {
        self.dir.path()
    }

    pub fn config_dir(&self) -> PathBuf {
        self.home().join("config")
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.home().join("cache")
    }

    pub fn run(&self, args: &[&str]) -> Run {
        let h = self.home();
        let cfg = self.config_dir();
        std::fs::create_dir_all(&cfg).unwrap();
        let config_file = cfg.join("config.json");
        if !config_file.exists() {
            std::fs::write(&config_file, json!({"domain": DOMAIN}).to_string()).unwrap();
        }
        if self.session {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs_f64();
            let session = json!({
                "saved_at": now,
                "domain": DOMAIN,
                "cookies": {"_x_m": "m", "_x_d": "d", "_x_w": "w", "fw-session-id": "s"},
            });
            std::fs::write(cfg.join("session.json"), session.to_string()).unwrap();
        }
        let routes = h.join("routes.json");
        std::fs::write(&routes, Value::Array(self.routes.clone()).to_string()).unwrap();
        let log = h.join("requests.log");
        let _ = std::fs::remove_file(&log);
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_fsv"));
        cmd.args(args)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", h)
            .env("FSV_CONFIG_DIR", &cfg)
            .env("FSV_CACHE_DIR", self.cache_dir())
            .env("XDG_STATE_HOME", h.join("state"))
            .env("FSV_NO_KEYCHAIN", "1")
            .env("NO_COLOR", "1")
            .env("COLUMNS", "200")
            .env("TZ", "UTC")
            .env("FSV_TEST_ROUTES", &routes)
            .env("FSV_TEST_LOG", &log)
            .envs(self.vars.iter().map(|(k, v)| (k, v)))
            .stdin(if self.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(cwd) = &self.cwd {
            cmd.current_dir(cwd);
        }
        let mut child = cmd.spawn().unwrap();
        if let Some(input) = &self.stdin {
            use std::io::Write;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(input.as_bytes())
                .unwrap();
        }
        let output = child.wait_with_output().unwrap();
        let requests = std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        Run {
            code: output.status.code().unwrap_or(-1),
            out: String::from_utf8_lossy(&output.stdout).into_owned(),
            err: String::from_utf8_lossy(&output.stderr).into_owned(),
            requests,
        }
    }
}

/// A minimal change form schema: status, priority, agent, group, one custom dropdown.
pub fn change_fields() -> Value {
    json!([
        {"name": "subject", "label": "Subject", "field_type": "default_subject", "default_field": true, "required": true},
        {"name": "status", "label": "Status", "field_type": "default_status", "default_field": true,
         "choices": [{"id": 1, "value": "Open"}, {"id": 2, "value": "Planning"}, {"id": 5, "value": "Closed"}]},
        {"name": "priority", "label": "Priority", "field_type": "default_priority", "default_field": true,
         "choices": [{"id": 1, "value": "Low"}, {"id": 2, "value": "Medium"}]},
        {"name": "agent", "label": "Agent", "field_type": "default_agent", "default_field": true},
        {"name": "group", "label": "Group", "field_type": "default_group", "default_field": true},
        {"name": "cf_env", "label": "Environment", "field_type": "custom_dropdown", "default_field": false,
         "choices": [{"id": 11, "value": "Prod"}, {"id": 12, "value": "UAT"}]}
    ])
}
