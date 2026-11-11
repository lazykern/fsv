//! Cookie paste login: hidden multi-line input and header extraction.

use std::io::{IsTerminal, Read, Write};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use super::{Cookies, validate};
use crate::config;
use crate::error::{Error, Result};
use crate::pystr::strip;

/// Parse `a=1; b=2` (parts without `=` or with an empty name/value are skipped).
pub fn parse_cookie_header(s: &str) -> Cookies {
    let mut out = Cookies::new();
    for part in s.split(';') {
        if !part.contains('=') {
            continue;
        }
        let part = strip(part);
        let (k, v) = part.split_once('=').unwrap_or((part, ""));
        if !k.is_empty() && !v.is_empty() {
            out.insert(strip(k).to_string(), strip(v).to_string());
        }
    }
    out
}

fn splitlines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut it = text.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        if matches!(
            c,
            '\n' | '\r'
                | '\u{0b}'
                | '\u{0c}'
                | '\u{1c}'
                | '\u{1d}'
                | '\u{1e}'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        ) {
            out.push(&text[start..i]);
            let mut end = i + c.len_utf8();
            if c == '\r'
                && let Some(&(j, '\n')) = it.peek()
            {
                it.next();
                end = j + 1;
            }
            start = end;
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// Extract the cookie value from any paste format:
///
/// 1. bare cookie string: `_x_m=a; _x_d=b`
/// 2. HTTP header line: `Cookie: _x_m=a; _x_d=b`
/// 3. Chrome DevTools alternating format (`cookie` line, value on the next line)
/// 4. cURL: `-H 'cookie: _x_m=a'` / `-H "cookie: _x_m=a"`
/// 5. Firefox JSON headers: `[{"name":"cookie","value":"_x_m=a"}]`
/// 6. fetch() headers object: `{"cookie": "_x_m=a"}` (optionally under `headers`)
pub fn extract_cookie_from_input(raw: &str) -> String {
    static CURL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)-H\s+['"]cookie:\s*([^'"]+)['"]"#).expect("regex"));
    let lines = splitlines(raw);
    if lines.len() > 1 {
        for (i, line) in lines[..lines.len() - 1].iter().enumerate() {
            if strip(line).to_lowercase() == "cookie" {
                return strip(lines[i + 1]).to_string();
            }
        }
    }
    for line in &lines {
        if line.to_lowercase().starts_with("cookie:") {
            return strip(line.split_once(':').map_or("", |(_, v)| v)).to_string();
        }
    }
    if let Some(c) = CURL.captures(raw) {
        return strip(c.get(1).map_or("", |m| m.as_str())).to_string();
    }
    match serde_json::from_str::<Value>(raw) {
        Ok(Value::Array(items)) => {
            for item in &items {
                let name = item.get("name").and_then(Value::as_str).unwrap_or("");
                if item.is_object()
                    && name.to_lowercase() == "cookie"
                    && let Some(v) = item.get("value")
                {
                    return strip(&fsv_py::py_str(v)).to_string();
                }
            }
        }
        Ok(data @ Value::Object(_)) => {
            let headers = data.get("headers").unwrap_or(&data);
            if let Value::Object(h) = headers {
                for (k, v) in h {
                    if k.to_lowercase() == "cookie" {
                        return strip(&fsv_py::py_str(v)).to_string();
                    }
                }
            }
        }
        _ => {}
    }
    strip(raw).to_string()
}

/// Read hidden input from the terminal.
///
/// Handles single-line (cookie value) and multi-line (full headers block)
/// pastes: after each newline, wait up to 80 ms for more data; if more
/// arrives it is a paste and reading continues, otherwise the user hit
/// Enter. Remaining terminal input is flushed so nothing leaks to the shell.
/// Without a terminal, all of stdin is read.
///
/// Port of fsv.session._read_secret.
pub fn read_secret(prompt: &str) -> Result<String> {
    eprint!("{prompt}");
    let _ = std::io::stderr().flush();
    if !std::io::stdin().is_terminal() {
        return read_piped(&mut std::io::stdin());
    }
    let result = read_secret_tty();
    eprintln!();
    result
}

/// Piped stdin as Python's text-mode `sys.stdin.read().rstrip("\n")`:
/// universal newlines turn `\r\n` and `\r` into `\n` first.
fn read_piped(r: &mut dyn Read) -> Result<String> {
    let mut s = String::new();
    r.read_to_string(&mut s)?;
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
    Ok(s.trim_end_matches('\n').to_string())
}

/// The key loop of the terminal reader. `next` yields input bytes (`None`
/// at end of input); `more_pending` reports whether more input arrives
/// within 80 ms of a newline, which marks a multi-line paste.
fn collect_secret(
    next: &mut dyn FnMut() -> Option<u8>,
    more_pending: &mut dyn FnMut() -> bool,
) -> Result<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut chars: Vec<char> = Vec::new();
    while let Some(b) = next() {
        let ch = if b < 0x80 { b as char } else { '\u{FFFD}' };
        match ch {
            '\x03' => return Err(Error::Aborted),
            '\x04' => break,
            '\x7f' | '\x08' => {
                chars.pop();
            }
            '\r' | '\n' => {
                lines.push(chars.drain(..).collect());
                if !more_pending() {
                    break;
                }
            }
            c => chars.push(c),
        }
    }
    if !chars.is_empty() {
        lines.push(chars.into_iter().collect());
    }
    Ok(lines.join("\n"))
}

