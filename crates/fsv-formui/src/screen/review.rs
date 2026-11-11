//! The review (same content as the CLI review) with Submit / Back to edit /
//! Save draft / Copy as command / Cancel.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use fsv_core::review::{NO_PAYLOAD, Review};
use fsv_core::term::{Column, Overflow, Table, escape};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;

use crate::backend::{FormKind, FormMode, Plan};
use crate::kit::draw::{Button, Variant, draw_lines, fill, wrap_line, wrap_lines};
use crate::kit::{Theme, markup};
use crate::session::FormSession;

pub const SR_SAFETY: &str =
    "This sends a real service request to the catalog item's fulfilment team.";

/// What the review asks the form screen to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewAction {
    Submit,
    Back,
    Save,
    Command,
    Cancel,
    /// Ctrl+S / F6 refused: show this warning.
    Refused(&'static str),
    ToggleAck,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewFocus {
    Ack,
    Submit,
    Back,
    Save,
    Command,
    Cancel,
}

/// The review lines `Review::render` prints, as styled lines.
pub fn review_lines(review: &Review, width: u16) -> Vec<Line<'static>> {
    let width = width.max(40);
    let mut lines: Vec<Line<'static>> = vec![];
    let push_md = |lines: &mut Vec<Line<'static>>, m: &str| {
        lines.extend(wrap_lines(&markup::lines(m, Style::default()), width));
    };
    push_md(
        &mut lines,
        &format!(
            "[bold]──────── Review: {} ────────[/bold]",
            escape(&review.title)
        ),
    );
    let push_table = |lines: &mut Vec<Line<'static>>, t: &Table| {
        for row in t.render_lines(usize::from(width), false) {
            lines.push(Line::from(row));
        }
    };
    if !review.rows.is_empty() {
        let mut t = Table::new().show_header(false).no_box().pad_edge(false);
        t.add_column(Column::new("").style("cyan").no_wrap());
        t.add_column(Column::new("").overflow(Overflow::Fold));
        for (label, value) in &review.rows {
            t.add_row([escape(label), escape(value)]);
        }
        push_table(&mut lines, &t);
    }
    if !review.changes.is_empty() {
        let mut t = Table::new().show_header(true).no_box().pad_edge(false);
        t.add_column(Column::new("Field").style("cyan").no_wrap());
        t.add_column(Column::new("Old").overflow(Overflow::Fold));
        t.add_column(Column::new("").no_wrap());
        t.add_column(Column::new("New").overflow(Overflow::Fold));
        for (label, old, new) in &review.changes {
            let dash = |s: &str| {
                if s.is_empty() {
                    "—".to_string()
                } else {
                    escape(s)
                }
            };
            t.add_row([escape(label), dash(old), "→".to_string(), dash(new)]);
        }
        push_table(&mut lines, &t);
    }
    let section = |lines: &mut Vec<Line<'static>>, title: &str, items: &[String], style: &str| {
        if items.is_empty() {
            return;
        }
        push_md(lines, &format!("[bold]{title}[/bold]"));
        for it in items {
            if style.is_empty() {
                push_md(lines, &format!("  {}", escape(it)));
            } else {
                push_md(lines, &format!("  [{style}]{}[/{style}]", escape(it)));
            }
        }
    };
    section(
        &mut lines,
        "Will notify / create work for",
        &review.notify,
        "yellow",
    );
    let msgs: Vec<String> = review
        .messages
        .iter()
        .map(|m| {
            let tag = if m.blocking && !m.acknowledged {
                "BLOCKING (needs --ack or y)"
            } else if m.blocking {
                "acknowledged"
            } else {
                "message"
            };
            format!("[{tag}] {}", m.text)
        })
        .collect();
    section(&mut lines, "Messages", &msgs, "magenta");
    section(
        &mut lines,
        "Dropped (hidden by rules, not sent)",
        &review.dropped,
        "dim",
    );
    section(&mut lines, "Rule notes", &review.notes, "dim");
    if !review.attachments.is_empty() {
        let items: Vec<String> = review
            .attachments
            .iter()
            .map(|a| {
                let short: String = a.sha256.chars().take(16).collect();
                format!(
                    "{}: {}  {} B  sha256:{short}",
                    a.target,
                    a.name,
                    crate::files::group_thousands(a.size)
                )
            })
            .collect();
        section(
            &mut lines,
            "Attachments (uploaded after you confirm)",
            &items,
            "",
        );
    }
    section(&mut lines, "Requires confirmation", &review.risks, "yellow");
    section(&mut lines, "Warnings", &review.warnings, "yellow");
    section(
        &mut lines,
        "Problems (must be fixed before sending)",
        &review.errors,
        "red",
    );
    if !review.metadata.is_empty() {
        let md = &review.metadata;
        let s = |k: &str| fsv_py::py_str(md.get(k).unwrap_or(&serde_json::Value::Null));
        let asof = md
            .get("updated_at")
            .filter(|v| fsv_py::truthy(v))
            .map_or("unknown".to_string(), fsv_py::py_str);
        push_md(
            &mut lines,
            &format!(
                "[dim]form metadata as of {} (fetched {}, {}, fingerprint {})[/dim]",
                escape(&asof),
                escape(&s("fetched_at")),
                escape(&s("source")),
                escape(&s("fingerprint"))
            ),
        );
    }
    if !review.plan_hash.is_empty() {
        push_md(
            &mut lines,
            &format!("[bold]plan[/bold] {}", review.plan_hash),
        );
    }
    lines
}

