//! Shared CLI helpers (`_err`, `_err_exc`, `_client`, `_cid`, ...).

use std::io::IsTerminal;
use std::sync::Arc;

use serde_json::{Map, Value, json};

use fsv_core::pystr::collapse_ws;
use fsv_core::resource::{self, Resource};
use fsv_core::term::{self, err, escape};
use fsv_core::{Client, Error, Result};

use crate::types::OutputFormat;

pub type CmdResult = Result<()>;

/// `_err(msg, code)`: `msg` is markup and is not escaped.
pub fn fail(msg: &str, code: i32) -> Error {
    err().print(&format!("[red]error[/red]: {msg}"));
    Error::exit(code, "")
}

/// `_err(msg)` with the default usage code.
pub fn usage(msg: &str) -> Error {
    fail(msg, 2)
}

/// Plain stderr message then exit (the `register()` modules' style).
pub fn plain_fail(msg: &str, code: i32) -> Error {
    eprintln!("{msg}");
    Error::exit(code, "")
}

/// `_err_exc`: print an error with its stable exit code.
pub fn fail_exc(e: Error) -> Error {
    match e {
        Error::Exit { .. } => e,
        Error::Aborted => {
            err().print("Aborted.");
            Error::exit(1, "")
        }
        other => {
            let mut msg = collapse_ws(&other.to_string());
            if msg.is_empty() {
                msg = "Error".into();
            }
            if fsv_core::runtime::verbose()
                && let Some(api) = other.as_api()
            {
                msg.push('\n');
                msg.push_str(&api.detail());
            }
            fail(&escape(&msg), other.exit_code())
        }
    }
}

/// An exception Python leaves uncaught (traceback, exit 1): same message, exit 1.
pub fn uncaught(e: Error) -> Error {
    match e {
        Error::Exit { .. } | Error::Aborted => e,
        other => fail(&escape(&collapse_ws(&other.to_string())), 1),
    }
}

/// Final exit code for a command result, printing what is still unprinted.
pub fn report(result: CmdResult) -> i32 {
    match result {
        Ok(()) => 0,
        Err(Error::Exit { code, message }) => {
            if !message.is_empty() {
                if code != 0 {
                    err().print(&format!("[red]error[/red]: {}", escape(&message)));
                } else {
                    err().print(&escape(&message));
                }
            }
            code
        }
        Err(e) => match fail_exc(e) {
            Error::Exit { code, .. } => code,
            _ => 1,
        },
    }
}

/// `_no_input(local)`.
pub fn no_input(local: bool) -> bool {
    fsv_core::runtime::no_input() || local
}

pub fn stdin_tty() -> bool {
    std::io::stdin().is_terminal()
}

/// `_interactive()`.
pub fn interactive() -> bool {
    !no_input(false) && stdin_tty()
}

/// `_client()`.
pub fn client() -> Result<Arc<Client>> {
    if fsv_core::session::session_age_hours().is_none() {
        err().print("[red]error[/red]: no session; run `fsv auth login`");
        return Err(Error::exit(3, ""));
    }
    match fsv_core::get_client() {
        Ok(c) => Ok(c),
        Err(e) => {
            let code = if matches!(e, Error::Session(_)) { 3 } else { 1 };
            err().print(&format!("[red]error[/red]: {e}"));
            Err(Error::exit(code, ""))
        }
    }
}

/// `_cid(id, res)`.
pub fn cid(id: &str, res: &Resource) -> Result<i64> {
    resource::parse_id(id, res).map_err(fail_exc)
}

pub fn format_cid(cid: i64, res: &Resource) -> String {
    resource::format_id(&json!({"id": cid}), res)
}

/// `_confirm_writes`.
pub fn confirm_writes(
    risks: &[String],
    subject: &str,
    yes: bool,
    local_no_input: bool,
) -> CmdResult {
    if risks.is_empty() || yes {
        return Ok(());
    }
    if no_input(local_no_input) || !stdin_tty() {
        return Err(usage(&format!(
            "{subject}: {}. Pass --yes to confirm.",
            risks.join("; ")
        )));
    }
    err().print(&format!("[bold]{subject}[/bold] will:"));
    for r in risks {
        err().print(&format!("  - {r}"));
    }
    term::confirm("Proceed?", Some(false), true)?;
    Ok(())
}

/// `_emit_dry_run`.
pub fn emit_dry_run(res: &Resource, cid: i64, body: Value, action: &str, risks: &[String]) {
    let mut out = Map::new();
    out.insert("dry_run".into(), json!(true));
    out.insert("action".into(), json!(action));
    out.insert("id".into(), json!(format_cid(cid, res)));
    out.insert("body".into(), body);
    if !risks.is_empty() {
        out.insert("requires_confirmation".into(), json!(risks));
    }
    fsv_core::output::emit_json(&Value::Object(out));
}

/// `_emit_fmt`: json/csv/tsv output; `false` means the caller draws a table.
pub fn emit_fmt(raw: &Value, flat: &[Map<String, Value>], fmt: OutputFormat) -> Result<bool> {
    match fmt {
        OutputFormat::Json => fsv_core::output::emit_json(raw),
        OutputFormat::Csv => fsv_core::output::emit_delimited(flat, ',')?,
        OutputFormat::Tsv => fsv_core::output::emit_delimited(flat, '\t')?,
        OutputFormat::Table => return Ok(false),
    }
    Ok(true)
}

/// `str(d.get(a) or d.get(b) or ... or default)`.
pub fn first_str(v: &Value, keys: &[&str], default: &str) -> String {
    keys.iter()
        .find_map(|k| v.get(*k).filter(|x| fsv_py::truthy(x)))
        .map_or_else(|| default.to_string(), fsv_py::py_str)
}

/// Python's `repr()` of a list of strings.
pub fn py_list(items: &[String]) -> String {
    fsv_py::py_repr(&json!(items))
}

/// Python `s[:n]` on characters.
pub fn head(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

pub fn obj(pairs: Vec<(&str, Value)>) -> Map<String, Value> {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}