#[cfg(unix)]
fn read_secret_tty() -> Result<String> {
    let fd = libc::STDIN_FILENO;
    // SAFETY: termios calls on the process's stdin with a zeroed struct filled by tcgetattr.
    let mut old: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(fd, &mut old) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let mut raw = old;
    // cbreak without ISIG so Ctrl-C arrives as a byte and the terminal is restored.
    raw.c_lflag &= !(libc::ECHO | libc::ICANON | libc::ISIG);
    raw.c_cc[libc::VMIN] = 1;
    raw.c_cc[libc::VTIME] = 0;
    unsafe { libc::tcsetattr(fd, libc::TCSAFLUSH, &raw) };
    let result = collect_secret(
        &mut || {
            let mut b = [0u8; 1];
            let n = unsafe { libc::read(fd, b.as_mut_ptr().cast(), 1) };
            (n > 0).then_some(b[0])
        },
        &mut || {
            let mut pfd = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            unsafe { libc::poll(&mut pfd, 1, 80) > 0 }
        },
    );
    unsafe {
        libc::tcflush(fd, libc::TCIFLUSH);
        libc::tcsetattr(fd, libc::TCSADRAIN, &old);
    }
    result
}

#[cfg(not(unix))]
fn read_secret_tty() -> Result<String> {
    use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, poll, read};
    use std::time::Duration;
    crossterm::terminal::enable_raw_mode()?;
    let mut lines: Vec<String> = Vec::new();
    let mut chars: Vec<char> = Vec::new();
    let mut outcome: Result<()> = Ok(());
    loop {
        let ev = match read() {
            Ok(ev) => ev,
            Err(e) => {
                outcome = Err(e.into());
                break;
            }
        };
        let Event::Key(key) = ev else {
            if let Event::Paste(text) = ev {
                chars.extend(text.chars());
            }
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                outcome = Err(Error::Aborted);
                break;
            }
            KeyCode::Char('d') | KeyCode::Char('z')
                if key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                break;
            }
            KeyCode::Backspace => {
                chars.pop();
            }
            KeyCode::Enter => {
                lines.push(chars.drain(..).collect());
                if !poll(Duration::from_millis(80)).unwrap_or(false) {
                    break;
                }
            }
            KeyCode::Char(c) => chars.push(c),
            KeyCode::Tab => chars.push('\t'),
            _ => {}
        }
    }
    if !chars.is_empty() {
        lines.push(chars.into_iter().collect());
    }
    while poll(Duration::from_millis(0)).unwrap_or(false) {
        let _ = read();
    }
    let _ = crossterm::terminal::disable_raw_mode();
    outcome.map(|()| lines.join("\n"))
}

/// Port of fsv.session.login_interactive: cookie paste login (instructions
/// on stderr, hidden input).
pub fn login_interactive() -> Result<Cookies> {
    login_interactive_with(&mut read_secret)
}

/// [`login_interactive`] with an injectable hidden-input reader.
pub fn login_interactive_with(
    read_secret: &mut dyn FnMut(&str) -> Result<String>,
) -> Result<Cookies> {
    eprintln!(
        "Open your Freshservice portal: https://{}/",
        config::domain()
    );
    eprintln!("DevTools → Network tab → click any /api/_/ request → Headers tab");
    eprintln!(
        "RIGHT-CLICK the Cookie header → Copy value  (don't drag-select, value is truncated)"
    );
    eprintln!("Also accepts: Cookie: header line, all request headers, cURL, or fetch snippet.");
    eprintln!("Input is hidden — paste and press Enter:");
    let raw = read_secret("> ")?;
    if raw.is_empty() {
        return Err(Error::session("empty input"));
    }
    let line = extract_cookie_from_input(&raw);
    if line.is_empty() {
        return Err(Error::session(
            "could not find Cookie header in pasted input",
        ));
    }
    let cookies = parse_cookie_header(&line);
    validate(&cookies)?;
    Ok(cookies)
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    fn expected() -> Value {
        serde_json::from_str(include_str!("../../tests/fixtures/session_expected.json"))
            .expect("session fixture")
    }

    /// Bytes written to a pty under Python's `_read_secret`
    /// (`gen_fsv_core_session.py`); a newline counts as a paste when more
    /// bytes are already queued.
    #[test]
    fn collect_secret_matches_python_tty() {
        for row in expected()["read_secret_tty"].as_array().expect("rows") {
            let bytes: Vec<u8> = row[0]
                .as_str()
                .expect("input")
                .chars()
                .map(|c| u8::try_from(u32::from(c)).expect("latin-1"))
                .collect();
            let pos = std::cell::Cell::new(0);
            let got = collect_secret(
                &mut || {
                    let b = bytes.get(pos.get()).copied();
                    pos.set(pos.get() + 1);
                    b
                },
                &mut || pos.get() < bytes.len(),
            )
            .expect("secret");
            assert_eq!(got, row[1].as_str().expect("output"), "{row}");
        }
    }

    /// Python raises `KeyboardInterrupt` on `\x03`.
    #[test]
    fn collect_secret_ctrl_c_aborts() {
        let bytes = b"ab\x03cd\n";
        let mut it = bytes.iter().copied();
        let got = collect_secret(&mut || it.next(), &mut || true);
        assert!(matches!(got, Err(Error::Aborted)));
    }

    #[test]
    fn read_piped_matches_python() {
        for row in expected()["read_secret_pipe"].as_array().expect("rows") {
            let mut input = row[0].as_str().expect("input").as_bytes();
            assert_eq!(
                read_piped(&mut input).expect("read"),
                row[1].as_str().expect("output"),
                "{row}"
            );
        }
    }
}
