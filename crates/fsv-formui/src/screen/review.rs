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
use crate::kit::{Theme, ansi, markup};
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

/// `review_text`: the review `Review::render` prints, as styled lines.
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
        for row in t.render_lines(usize::from(width), true) {
            lines.push(ansi::line(&row));
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

    /// Submit (Ctrl+S), refused while the plan has problems.
    pub fn action_submit(&self) -> ReviewAction {
        if self.submit_disabled() {
            ReviewAction::Refused("fix the problems first (Back to edit)")
        } else {
            ReviewAction::Submit
        }
    }

    /// Copy as command (F6), refused without a complete plan.
    pub fn action_command(&self) -> ReviewAction {
        if self.command_disabled() {
            ReviewAction::Refused("no complete plan yet")
        } else {
            ReviewAction::Command
        }
    }

    /// Show or hide the payload under the review (F7).
    pub fn action_payload(&mut self) {
        self.show_payload = !self.show_payload;
    }

    /// `ReviewScreen.action_back` (Esc, Back to edit).
    pub fn action_back(&self) -> ReviewAction {
        ReviewAction::Back
    }

    /// `ReviewScreen._cancel` (the Cancel button).
    pub fn cancel(&self) -> ReviewAction {
        ReviewAction::Cancel
    }

    pub fn handle_key(&mut self, k: &KeyEvent) -> Option<ReviewAction> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Char('s') if ctrl => return Some(self.action_submit()),
            KeyCode::Esc => return Some(self.action_back()),
            KeyCode::F(4) => return Some(ReviewAction::Save),
            KeyCode::F(6) => return Some(self.action_command()),
            KeyCode::F(7) => self.action_payload(),
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
                    ReviewFocus::Submit => self.action_submit(),
                    ReviewFocus::Back => self.action_back(),
                    ReviewFocus::Save => ReviewAction::Save,
                    ReviewFocus::Command => self.action_command(),
                    ReviewFocus::Cancel => self.cancel(),
                });
            }
            _ => {}
        }
        None
    }

    /// `ReviewScreen._refresh_review`: the review, then the payload when shown.
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    fn cells(l: &Line<'_>) -> Vec<(char, Style)> {
        l.spans
            .iter()
            .flat_map(|s| s.content.chars().map(move |c| (c, s.style)))
            .collect()
    }

    /// `review_text(review, 60)` in Python, colours on.
    #[test]
    fn review_text_keeps_the_cli_layout_and_the_cyan_labels() {
        let mut r = Review::new("New Change");
        r.rows = vec![
            ("Subject".into(), "Billing: rotate keys".into()),
            ("Priority".into(), "High".into()),
        ];
        r.changes = vec![
            ("Priority".into(), "Medium".into(), "High".into()),
            ("Stage".into(), String::new(), "Live".into()),
        ];
        r.errors = vec!["Subject is required".into()];
        r.plan_hash = "0123456789abcdef".into();
        let lines = review_lines(&r, 60);
        let plain: Vec<String> = lines
            .iter()
            .map(|l| cells(l).into_iter().map(|c| c.0).collect())
            .collect();
        assert_eq!(
            plain,
            [
                "──────── Review: New Change ────────",
                "Subject   Billing: rotate keys",
                "Priority  High                ",
                "Field     Old        New ",
                "Priority  Medium  →  High",
                "Stage     —       →  Live",
                "Problems (must be fixed before sending)",
                "  Subject is required",
                "plan 0123456789abcdef",
            ]
        );
        for i in [1, 2, 4, 5] {
            let c = cells(&lines[i]);
            assert!(c[..9].iter().all(|x| x.1.fg == Some(Color::Cyan)), "{i}");
            assert_eq!(c[9].1.fg, None, "{i}");
        }
        let head = cells(&lines[3]);
        assert!(
            head[..8]
                .iter()
                .all(|x| x.1.add_modifier.contains(Modifier::BOLD))
        );
        let err = cells(&lines[7]);
        assert_eq!(err[2].1.fg, Some(Color::Red));
        assert_eq!(err[1].1.fg, None);
    }

    /// `ReviewScreen.action_submit` / `action_command` / `action_payload`.
    #[test]
    fn review_actions_refuse_an_incomplete_plan() {
        let (_acme, mut session) = crate::acme::acme();
        let plan = session.build_plan(None).expect("plan");
        let mut r = ReviewState::new(plan, &session);
        assert_eq!(r.action_submit(), ReviewAction::Submit);
        assert_eq!(r.action_command(), ReviewAction::Command);
        r.action_payload();
        assert!(r.show_payload);
        r.action_payload();
        assert!(!r.show_payload);
        r.plan.hash.clear();
        assert_eq!(
            r.action_submit(),
            ReviewAction::Refused("fix the problems first (Back to edit)")
        );
        assert_eq!(
            r.action_command(),
            ReviewAction::Refused("no complete plan yet")
        );
        r.refocus();
        assert_eq!(r.focus, ReviewFocus::Back);
        assert_eq!(ReviewState::submit_label(&session), "Create");
        assert_eq!(r.action_back(), ReviewAction::Back);
        assert_eq!(r.cancel(), ReviewAction::Cancel);
        r.focus = ReviewFocus::Cancel;
        assert_eq!(
            r.handle_key(&crate::kit::parse_key("enter")),
            Some(ReviewAction::Cancel)
        );
        assert_eq!(
            r.handle_key(&crate::kit::parse_key("escape")),
            Some(ReviewAction::Back)
        );
    }

    fn plain(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| cells(l).into_iter().map(|c| c.0).collect())
            .collect()
    }

    /// `ReviewScreen._refresh_review`: the review, and under it (F7) the
    /// payload as `json.dumps(indent=2, ensure_ascii=False)`, or the
    /// no-payload line while the plan has problems.
    #[test]
    fn refresh_review_body_shows_the_payload_when_asked() {
        let theme = Theme::textual_dark();
        let plan = Plan {
            review: Review::new("New Change"),
            payload: Some(serde_json::json!({"subject": "Mail ✓", "tags": [], "n": 1})),
            ..Plan::default()
        };
        let (_acme, session) = crate::acme::acme();
        let mut r = ReviewState::new(plan, &session);
        let review = plain(&review_lines(&r.plan.review, 60));
        assert_eq!(plain(&r.body(60, &theme)), review);
        r.action_payload();
        let body = plain(&r.body(60, &theme));
        assert_eq!(body[..review.len()], review[..]);
        assert_eq!(
            body[review.len()..],
            [
                "Payload",
                "{",
                "  \"subject\": \"Mail ✓\",",
                "  \"tags\": [],",
                "  \"n\": 1",
                "}"
            ]
        );
        r.plan.errors.push("Subject is required".into());
        let body = plain(&r.body(60, &theme));
        assert_eq!(body.last().map(String::as_str), Some(NO_PAYLOAD));
    }
}
