//! `fsv completion show|install|doctor` and the root completion flags.

use std::path::{Path, PathBuf};

use fsv_completion::Node;
use fsv_core::resource::REGISTRY;
use fsv_core::term::{Column, Table, out};
use fsv_core::{Error, Result, config};

use crate::app::{ROOT_FLAGS_ANYWHERE, command};
use crate::generated::COMPLETION_HINTS;
use crate::util::{CmdResult, fail, usage};

const SUPPORTED: &[&str] = &["bash", "zsh", "fish", "powershell", "pwsh"];

/// `completion._completion_network`.
pub fn network_enabled() -> bool {
    if std::env::var("FSV_COMPLETION_NETWORK").is_ok_and(|v| v == "1") {
        return true;
    }
    let cfg = crate::cmd::config::read_config();
    match cfg.get("completion").and_then(|c| c.get("network")) {
        Some(serde_json::Value::String(s)) => matches!(s.as_str(), "on" | "true" | "1"),
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::Number(n)) => n.as_i64() == Some(1),
        _ => false,
    }
}

fn env_shell() -> String {
    let shell = std::env::var("SHELL").unwrap_or_default();
    Path::new(&shell)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// `_completion_shell`: an explicit shell is used as given.
fn completion_shell(shell: Option<String>) -> Result<String> {
    if let Some(s) = shell.filter(|s| !s.is_empty()) {
        return Ok(s);
    }
    let s = env_shell();
    if SUPPORTED.contains(&s.as_str()) {
        return Ok(s);
    }
    Err(usage(
        "could not detect shell; pass one of: bash, zsh, fish, powershell, pwsh",
    ))
}

/// `_shell_from_env` (typer's fallback detection; `sh` counts as bash).
pub fn shell_from_env() -> Option<String> {
    let s = fsv_core::pystr::casefold(&env_shell());
    let s = if s == "sh" { "bash".to_string() } else { s };
    SUPPORTED.contains(&s.as_str()).then_some(s)
}

/// The completion tree for this binary.
pub fn tree(prog: &str) -> Node {
    fsv_completion::tree_from_clap(&command(), prog, COMPLETION_HINTS, ROOT_FLAGS_ANYWHERE)
}

fn exe() -> String {
    std::env::current_exe().map_or_else(|_| "fsv".to_string(), |p| p.display().to_string())
}

/// `completion_gen.build_script`.
pub fn build_script(shell: &str, prog: &str) -> Result<String> {
    fsv_completion::build_script(shell, &tree(prog), prog, &exe()).map_err(Error::other)
}

fn home() -> PathBuf {
    config::home_dir()
}

/// Write the script where `shell` loads it; returns the lines to print.
fn install_script(shell: &str, prog: &str) -> Result<Vec<String>> {
    let script = build_script(shell, prog)?;
    let mut lines = Vec::new();
    if shell == "fish" {
        let dir = home().join(".config").join("fish").join("completions");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{prog}.fish"));
        std::fs::write(&path, script)?;
        lines.push(format!("fish completion installed in {}", path.display()));
    } else {
        let script_path = home().join(format!(".{prog}-complete.{shell}"));
        std::fs::write(&script_path, script)?;
        let rc = home().join(if shell == "bash" { ".bashrc" } else { ".zshrc" });
        let existing = std::fs::read_to_string(&rc).unwrap_or_default();
        if !existing.contains(&script_path.display().to_string()) {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&rc)?;
            f.write_all(format!("\n. {}\n", script_path.display()).as_bytes())?;
        }
        lines.push(format!(
            "{shell} completion installed in {}",
            script_path.display()
        ));
        lines.push(format!("sourced from {}", rc.display()));
    }
    Ok(lines)
}

/// `fsv completion show [SHELL]`.
pub fn show(shell: Option<String>, prog: &str) -> CmdResult {
    let shell = completion_shell(shell)?;
    print!("{}", build_script(&shell, prog)?);
    Ok(())
}

/// `fsv completion install [SHELL]`.
pub fn install(shell: Option<String>, prog: &str) -> CmdResult {
    let shell = completion_shell(shell)?;
    for line in install_script(&shell, prog)? {
        out().print(&line);
    }
    out().print("restart shell: exec $SHELL");
    Ok(())
}

/// `fsv completion doctor`.
pub fn doctor(prog: &str) -> CmdResult {
    let mut t = Table::new().title("completion doctor");
    t.add_column(Column::new("check"));
    t.add_column(Column::new("value"));
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "?".into());
    t.add_row(["shell".to_string(), shell]);
    let path = which::which("fsv").map_or_else(|_| prog.to_string(), |p| p.display().to_string());
    t.add_row(["fsv path".to_string(), path]);
    t.add_row([
        "network".to_string(),
        if network_enabled() { "on" } else { "off" }.to_string(),
    ]);
    for res in REGISTRY {
        let ok = config::schema_cache_candidates(res.name, None)
            .iter()
            .any(|p| p.exists());
        t.add_row([
            format!("schema {}", res.name),
            if ok { "ok" } else { "missing" }.to_string(),
        ]);
    }
    out().print_table(&t);
    out().print("Use TAB after a prefix, e.g. `fsv changes ls --where requester=phu<TAB>`.");
    Ok(())
}

/// Root `--show-completion`.
pub fn show_eager(prog: &str) -> CmdResult {
    let Some(shell) = shell_from_env() else {
        return Err(fail("could not detect shell", 1));
    };
    print!("{}", build_script(&shell, prog)?);
    Ok(())
}

/// Root `--install-completion`.
pub fn install_eager(prog: &str) -> CmdResult {
    let Some(shell) = shell_from_env() else {
        return Err(fail("could not detect shell", 1));
    };
    for line in install_script(&shell, prog)? {
        out().print(&line);
    }
    Ok(())
}
