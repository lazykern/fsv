//! Dates, durations and rich-text flattening as the agent web shows them
//! (`views.py` "dates and text").

use std::sync::LazyLock;

use fancy_regex::Regex as FancyRegex;
use jiff::civil::Date;
use jiff::tz::{Offset, TimeZone};
use jiff::{Timestamp, Zoned};
use regex::Regex;
use serde_json::Value;

use fsv_core::pystr;
use fsv_forms::pydt;

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// `parse_dt`: ISO text → instant. Naive values are UTC. `""`/`null` → `None`.
pub fn parse_dt(value: &Value) -> Option<Timestamp> {
    let text = match value {
        Value::Null => return None,
        Value::String(s) if s.is_empty() => return None,
        Value::String(s) => s.clone(),
        other => fsv_py::py_str(other),
    };
    parse_dt_str(&text)
}

pub fn parse_dt_str(text: &str) -> Option<Timestamp> {
    let normalized = text.replace('Z', "+00:00");
    let (local, offset) = pydt::datetime_fromisoformat(&normalized)?;
    offset.unwrap_or(Offset::UTC).to_timestamp(local).ok()
}

pub fn is_date_only(value: &Value) -> bool {
    static DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2}$").expect("regex"));
    value.as_str().is_some_and(|s| DATE.is_match(pystr::strip(s)))
}

/// `fmt_when`: `Fri, 27 Nov 2026 14:07` in `tz` (system zone when `None`).
/// Date-only values show no time. Unparseable values come back as text.
pub fn fmt_when(value: &Value, tz: Option<&TimeZone>, year: bool) -> String {
    if value.is_null() || value.as_str() == Some("") {
        return String::new();
    }
    if is_date_only(value) {
        let text = pystr::strip(value.as_str().unwrap_or_default());
        return match pydt::date_fromisoformat(text) {
            Some(d) => format!("{}, {} {} {}", weekday(d), d.day(), month(d), d.year()),
            None => text.to_string(),
        };
    }
    let Some(when) = parse_dt(value) else {
        return fsv_py::py_str(value);
    };
    let zoned = to_zone(when, tz);
    let yr = if year { format!(" {}", zoned.year()) } else { String::new() };
    format!(
        "{}, {} {}{yr} {:02}:{:02}",
        weekday(zoned.date()),
        zoned.day(),
        month(zoned.date()),
        zoned.hour(),
        zoned.minute()
    )
}

pub fn to_zone(when: Timestamp, tz: Option<&TimeZone>) -> Zoned {
    match tz {
        Some(tz) => when.to_zoned(tz.clone()),
        None => when.to_zoned(TimeZone::system()),
    }
}

fn weekday(d: Date) -> &'static str {
    WEEKDAYS[d.weekday().to_monday_zero_offset() as usize]
}

fn month(d: Date) -> &'static str {
    MONTHS[(d.month() - 1) as usize]
}

/// `relative`: `2 days ago` / `in 3 hours`, the web's moment.js wording.
pub fn relative(value: &Value, now: Option<Timestamp>) -> String {
    let Some(when) = parse_dt(value) else {
        return String::new();
    };
    let now = now.unwrap_or_else(Timestamp::now);
    let secs = (now.as_microsecond() - when.as_microsecond()) as f64 / 1_000_000.0;
    let future = secs < 0.0;
    let s = secs.abs();
    let r = |x: f64| pystr::round_half_even(x) as i64;
    let text = if s < 45.0 {
        "a few seconds".to_string()
    } else if s < 90.0 {
        "a minute".to_string()
    } else if s < 45.0 * 60.0 {
        format!("{} minutes", r(s / 60.0))
    } else if s < 90.0 * 60.0 {
        "an hour".to_string()
    } else if s < 22.0 * 3600.0 {
        format!("{} hours", r(s / 3600.0))
    } else if s < 36.0 * 3600.0 {
        "a day".to_string()
    } else if s < 26.0 * 86400.0 {
        format!("{} days", r(s / 86400.0))
    } else if s < 45.0 * 86400.0 {
        "a month".to_string()
    } else if s < 320.0 * 86400.0 {
        format!("{} months", r(s / (30.0 * 86400.0)))
    } else if s < 548.0 * 86400.0 {
        "a year".to_string()
    } else {
        format!("{} years", r(s / (365.0 * 86400.0)))
    };
    if future { format!("in {text}") } else { format!("{text} ago") }
}

/// `span`: two most significant units (`2w 1d`, `3h 20m`, `45m`).
pub fn span(seconds: Option<f64>) -> String {
    let Some(seconds) = seconds else {
        return String::new();
    };
    let s = seconds.abs().trunc() as i64;
    let units: [(&str, i64); 4] = [("w", 7 * 86400), ("d", 86400), ("h", 3600), ("m", 60)];
    for (i, (name, size)) in units.iter().enumerate() {
        let (mut n, rest) = (s / size, s % size);
        if n == 0 {
            continue;
        }
        if let Some((sub_name, sub_size)) = units.get(i + 1) {
            let mut sub = pystr::round_half_even(rest as f64 / *sub_size as f64) as i64;
            if sub * sub_size >= *size {
                n += 1;
                sub = 0;
            }
            return if sub != 0 { format!("{n}{name} {sub}{sub_name}") } else { format!("{n}{name}") };
        }
        return format!("{n}{name}");
    }
    "0m".to_string()
}

