//! Terminal output replacing `rich` (Console, markup, Table, rule) and
//! `typer.prompt`/`typer.confirm`.
//!
//! Markup becomes ANSI when the stream is a terminal and `NO_COLOR` is
//! unset (`FORCE_COLOR` forces color); otherwise tags are stripped. Plain
//! `print` output is not hard-wrapped (Rich soft-wraps at terminal width).

mod markup;
mod prompt;
mod table;
mod text;

use std::io::{IsTerminal, Write};

pub use markup::{Color, Span, Style, Text, escape, render as parse_markup, strip as strip_markup};
pub use prompt::{confirm, prompt, prompt_int};
pub use table::{BoxStyle, Column, Table};
pub use text::{Justify, Line, Overflow, cell_len, measure, to_ansi, wrap};

/// Output stream of a [`Console`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// A Rich-like console bound to stdout or stderr.
#[derive(Clone, Debug)]
pub struct Console {
    stream: Stream,
    base: Style,
}

static OUT: Console = Console {
    stream: Stream::Stdout,
    base: NULL_STYLE,
};
static ERR: Console = Console {
    stream: Stream::Stderr,
    base: Style {
        bold: None,
        dim: Some(true),
        italic: None,
        underline: None,
        blink: None,
        reverse: None,
        conceal: None,
        strike: None,
        fg: None,
        bg: None,
    },
};

const NULL_STYLE: Style = Style {
    bold: None,
    dim: None,
    italic: None,
    underline: None,
    blink: None,
    reverse: None,
    conceal: None,
    strike: None,
    fg: None,
    bg: None,
};

/// `render.console` (stdout).
pub fn out() -> &'static Console {
    &OUT
}

/// `render.err` (stderr, dim).
pub fn err() -> &'static Console {
    &ERR
}

thread_local! {
    static CAPTURE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Run `f` and return what it printed through [`Console`] and
/// [`crate::output`] on this thread (stdout and stderr, in order) instead of
/// writing it. Captured output is not a terminal: no color unless
/// `FORCE_COLOR`, width `$COLUMNS` or 80.
pub fn capture<R>(f: impl FnOnce() -> R) -> (R, String) {
    let outer = CAPTURE.with(|c| c.borrow_mut().replace(String::new()));
    let r = f();
    let text = CAPTURE
        .with(|c| std::mem::replace(&mut *c.borrow_mut(), outer))
        .unwrap_or_default();
    (r, text)
}

fn capturing() -> bool {
    CAPTURE.with(|c| c.borrow().is_some())
}

/// Append to the active [`capture`] buffer; `false` when not capturing.
fn captured(s: &str) -> bool {
    CAPTURE.with(|c| match c.borrow_mut().as_mut() {
        Some(buf) => {
            buf.push_str(s);
            true
        }
        None => false,
    })
}

/// Write `s` to stdout (or the active [`capture`]) and flush.
pub(crate) fn write_stdout(s: &str) -> std::io::Result<()> {
    if captured(s) {
        return Ok(());
    }
    let mut o = std::io::stdout().lock();
    o.write_all(s.as_bytes())?;
    o.flush()
}

fn env_set(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|v| !v.is_empty())
}

/// Terminal size `(columns, lines)` of the first std stream that is a terminal.
pub fn terminal_size() -> Option<(usize, usize)> {
    #[cfg(unix)]
    {
        for fd in [0, 1, 2] {
            // SAFETY: TIOCGWINSZ only writes into the provided winsize struct.
            let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
            let rc = unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) };
            if rc == 0 && ws.ws_col > 0 {
                return Some((ws.ws_col as usize, ws.ws_row as usize));
            }
        }
        None
    }
    #[cfg(not(unix))]
    {
        crossterm::terminal::size()
            .ok()
            .map(|(c, r)| (c as usize, r as usize))
    }
}

impl Console {
    pub fn stream(&self) -> Stream {
        self.stream
    }

    /// Whether the stream is a terminal.
    pub fn is_terminal(&self) -> bool {
        if capturing() {
            return false;
        }
        match self.stream {
            Stream::Stdout => std::io::stdout().is_terminal(),
            Stream::Stderr => std::io::stderr().is_terminal(),
        }
    }

    /// Whether styles are emitted.
    pub fn color(&self) -> bool {
        if env_set("NO_COLOR") {
            return false;
        }
        if env_set("FORCE_COLOR") {
            return true;
        }
        if std::env::var("TERM").is_ok_and(|t| t.eq_ignore_ascii_case("dumb")) {
            return false;
        }
        self.is_terminal()
    }

    /// Console width: `$COLUMNS`, else the terminal size, else 80.
    pub fn width(&self) -> usize {
        if let Some(c) = std::env::var("COLUMNS")
            .ok()
            .and_then(|c| c.parse::<usize>().ok())
        {
            return c;
        }
        if capturing() || std::env::var("TERM").is_ok_and(|t| t.eq_ignore_ascii_case("dumb")) {
            return 80;
        }
        terminal_size().map_or(80, |(c, _)| c)
    }

    fn write_str(&self, s: &str) {
        if captured(s) {
            return;
        }
        let _ = match self.stream {
            Stream::Stdout => write_stdout(s),
            Stream::Stderr => {
                let mut o = std::io::stderr().lock();
                o.write_all(s.as_bytes()).and_then(|_| o.flush())
            }
        };
    }

    /// Render markup to the string this console would print (no newline).
    pub fn render(&self, markup: &str) -> String {
        self.render_text(&markup::render(markup))
    }