pub struct ReviewState {
    pub plan: Plan,
    pub scroll: usize,
    pub show_payload: bool,
    pub has_ack: bool,
    pub focus: ReviewFocus,
}

impl ReviewState {
    pub fn new(plan: Plan, session: &FormSession) -> Self {
        let has_ack = !plan.review.blocking_unacked().is_empty() || session.ack;
        let mut r = ReviewState {
            plan,
            scroll: 0,
            show_payload: false,
            has_ack,
            focus: ReviewFocus::Back,
        };
        if !r.submit_disabled() {
            r.focus = ReviewFocus::Submit;
        }
        r
    }

    pub fn submit_disabled(&self) -> bool {
        !self.plan.ok()
            || self.plan.hash.is_empty()
            || !self.plan.review.blocking_unacked().is_empty()
    }

    pub fn command_disabled(&self) -> bool {
        self.plan.hash.is_empty()
    }

    pub fn submit_label(session: &FormSession) -> &'static str {
        if session.dry_run {
            "Finish (dry run)"
        } else if session.kind == FormKind::Sr {
            "Place request"
        } else if session.mode == FormMode::Edit {
            "Update"
        } else {
            "Create"
        }
    }

    pub fn banner(&self, session: &FormSession) -> Vec<String> {
        let mut banner = vec![];
        if session.kind == FormKind::Sr {
            banner.push(format!("⚠ {SR_SAFETY}"));
        }
        if session.dry_run {
            banner.push("Dry run: Finish shows the plan in the terminal; nothing is sent.".into());
        }
        if self.plan.ok() && self.plan.hash.is_empty() && session.mode == FormMode::Edit {
            banner.push("No changes: nothing would be sent.".into());
        }
        if !self.plan.review.blocking_unacked().is_empty() {
            banner.push(
                "A blocking rule message needs your acknowledgement (tick the box below).".into(),
            );
        }
        banner
    }

    fn ring(&self) -> Vec<ReviewFocus> {
        let mut ring = vec![];
        if self.has_ack {
            ring.push(ReviewFocus::Ack);
        }
        if !self.submit_disabled() {
            ring.push(ReviewFocus::Submit);
        }
        ring.push(ReviewFocus::Back);
        ring.push(ReviewFocus::Save);
        if !self.command_disabled() {
            ring.push(ReviewFocus::Command);
        }
        ring.push(ReviewFocus::Cancel);
        ring
    }

    /// After a rebuild: keep focus on something that can take it.
    pub fn refocus(&mut self) {
        if !self.ring().contains(&self.focus) {
            self.focus = ReviewFocus::Back;
        }
    }

    pub fn handle_key(&mut self, k: &KeyEvent) -> Option<ReviewAction> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Char('s') if ctrl => {
                return Some(if self.submit_disabled() {
                    ReviewAction::Refused("fix the problems first (Back to edit)")
                } else {
                    ReviewAction::Submit
                });
            }
            KeyCode::Esc => return Some(ReviewAction::Back),
            KeyCode::F(4) => return Some(ReviewAction::Save),
            KeyCode::F(6) => {
                return Some(if self.command_disabled() {
                    ReviewAction::Refused("no complete plan yet")
                } else {
                    ReviewAction::Command
                });
            }
            KeyCode::F(7) => self.show_payload = !self.show_payload,
            KeyCode::Tab | KeyCode::BackTab => {
                let back = k.code == KeyCode::BackTab || k.modifiers.contains(KeyModifiers::SHIFT);
                let ring = self.ring();
                let i = ring.iter().position(|f| *f == self.focus).unwrap_or(0) as isize;
                let n = ring.len() as isize;
                self.focus = ring[(i + if back { -1 } else { 1 }).rem_euclid(n) as usize];
            }
            KeyCode::Left | KeyCode::Right if self.focus != ReviewFocus::Ack => {
                let ring: Vec<_> = self
                    .ring()
                    .into_iter()
                    .filter(|f| *f != ReviewFocus::Ack)
                    .collect();
                let i = ring.iter().position(|f| *f == self.focus).unwrap_or(0) as isize;
                let by = if k.code == KeyCode::Left { -1 } else { 1 };
                self.focus = ring[(i + by).clamp(0, ring.len() as isize - 1) as usize];
            }
            KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::Down => self.scroll += 1,
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(10),
            KeyCode::PageDown => self.scroll += 10,
            KeyCode::Home => self.scroll = 0,
            KeyCode::End => self.scroll = usize::MAX / 2,
            KeyCode::Enter | KeyCode::Char(' ') => {
                return Some(match self.focus {
                    ReviewFocus::Ack => ReviewAction::ToggleAck,
                    ReviewFocus::Submit => ReviewAction::Submit,
                    ReviewFocus::Back => ReviewAction::Back,
                    ReviewFocus::Save => ReviewAction::Save,
                    ReviewFocus::Command => ReviewAction::Command,
                    ReviewFocus::Cancel => ReviewAction::Cancel,
                });
            }
            _ => {}
        }
        None
    }

    fn body(&self, width: u16, theme: &Theme) -> Vec<Line<'static>> {
        let mut lines = review_lines(&self.plan.review, width);
        if self.show_payload {
            let text = match (&self.plan.payload, self.plan.ok()) {
                (Some(p), true) => format!(
                    "Payload\n{}",
                    serde_json::to_string_pretty(p).unwrap_or_default()
                ),
                _ => NO_PAYLOAD.to_string(),
            };
            for l in text.split('\n') {
                lines.extend(wrap_line(
                    &Line::styled(l.to_string(), Style::default().fg(theme.text_muted)),
                    width,
                ));
            }
        }
        lines
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer, session: &FormSession, theme: &Theme) {
        fill(
            area,
            buf,
            Style::default().bg(theme.background).fg(theme.foreground),
        );
        let mut y = area.y;
        let title = Rect::new(area.x, y, area.width, 1);
        fill(
            title,
            buf,
            Style::default()
                .bg(theme.primary)
                .fg(theme.text)
                .add_modifier(Modifier::BOLD),
        );
        buf.set_stringn(
            area.x + 1,
            y,
            format!("Review: {}", session.title),
            usize::from(area.width.saturating_sub(2)),
            Style::default()
                .bg(theme.primary)
                .fg(theme.text)
                .add_modifier(Modifier::BOLD),
        );
        y += 1;
        let inner_w = area.width.saturating_sub(2);
        let banner: Vec<Line<'static>> = self
            .banner(session)
            .iter()
            .flat_map(|b| {
                wrap_line(
                    &Line::styled(
                        b.clone(),
                        Style::default()
                            .fg(theme.warning)
                            .add_modifier(Modifier::BOLD),
                    ),
                    inner_w,
                )
            })
            .collect();
        draw_lines(
            Rect::new(area.x + 1, y, inner_w, banner.len() as u16),
            buf,
            &banner,
            0,
        );
        y += banner.len() as u16;
        let bottom_h = 3 + u16::from(self.has_ack) * 2 + 1;
        let body_h = area.bottom().saturating_sub(y + bottom_h);
        let body = self.body(area.width.saturating_sub(4).max(40), theme);
        let max_scroll = body.len().saturating_sub(usize::from(body_h));
        self.scroll = self.scroll.min(max_scroll);
        draw_lines(
            Rect::new(area.x + 1, y, inner_w, body_h),
            buf,
            &body,
            self.scroll,
        );
        y += body_h;
        if self.has_ack {
            let mark = if session.ack { "X" } else { " " };
            let mut style = Style::default();
            if self.focus == ReviewFocus::Ack {
                style = style.add_modifier(Modifier::REVERSED);
            }
            buf.set_stringn(
                area.x + 1,
                y,
                format!("▐{mark}▌ I acknowledge the blocking rule message(s)"),
                usize::from(inner_w),
                style,
            );
            y += 2;
        }
        let mut x = area.x + 1;
        let submit_label = Self::submit_label(session);
        for (label, variant, f, disabled) in [
            (
                submit_label,
                Variant::Success,
                ReviewFocus::Submit,
                self.submit_disabled(),
            ),
            ("Back to edit", Variant::Primary, ReviewFocus::Back, false),
            ("Save draft", Variant::Default, ReviewFocus::Save, false),
            (
                "Copy as command",
                Variant::Default,
                ReviewFocus::Command,
                self.command_disabled(),
            ),
            ("Cancel", Variant::Error, ReviewFocus::Cancel, false),
        ] {
            x += Button {
                label,
                variant,
                focused: self.focus == f,
                disabled,
            }
            .draw(x, y, buf, theme)
                + 1;
        }
        super::footer(
            Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
            buf,
            &[
                ("esc", "Back to edit"),
                ("^s", "Submit"),
                ("f4", "Save draft"),
                ("f6", "Copy as command"),
                ("f7", "Show payload"),
            ],
            theme,
        );
    }
}