/// `html_to_text`: paragraphs and list items keep their breaks; scripts,
/// styles and tags are dropped.
pub fn html_to_text(value: &Value) -> String {
    let raw = match value {
        Value::Null => return String::new(),
        Value::String(s) if s.is_empty() => return String::new(),
        Value::String(s) => s.clone(),
        other => fsv_py::py_str(other),
    };
    html_to_text_str(&raw)
}

pub fn html_to_text_str(raw: &str) -> String {
    static SCRIPT: LazyLock<FancyRegex> =
        LazyLock::new(|| FancyRegex::new(r"(?is)<(script|style)\b.*?</\1>").expect("regex"));
    static BR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<br\s*/?>").expect("regex"));
    static LI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<li\b[^>]*>").expect("regex"));
    static BLOCK_END: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)</(p|div|li|tr|h[1-6]|ul|ol|table|blockquote)>").expect("regex"));
    static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").expect("regex"));
    static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ \t]+").expect("regex"));

    let s = SCRIPT.replace_all(raw, "").into_owned();
    let s = BR.replace_all(&s, "\n");
    let s = LI.replace_all(&s, "\n• ");
    let s = BLOCK_END.replace_all(&s, "\n");
    let s = TAG.replace_all(&s, "");
    let s = pystr::html_unescape(&s).replace('\u{a0}', " ");
    let mut out: Vec<String> = Vec::new();
    for line in py_splitlines(&s) {
        let ln = pystr::strip(&SPACES.replace_all(line, " ")).to_string();
        if ln.is_empty() && out.last().is_none_or(|l| l.is_empty()) {
            continue;
        }
        out.push(ln);
    }
    pystr::strip(&out.join("\n")).to_string()
}

/// `one_line`.
pub fn one_line(value: &Value) -> String {
    pystr::collapse_ws(&html_to_text(value))
}

/// `str.splitlines()` (the line-boundary set Python uses).
pub fn py_splitlines(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let boundary = matches!(
            c,
            '\n' | '\r' | '\u{0b}' | '\u{0c}' | '\u{1c}' | '\u{1d}' | '\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}'
        );
        if !boundary {
            continue;
        }
        out.push(&s[start..i]);
        let mut end = i + c.len_utf8();
        if c == '\r' {
            if let Some((j, '\n')) = chars.peek().copied() {
                chars.next();
                end = j + 1;
            }
        }
        start = end;
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn span_rounds_into_bigger_unit() {
        assert_eq!(span(None), "");
        assert_eq!(span(Some(0.0)), "0m");
        assert_eq!(span(Some(45.0 * 60.0)), "45m");
        assert_eq!(span(Some(3.0 * 3600.0 + 20.0 * 60.0)), "3h 20m");
        assert_eq!(span(Some(15.0 * 86400.0)), "2w 1d");
        // 23h 59.67m rounds to 24h. Python does not carry that into 1d.
        assert_eq!(span(Some(3600.0 * 23.0 + 59.0 * 60.0 + 40.0)), "24h");
    }

    #[test]
    fn relative_wording() {
        let now = parse_dt_str("2026-11-12T12:00:00Z").unwrap();
        assert_eq!(relative(&json!("2026-11-12T11:59:30Z"), Some(now)), "a few seconds ago");
        assert_eq!(relative(&json!("2026-11-10T12:00:00Z"), Some(now)), "2 days ago");
        assert_eq!(relative(&json!("2026-11-12T15:00:00Z"), Some(now)), "in 3 hours");
        assert_eq!(relative(&json!("nope"), Some(now)), "");
    }

    #[test]
    fn fmt_when_formats() {
        let utc = TimeZone::UTC;
        assert_eq!(fmt_when(&json!("2026-11-27T14:07:00Z"), Some(&utc), true), "Fri, 27 Nov 2026 14:07");
        assert_eq!(fmt_when(&json!("2026-11-09T14:07:00Z"), Some(&utc), false), "Mon, 9 Nov 14:07");
        assert_eq!(fmt_when(&json!("2026-11-27"), Some(&utc), true), "Fri, 27 Nov 2026");
        assert_eq!(fmt_when(&json!("2031-02-30"), Some(&utc), true), "2031-02-30");
        assert_eq!(fmt_when(&json!("soon"), Some(&utc), true), "soon");
        assert_eq!(fmt_when(&json!(null), Some(&utc), true), "");
    }

    #[test]
    fn html_flattening() {
        assert_eq!(
            html_to_text_str("<p>One&nbsp;<b>two</b></p><ul><li>a</li><li>b</li></ul><script>x</script>"),
            "One two\n\n• a\n\n• b"
        );
        assert_eq!(one_line(&json!("<div>a</div>\n<div>b</div>")), "a b");
    }
}
