//! Generic output helpers from `render.py`: HTML stripping, value/date
//! display, `--json` and CSV/TSV emitters.

use std::io::Write;
use std::sync::LazyLock;

use jiff::civil::DateTime;
use jiff::tz::{Offset, TimeZone};
use regex::Regex;
use serde_json::{Map, Value};

use crate::error::{Error, Result};
use crate::value::truthy;

/// Keys fsv adds to server objects for its own bookkeeping.
pub const INTERNAL_KEYS: &[&str] = &["_resource", "_search_result", "_agent", "_group"];

/// Drop [`INTERNAL_KEYS`] recursively.
pub fn strip_internal(obj: &Value) -> Value {
    match obj {
        Value::Object(m) => Value::Object(
            m.iter()
                .filter(|(k, _)| !INTERNAL_KEYS.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), strip_internal(v)))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(strip_internal).collect()),
        other => other.clone(),
    }
}

/// Port of fsv.render.strip_html: tags replaced by spaces, entities
/// unescaped, trimmed.
pub fn strip_html(s: Option<&str>) -> String {
    static TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").expect("regex"));
    let Some(s) = s.filter(|s| !s.is_empty()) else {
        return String::new();
    };
    crate::pystr::strip(&crate::pystr::html_unescape(&TAGS.replace_all(s, " "))).to_string()
}

/// Human display of a custom-field value (lists comma-separated, no Python reprs).
pub fn fmt_value(v: &Value) -> String {
    match v {
        Value::Array(items) => items.iter().map(fmt_value).collect::<Vec<_>>().join(", "),
        Value::Object(m) => match ["name", "value", "label"]
            .iter()
            .filter_map(|k| m.get(*k))
            .find(|x| truthy(x))
        {
            Some(x) => fsv_py::py_str(x),
            None => m
                .iter()
                .map(|(k, x)| format!("{k}={}", fmt_value(x)))
                .collect::<Vec<_>>()
                .join(", "),
        },
        Value::Bool(true) => "Yes".into(),
        Value::Bool(false) => "No".into(),
        Value::Null => "-".into(),
        other => fsv_py::py_str(other),
    }
}

/// `datetime.fromisoformat` subset used by Freshservice timestamps:
/// `(civil datetime, utc offset seconds)`.
pub fn parse_isoformat(text: &str) -> Option<(DateTime, Option<i32>)> {
    let b = text.as_bytes();
    let digits = |s: &str| -> Option<i32> {
        if !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit()) {
            s.parse().ok()
        } else {
            None
        }
    };
    let (year, month, day, date_len) = if b.len() >= 10 && b[4] == b'-' && b[7] == b'-' {
        (
            digits(text.get(0..4)?)?,
            digits(text.get(5..7)?)?,
            digits(text.get(8..10)?)?,
            10,
        )
    } else if b.len() >= 8 && b[..8].iter().all(u8::is_ascii_digit) {
        (
            digits(&text[0..4])?,
            digits(&text[4..6])?,
            digits(&text[6..8])?,
            8,
        )
    } else {
        return None;
    };
    let date = jiff::civil::Date::new(
        i16::try_from(year).ok()?,
        i8::try_from(month).ok()?,
        i8::try_from(day).ok()?,
    )
    .ok()?;
    let rest = &text[date_len..];
    if rest.is_empty() {
        return Some((date.to_datetime(jiff::civil::Time::midnight()), None));
    }
    let mut chars = rest.chars();
    chars.next()?;
    let time_part = chars.as_str();
    let tz_pos = time_part.find(['+', '-']);
    let (clock, tz) = match tz_pos {
        Some(p) => (&time_part[..p], Some(&time_part[p..])),
        None => (time_part, None),
    };
    let (main, frac) = match clock.find(['.', ',']) {
        Some(p) => (&clock[..p], Some(&clock[p + 1..])),
        None => (clock, None),
    };
    let parts: Vec<&str> = if main.contains(':') {
        main.split(':').collect()
    } else {
        main.as_bytes()
            .chunks(2)
            .filter_map(|c| std::str::from_utf8(c).ok())
            .collect()
    };
    if parts.is_empty() || parts.len() > 3 || parts.iter().any(|p| p.len() != 2) {
        return None;
    }
    let hour = digits(parts[0])?;
    let minute = parts.get(1).map_or(Some(0), |p| digits(p))?;
    let second = parts.get(2).map_or(Some(0), |p| digits(p))?;
    let mut nanos = 0;
    if let Some(f) = frac {
        if parts.len() != 3 || f.is_empty() || !f.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let micro: String = f.chars().take(6).collect();
        let micro = format!("{micro:0<6}");
        nanos = micro.parse::<i32>().ok()? * 1000;
    }
    let time = jiff::civil::Time::new(
        i8::try_from(hour).ok()?,
        i8::try_from(minute).ok()?,
        i8::try_from(second).ok()?,
        nanos,
    )
    .ok()?;
    let offset = match tz {
        None => None,
        Some(tz) => {
            let sign = if tz.starts_with('-') { -1 } else { 1 };
            let body = &tz[1..];
            let body = body.split(['.', ',']).next().unwrap_or("");
            let tparts: Vec<&str> = if body.contains(':') {
                body.split(':').collect()
            } else {
                body.as_bytes()
                    .chunks(2)
                    .filter_map(|c| std::str::from_utf8(c).ok())
                    .collect()
            };
            if tparts.is_empty() || tparts.len() > 3 || tparts.iter().any(|p| p.len() != 2) {
                return None;
            }
            let h = digits(tparts[0])?;
            let m = tparts.get(1).map_or(Some(0), |p| digits(p))?;
            let s = tparts.get(2).map_or(Some(0), |p| digits(p))?;
            if h > 23 || m > 59 || s > 59 {
                return None;
            }
            Some(sign * (h * 3600 + m * 60 + s))
        }
    };
    Some((date.to_datetime(time), offset))
}

