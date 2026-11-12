//! Unit-test fixtures: a real [`Client`] over the canned routes in
//! `tests/fixtures/routes` (the ones the binary tests use), run as the
//! command client, and the Python expected values in `tests/golden/units.json`
//! (`tests/golden/gen_units.py`).

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use serde_json::{Value, json};

use fsv_core::client::mock::MockTransport;
use fsv_core::{Client, Ctx, Paths};

pub const HOST: &str = "acme.freshservice.com";

pub static UNITS: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../tests/golden/units.json")).expect("units.json")
});

/// `f`'s result as `gen_units.py` `run()` records it: `{"ok": value}` or
/// `{"exit": code}` (plus a non-empty exit message), with everything printed
/// to stdout and stderr under `"text"`.
pub fn outcome<T: Into<Value>>(f: impl FnOnce() -> fsv_core::Result<T>) -> Value {
    let (r, text) = fsv_core::term::capture(f);
    let mut v = match r {
        Ok(v) => json!({"ok": v.into()}),
        Err(fsv_core::Error::Exit { code, message }) => {
            let mut v = json!({"exit": code});
            if !message.is_empty() {
                v["message"] = json!(message);
            }
            v
        }
        Err(e) => json!({"error": e.to_string()}),
    };
    v["text"] = json!(text);
    v
}

/// [`outcome`] of a command (`None` on success).
pub fn cmd_outcome(f: impl FnOnce() -> crate::util::CmdResult) -> Value {
    outcome(|| f().map(|()| Value::Null))
}

/// A Python `run()` record in [`outcome`]'s shape.
pub fn python(want: &Value) -> Value {
    let mut v = want.clone();
    let m = v.as_object_mut().expect("run() record");
    let text = [m.remove("out"), m.remove("err")]
        .into_iter()
        .flatten()
        .map(|t| t.as_str().unwrap_or_default().to_string())
        .collect::<String>();
    if m.get("message").is_some_and(|x| x == "") {
        m.remove("message");
    }
    m.insert("text".into(), json!(text));
    v
}

pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/routes/{name}.json"))
}

pub struct Tenant {
    pub client: Arc<Client>,
    pub mock: Arc<MockTransport>,
    _dir: tempfile::TempDir,
}

impl Tenant {
    /// `METHOD /path` of each API, search and `/cmdb` request, in order.
    pub fn requests(&self) -> Vec<String> {
        self.mock
            .requests()
            .iter()
            .filter_map(|r| {
                let path = url::Url::parse(&r.url).ok()?.path().to_string();
                ["/api/", "/search/", "/cmdb/"]
                    .iter()
                    .any(|p| path.starts_with(p))
                    .then(|| format!("{} {path}", r.method))
            })
            .collect()
    }
}

/// A tenant answering from `tests/fixtures/routes/<name>.json`.
pub fn tenant(name: &str) -> Tenant {
    let dir = tempfile::tempdir().expect("tempdir");
    let mock = crate::mock::routed(&fixture(name), None).expect("routes");
    let ctx = Arc::new(Ctx::new(Paths::under(dir.path())).with_tenant(HOST));
    let cookies = [("_x_w", "w"), ("fw-session-id", "fw1")]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let client = Client::builder(ctx)
        .transport(mock.clone())
        .cookies(cookies)
        .sleeper(|_| {})
        .build()
        .expect("client");
    Tenant {
        client: Arc::new(client),
        mock,
        _dir: dir,
    }
}

/// Run `f` with `client` as the command client on this thread.
pub fn with_client<R>(client: &Arc<Client>, f: impl FnOnce() -> R) -> R {
    let prev = crate::util::TEST_CLIENT.with(|t| t.replace(Some(client.clone())));
    let r = f();
    crate::util::TEST_CLIENT.with(|t| *t.borrow_mut() = prev);
    r
}

static LOCK: Mutex<()> = Mutex::new(());

/// Process-wide config, cache and state in a temp dir; tests holding one run
/// one at a time.
pub struct Isolated {
    pub dir: tempfile::TempDir,
    _lock: MutexGuard<'static, ()>,
}

fn set_env(name: &str, value: impl AsRef<std::ffi::OsStr>) {
    // SAFETY: tests that change the process environment hold LOCK.
    unsafe { std::env::set_var(name, value) }
}

fn remove_env(name: &str) {
    // SAFETY: as in `set_env`.
    unsafe { std::env::remove_var(name) }
}

pub fn isolated() -> Isolated {
    let lock = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    set_env("FSV_CONFIG_DIR", dir.path().join("config"));
    set_env("FSV_CACHE_DIR", dir.path().join("cache"));
    set_env("XDG_STATE_HOME", dir.path().join("state"));
    set_env("FSV_NO_KEYCHAIN", "1");
    fsv_core::config::reload();
    Isolated { dir, _lock: lock }
}

impl Isolated {
    /// Write `config.json` (with the test domain) and reload it.
    pub fn config(&self, extra: Value) {
        let mut cfg = serde_json::json!({"domain": HOST});
        if let (Some(m), Value::Object(e)) = (cfg.as_object_mut(), extra) {
            m.extend(e);
        }
        let d = self.dir.path().join("config");
        std::fs::create_dir_all(&d).expect("config dir");
        std::fs::write(d.join("config.json"), cfg.to_string()).expect("config");
        fsv_core::config::reload();
    }

    pub fn set_env(&self, name: &str, value: Option<&str>) {
        match value {
            Some(v) => set_env(name, v),
            None => remove_env(name),
        }
    }
}
