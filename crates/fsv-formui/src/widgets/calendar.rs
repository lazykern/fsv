//! A month grid: arrows move a day/week, PgUp/PgDn a month, Home today,
//! Enter picks.

use crossterm::event::{KeyCode, KeyEvent};
use fsv_forms::when::{DAY_NAMES, MONTH_NAMES, add_months, month_weeks};
use jiff::ToSpan;
use jiff::civil::Date;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::kit::Theme;

pub const WIDTH: u16 = 22;
pub const HEIGHT: u16 = 9;

/// CPython `str.center(width)`: the odd space goes left when the padding
/// and the width are both odd.
pub fn center(s: &str, width: usize) -> String {
    let n = s.chars().count();
    if n >= width {
        return s.to_string();
    }
    let pad = width - n;
    let left = pad / 2 + (pad & width & 1);
    format!("{}{s}{}", " ".repeat(left), " ".repeat(pad - left))
}

pub enum CalendarEvent {
    Picked(Date),
    Handled,
    Ignored,
}

pub struct CalendarGrid {
    pub cursor: Date,
    pub today: Date,
}

impl CalendarGrid {
    pub fn new(start: Date, today: Date) -> Self {
        CalendarGrid {
            cursor: start,
            today,
        }
    }

    fn move_days(&mut self, n: i64) {
        if let Ok(d) = self.cursor.checked_add(n.days()) {
            self.cursor = d;
        }
    }

    fn move_months(&mut self, n: i64) {
        if let Ok(d) = add_months(self.cursor, n) {
            self.cursor = d;
        }
    }

    pub fn handle_key(&mut self, k: &KeyEvent) -> CalendarEvent {
        match k.code {
            KeyCode::Left => self.move_days(-1),
            KeyCode::Right => self.move_days(1),
            KeyCode::Up => self.move_days(-7),
            KeyCode::Down => self.move_days(7),
            KeyCode::PageUp => self.move_months(-1),
            KeyCode::PageDown => self.move_months(1),
            KeyCode::Home => self.cursor = self.today,
            KeyCode::Enter => return CalendarEvent::Picked(self.cursor),
            _ => return CalendarEvent::Ignored,
        }
        CalendarEvent::Handled
    }

    /// The grid as text rows (header, weekday names, weeks).
    pub fn rows(&self) -> Vec<String> {
        let d = self.cursor;
        let head = format!("{} {}", MONTH_NAMES[d.month() as usize - 1], d.year());
        let mut rows = vec![
            format!("‹{}›", center(&head, 18)),
            DAY_NAMES
                .iter()
                .map(|n| &n[..2])
                .collect::<Vec<_>>()
                .join(" "),
        ];
        for week in self.weeks() {
            rows.push(
                week.iter()
                    .map(|day| format!("{:2}", day.day()))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
        rows
    }

    fn weeks(&self) -> Vec<Vec<Date>> {
        month_weeks(
            i64::from(self.cursor.year()),
            i64::from(self.cursor.month()),
            0,
        )
        .unwrap_or_default()
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer, focused: bool, theme: &Theme) {
        let base = if focused {
            Style::default().bg(theme.boost)
        } else {
            Style::default()
        };
        let area = Rect::new(
            area.x,
            area.y,
            area.width.min(WIDTH),
            area.height.min(HEIGHT),
        );
        buf.set_style(area, base);
        let rows = self.rows();
        let w = usize::from(area.width);
        for (i, row) in rows.iter().enumerate().take(usize::from(area.height)) {
            let y = area.y + i as u16;
            let style = match i {
                0 => base.add_modifier(Modifier::BOLD),
                1 => base.add_modifier(Modifier::DIM),
                _ => base,
            };
            buf.set_stringn(area.x, y, row, w, style);
        }
        for (r, week) in self.weeks().iter().enumerate() {
            let y = area.y + 2 + r as u16;
            if y >= area.bottom() {
                break;
            }
            for (c, day) in week.iter().enumerate() {
                let x = area.x + c as u16 * 3;
                let style = if *day == self.cursor {
                    Some(Modifier::REVERSED | Modifier::BOLD)
                } else if day.month() != self.cursor.month() {
                    Some(Modifier::DIM)
                } else if *day == self.today {
                    Some(Modifier::UNDERLINED)
                } else {
                    None
                };
                if let Some(m) = style {
                    buf.set_style(
                        Rect::new(x, y, 2, 1).intersection(area),
                        base.add_modifier(m),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn center_matches_cpython() {
        assert_eq!(center("Nov 2026", 18), "     Nov 2026     ");
        assert_eq!(center("May 2026", 18), "     May 2026     ");
        assert_eq!(center("abc", 6), " abc  ");
        assert_eq!(center("abc", 8), "  abc   ");
        assert_eq!(center("ab", 5), "  ab ");
        assert_eq!(center("abc", 4), "abc ");
        assert_eq!(center("ab", 3), " ab");
    }

    #[test]
    fn grid_rows() {
        let d = Date::new(2026, 11, 12).unwrap();
        let g = CalendarGrid::new(d, d);
        let rows = g.rows();
        assert_eq!(rows[0], "‹     Nov 2026     ›");
        assert_eq!(rows[1], "Mo Tu We Th Fr Sa Su");
        assert_eq!(rows[2], "26 27 28 29 30 31  1");
        assert_eq!(rows.len(), 2 + 6);
    }

    #[test]
    fn keys_move() {
        let d = Date::new(2026, 1, 31).unwrap();
        let mut g = CalendarGrid::new(d, d);
        g.handle_key(&KeyEvent::from(KeyCode::PageDown));
        assert_eq!(g.cursor, Date::new(2026, 2, 28).unwrap());
        g.handle_key(&KeyEvent::from(KeyCode::Down));
        assert_eq!(g.cursor, Date::new(2026, 3, 7).unwrap());
        assert!(matches!(
            g.handle_key(&KeyEvent::from(KeyCode::Enter)),
            CalendarEvent::Picked(_)
        ));
    }
}