/// ISO timestamp → local `YYYY-MM-DD HH:MM` (`YYYY-MM-DD` for dates);
/// `-` for null/empty; unparseable text is returned unchanged.
pub fn fmt_dt(value: &Value) -> String {
    fmt_dt_in(value, &TimeZone::system())
}

/// [`fmt_dt`] in an explicit time zone.
pub fn fmt_dt_in(value: &Value, tz: &TimeZone) -> String {
    if value.is_null() || value.as_str() == Some("") {
        return "-".into();
    }
    let text = fsv_py::py_str(value);
    let Some((dt, offset)) = parse_isoformat(&text.replace('Z', "+00:00")) else {
        return text;
    };
    let dt = match offset {
        Some(secs) => {
            let Ok(off) = Offset::from_seconds(secs) else {
                return text;
            };
            match off.to_timestamp(dt) {
                Ok(ts) => ts.to_zoned(tz.clone()).datetime(),
                Err(_) => return text,
            }
        }
        None => dt,
    };
    if text.contains('T') || text.contains(' ') {
        dt.strftime("%Y-%m-%d %H:%M").to_string()
    } else {
        dt.strftime("%Y-%m-%d").to_string()
    }
}

/// `--json` output: `json.dump(strip_internal(obj), indent=2, default=str)` + newline.
pub fn emit_json(obj: &Value) {
    let _ = crate::term::write_stdout(&json_text(obj));
}

/// The text [`emit_json`] prints.
pub fn json_text(obj: &Value) -> String {
    fsv_py::pretty(&strip_internal(obj)) + "\n"
}

fn csv_field(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(other) => fsv_py::py_str(other),
    }
}

