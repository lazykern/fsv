//! `fsv config list|get|set` and `fsv config defaults show|set|clear`.

use serde_json::{Map, Value};

use fsv_core::config;
use fsv_core::resource::{REGISTRY, lookup};
use fsv_core::term::out;

use crate::generated::{
    ConfigDefaultsClearArgs, ConfigDefaultsSetArgs, ConfigDefaultsShowArgs, ConfigGetArgs,
    ConfigSetArgs,
};
use crate::util::{CmdResult, py_list, usage};

/// `_CONFIG_KEYS`, in display order.
pub const CONFIG_KEYS: &[(&str, &str)] = &[
    (
        "completion.network",
        "enable remote requester/agent completion [on|off]",
    ),
    (
        "defaults.requester",
        "requester email for `changes create` when --requester is not given",
    ),
    (
        "changes.attach_flow",
        "`changes create` attachments: after-create (default) | before-create (unverified vs the web)",
    ),
    (
        "forms.workspace_id",
        "workspace whose business rules apply (default: the form metadata, else the account)",
    ),
    (
        "defaults.sr_requester",
        "requester email for `sr create` when --requester is not given",
    ),
    (
        "tui.read_only",
        "open the TUI without write actions [on|off]",
    ),
];

/// `completion._config_load`: never writes, `{}` when unreadable.
pub fn read_config() -> Map<String, Value> {
    std::fs::read_to_string(config::config_dir().join("config.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn value_of(cfg: &Map<String, Value>, key: &str) -> String {
    let (section, prop) = key.split_once('.').unwrap_or((key, ""));
    cfg.get(section)
        .filter(|v| fsv_py::truthy(v))
        .and_then(|s| s.get(prop))
        .map_or_else(|| "(unset)".to_string(), fsv_py::py_str)
}

fn unknown_key(key: &str) -> fsv_core::Error {
    let keys: Vec<String> = CONFIG_KEYS.iter().map(|(k, _)| k.to_string()).collect();
    usage(&format!(
        "unknown key: {key}; supported: {}",
        py_list(&keys)
    ))
}

/// `fsv config list`.
pub fn list() -> CmdResult {
    let cfg = read_config();
    for (key, desc) in CONFIG_KEYS {
        out().print(&format!(
            "{key} = {}  [dim]# {desc}[/]",
            value_of(&cfg, key)
        ));
    }
    Ok(())
}

/// `fsv config get KEY`.
pub fn get(a: ConfigGetArgs) -> CmdResult {
    if !CONFIG_KEYS.iter().any(|(k, _)| *k == a.key) {
        return Err(unknown_key(&a.key));
    }
    out().print(&value_of(&read_config(), &a.key));
    Ok(())
}

/// `fsv config set KEY VALUE`.
pub fn set(a: ConfigSetArgs) -> CmdResult {
    let (section, prop) = a.key.split_once('.').unwrap_or((a.key.as_str(), ""));
    if section.is_empty() || !CONFIG_KEYS.iter().any(|(k, _)| *k == a.key) {
        return Err(unknown_key(&a.key));
    }
    let mut cfg = config::load_config();
    crate::util::config_section(&mut cfg, section)
        .map_err(crate::util::uncaught)?
        .insert(prop.to_string(), Value::String(a.value.clone()));
    config::save_config(&cfg)?;
    out().print(&format!("{} = {}", a.key, a.value));
    Ok(())
}

/// `fsv config defaults show [RESOURCE]`.
pub fn defaults_show(a: ConfigDefaultsShowArgs) -> CmdResult {
    let names: Vec<String> = match a.resource.filter(|r| !r.is_empty()) {
        Some(r) => vec![r],
        None => REGISTRY.iter().map(|r| r.name.to_string()).collect(),
    };
    for name in names {
        let exprs = config::get_default_where(&name);
        if exprs.is_empty() {
            out().print(&format!("{name}: (none)"));
        }
        for expr in exprs {
            out().print(&format!("{name}: {expr}"));
        }
    }
    Ok(())
}

fn check_resource(name: &str) -> CmdResult {
    if lookup(name).is_none() {
        let names: Vec<&str> = REGISTRY.iter().map(|r| r.name).collect();
        return Err(usage(&format!(
            "unknown resource {}; choose: {}",
            fsv_py::py_repr_str(name),
            names.join(", ")
        )));
    }
    Ok(())
}

/// `fsv config defaults set RESOURCE --where ...`.
pub fn defaults_set(a: ConfigDefaultsSetArgs) -> CmdResult {
    check_resource(&a.resource)?;
    config::set_default_where(&a.resource, &a.r#where)?;
    out().print(&format!(
        "defaults.{}.where = {}",
        a.resource,
        py_list(&a.r#where)
    ));
    Ok(())
}

/// `fsv config defaults clear RESOURCE`.
pub fn defaults_clear(a: ConfigDefaultsClearArgs) -> CmdResult {
    check_resource(&a.resource)?;
    config::clear_default_where(&a.resource)?;
    out().print(&format!("defaults.{}.where cleared", a.resource));
    Ok(())
}
