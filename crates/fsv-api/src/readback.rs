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
