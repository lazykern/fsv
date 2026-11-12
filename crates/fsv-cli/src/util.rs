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
/// Port of fsv.cli._flow_exit (the `Error::Exit` arm).
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

/// `sys.stdin.isatty()`. Debug builds treat `FSV_TEST_TTY=1` as a terminal
/// so tests can script prompts through a pipe, and `FSV_TEST_TTY=0` as none.
pub fn stdin_tty() -> bool {
    #[cfg(debug_assertions)]
    if let Some(v) = std::env::var_os("FSV_TEST_TTY").filter(|v| !v.is_empty()) {
        return v != "0";
    }
    std::io::stdin().is_terminal()
}

/// Port of fsv.cli._interactive.
pub fn interactive() -> bool {
    !no_input(false) && stdin_tty()
}

#[cfg(test)]
thread_local! {
    /// The client unit tests run commands with (`testkit::with_client`).
    pub(crate) static TEST_CLIENT: std::cell::RefCell<Option<Arc<Client>>> =
        const { std::cell::RefCell::new(None) };
}

/// `get_client()`: the process-wide client.
pub fn shared_client() -> Result<Arc<Client>> {
    #[cfg(test)]
    if let Some(c) = TEST_CLIENT.with(|t| t.borrow().clone()) {
        return Ok(c);
    }
    #[cfg(debug_assertions)]
    {
        static MOCK: std::sync::OnceLock<Option<Arc<Client>>> = std::sync::OnceLock::new();
        let mock = MOCK.get_or_init(|| {
            let t = crate::mock::transport()?;
            let ctx = Arc::new(fsv_core::Ctx::from_process());
            Client::builder(ctx).transport(t).build().ok().map(Arc::new)
        });
        if let Some(c) = mock {
            return Ok(c.clone());
        }
    }
    fsv_core::get_client()
}

/// A client probing `cookies` instead of the stored session.
pub fn client_with_cookies(cookies: &fsv_core::session::Cookies) -> Result<Client> {
    let ctx = Arc::new(fsv_core::Ctx::from_process());
    #[cfg(debug_assertions)]
    if let Some(t) = crate::mock::transport() {
        return Client::builder(ctx)
            .transport(t)
            .cookies(cookies.clone())
            .build();
    }
    Client::with_cookies(ctx, cookies.clone())
}

/// `_client()`.
pub fn client() -> Result<Arc<Client>> {
    #[cfg(test)]
    if let Some(c) = TEST_CLIENT.with(|t| t.borrow().clone()) {
        return Ok(c);
    }
    if fsv_core::session::session_age_hours().is_none() {
        err().print("[red]error[/red]: no session; run `fsv auth login`");
        return Err(Error::exit(3, ""));
    }
    match shared_client() {
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
    emit_dry_run_as(res, cid, body, action, risks, None);
}

/// `_emit_dry_run(..., display_id=...)`.
pub fn emit_dry_run_as(
    res: &Resource,
    cid: i64,
    body: Value,
    action: &str,
    risks: &[String],
    display_id: Option<String>,
) {
    emit_dry_run_full(res, cid, body, action, risks, display_id, &[]);
}

/// `_emit_dry_run(..., display_id=..., warnings=...)`.
pub fn emit_dry_run_full(
    res: &Resource,
    cid: i64,
    body: Value,
    action: &str,
    risks: &[String],
    display_id: Option<String>,
    warnings: &[String],
) {
    let mut out = Map::new();
    out.insert("dry_run".into(), json!(true));
    out.insert("action".into(), json!(action));
    out.insert(
        "id".into(),
        json!(display_id.unwrap_or_else(|| format_cid(cid, res))),
    );
    out.insert("body".into(), body);
    if !risks.is_empty() {
        out.insert("requires_confirmation".into(), json!(risks));
    }
    if !warnings.is_empty() {
        out.insert("warnings".into(), json!(warnings));
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

/// `_edit_body`: open `$EDITOR` on `data` as JSON and return the edited object.
pub fn edit_body(data: &Value, hint: &str, local_no_input: bool) -> Result<Map<String, Value>> {
    if no_input(local_no_input) {
        return Err(usage(
            "editor disabled by --no-input; use --dry-run/--dry or provide non-interactive flags",
        ));
    }
    match fsv_core::editor::edit_json(data, hint) {
        Ok(Value::Object(m)) => Ok(m),
        Ok(other) => Err(uncaught(Error::other(format!(
            "edited JSON must be an object, got {}",
            fsv_py::py_str(&other)
        )))),
        Err(e) => Err(fail_exc(e)),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{UNITS, isolated, python};
    use serde_json::json;

    /// `units.json` `flow_exit`: `_flow_exit(FlowExit(code, message))`.
    #[test]
    fn report_matches_python() {
        for case in UNITS["flow_exit"].as_array().unwrap() {
            let code = case["code"].as_i64().unwrap() as i32;
            let message = case["message"].as_str().unwrap();
            let (got, text) = fsv_core::term::capture(|| report(Err(Error::exit(code, message))));
            assert_eq!(
                json!({"exit": got, "text": text}),
                python(&case["want"]),
                "{case}"
            );
        }
    }

    /// `units.json` `interactive`: `_interactive()` under `NO_INPUT` and a
    /// stdin terminal or not.
    #[test]
    fn interactive_matches_python() {
        let env = isolated();
        for case in UNITS["interactive"].as_array().unwrap() {
            fsv_core::runtime::set_no_input(case["no_input"].as_bool().unwrap());
            let tty = case["tty"].as_bool().unwrap();
            env.set_env("FSV_TEST_TTY", Some(if tty { "1" } else { "0" }));
            assert_eq!(json!(interactive()), case["want"], "{case}");
        }
        fsv_core::runtime::set_no_input(false);
        env.set_env("FSV_TEST_TTY", None);
    }
}
