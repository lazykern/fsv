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
    let result = run(read_only, backend, pages);
    restore();
    let _ = std::panic::take_hook();
    result
}

fn run(read_only: bool, backend: Arc<dyn Backend>, pages: Arc<dyn Pages>) -> io::Result<()> {
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut app = App::new(backend, pages, read_only);
    app.mount();
    while !app.quit {
        app.tick();
        terminal.draw(|f| view::draw(&app, f))?;
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(k) if k.kind != KeyEventKind::Release => app.handle_key(&k),
                _ => {}
            }
        }
    }
    Ok(())
}
