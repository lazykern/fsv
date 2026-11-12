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
    let env = std::env::var("FSV_COMPLETION_NETWORK").ok();
    let cfg = serde_json::Value::Object(crate::cmd::config::read_config());
    fsv_completion::completion_network(env.as_deref(), &cfg)
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

/// Root `--show-completion`. Port of fsv.cli._patch_typer_shell_detection
/// (shell detection via `shell_from_env`).
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

#[cfg(test)]
mod tests {
    /// Expected values from Python `completion._completion_network` on
    /// `{"completion": {"network": <json>}}`, the config `network_enabled`
    /// passes on.
    #[test]
    fn network_truthiness_matches_python() {
        let cases = [
            ("1", true),
            ("1.0", true),
            ("1e0", true),
            ("100e-2", true),
            ("1.0000000000000001", true),
            ("true", true),
            (r#""true""#, true),
            (r#""1""#, true),
            (r#""on""#, true),
            (r#""yes""#, false),
            (r#""True""#, false),
            (r#""ON""#, false),
            (r#""1.0""#, false),
            (r#""0""#, false),
            (r#""off""#, false),
            (r#""""#, false),
            ("0", false),
            ("0.0", false),
            ("2", false),
            ("1.5", false),
            ("-1", false),
            ("null", false),
            ("false", false),
            ("[1]", false),
            (r#"{"a":1}"#, false),
        ];
        for (raw, want) in cases {
            let v: serde_json::Value = serde_json::from_str(raw).unwrap();
            let cfg = serde_json::json!({"completion": {"network": v}});
            assert_eq!(
                fsv_completion::completion_network(None, &cfg),
                want,
                "{raw}"
            );
        }
        let off = serde_json::json!({"completion": {"network": 0}});
        for (env, want) in [("1", true), ("true", false), ("0", false), ("", false)] {
            assert_eq!(
                fsv_completion::completion_network(Some(env), &off),
                want,
                "{env:?}"
            );
        }
    }
}
