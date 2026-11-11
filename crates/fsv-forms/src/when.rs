//! Typed dates and date-times for the form, with a live preview
//! (`fsv.formui.when`).
//!
//! Accepted (case-insensitive; the time zone is the form's, see
//! `FormState::tz`, unless an offset is typed):
//!
//! * `2026-11-26 19:00`, `2026-11-26T19:00+07:00`, `2026/11/26 7pm`
//! * `26 Nov 2026 19:00`, `Nov 26 2026 7:30pm`, `Thu 26 Nov 2026 19:00`
//! * `today 18:00`, `tomorrow 9am`, `+2d 10:00`, `+1w 09:00`, `fri 22:00`
//! * a time alone (`19:00`) means today
//!
//! Month and weekday names are matched in English *and* by number, so any
//! locale can type `2026-11-26`. The preview never depends on the
//! machine's locale.

use std::sync::LazyLock;

use jiff::ToSpan;
use jiff::civil::{Date, Time};
use jiff::tz::Offset;
use regex::Regex;

use crate::errors::{FormError, Result};
use crate::pydt::{DateTimeValue, date_iso, make_date, offset_text as offset_str, timedelta_repr};
use crate::pytext::{casefold, is_digits, parse_int, split_ws, strip, take_chars};
use crate::tz::{Zone, localize};
use crate::value::Value;

pub const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
pub const WEEKDAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
pub const MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
pub const DAY_NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
pub const TIME_SUGGESTIONS: [&str; 8] = [
    "09:00", "10:00", "13:00", "14:00", "18:00", "19:00", "20:00", "22:00",
];

static TIME_RX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?:^|[\sT]+)(?:at\s+)?(?P<h>\d{1,2})(?::(?P<m>\d{2}))?(?::(?P<s>\d{2}))?\s*(?P<ap>am|pm)?\s*(?P<off>z|[+-]\d{2}:?\d{2})?\s*$",
    )
    .expect("time regex")
});
static ISO_RX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d{4})[-/.](\d{1,2})[-/.](\d{1,2})$").expect("iso regex"));
static DMY_RX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(\d{1,2})\s*([a-z]+)\.?,?\s*(\d{4})?$").expect("dmy regex"));
static MDY_RX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^([a-z]+)\.?\s*(\d{1,2}),?\s*(\d{4})?$").expect("mdy regex"));
static REL_RX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\+\s*(\d+)\s*([dw])$").expect("rel regex"));
static DIGIT_RX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d").expect("digit regex"));

/// What the person typed, read: `value` is `Value::Date`, `Value::DateTime`
/// or `Value::Null` (empty input).
#[derive(Clone, Debug)]
pub struct Parsed {
    pub value: Value,
    pub preview: String,
    pub error: String,
}

impl Default for Parsed {
    fn default() -> Self {
        Parsed {
            value: Value::Null,
            preview: String::new(),
            error: String::new(),
        }
    }
}

impl Parsed {
    pub fn ok(&self) -> bool {
        self.error.is_empty()
    }

    fn error(msg: impl Into<String>) -> Parsed {
        Parsed {
            error: msg.into(),
            ..Parsed::default()
        }
    }
}

fn month(word: &str) -> Option<i64> {
    let w = casefold(word);
    let w = take_chars(&w, 3);
    MONTHS.iter().position(|m| *m == w).map(|i| i as i64 + 1)
}

fn weekday0(d: Date) -> i64 {
    d.weekday().to_monday_zero_offset() as i64
}

/// `d + timedelta(days=n)` within Python's date range.
fn add_days(d: Date, n: i64) -> std::result::Result<Date, String> {
    let out = i32::try_from(n)
        .ok()
        .and_then(|n| d.checked_add(n.days()).ok());
    match out {
        Some(v) if v.year() >= 1 => Ok(v),
        _ => Err("date value out of range".to_string()),
    }
}

