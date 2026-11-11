//! The TUI's OS hooks: yank pipes the id to `pbcopy`
//! (`subprocess.run(["pbcopy"], input=..., check=True, capture_output=True)`)
//! and open follows `webbrowser.open`'s try order.

use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::Arc;

use serde_json::json;

/// Copies text; the error is the toast's `Copy failed: {error}`.
pub type Clipboard = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;
/// Opens a URL; `false` is `Browser open failed`.
pub type Browser = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// `str(exc)` of the `OSError` `subprocess` raises when `program` cannot start.
fn spawn_error(e: &std::io::Error, program: &str) -> String {
    let name = fsv_py::py_repr_str(program);
    match e.kind() {
        std::io::ErrorKind::NotFound => format!("[Errno 2] No such file or directory: {name}"),
        std::io::ErrorKind::PermissionDenied => format!("[Errno 13] Permission denied: {name}"),
        _ => e.to_string(),
    }
}

/// `subprocess.run(cmd, input=input, check=True, capture_output=True)`;
/// failures read like Python's `str(exc)`.
pub fn run_with_input(cmd: &[&str], input: &[u8]) -> Result<(), String> {
    let (program, args) = cmd.split_first().ok_or("empty command")?;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| spawn_error(&e, program))?;
    if let Some(mut stdin) = child.stdin.take()
        && let Err(e) = stdin.write_all(input)
        && e.kind() != std::io::ErrorKind::BrokenPipe
    {
        return Err(e.to_string());
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(());
    }
    let shown = fsv_py::py_repr(&json!(cmd));
    Err(match out.status.code() {
        Some(code) => format!("Command '{shown}' returned non-zero exit status {code}."),
        None => format!("Command '{shown}' died with {}.", signal_text(&out.status)),
    })
}

/// `repr(signal.Signals(n))`, else `unknown signal n`.
fn signal_text(status: &std::process::ExitStatus) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        let n = status.signal().unwrap_or(0);
        let name = match n {
            1 => "SIGHUP",
            2 => "SIGINT",
            3 => "SIGQUIT",
            6 => "SIGABRT",
            9 => "SIGKILL",
            13 => "SIGPIPE",
            15 => "SIGTERM",
            _ => return format!("unknown signal {n}"),
        };
        format!("<Signals.{name}: {n}>")
    }
    #[cfg(not(unix))]
    {
        let _ = status;
        "unknown signal 0".into()
    }
}

/// The yank command.
pub fn pbcopy(text: &str) -> Result<(), String> {
    run_with_input(&["pbcopy"], text.as_bytes())
}

/// One `webbrowser` controller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Launch {
    /// `GenericBrowser`: run and wait; success is exit status 0.
    Wait(Vec<String>),
    /// `BackgroundBrowser`: success is starting the process.
    Background(Vec<String>),
    /// `MacOSXOSAScript`: this script on `osascript`'s stdin; success is exit 0.
    Osascript(String),
}

/// `MacOSXOSAScript.open`'s script (`name` is `default` or an application).
pub fn osascript_script(name: &str, url: &str) -> String {
    let url = url.replace('"', "%22");
    if name == "default" {
        format!("open location \"{url}\"")
    } else {
        format!(
            "\n   tell application \"{name}\"\n       activate\n       open location \"{url}\"\n   end\n   "
        )
    }
}

/// `webbrowser._tryorder` for `url`: each `$BROWSER` entry (a whole
/// command name, `GenericBrowser(cmdline)`), then the platform defaults —
/// macOS's AppleScript controllers, or `xdg-open`/`gio open` under a
/// display elsewhere.
pub fn browser_plan(
    url: &str,
    browser_env: Option<&str>,
    macos: bool,
    display: bool,
) -> Vec<Launch> {
    let mut out: Vec<Launch> = browser_env
        .into_iter()
        .flat_map(|v| v.split(':'))
        .filter(|c| !c.is_empty())
        .map(|c| Launch::Wait(vec![c.to_string(), url.to_string()]))
        .collect();
    if macos {
        for name in ["default", "google chrome", "firefox", "safari"] {
            out.push(Launch::Osascript(osascript_script(name, url)));
        }
    } else if display {
        out.push(Launch::Background(vec!["xdg-open".into(), url.into()]));
        out.push(Launch::Background(vec![
            "gio".into(),
            "open".into(),
            url.into(),
        ]));
    }
    out
}

fn launch(l: &Launch) -> bool {
    match l {
        Launch::Wait(cmd) => Command::new(&cmd[0])
            .args(&cmd[1..])
            .status()
            .is_ok_and(|s| s.success()),
        Launch::Background(cmd) => Command::new(&cmd[0])
            .args(&cmd[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok(),
        Launch::Osascript(script) => run_with_input(&["osascript"], script.as_bytes()).is_ok(),
    }
}

/// `webbrowser.open(url)`: the first controller that succeeds.
pub fn open_in_browser(url: &str) -> bool {
    let env = std::env::var("BROWSER").ok();
    let display =
        std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some();
    browser_plan(url, env.as_deref(), cfg!(target_os = "macos"), display)
        .iter()
        .any(launch)
}