    fn render_text(&self, t: &Text) -> String {
        let color = self.color();
        text::lines(t, self.base)
            .iter()
            .map(|l| text::to_ansi(l, color))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// `console.print(markup)`.
    pub fn print(&self, markup: &str) {
        self.write_str(&(self.render(markup) + "\n"));
    }

    /// `console.print(text, markup=False)`.
    pub fn print_plain(&self, text: &str) {
        self.write_str(&(self.render_text(&Text::plain(text)) + "\n"));
    }

    /// `console.print()` (empty line).
    pub fn blank(&self) {
        self.write_str("\n");
    }

    /// `console.print(table)`.
    pub fn print_table(&self, table: &Table) {
        let mut s = String::new();
        for line in table.render_lines(self.width(), self.color()) {
            s.push_str(&line);
            s.push('\n');
        }
        self.write_str(&s);
    }

    /// `console.rule(title)`.
    pub fn rule(&self, title: &str) {
        self.write_str(&(rule_line(title, self.width(), self.color(), self.base) + "\n"));
    }

    /// `console.status(...)`: a no-op guard (spinners are not reproduced).
    pub fn status(&self, _message: &str) -> StatusGuard {
        StatusGuard(())
    }
}

/// Returned by [`Console::status`]; dropping it ends the status.
#[must_use]
pub struct StatusGuard(());

/// Rich `Rule` (center aligned, `─`) rendered to `width` cells.
pub fn rule_line(title: &str, width: usize, color: bool, base: Style) -> String {
    let line_style = base.combine(&Style::parse("bright_green"));
    let ch = '─';
    let mk = |n: usize| -> Line { std::iter::repeat_n((ch, line_style), n).collect() };
    let title_text = markup::render(&title.replace('\n', " "));
    if title_text.plain.is_empty() {
        return text::to_ansi(&mk(width), color);
    }
    if width < 5 {
        return text::to_ansi(&mk(width), color);
    }
    let truncate_width = width - 4;
    let line_cells = |l: &Line| -> usize { l.iter().map(|(c, _)| text::char_width(*c)).sum() };
    let mut title_line: Line = text::lines(&title_text, base)
        .into_iter()
        .next()
        .unwrap_or_default();
    if line_cells(&title_line) > truncate_width {
        title_line = wrap(
            &title_text,
            base,
            truncate_width,
            Justify::Left,
            Overflow::Ellipsis,
            true,
        )
        .into_iter()
        .next()
        .unwrap_or_default();
    }
    let title_len = line_cells(&title_line);
    let side = width.saturating_sub(title_len) / 2;
    let left_len = side.saturating_sub(1);
    let right_len = width.saturating_sub(left_len + title_len).min(side + 1);
    let mut line = mk(left_len);
    line.push((' ', line_style));
    line.extend(title_line);
    line.push((' ', line_style));
    line.extend(mk(right_len));
    let mut out: Line = Vec::new();
    let mut cells = 0;
    for (c, s) in line {
        let w = text::char_width(c);
        if cells + w > width {
            break;
        }
        cells += w;
        out.push((c, s));
    }
    out.extend(std::iter::repeat_n((' ', base), width - cells));
    text::to_ansi(&out, color)
}

/// `PAGER` (default `less -R`) through the shell; `false` when it failed.
pub fn page_text(text: &str) -> bool {
    let pager = std::env::var("PAGER")
        .ok()
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| "less -R".into());
    #[cfg(windows)]
    let mut cmd = {
        let mut c = std::process::Command::new("cmd");
        c.arg("/C").arg(&pager);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = std::process::Command::new("/bin/sh");
        c.arg("-c").arg(&pager);
        c
    };
    let Ok(mut child) = cmd.stdin(std::process::Stdio::piped()).spawn() else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
    }
    child.wait().is_ok_and(|s| s.success())
}

/// Page when enabled, stdout is a terminal and the rows do not fit.
pub fn should_page(row_count: usize, enabled: bool) -> bool {
    if !enabled || !std::io::stdout().is_terminal() {
        return false;
    }
    let lines = std::env::var("LINES")
        .ok()
        .and_then(|l| l.parse::<usize>().ok())
        .filter(|l| *l > 0)
        .or_else(|| terminal_size().map(|(_, r)| r).filter(|r| *r > 0))
        .unwrap_or(24);
    row_count + 4 > lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_matches_rich_shape() {
        let r = rule_line("CHN-1  x", 20, false, Style::default());
        assert_eq!(r, "───── CHN-1  x ─────");
        assert_eq!(cell_len(&r), 20);
        assert_eq!(rule_line("", 5, false, Style::default()), "─────");
    }

    #[cfg(unix)]
    #[test]
    fn page_text_runs_pager_through_shell() {
        let env = crate::testenv::isolated();
        crate::testenv::set("PAGER", "true");
        assert!(page_text("x"));
        crate::testenv::set("PAGER", "false");
        assert!(!page_text("x"));
        let sink = env.root().join("paged");
        crate::testenv::set("PAGER", format!("cat > '{}'", sink.display()));
        assert!(page_text("row 1\nrow 2\n"));
        assert_eq!(std::fs::read_to_string(&sink).unwrap(), "row 1\nrow 2\n");
    }

    #[test]
    fn should_page_only_when_enabled_on_a_terminal() {
        let _env = crate::testenv::isolated();
        crate::testenv::set("LINES", "10");
        assert!(!should_page(1000, false));
        if !std::io::stdout().is_terminal() {
            assert!(!should_page(1000, true));
        }
    }

    #[test]
    fn blank_prints_an_empty_line() {
        let ((), text) = capture(|| {
            let c = out();
            c.print_plain("a");
            c.blank();
            c.print_plain("b");
        });
        assert_eq!(text, "a\n\nb\n");
    }
}