fn int(s: &str) -> std::result::Result<i64, String> {
    parse_int(s).ok_or_else(|| {
        format!(
            "invalid literal for int() with base 10: {}",
            fsv_py::py_repr_str(s)
        )
    })
}

fn parse_date(text: &str, today: Date) -> std::result::Result<Option<Date>, String> {
    let folded = casefold(strip(text)).replace(',', " ");
    let mut t = split_ws(&folded).collect::<Vec<_>>().join(" ");
    if t.is_empty() {
        return Ok(None);
    }
    if let Some((first, rest)) = t.split_once(' ')
        && WEEKDAYS.contains(&take_chars(first, 3))
        && !is_digits(first)
        && DIGIT_RX.is_match(rest)
    {
        t = rest.to_string(); // "thu 26 nov 2026": the weekday is decoration
    }
    match t.as_str() {
        "today" | "now" => return Ok(Some(today)),
        "tomorrow" => return add_days(today, 1).map(Some),
        "yesterday" => return add_days(today, -1).map(Some),
        _ => {}
    }
    if let Some(m) = REL_RX.captures(&t) {
        let n = int(&m[1]).map_err(|_| "date value out of range".to_string())?;
        let per = if m[2].eq_ignore_ascii_case("w") { 7 } else { 1 };
        let days = n
            .checked_mul(per)
            .ok_or_else(|| "date value out of range".to_string())?;
        return add_days(today, days).map(Some);
    }
    if WEEKDAYS.contains(&take_chars(&t, 3)) && t.chars().all(char::is_alphabetic) {
        let target = WEEKDAYS
            .iter()
            .position(|w| *w == take_chars(&t, 3))
            .unwrap_or(0) as i64;
        let ahead = match (target - weekday0(today)).rem_euclid(7) {
            0 => 7,
            n => n,
        };
        return add_days(today, ahead).map(Some);
    }
    if let Some(m) = ISO_RX.captures(&t) {
        return make_date(int(&m[1])?, int(&m[2])?, int(&m[3])?).map(Some);
    }
    for (rx, dmy) in [(&*DMY_RX, true), (&*MDY_RX, false)] {
        let Some(m) = rx.captures(&t) else { continue };
        let (day, mon, year) = if dmy {
            (&m[1], month(&m[2]), m.get(3))
        } else {
            (&m[2], month(&m[1]), m.get(3))
        };
        let Some(mon) = mon else { continue };
        let day = int(day)?;
        if let Some(year) = year {
            return make_date(int(year.as_str())?, mon, day).map(Some);
        }
        let cand = make_date(today.year() as i64, mon, day)?;
        if cand >= today {
            return Ok(Some(cand));
        }
        return make_date(today.year() as i64 + 1, mon, day).map(Some);
    }
    Err(format!(
        "cannot read {} as a date",
        fsv_py::py_repr_str(strip(text))
    ))
}

type SplitTime = (String, Option<Time>, Option<Offset>);

