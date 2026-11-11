//! Run the form as a standalone full-screen app (the CLI face).

use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind, KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::kit::Theme;
use crate::kit::draw::Severity;
pub use crate::screen::FormResult;
use crate::screen::FormScreen;
use crate::session::FormSession;

/// Can the full-screen form run here? (a terminal on stdin and stdout;
/// `FSV_NO_FORM=1` or `TERM=dumb` turn it off). Reads the environment:
/// call it at the binary edge.
pub fn form_available() -> bool {
    let no_form = std::env::var_os("FSV_NO_FORM").is_some_and(|v| !v.is_empty());
    let term = std::env::var("TERM").ok();
    let tty = io::stdin().is_terminal() && io::stdout().is_terminal();
    form_available_in(no_form, term.as_deref(), tty)
}

pub fn form_available_in(no_form: bool, term: Option<&str>, tty: bool) -> bool {
    !no_form && term != Some("dumb") && tty
}

fn restore() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), DisableBracketedPaste, LeaveAlternateScreen);
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(data: &[u8]) -> String {
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// OSC 52: the terminal's clipboard (where it allows it).
fn copy_to_clipboard(text: &str) {
    let mut out = io::stdout();
    let _ = write!(out, "\x1b]52;c;{}\x07", base64(text.as_bytes()));
    let _ = out.flush();
}

/// Open the form full-screen and return what the person did. Nothing is
/// sent here: a `Submit` result carries the reviewed plan for the
/// caller's send path.
pub fn run_form(session: FormSession, theme: Theme) -> io::Result<FormResult> {
    enable_raw_mode()?;
    if let Err(e) = execute!(io::stdout(), EnterAlternateScreen, EnableBracketedPaste) {
        restore();
        return Err(e);
    }
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        prev(info);
    }));
    let result = event_loop(session, theme);
    restore();
    let _ = std::panic::take_hook();
    result
}

fn event_loop(session: FormSession, theme: Theme) -> io::Result<FormResult> {
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut screen = FormScreen::new(session, theme);
    let start = Instant::now();
    loop {
        screen.tick(start.elapsed());
        if let Some(text) = screen.take_clipboard() {
            copy_to_clipboard(&text);
        }
        if screen.done().is_some() {
            break;
        }
        terminal.draw(|f| {
            let area = f.area();
            if let Some(p) = screen.render(area, f.buffer_mut()) {
                f.set_cursor_position(p);
            }
        })?;
        let now = start.elapsed();
        let mut wait = screen
            .next_deadline()
            .map_or(Duration::from_millis(250), |d| d.saturating_sub(now));
        if screen.busy() {
            wait = wait.min(Duration::from_millis(30));
        }
        if !event::poll(wait.max(Duration::from_millis(1)))? {
            continue;
        }
        screen.tick(start.elapsed());
        match event::read()? {
            Event::Key(k) if k.kind != KeyEventKind::Release => {
                let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
                match k.code {
                    KeyCode::Char('q') if ctrl => screen.quit(),
                    KeyCode::Char('c') if ctrl => screen.notify(
                        "Press ctrl+q to quit the app",
                        "Do you want to quit?",
                        Severity::Information,
                        Duration::from_secs(5),
                    ),
                    _ => screen.handle_key(k),
                }
            }
            Event::Paste(text) => screen.handle_paste(&text),
            _ => {}
        }
    }
    Ok(screen.into_result())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability() {
        assert!(form_available_in(false, Some("xterm-256color"), true));
        assert!(form_available_in(false, None, true));
        assert!(!form_available_in(true, Some("xterm"), true));
        assert!(!form_available_in(false, Some("dumb"), true));
        assert!(!form_available_in(false, Some("xterm"), false));
    }

    #[test]
    fn base64_matches() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64("ช".as_bytes()), "4LiK");
    }
}
