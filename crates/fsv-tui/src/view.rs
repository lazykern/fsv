//! Drawing (`compose` + `app.tcss`). Pure: reads [`App`] state, records
//! the detail scroll range and table viewport for the key handler.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, Content, Focus, Severity};
use crate::markup::{plain_text, to_line, to_text};
use crate::modals::{ERROR, PRIMARY, SUCCESS, SURFACE, WARNING, input_line};

const MUTED: Modifier = Modifier::DIM;

fn pad_to(s: &str, width: usize) -> String {
    let w = UnicodeWidthStr::width(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - w))
    }
}

fn wrapped_rows(text: &Text, width: u16) -> usize {
    let w = width.max(1) as usize;
    text.lines
        .iter()
        .map(|l| l.width().div_ceil(w).max(1))
        .sum()
}

pub fn draw(app: &App, f: &mut Frame) {
    let area = f.area();
    if let Some(form) = &app.form {
        f.render_widget(Clear, area);
        if let Some(p) = form.view.borrow_mut().render(area, f.buffer_mut()) {
            f.set_cursor_position(p);
        }
        draw_overlays(app, f, area);
        return;
    }
    let [header, main, status, hints] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    f.render_widget(
        Paragraph::new(to_line(&app.header_markup()))
            .block(Block::default().padding(Padding::horizontal(1))),
        header,
    );
    let [list, detail] = Layout::vertical([
        Constraint::Fill(app.list_fr),
        Constraint::Fill(app.detail_fr),
    ])
    .areas(main);
    let (list, detail) = enforce_min(list, detail, 8);
    draw_list_pane(app, f, list);
    draw_detail_pane(app, f, detail);
    let (label, suffix) = app.status_parts();
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw(label),
            Span::styled(suffix, Style::default().add_modifier(MUTED)),
        ])),
        status,
    );
    f.render_widget(
        Paragraph::new(
            to_line(&app.key_hints_markup()).patch_style(Style::default().add_modifier(MUTED)),
        ),
        hints,
    );
    draw_overlays(app, f, area);
}