/// (date text, time, typed offset). A time needs `:` or am/pm.
fn split_time(text: &str) -> std::result::Result<SplitTime, String> {
    let Some(m) = TIME_RX.captures(text) else {
        return Ok((text.to_string(), None, None));
    };
    if m.name("m").is_none() && m.name("ap").is_none() {
        return Ok((text.to_string(), None, None));
    }
    let mut h = int(&m["h"])?;
    let mi = m
        .name("m")
        .map(|x| int(x.as_str()))
        .transpose()?
        .unwrap_or(0);
    let s = m
        .name("s")
        .map(|x| int(x.as_str()))
        .transpose()?
        .unwrap_or(0);
    let ap = m
        .name("ap")
        .map(|x| x.as_str().to_lowercase())
        .unwrap_or_default();
    if !ap.is_empty() {
        if !(1..=12).contains(&h) {
            return Err(format!("{h}{ap}: the hour must be 1-12 with am/pm"));
        }
        h = h % 12 + if ap == "pm" { 12 } else { 0 };
    }
    let whole = m.get(0).map_or("", |x| x.as_str());
    if h > 23 || mi > 59 || s > 59 {
        return Err(format!(
            "{} is not a time of day",
            fsv_py::py_repr_str(strip(whole))
        ));
    }
    let mut off = None;
    if let Some(o) = m.name("off") {
        let o = o.as_str().to_uppercase();
        if o == "Z" {
            off = Some(Offset::UTC);
        } else {
            let sign = if o.starts_with('-') { -1 } else { 1 };
            let digits: String = o.chars().skip(1).filter(|c| *c != ':').collect();
            let hours = int(take_chars(&digits, 2))?;
            let minutes = int(digits.chars().skip(2).collect::<String>().as_str())?;
            let secs = sign * (hours * 3600 + minutes * 60);
            if secs.abs() >= 86400 {
                return Err(format!(
                    "offset must be a timedelta strictly between -timedelta(hours=24) and timedelta(hours=24), not {}.",
                    timedelta_repr(secs)
                ));
            }
            off = Some(Offset::from_seconds(secs as i32).map_err(|e| e.to_string())?);
        }
    }
    let start = m.get(0).map_or(0, |x| x.start());
    let time = Time::new(h as i8, mi as i8, s as i8, 0).map_err(|e| e.to_string())?;
    Ok((strip(&text[..start]).to_string(), Some(time), off))
}

/// `±HH:MM` of an aware date-time.
pub fn offset_text(v: &DateTimeValue) -> String {
    offset_str(v.offset)
}

/// Where the form's zone came from, for the preview.
pub fn tz_phrase(tz_source: &str) -> &'static str {
    if tz_source.starts_with("Freshservice user time zone") {
        return "from your Freshservice profile tz";
    }
    match tz_source {
        "FSV_TZ" => "from FSV_TZ",
        "tz argument" => "from the form's tz",
        _ => "from this computer's tz (no Freshservice profile tz)",
    }
}

/// `Thu 26 Nov 2026`.
pub fn date_text(d: Date) -> String {
    format!(
        "{} {} {} {}",
        DAY_NAMES[weekday0(d) as usize],
        d.day(),
        MONTH_NAMES[(d.month() - 1) as usize],
        d.year()
    )
}

/// The preview line for a parsed value (`""` for anything else).
pub fn preview(value: &Value, tz_source: &str, typed_offset: bool) -> String {
    match value {
        Value::DateTime(v) => {
            let where_ = if typed_offset {
                "as typed"
            } else {
                tz_phrase(tz_source)
            };
            format!(
                "{} {:02}:{:02} ({} {where_})",
                date_text(v.date()),
                v.local.hour(),
                v.local.minute(),
                offset_text(v)
            )
        }
        Value::Date(d) => date_text(*d),
        _ => String::new(),
    }
}

/// Options of [`parse_when`] besides the zone.
#[derive(Clone, Debug, Default)]
pub struct WhenOptions<'a> {
    pub tz_source: &'a str,
    pub date_only: bool,
    /// Defaults to today in `tz`.
    pub today: Option<Date>,
}

/// Parse what the person typed; never fails (errors go to `Parsed::error`).
pub fn parse_when(text: &str, tz: &Zone, opts: WhenOptions<'_>) -> Parsed {
    let raw = strip(text);
    if raw.is_empty() {
        return Parsed::default();
    }
    let today = opts
        .today
        .unwrap_or_else(|| jiff::Timestamp::now().to_zoned(tz.tz.clone()).date());
    match parse_inner(raw, tz, &opts, today) {
        Ok(p) => p,
        Err(msg) if msg.contains("day is out of range") || msg.contains("month must be") => {
            Parsed::error(format!("{} is not a real date", fsv_py::py_repr_str(raw)))
        }
        Err(msg) => Parsed::error(msg),
    }
}

