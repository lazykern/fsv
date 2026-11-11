//! Python `datetime` behaviour the engine relies on: CPython 3.12's
//! `date.fromisoformat` / `datetime.fromisoformat` (the C implementation),
//! `isoformat()`, `str()` and `timedelta` repr.

use std::cmp::Ordering;
use std::fmt::Write;

use jiff::Timestamp;
use jiff::civil::{Date, DateTime, Time};
use jiff::tz::Offset;

/// An aware Python `datetime`: wall-clock time plus its UTC offset.
///
/// Values the engine produces carry the offset their zone gives at that
/// wall time (Python keeps the `ZoneInfo`, but only `utcoffset()` is ever
/// observed: `isoformat`, display, comparisons).
#[derive(Clone, Copy, Debug)]
pub struct DateTimeValue {
    pub local: DateTime,
    pub offset: Offset,
}

impl DateTimeValue {
    pub fn new(local: DateTime, offset: Offset) -> Self {
        Self { local, offset }
    }

    /// The instant (Python compares aware datetimes by instant).
    pub fn timestamp(&self) -> Timestamp {
        self.offset
            .to_timestamp(self.local)
            .unwrap_or(Timestamp::MIN)
    }

    pub fn date(&self) -> Date {
        self.local.date()
    }

    /// `isoformat()`: microseconds only when non-zero.
    pub fn isoformat(&self) -> String {
        self.format('T', true)
    }

    /// `isoformat(timespec="seconds")`.
    pub fn isoformat_seconds(&self) -> String {
        self.format('T', false)
    }

    /// `str(dt)`.
    pub fn py_str(&self) -> String {
        self.format(' ', true)
    }

    fn format(&self, sep: char, micros: bool) -> String {
        let mut s = format!(
            "{}{sep}{}",
            date_iso(self.local.date()),
            time_iso(self.local.time(), micros)
        );
        s.push_str(&offset_iso(self.offset));
        s
    }

    /// Port of fsv.forms.state._offset: `±HH:MM` (minutes floored).
    pub fn offset_text(&self) -> String {
        offset_text(self.offset)
    }
}

impl PartialEq for DateTimeValue {
    fn eq(&self, other: &Self) -> bool {
        self.timestamp() == other.timestamp()
    }
}

impl PartialOrd for DateTimeValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.timestamp().cmp(&other.timestamp()))
    }
}

/// Port of fsv.forms.tz._off: `±HH:MM` with minutes floored
/// (`int(off.total_seconds() // 60)`).
pub fn offset_text(off: Offset) -> String {
    let total = (off.seconds() as i64).div_euclid(60);
    let sign = if total >= 0 { '+' } else { '-' };
    let total = total.abs();
    format!("{sign}{:02}:{:02}", total / 60, total % 60)
}

/// Python's offset suffix in `isoformat()`: `+HH:MM[:SS]`.
fn offset_iso(off: Offset) -> String {
    let secs = off.seconds();
    let sign = if secs < 0 { '-' } else { '+' };
    let a = secs.unsigned_abs();
    let mut s = format!("{sign}{:02}:{:02}", a / 3600, (a % 3600) / 60);
    if !a.is_multiple_of(60) {
        let _ = write!(s, ":{:02}", a % 60);
    }
    s
}

/// `date.isoformat()`.
pub fn date_iso(d: Date) -> String {
    format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day())
}

fn time_iso(t: Time, micros: bool) -> String {
    let mut s = format!("{:02}:{:02}:{:02}", t.hour(), t.minute(), t.second());
    let us = t.subsec_nanosecond() / 1000;
    if micros && us != 0 {
        let _ = write!(s, ".{us:06}");
    }
    s
}

/// `datetime.date(y, m, d)` with Python's range (years 1-9999).
pub fn make_date(year: i64, month: i64, day: i64) -> Result<Date, String> {
    if !(1..=9999).contains(&year) {
        return Err(format!("year {year} is out of range"));
    }
    if !(1..=12).contains(&month) {
        return Err("month must be in 1..12".to_string());
    }
    Date::new(year as i16, month as i8, day as i8)
        .map_err(|_| "day is out of range for month".to_string())
}