/// The app drawn on a `width` x `height` off-screen terminal, one line per row.
pub fn screen_text(app: &App, width: u16, height: u16) -> String {
    let Ok(mut term) = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height));
    if term.draw(|f| draw(app, f)).is_err() {
        return String::new();
    }
    let buf = term.backend().buffer();
    (0..height)
        .map(|y| (0..width).map(|x| buf[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Toasts and modals over whatever screen is up.
fn draw_overlays(app: &App, f: &mut Frame, area: Rect) {
    draw_toasts(
        app,
        f,
        Rect {
            height: area.height.saturating_sub(1),
            ..area
        },
    );
    for (modal, _) in &app.modals {
        modal.render(f, area);
    }
}

/// Both panes keep at least `min` rows when the terminal allows it.
fn enforce_min(a: Rect, b: Rect, min: u16) -> (Rect, Rect) {
    let total = a.height + b.height;
    if total < min * 2 {
        return (a, b);
    }
    let ah = a.height.clamp(min, total - min);
    (
        Rect { height: ah, ..a },
        Rect {
            y: a.y + ah,
            height: total - ah,
            ..b
        },
    )
}

fn pane_border(focused: bool) -> Style {
    Style::default().fg(if focused { SUCCESS } else { SURFACE })
}

fn draw_list_pane(app: &App, f: &mut Frame, area: Rect) {
    let focused = matches!(app.focus, Focus::List | Focus::Filter | Focus::Search);
    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_type(BorderType::Thick)
        .border_style(pane_border(focused));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let mut constraints = vec![Constraint::Min(4)];
    if app.filter.visible {
        constraints.push(Constraint::Length(1));
    }
    if app.search_input_visible {
        constraints.push(Constraint::Length(1));
    }
    if app.filter.bar.is_some() {
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Length(1));
    let parts = Layout::vertical(constraints).split(inner);
    let mut i = 0;
    draw_table(app, f, parts[i]);
    i += 1;
    if app.filter.visible {
        input_line(
            f,
            pad1(parts[i]),
            &app.filter.input,
            app.focus == Focus::Filter,
        );
        i += 1;
    }
    if app.search_input_visible {
        input_line(
            f,
            pad1(parts[i]),
            &app.search_input,
            app.focus == Focus::Search,
        );
        i += 1;
    }
    if let Some(bar) = &app.filter.bar {
        f.render_widget(Paragraph::new(to_line(bar)), parts[i]);
        i += 1;
    }
    f.render_widget(
        Paragraph::new(
            to_line(&app.filter_bar_markup()).patch_style(Style::default().add_modifier(MUTED)),
        ),
        parts[i],
    );
}

fn pad1(r: Rect) -> Rect {
    Rect {
        x: r.x + 1,
        width: r.width.saturating_sub(2),
        ..r
    }
}

fn draw_table(app: &App, f: &mut Frame, area: Rect) {
    let ncols = app.table_columns.len();
    if ncols == 0 || area.height == 0 {
        app.table_height.set(area.height.saturating_sub(1) as usize);
        return;
    }
    let mut widths: Vec<usize> = app
        .table_columns
        .iter()
        .map(|c| UnicodeWidthStr::width(c.as_str()))
        .collect();
    for row in &app.rows {
        for (i, (cell, _)) in row.cells.iter().enumerate().take(ncols) {
            widths[i] = widths[i].max(UnicodeWidthStr::width(cell.text.as_str()));
        }
    }
    let visible = area.height.saturating_sub(1) as usize;
    app.table_height.set(visible.max(1));
    let mut offset = app.table_offset.get();
    if app.cursor < offset {
        offset = app.cursor;
    } else if visible > 0 && app.cursor >= offset + visible {
        offset = app.cursor + 1 - visible;
    }
    app.table_offset.set(offset);
    let mut lines = Vec::with_capacity(visible + 1);
    let header: Vec<Span> = app
        .table_columns
        .iter()
        .enumerate()
        .map(|(i, c)| {
            Span::styled(
                format!(" {} ", pad_to(c, widths[i])),
                Style::default().add_modifier(Modifier::BOLD),
            )
        })
        .collect();
    lines.push(Line::from(header));
    for (ri, row) in app.rows.iter().enumerate().skip(offset).take(visible) {
        let cursor = ri == app.cursor;
        let mut spans = Vec::new();
        for (i, (cell, prefix)) in row.cells.iter().enumerate().take(ncols) {
            let padded = pad_to(&cell.text, widths[i]);
            let mut push = |text: String, style: Style| {
                let style = if cursor {
                    style.bg(PRIMARY).fg(Color::White)
                } else {
                    style
                };
                spans.push(Span::styled(text, style));
            };
            push(" ".into(), Style::default());
            if *prefix > 0 {
                let head: String = padded.chars().take(*prefix).collect();
                let tail: String = padded.chars().skip(*prefix).collect();
                push(
                    head,
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
                );
                push(tail, cell.style);
            } else {
                push(padded, cell.style);
            }
            push(" ".into(), Style::default());
        }
        lines.push(Line::from(spans));
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn draw_detail_pane(app: &App, f: &mut Frame, area: Rect) {
    let focused = app.focus == Focus::Detail;
    let block = Block::default()
        .borders(Borders::TOP)
        .border_type(BorderType::Thick)
        .border_style(pane_border(focused));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let d = &app.detail;
    let mut constraints = vec![Constraint::Length(1)];
    if !d.state_flow.is_empty() {
        constraints.push(Constraint::Length(1));
    }
    if !d.reporter.is_empty() {
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Length(1));
    constraints.push(Constraint::Fill(1));
    let parts = Layout::vertical(constraints).split(inner);
    let mut i = 0;
    let bar: Vec<Span> = d
        .bar
        .iter()
        .map(|(t, s)| Span::styled(t.clone(), *s))
        .collect();
    f.render_widget(Paragraph::new(Line::from(bar)), parts[i]);
    i += 1;
    if !d.state_flow.is_empty() {
        f.render_widget(Paragraph::new(to_line(&d.state_flow)), parts[i]);
        i += 1;
    }
    if !d.reporter.is_empty() {
        f.render_widget(Paragraph::new(to_line(&d.reporter)), parts[i]);
        i += 1;
    }
    f.render_widget(Paragraph::new(to_line(&d.tabs)), parts[i]);
    i += 1;
    let body = parts[i];
    let (content_area, side_area) = match &d.side {
        Some(_) => {
            let [c, s] = Layout::horizontal([Constraint::Fill(3), Constraint::Fill(2)]).areas(body);
            (c, Some(s))
        }
        None => (body, None),
    };
    let text = match &d.content {
        Content::Markup(m) => to_text(m),
        Content::Plain(t, style) => plain_text(t).patch_style(*style),
    };
    let content_area = pad1(content_area);
    let rows = wrapped_rows(&text, content_area.width);
    app.detail_max_scroll.set(
        rows.saturating_sub(content_area.height as usize)
            .min(u16::MAX as usize) as u16,
    );
    f.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .scroll((d.scroll, 0)),
        content_area,
    );
    if let (Some(lines), Some(side)) = (&d.side, side_area) {
        let block = Block::default()
            .borders(Borders::LEFT)
            .border_type(BorderType::Thick)
            .border_style(Style::default().fg(SURFACE))
            .padding(Padding::horizontal(1));
        f.render_widget(
            Paragraph::new(to_text(&lines.join("\n")))
                .wrap(Wrap { trim: false })
                .block(block),
            side,
        );
    }
}

fn draw_toasts(app: &App, f: &mut Frame, area: Rect) {
    let mut bottom = area.y + area.height;
    for t in app.toasts.iter().rev() {
        let width = 50.min(area.width.saturating_sub(2));
        if width < 10 {
            return;
        }
        let mut lines: Vec<Line> = Vec::new();
        if let Some(title) = &t.title {
            lines.push(Line::from(Span::styled(
                title.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            )));
        }
        lines.extend(plain_text(&t.message).lines);
        let text = Text::from(lines);
        let h = (wrapped_rows(&text, width.saturating_sub(4)) as u16 + 2).min(area.height);
        if bottom < area.y + h {
            return;
        }
        bottom -= h;
        let rect = Rect {
            x: area.x + area.width - width - 1,
            y: bottom,
            width,
            height: h,
        };
        let color = match t.severity {
            Severity::Information => PRIMARY,
            Severity::Warning => WARNING,
            Severity::Error => ERROR,
        };
        f.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::LEFT | Borders::RIGHT)
            .border_type(BorderType::Thick)
            .border_style(Style::default().fg(color))
            .padding(Padding::new(1, 1, 1, 0))
            .style(Style::default().bg(SURFACE));
        f.render_widget(
            Paragraph::new(text).wrap(Wrap { trim: false }).block(block),
            rect,
        );
    }
}