fn parse_inner(
    raw: &str,
    tz: &Zone,
    opts: &WhenOptions<'_>,
    today: Date,
) -> std::result::Result<Parsed, String> {
    let (date_part, time, off) = split_time(raw)?;
    if opts.date_only {
        if time.is_some() {
            return Ok(Parsed::error("this field takes a date only (no time)"));
        }
        let value = parse_date(&date_part, today)?.map_or(Value::Null, Value::Date);
        let preview = preview(&value, "", false);
        return Ok(Parsed {
            value,
            preview,
            error: String::new(),
        });
    }
    let d = if !date_part.is_empty() {
        parse_date(&date_part, today)?
    } else if time.is_some() {
        Some(today)
    } else {
        None
    };
    let Some(d) = d else {
        return Ok(Parsed::error(
            "type a date and a time, e.g. 2026-11-26 19:00",
        ));
    };
    let Some(time) = time else {
        return Ok(Parsed::error(format!(
            "{}: add a time, e.g. {} 19:00",
            date_text(d),
            date_iso(d)
        )));
    };
    let naive = d.to_datetime(time);
    let value = match off {
        Some(off) => DateTimeValue::new(naive, off),
        // a daylight-saving gap or overlap is refused with the offsets to pick
        None => localize(naive, tz)?,
    };
    let value = Value::DateTime(value);
    let preview = preview(&value, opts.tz_source, off.is_some());
    Ok(Parsed {
        value,
        preview,
        error: String::new(),
    })
}

/// The input text for a stored value (round-trips through [`parse_when`]
/// to the same instant): seconds are kept when set, and the offset is
/// written out when it is not what the form zone `tz` gives that wall time
/// (or when no zone is known).
pub fn to_text(value: &Value, tz: Option<&Zone>) -> String {
    match value {
        Value::DateTime(v) => {
            let l = v.local;
            let mut text = format!(
                "{:04}-{:02}-{:02} {:02}:{:02}",
                l.year(),
                l.month(),
                l.day(),
                l.hour(),
                l.minute()
            );
            if l.second() != 0 {
                text.push_str(&format!(":{:02}", l.second()));
            }
            // an ambiguous or skipped wall time always needs its offset
            let same = tz.is_some_and(|tz| localize(l, tz).is_ok_and(|x| x.offset == v.offset));
            if !same {
                text.push(' ');
                text.push_str(&offset_text(v));
            }
            text
        }
        Value::Date(d) => date_iso(*d),
        Value::Null => String::new(),
        other => other.py_str(),
    }
}

/// `calendar.Calendar(firstweekday).monthdatescalendar(year, month)`: full
/// weeks (adjacent-month days included) covering the month.
pub fn month_weeks(year: i64, month: i64, firstweekday: i64) -> Result<Vec<Vec<Date>>> {
    if !(1..=12).contains(&month) {
        return Err(FormError::Value(format!(
            "bad month number {month}; must be 1-12"
        )));
    }
    let first = make_date(year, month, 1).map_err(FormError::Value)?;
    let fw = firstweekday.rem_euclid(7);
    let before = (weekday0(first) - fw).rem_euclid(7);
    let mut day = add_days(first, -before).map_err(FormError::Value)?;
    let mut weeks = Vec::new();
    loop {
        let mut week = Vec::with_capacity(7);
        for _ in 0..7 {
            week.push(day);
            day = add_days(day, 1).map_err(FormError::Value)?;
        }
        weeks.push(week);
        let past =
            day.year() as i64 > year || (day.year() as i64 == year && day.month() as i64 > month);
        if past {
            break;
        }
    }
    Ok(weeks)
}