fn byte(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

fn digits(s: &[u8], pos: usize, n: usize) -> Option<(i64, usize)> {
    let mut v = 0i64;
    for i in 0..n {
        let c = byte(s, pos + i);
        if !c.is_ascii_digit() {
            return None;
        }
        v = v * 10 + (c - b'0') as i64;
    }
    Some((v, pos + n))
}

fn find_separator(s: &[u8]) -> Option<usize> {
    let len = s.len();
    if len == 7 {
        return Some(7);
    }
    if byte(s, 4) == b'-' {
        if byte(s, 5) == b'W' {
            if len > 8 && byte(s, 8) == b'-' {
                if len == 9 {
                    return None;
                }
                if len > 10 && byte(s, 10).is_ascii_digit() {
                    return Some(8);
                }
                return Some(10);
            }
            return Some(8);
        }
        return Some(10);
    }
    if byte(s, 4) == b'W' {
        let mut idx = 7;
        while idx < len && byte(s, idx).is_ascii_digit() {
            idx += 1;
        }
        if idx < 9 {
            return Some(idx);
        }
        return Some(if idx % 2 == 0 { 7 } else { 8 });
    }
    Some(8)
}

fn parse_date_part(s: &[u8], len: usize) -> Option<Date> {
    let (year, mut p) = digits(s, 0, 4)?;
    let sep = byte(s, p) == b'-';
    if sep {
        p += 1;
    }
    if byte(s, p) == b'W' {
        p += 1;
        let (week, np) = digits(s, p, 2)?;
        p = np;
        let mut day = 1;
        if p < len {
            if sep {
                if byte(s, p) != b'-' {
                    return None;
                }
                p += 1;
            }
            day = digits(s, p, 1)?.0;
        }
        if !(1..=9999).contains(&year) {
            return None;
        }
        let wd = jiff::civil::Weekday::from_monday_one_offset(i8::try_from(day).ok()?).ok()?;
        let iso = jiff::civil::ISOWeekDate::new(year as i16, i8::try_from(week).ok()?, wd).ok()?;
        let d = iso.date();
        return (1..=9999).contains(&(d.year() as i64)).then_some(d);
    }
    let (month, np) = digits(s, p, 2)?;
    p = np;
    if sep {
        if byte(s, p) != b'-' {
            return None;
        }
        p += 1;
    }
    let (day, _) = digits(s, p, 2)?;
    make_date(year, month, day).ok()
}

/// `date.fromisoformat(s)` (CPython 3.12, C implementation).
pub fn date_fromisoformat(s: &str) -> Option<Date> {
    if !s.is_ascii() {
        return None;
    }
    let b = s.as_bytes();
    if !matches!(b.len(), 7 | 8 | 10) {
        return None;
    }
    parse_date_part(b, b.len())
}

/// Returns (h, m, s, us, rv) where rv is 0 at the end and 1 when chars remain.
fn parse_hh_mm_ss_ff(s: &[u8], start: usize, end: usize) -> Result<([i64; 4], bool), ()> {
    let mut vals = [0i64; 4];
    let mut p = start;
    let mut has_sep = true;
    for (i, slot) in vals.iter_mut().take(3).enumerate() {
        let (v, np) = digits(s, p, 2).ok_or(())?;
        *slot = v;
        p = np;
        let c = byte(s, p);
        p += 1;
        if i == 0 {
            has_sep = c == b':';
        }
        if p >= end {
            return Ok((vals, c != 0));
        } else if has_sep && c == b':' {
            continue;
        } else if c == b'.' || c == b',' {
            break;
        } else if !has_sep {
            p -= 1;
        } else {
            return Err(());
        }
    }
    let remains = end.saturating_sub(p);
    let n = remains.min(6);
    if n == 0 {
        return Err(());
    }
    let (us, np) = digits(s, p, n).ok_or(())?;
    const CORRECTION: [i64; 5] = [100000, 10000, 1000, 100, 10];
    vals[3] = if n < 6 { us * CORRECTION[n - 1] } else { us };
    p = np;
    while p < s.len() && byte(s, p).is_ascii_digit() {
        p += 1;
    }
    Ok((vals, p < s.len()))
}

/// `datetime.fromisoformat(s)` (CPython 3.12, C implementation): the
/// wall-clock time and its offset (`None` for a naive result).
///
/// Offsets with sub-second parts are refused (CPython accepts them; no
/// Freshservice value carries one).
pub fn datetime_fromisoformat(s: &str) -> Option<(DateTime, Option<Offset>)> {
    if !s.is_ascii() || s.len() < 7 {
        return None;
    }
    let b = s.as_bytes();
    let sep = find_separator(b)?;
    let date = parse_date_part(b, sep)?;
    let (mut hms, mut offset) = ([0i64; 4], None);
    if b.len() > sep {
        let start = sep + 1;
        let end = b.len();
        let mut tz = start;
        loop {
            let c = byte(b, tz);
            if c == b'Z' || c == b'+' || c == b'-' {
                break;
            }
            tz += 1;
            if tz >= end {
                break;
            }
        }
        let tz = tz.min(end);
        let (vals, rest) = parse_hh_mm_ss_ff(b, start, tz).ok()?;
        hms = vals;
        if tz == end {
            if rest {
                return None;
            }
        } else if byte(b, tz) == b'Z' {
            if tz + 1 != end {
                return None;
            }
            offset = Some(Offset::UTC);
        } else {
            let sign = if byte(b, tz) == b'-' { -1 } else { 1 };
            let (tv, rest) = parse_hh_mm_ss_ff(b, tz + 1, end).ok()?;
            if rest || tv[3] != 0 {
                return None;
            }
            let secs = sign * (tv[0] * 3600 + tv[1] * 60 + tv[2]);
            if secs.abs() >= 86400 {
                return None;
            }
            offset = Some(Offset::from_seconds(secs as i32).ok()?);
        }
    }
    if hms[0] > 23 || hms[1] > 59 || hms[2] > 59 {
        return None;
    }
    let time = Time::new(
        hms[0] as i8,
        hms[1] as i8,
        hms[2] as i8,
        (hms[3] * 1000) as i32,
    )
    .ok()?;
    Some((date.to_datetime(time), offset))
}

/// `repr(datetime.timedelta(seconds=total))`.
pub fn timedelta_repr(total_seconds: i64) -> String {
    let days = total_seconds.div_euclid(86400);
    let secs = total_seconds.rem_euclid(86400);
    let mut parts = Vec::new();
    if days != 0 {
        parts.push(format!("days={days}"));
    }
    if secs != 0 {
        parts.push(format!("seconds={secs}"));
    }
    if parts.is_empty() {
        return "datetime.timedelta(0)".to_string();
    }
    format!("datetime.timedelta({})", parts.join(", "))
}

const MONTH_ABBR: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `strftime("%d %b %Y")` in the C locale.
pub fn date_dmy(d: Date) -> String {
    format!(
        "{:02} {} {:04}",
        d.day(),
        MONTH_ABBR[(d.month() - 1) as usize],
        d.year()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_text_floors_minutes() {
        let off = |s| offset_text(Offset::from_seconds(s).unwrap());
        assert_eq!(off(-12600), "-03:30");
        assert_eq!(off(-30), "-00:01");
        assert_eq!(off(19800), "+05:30");
        assert_eq!(off(0), "+00:00");
        assert_eq!(off(45), "+00:00");
    }

    #[test]
    fn iso_dates() {
        assert_eq!(
            date_fromisoformat("2030-12-03").map(date_iso).as_deref(),
            Some("2030-12-03")
        );
        assert_eq!(
            date_fromisoformat("20301203").map(date_iso).as_deref(),
            Some("2030-12-03")
        );
        assert_eq!(
            date_fromisoformat("2030-W01-1").map(date_iso).as_deref(),
            Some("2029-12-31")
        );
        assert!(date_fromisoformat("03/12/2030").is_none());
        assert!(date_fromisoformat("2030-02-30").is_none());
    }

    #[test]
    fn iso_datetimes() {
        let (dt, off) = datetime_fromisoformat("2030-11-20T10:00+09:00").unwrap();
        let v = DateTimeValue::new(dt, off.unwrap());
        assert_eq!(v.isoformat(), "2030-11-20T10:00:00+09:00");
        let (_, off) = datetime_fromisoformat("2030-11-20T03:00:00Z").unwrap();
        assert_eq!(off, Some(Offset::UTC));
        assert!(
            datetime_fromisoformat("2030-11-20 10:00")
                .unwrap()
                .1
                .is_none()
        );
        assert!(datetime_fromisoformat("next tuesday").is_none());
    }

    #[test]
    fn timedelta() {
        assert_eq!(
            timedelta_repr(99 * 3600),
            "datetime.timedelta(days=4, seconds=10800)"
        );
        assert_eq!(
            timedelta_repr(-99 * 3600),
            "datetime.timedelta(days=-5, seconds=75600)"
        );
        assert_eq!(timedelta_repr(86400), "datetime.timedelta(days=1)");
    }
}
