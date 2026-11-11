//! Hermetic process environment for tests that touch process-wide config
//! (mirrors the Python `conftest.py`): config, cache and journal state live in
//! a temp dir, the macOS Keychain is disabled, and tests holding an
//! [`Isolated`] guard run one at a time.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

static LOCK: Mutex<()> = Mutex::new(());

const CLEARED: &[&str] = &[
    "VISUAL",
    "EDITOR",
    "FSV_BROWSER",
    "FSV_BROWSER_LOGIN_TIMEOUT",
    "PAGER",
    "LINES",
    "COLUMNS",
];

pub(crate) struct Isolated {
    pub dir: tempfile::TempDir,
    _lock: MutexGuard<'static, ()>,
}

impl Isolated {
    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    pub fn config_dir(&self) -> PathBuf {
        self.root().join("config")
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.root().join("cache")
    }

    /// Write `{"domain": domain}` to `config.json` and reload the process config.
    pub fn with_domain(self, domain: &str) -> Self {
        std::fs::create_dir_all(self.config_dir()).expect("config dir");
        std::fs::write(
            self.config_dir().join("config.json"),
            format!("{{\"domain\": \"{domain}\"}}"),
        )
        .expect("config");
        crate::config::reload();
        self
    }
}

pub(crate) fn set(name: &str, value: impl AsRef<std::ffi::OsStr>) {
    // SAFETY: every test that reads or writes the process environment holds LOCK.
    unsafe { std::env::set_var(name, value) }
}

pub(crate) fn unset(name: &str) {
    // SAFETY: as in `set`.
    unsafe { std::env::remove_var(name) }
}

/// Fresh temp config/cache/state dirs, no domain, empty session cache.
pub(crate) fn isolated() -> Isolated {
    let lock = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    set("FSV_CONFIG_DIR", dir.path().join("config"));
    set("FSV_CACHE_DIR", dir.path().join("cache"));
    set("XDG_STATE_HOME", dir.path().join("state"));
    set("FSV_NO_KEYCHAIN", "1");
    for name in CLEARED {
        unset(name);
    }
    crate::config::reload();
    crate::session::clear_cache();
    Isolated { dir, _lock: lock }
}