/// `d` moved by `n` months, the day clamped to the target month's length.
pub fn add_months(d: Date, n: i64) -> Result<Date> {
    let m = d.month() as i64 - 1 + n;
    let (y, m) = (d.year() as i64 + m.div_euclid(12), m.rem_euclid(12) + 1);
    let last = make_date(y, m, 1)
        .map(|first| first.days_in_month() as i64)
        .map_err(FormError::Value)?;
    make_date(y, m, (d.day() as i64).min(last)).map_err(FormError::Value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tz::zone;

    fn bkk() -> Zone {
        zone("Asia/Bangkok").expect("zone")
    }

    fn today() -> Date {
        Date::new(2026, 11, 12).expect("date")
    }

    fn run(text: &str) -> (Option<String>, String, String) {
        let p = parse_when(
            text,
            &bkk(),
            WhenOptions {
                tz_source: "FSV_TZ",
                today: Some(today()),
                ..Default::default()
            },
        );
        let v = match &p.value {
            Value::DateTime(v) => Some(v.isoformat()),
            Value::Date(d) => Some(date_iso(*d)),
            _ => None,
        };
        (v, p.preview, p.error)
    }

    #[test]
    fn docstring_examples_match_python() {
        let cases = [
            (
                "2026-11-26 19:00",
                "2026-11-26T19:00:00+07:00",
                "Thu 26 Nov 2026 19:00 (+07:00 from FSV_TZ)",
            ),
            (
                "2026-11-26T19:00+07:00",
                "2026-11-26T19:00:00+07:00",
                "Thu 26 Nov 2026 19:00 (+07:00 as typed)",
            ),
            (
                "2026/11/26 7pm",
                "2026-11-26T19:00:00+07:00",
                "Thu 26 Nov 2026 19:00 (+07:00 from FSV_TZ)",
            ),
            (
                "26 Nov 2026 19:00",
                "2026-11-26T19:00:00+07:00",
                "Thu 26 Nov 2026 19:00 (+07:00 from FSV_TZ)",
            ),
            (
                "Nov 26 2026 7:30pm",
                "2026-11-26T19:30:00+07:00",
                "Thu 26 Nov 2026 19:30 (+07:00 from FSV_TZ)",
            ),
            (
                "Thu 26 Nov 2026 19:00",
                "2026-11-26T19:00:00+07:00",
                "Thu 26 Nov 2026 19:00 (+07:00 from FSV_TZ)",
            ),
            (
                "today 18:00",
                "2026-11-12T18:00:00+07:00",
                "Thu 12 Nov 2026 18:00 (+07:00 from FSV_TZ)",
            ),
            (
                "tomorrow 9am",
                "2026-11-13T09:00:00+07:00",
                "Fri 13 Nov 2026 09:00 (+07:00 from FSV_TZ)",
            ),
            (
                "+2d 10:00",
                "2026-11-14T10:00:00+07:00",
                "Sat 14 Nov 2026 10:00 (+07:00 from FSV_TZ)",
            ),
            (
                "+1w 09:00",
                "2026-11-19T09:00:00+07:00",
                "Thu 19 Nov 2026 09:00 (+07:00 from FSV_TZ)",
            ),
            (
                "fri 22:00",
                "2026-11-13T22:00:00+07:00",
                "Fri 13 Nov 2026 22:00 (+07:00 from FSV_TZ)",
            ),
            (
                "19:00",
                "2026-11-12T19:00:00+07:00",
                "Thu 12 Nov 2026 19:00 (+07:00 from FSV_TZ)",
            ),
            (
                "26 nov 10:00",
                "2026-11-26T10:00:00+07:00",
                "Thu 26 Nov 2026 10:00 (+07:00 from FSV_TZ)",
            ),
            (
                "jan 5 09:00",
                "2027-01-05T09:00:00+07:00",
                "Tue 5 Jan 2027 09:00 (+07:00 from FSV_TZ)",
            ),
            (
                "2026-11-26 19:00Z",
                "2026-11-26T19:00:00+00:00",
                "Thu 26 Nov 2026 19:00 (+00:00 as typed)",
            ),
            (
                "2026-11-26 19:00 -0530",
                "2026-11-26T19:00:00-05:30",
                "Thu 26 Nov 2026 19:00 (-05:30 as typed)",
            ),
            (
                "2026-11-26 19:00:30",
                "2026-11-26T19:00:30+07:00",
                "Thu 26 Nov 2026 19:00 (+07:00 from FSV_TZ)",
            ),
        ];
        for (text, iso, prev) in cases {
            assert_eq!(
                run(text),
                (Some(iso.to_string()), prev.to_string(), String::new()),
                "{text}"
            );
        }
    }

    #[test]
    fn errors_match_python() {
        let cases = [
            ("2026-02-30 10:00", "'2026-02-30 10:00' is not a real date"),
            ("13pm", "13pm: the hour must be 1-12 with am/pm"),
            ("25:00", "'25:00' is not a time of day"),
            (
                "2026-11-26",
                "Thu 26 Nov 2026: add a time, e.g. 2026-11-26 19:00",
            ),
            ("nonsense 10:00", "cannot read 'nonsense' as a date"),
            (
                "2026-11-26 19:00 +99:00",
                "offset must be a timedelta strictly between -timedelta(hours=24) and timedelta(hours=24), not \
                 datetime.timedelta(days=4, seconds=10800).",
            ),
        ];
        for (text, err) in cases {
            assert_eq!(run(text), (None, String::new(), err.to_string()), "{text}");
        }
        assert_eq!(run(""), (None, String::new(), String::new()));
    }

    #[test]
    fn date_only_and_dst() {
        let p = |t: &str| {
            parse_when(
                t,
                &bkk(),
                WhenOptions {
                    date_only: true,
                    today: Some(today()),
                    ..Default::default()
                },
            )
        };
        assert_eq!(p("2026-11-26").preview, "Thu 26 Nov 2026");
        assert_eq!(p("tomorrow").preview, "Fri 13 Nov 2026");
        assert_eq!(p("sat").preview, "Sat 14 Nov 2026");
        assert_eq!(
            p("2026-11-26 10:00").error,
            "this field takes a date only (no time)"
        );

        let ny = zone("America/New_York").expect("zone");
        let o = WhenOptions {
            tz_source: "Freshservice user time zone (Eastern)",
            today: Some(today()),
            ..Default::default()
        };
        assert_eq!(
            parse_when("2026-03-08 02:30", &ny, o.clone()).error,
            "2026-03-08 02:30 does not exist in America/New_York (the clocks skip ahead then); type another time, or \
             add an offset (-05:00 or -04:00)"
        );
        assert_eq!(
            parse_when("2026-11-01 01:30", &ny, o).error,
            "2026-11-01 01:30 happens twice in America/New_York (the clocks go back then); add the offset: -04:00 for \
             the first, -05:00 for the second"
        );
    }

    #[test]
    fn to_text_month_weeks_add_months() {
        let tz = bkk();
        let at = |h, s, off: i32| {
            Value::DateTime(DateTimeValue::new(
                Date::new(2026, 11, 26).expect("d").at(h, 0, s, 0),
                Offset::from_seconds(off).expect("off"),
            ))
        };
        assert_eq!(to_text(&at(19, 0, 7 * 3600), Some(&tz)), "2026-11-26 19:00");
        assert_eq!(
            to_text(&at(19, 5, 2 * 3600), Some(&tz)),
            "2026-11-26 19:00:05 +02:00"
        );
        assert_eq!(
            to_text(&at(19, 0, 7 * 3600), None),
            "2026-11-26 19:00 +07:00"
        );
        assert_eq!(
            to_text(&Value::Date(Date::new(2026, 1, 2).expect("d")), Some(&tz)),
            "2026-01-02"
        );

        let weeks = month_weeks(2026, 2, 0).expect("weeks");
        assert_eq!(weeks.len(), 5);
        assert_eq!(date_iso(weeks[0][0]), "2026-01-26");
        assert_eq!(date_iso(weeks[0][6]), "2026-02-01");
        assert_eq!(month_weeks(2026, 3, 6).expect("weeks").len(), 5);
        assert!(month_weeks(2026, 13, 0).is_err());

        let d = Date::new(2026, 1, 31).expect("d");
        assert_eq!(date_iso(add_months(d, 1).expect("m")), "2026-02-28");
        assert_eq!(date_iso(add_months(d, -2).expect("m")), "2025-11-30");
    }
}
