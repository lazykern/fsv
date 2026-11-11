//! Strict semantic comparison of server-normalized readback values
//! (`readback.py`).

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use fsv_core::error::{Error, Result};
use fsv_forms::pydt::datetime_fromisoformat;

/// Port of fsv.readback.same_instant: both values are text with an explicit
/// offset and name the same UTC instant. A naive or date-only value never
/// matches; no local timezone is assumed.
pub fn same_instant(actual: &Value, expected: &Value) -> bool {
    fn parse(value: &Value) -> Option<jiff::Timestamp> {
        let text = value.as_str()?.replace('Z', "+00:00");
        let (dt, off) = datetime_fromisoformat(&text)?;
        off?.to_timestamp(dt).ok()
    }
    match (parse(actual), parse(expected)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// Port of fsv.readback.duration_seconds: `HH:MM` or `HH:MM:SS` with
/// unbounded hours; never wraps at 24h.
pub fn duration_seconds(value: &Value) -> Result<i64> {
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^([0-9]{2,}):([0-5][0-9])(?::([0-5][0-9]))?$").expect("regex")
    });
    let bad = || Error::value("duration must be HH:MM or HH:MM:SS");
    let caps = value
        .as_str()
        .and_then(|s| RE.captures(s))
        .ok_or_else(bad)?;
    let num = |i: usize| -> Result<i64> {
        caps.get(i)
            .map_or(Ok(0), |m| m.as_str().parse::<i64>().map_err(|_| bad()))
    };
    let rest = num(2)? * 60 + num(3)?;
    num(1)?
        .checked_mul(3600)
        .and_then(|h| h.checked_add(rest))
        .ok_or_else(bad)
}

/// Port of fsv.readback.same_duration.
pub fn same_duration(actual: &Value, expected: &Value) -> bool {
    match (duration_seconds(actual), duration_seconds(expected)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Python `==` on JSON values: `1 == 1.0 == True`, containers compared
/// element-wise, dicts regardless of key order.
pub fn py_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| py_equal(p, q))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| py_equal(v, w)))
        }
        _ => crate::schema::py_eq(a, b),
    }
}

/// Port of fsv.readback.same_text: both are text and equal after rich-text
/// wrappers, entities and whitespace are flattened (`views.one_line`).
pub fn same_text(actual: &Value, expected: &Value) -> bool {
    actual.is_string()
        && expected.is_string()
        && crate::site::one_line(actual) == crate::site::one_line(expected)
}

#[derive(PartialEq)]
enum PyType {
    None,
    Bool,
    Int,
    Float,
    Str,
    List,
    Dict,
}

fn py_type(v: &Value) -> PyType {
    match v {
        Value::Null => PyType::None,
        Value::Bool(_) => PyType::Bool,
        Value::Number(n) if n.is_f64() => PyType::Float,
        Value::Number(_) => PyType::Int,
        Value::String(_) => PyType::Str,
        Value::Array(_) => PyType::List,
        Value::Object(_) => PyType::Dict,
    }
}

/// `type(actual) is type(expected) and actual == expected`.
pub fn strict_equal(actual: &Value, expected: &Value) -> bool {
    py_type(actual) == py_type(expected) && py_equal(actual, expected)
}

/// `Decimal(text)` as `(negative, significant digits, exponent)` with zero
/// canonical; `None` for text Decimal rejects or a non-finite value.
fn decimal(text: &str) -> Option<(bool, String, i64)> {
    // `Decimal` strips the text and drops every underscore before parsing.
    let t = fsv_core::pystr::strip(text).replace('_', "");
    let (neg, t) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, &t[..]),
    };
    let (mantissa, exp) = match t.find(['e', 'E']) {
        Some(i) => (&t[..i], Some(&t[i + 1..])),
        None => (t, None),
    };
    let digits_ok = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let (int, frac) = match mantissa.split_once('.') {
        Some((i, f)) => (i, f),
        None => (mantissa, ""),
    };
    if (int.is_empty() && frac.is_empty())
        || (!int.is_empty() && !digits_ok(int))
        || (!frac.is_empty() && !digits_ok(frac))
    {
        return None;
    }
    let mut e: i64 = match exp {
        Some(x) => {
            let (eneg, x) = match x.as_bytes().first() {
                Some(b'-') => (true, &x[1..]),
                Some(b'+') => (false, &x[1..]),
                _ => (false, x),
            };
            if !digits_ok(x) {
                return None;
            }
            let v: i64 = x.parse().ok()?;
            if eneg { -v } else { v }
        }
        None => 0,
    };
    let mut digits = format!("{int}{frac}");
    e -= frac.len() as i64;
    let lead = digits.len() - digits.trim_start_matches('0').len();
    digits.drain(..lead);
    if digits.is_empty() {
        return Some((false, String::new(), 0));
    }
    let trimmed = digits.trim_end_matches('0').len();
    e += (digits.len() - trimmed) as i64;
    digits.truncate(trimmed);
    Some((neg, digits, e))
}