fn csv_quote(field: &str, delimiter: char) -> String {
    if field.contains(delimiter)
        || field.contains('"')
        || field.contains('\r')
        || field.contains('\n')
    {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

fn csv_row(fields: &[String], delimiter: char) -> String {
    if fields.len() == 1 && fields[0].is_empty() {
        return "\"\"\r\n".into();
    }
    let joined: Vec<String> = fields.iter().map(|f| csv_quote(f, delimiter)).collect();
    format!("{}\r\n", joined.join(&delimiter.to_string()))
}

/// `csv.DictWriter(out, fieldnames=list(rows[0]), delimiter=...)` header + rows.
pub fn write_delimited(
    out: &mut dyn Write,
    rows: &[Map<String, Value>],
    delimiter: char,
) -> Result<()> {
    let Some(first) = rows.first() else {
        return Ok(());
    };
    let names: Vec<String> = first.keys().cloned().collect();
    out.write_all(csv_row(&names, delimiter).as_bytes())?;
    for row in rows {
        let extra: Vec<String> = row
            .keys()
            .filter(|k| !names.contains(k))
            .map(|k| fsv_py::py_repr_str(k))
            .collect();
        if !extra.is_empty() {
            return Err(Error::value(format!(
                "dict contains fields not in fieldnames: {}",
                extra.join(", ")
            )));
        }
        let fields: Vec<String> = names.iter().map(|n| csv_field(row.get(n))).collect();
        out.write_all(csv_row(&fields, delimiter).as_bytes())?;
    }
    Ok(())
}

/// [`write_delimited`] to stdout.
pub fn emit_delimited(rows: &[Map<String, Value>], delimiter: char) -> Result<()> {
    let mut buf = Vec::new();
    let written = write_delimited(&mut buf, rows, delimiter);
    crate::term::write_stdout(&String::from_utf8_lossy(&buf))?;
    written
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// `render.strip_html` outputs from `gen_fsv_views_extra.py`.
    #[test]
    fn strip_html_matches_python() {
        let rows: Value =
            serde_json::from_str(include_str!("../tests/fixtures/strip_html.json")).unwrap();
        for row in rows.as_array().unwrap() {
            assert_eq!(
                strip_html(row[0].as_str()),
                row[1].as_str().unwrap(),
                "{row}"
            );
        }
    }

    #[test]
    fn values_and_dates() {
        assert_eq!(fmt_value(&json!(["a", "b"])), "a, b");
        assert_eq!(fmt_value(&json!(true)), "Yes");
        let utc = TimeZone::UTC;
        assert_eq!(
            fmt_dt_in(&json!("2030-11-26T12:00:00Z"), &utc),
            "2030-11-26 12:00"
        );
        assert_eq!(fmt_dt_in(&Value::Null, &utc), "-");
        assert_eq!(fmt_dt_in(&json!("garbage"), &utc), "garbage");
        assert_eq!(fmt_dt_in(&json!("2030-11-26"), &utc), "2030-11-26");
    }

    #[test]
    fn csv_like_python() {
        let rows = vec![
            json!({"id": "CHN-1", "subject": "a, \"b\"", "n": null})
                .as_object()
                .cloned()
                .unwrap_or_default(),
            json!({"id": "CHN-2", "subject": "x\ny", "n": 1.5})
                .as_object()
                .cloned()
                .unwrap_or_default(),
        ];
        let mut buf = Vec::new();
        write_delimited(&mut buf, &rows, ',').unwrap();
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "id,subject,n\r\nCHN-1,\"a, \"\"b\"\"\",\r\nCHN-2,\"x\ny\",1.5\r\n"
        );
    }

    #[test]
    fn emit_json_prints_python_json_dumps_indent_2() {
        let ((), text) = crate::term::capture(|| {
            emit_json(
                &json!({"id": 1, "_raw": {"x": 1}, "name": "é", "l": [1, {"_k": 2, "v": null}]}),
            )
        });
        assert_eq!(
            text,
            "{\n  \"id\": 1,\n  \"_raw\": {\n    \"x\": 1\n  },\n  \"name\": \"\\u00e9\",\n  \"l\": [\n    1,\n    {\n      \"_k\": 2,\n      \"v\": null\n    }\n  ]\n}\n"
        );
    }

    #[test]
    fn emit_delimited_writes_csv_dialect_rows() {
        let rows: Vec<Map<String, Value>> =
            [json!({"a": 1, "b": "x\ty"}), json!({"a": null, "b": "q"})]
                .iter()
                .map(|v| v.as_object().cloned().unwrap())
                .collect();
        let (r, text) =
            crate::term::capture(|| (emit_delimited(&rows, '\t'), emit_delimited(&[], ',')));
        assert!(r.0.is_ok() && r.1.is_ok());
        assert_eq!(text, "a\tb\r\n1\t\"x\ty\"\r\n\tq\r\n");
    }
}
