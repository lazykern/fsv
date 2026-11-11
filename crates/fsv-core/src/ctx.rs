//! Explicit context handed to library code instead of process globals.

use std::path::Path;

pub use crate::config::Paths;

impl Paths {
    /// Every location under `root` (`root/config`, `root/cache`); for tests.
    pub fn under(root: &Path) -> Paths {
        let config_dir = root.join("config");
        let cache_dir = root.join("cache");
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

/// The Freshservice tenant and its base URLs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tenant {
    pub domain: String,
    pub api_v2: String,
    pub api_int: String,
    pub login_url: String,
}

impl Tenant {
    /// URLs for an already-normalized host (`acme.freshservice.com`).
    pub fn new(domain: &str) -> Tenant {
        Tenant {
            domain: domain.to_string(),
            api_v2: format!("https://{domain}/api/v2"),
            api_int: format!("https://{domain}/api/_"),
            login_url: format!("https://{domain}/"),
        }
    }
}

/// Global CLI flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    pub verbose: bool,
    pub no_input: bool,
}

/// Everything a command needs to know about its environment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ctx {
    pub paths: Paths,
    pub tenant: Option<Tenant>,
    pub flags: Flags,
    /// Click's `ctx.command_path`, recorded in journal entries.
    pub command_path: Option<String>,
}

impl Ctx {
    pub fn new(paths: Paths) -> Ctx {
        Ctx {
            paths,
            tenant: None,
            flags: Flags::default(),
            command_path: None,
        }
    }

    pub fn with_tenant(mut self, domain: &str) -> Ctx {
        self.tenant = Some(Tenant::new(domain));
        self
    }

    /// Snapshot of the process-wide config and runtime flags. Binary edge only.
    pub fn from_process() -> Ctx {
        let domain = crate::config::domain();
        Ctx {
            paths: crate::config::paths(),
            tenant: (!domain.is_empty()).then(|| Tenant::new(&domain)),
            flags: Flags {
                verbose: crate::runtime::verbose(),
                no_input: crate::runtime::no_input(),
            },
            command_path: crate::runtime::command_path(),
        }
    }

    /// The tenant, or the `config.require_domain()` error.
    pub fn require_tenant(&self) -> crate::Result<&Tenant> {
        self.tenant
            .as_ref()
            .filter(|t| !t.domain.is_empty())
            .ok_or_else(|| {
                crate::Error::other(
                    "no Freshservice domain; run `fsv auth login --domain acme.freshservice.com`",
                )
            })
    }
}