/// Port of fsv.readback.submitted_value_matches: only the submitted keys,
/// allowing canonical scalar wire normalization. Missing keys, nulls,
/// booleans and ordered lists stay distinct; numeric-looking text is never
/// turned into an id.
pub fn submitted_value_matches(actual: &Value, expected: &Value) -> bool {
    match expected {
        Value::Object(e) => {
            return actual.as_object().is_some_and(|a| {
                e.iter()
                    .all(|(k, v)| a.get(k).is_some_and(|x| submitted_value_matches(x, v)))
            });
        }
        Value::Array(e) => {
            return actual.as_array().is_some_and(|a| {
                a.len() == e.len() && a.iter().zip(e).all(|(x, v)| submitted_value_matches(x, v))
            });
        }
        _ => {}
    }
    let (ta, te) = (py_type(actual), py_type(expected));
    if ta == te {
        return actual == expected;
    }
    if te == PyType::Bool {
        return actual.as_str().is_some_and(|s| {
            fsv_core::pystr::casefold(s) == fsv_py::py_str(expected).to_lowercase()
        });
    }
    if ta == PyType::Bool || ta == PyType::None || te == PyType::None {
        return false;
    }
    if matches!(te, PyType::Int | PyType::Float) && ta == PyType::Str {
        return match (
            decimal(actual.as_str().unwrap_or_default()),
            decimal(&fsv_py::py_str(expected)),
        ) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        };
    }
    if te == PyType::Str && matches!(ta, PyType::Int | PyType::Float) {
        return expected.as_str() == Some(fsv_py::py_str(actual).as_str());
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// `tests/test_readback_normalization.py::test_duration_semantics`.
    #[test]
    fn duration_semantics() {
        let cases = [
            (json!("00:05:00"), "00:05", true),
            (json!("125:05:00"), "125:05", true),
            (json!("00:05:01"), "00:05", false),
            (json!("00:06:00"), "00:05", false),
            (json!("00:65:00"), "01:05", false),
            (json!(null), "00:05", false),
            (json!(300), "00:05", false),
            (json!("00:05:00.0"), "00:05", false),
        ];
        for (actual, expected, matched) in cases {
            assert_eq!(
                same_duration(&actual, &json!(expected)),
                matched,
                "{actual}"
            );
        }
        assert_eq!(duration_seconds(&json!("125:05")).unwrap(), 450300);
        assert_eq!(
            duration_seconds(&json!("1:00")).unwrap_err().to_string(),
            "duration must be HH:MM or HH:MM:SS"
        );
    }

    /// `tests/test_readback_normalization.py::test_timestamp_semantics`.
    #[test]
    fn timestamp_semantics() {
        let expected = json!("2030-01-02T03:00:00+07:00");
        let cases = [
            (json!("2030-01-01T20:00:00Z"), true),
            (json!("2030-01-02T03:00:00+07:00"), true),
            (json!("2030-01-01T15:00:00-05:00"), true),
            (json!("2030-01-01T20:00:00.000+00:00"), true),
            (json!("2030-01-01T20:00:01Z"), false),
            (json!("2030-01-02T03:00:00"), false),
            (json!("2030-01-02"), false),
            (json!(null), false),
            (json!(false), false),
            (json!("invalid"), false),
        ];
        for (actual, matched) in cases {
            assert_eq!(same_instant(&actual, &expected), matched, "{actual}");
        }
        assert!(!same_instant(
            &json!("2030-01-02T03:00:00"),
            &json!("2030-01-02T03:00:00")
        ));
    }
}
