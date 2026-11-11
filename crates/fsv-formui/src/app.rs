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
use ratatui::backend::{Backend, CrosstermBackend};

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
    let result = Terminal::new(CrosstermBackend::new(io::stdout())).and_then(|mut terminal| {
        let screen = FormScreen::new(session, theme);
        let start = Instant::now();
        event_loop(
            &mut terminal,
            screen,
            || start.elapsed(),
            |wait| {
                if event::poll(wait)? {
                    event::read().map(Some)
                } else {
                    Ok(None)
                }
            },
            copy_to_clipboard,
        )
    });
    restore();
    let _ = std::panic::take_hook();
    result
}

fn to_io<E: std::error::Error + Send + Sync + 'static>(e: E) -> io::Error {
    let b: Box<dyn std::error::Error + Send + Sync> = Box::new(e);
    match b.downcast::<io::Error>() {
        Ok(e) => *e,
        Err(b) => io::Error::other(b),
    }
}

/// The loop behind [`run_form`]: `next_event(wait)` is `None` when `wait`
/// passed without input.
fn event_loop<B: Backend>(
    terminal: &mut Terminal<B>,
    mut screen: FormScreen,
    elapsed: impl Fn() -> Duration,
    mut next_event: impl FnMut(Duration) -> io::Result<Option<Event>>,
    mut clipboard: impl FnMut(&str),
) -> io::Result<FormResult>
where
    B::Error: Send + Sync + 'static,
{
    loop {
        screen.tick(elapsed());
        if let Some(text) = screen.take_clipboard() {
            clipboard(&text);
        }
        if screen.done().is_some() {
            break;
        }
        terminal
            .draw(|f| {
                let area = f.area();
                if let Some(p) = screen.render(area, f.buffer_mut()) {
                    f.set_cursor_position(p);
                }
            })
            .map_err(to_io)?;
        let now = elapsed();
        let mut wait = screen
            .next_deadline()
            .map_or(Duration::from_millis(250), |d| d.saturating_sub(now));
        if screen.busy() {
            wait = wait.min(Duration::from_millis(30));
        }
        let Some(ev) = next_event(wait.max(Duration::from_millis(1)))? else {
            continue;
        };
        screen.tick(elapsed());
        match ev {
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

    /// `form_available`: the environment and both streams being terminals.
    #[test]
    fn form_available_reads_env_and_ttys() {
        let no_form = std::env::var_os("FSV_NO_FORM").is_some_and(|v| !v.is_empty());
        let term = std::env::var("TERM").ok();
        let tty = io::stdin().is_terminal() && io::stdout().is_terminal();
        assert_eq!(
            form_available(),
            form_available_in(no_form, term.as_deref(), tty)
        );
    }

    /// The `run_form` loop on a test terminal: the result and the last frame.
    fn run(events: Vec<Event>) -> (io::Result<FormResult>, String) {
        let (_acme, session) = crate::acme::acme();
        let screen = FormScreen::with_workers(
            session,
            Theme::textual_dark(),
            crate::kit::WorkersMode::Inline,
        );
        let mut terminal =
            Terminal::new(ratatui::backend::TestBackend::new(120, 40)).expect("terminal");
        let mut events = std::collections::VecDeque::from(events);
        let r = event_loop(
            &mut terminal,
            screen,
            || Duration::ZERO,
            |_| {
                events
                    .pop_front()
                    .map(Some)
                    .ok_or_else(|| io::Error::other("no more events"))
            },
            |_| {},
        );
        let text = crate::kit::draw::buffer_text(terminal.backend().buffer());
        (r, text)
    }

    fn key(name: &str) -> Event {
        Event::Key(crate::kit::parse_key(name))
    }

    /// `run_form`: the first Esc closes the requester list, the second
    /// cancels the untouched form, without a plan.
    #[test]
    fn run_form_loop_cancels_an_untouched_form() {
        let (r, text) = run(vec![key("escape"), key("escape")]);
        let r = r.expect("result");
        assert_eq!(r.action, crate::screen::Action::Cancel);
        assert!(r.plan.is_none());
        assert!(text.contains("New Change"));
    }

    /// Ctrl+C only tells how to quit; Ctrl+Q quits; key releases and paste
    /// reach the screen as in Textual (releases ignored).
    #[test]
    fn run_form_loop_ctrl_c_hints_and_ctrl_q_quits() {
        let mut release = crate::kit::parse_key("escape");
        release.kind = KeyEventKind::Release;
        let (r, text) = run(vec![
            Event::Key(release),
            key("ctrl+c"),
            Event::Paste("Ma".into()),
            key("ctrl+q"),
        ]);
        assert_eq!(r.expect("result").action, crate::screen::Action::Cancel);
        assert!(text.contains("Press ctrl+q to quit the app"), "{text}");
        assert!(matches!(
            run(vec![]).0,
            Err(e) if e.to_string() == "no more events"
        ));
    }

    #[test]
    fn to_io_keeps_an_io_error() {
        let e = to_io(io::Error::new(io::ErrorKind::BrokenPipe, "gone"));
        assert_eq!(e.kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(to_io(std::fmt::Error).kind(), io::ErrorKind::Other);
    }

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
