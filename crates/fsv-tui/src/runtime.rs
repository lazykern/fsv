//! Terminal setup and the event loop (`launch`).

use std::io;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::app::App;
use crate::backend::{Backend, Pages};
use crate::view;

fn restore() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen);
}

/// `fsv tui [--read-only]`.
pub fn launch(read_only: bool, backend: Arc<dyn Backend>, pages: Arc<dyn Pages>) -> io::Result<()> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        prev(info);
    }));
    let result = Terminal::new(CrosstermBackend::new(io::stdout())).and_then(|mut terminal| {
        run(
            &mut terminal,
            App::new(backend, pages, read_only),
            poll_event,
        )
    });
    restore();
    let _ = std::panic::take_hook();
    result.map(|_| ())
}

fn poll_event() -> io::Result<Option<Event>> {
    if event::poll(Duration::from_millis(50))? {
        event::read().map(Some)
    } else {
        Ok(None)
    }
}

/// The loop behind [`launch`]: mount, then tick, draw and dispatch key
/// presses until the app quits. `next_event` yields `None` when idle; an
/// error ends the loop.
pub fn run<B>(
    terminal: &mut Terminal<B>,
    mut app: App,
    mut next_event: impl FnMut() -> io::Result<Option<Event>>,
) -> io::Result<App>
where
    B: ratatui::backend::Backend,
    B::Error: Send + Sync + 'static,
{
    app.mount();
    while !app.quit {
        app.tick();
        terminal
            .draw(|f| view::draw(&app, f))
            .map_err(io::Error::other)?;
        match next_event()? {
            Some(Event::Key(k)) if k.kind != KeyEventKind::Release => app.handle_key(&k),
            _ => {}
        }
    }
    Ok(app)
}
