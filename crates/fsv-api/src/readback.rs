//! Strict semantic comparison of server-normalized readback values
//! (`readback.py`).

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use fsv_core::error::{Error, Result};
use fsv_forms::pydt::datetime_fromisoformat;

/// `readback.REMINDER_NOT_EXPOSED`.
pub const REMINDER_NOT_EXPOSED: &str = "unavailable (not exposed by the server)";
/// `readback.REMINDER_NOTE`.
pub const REMINDER_NOTE: &str = "The task reminder (notify_before) is not returned by the tenant's task reads; it was sent exactly as reviewed but cannot be read back. Every readable field matched.";

/// Port of fsv.readback.exempt_unexposed_reminder: only an absent
/// `notify_before` on an exactly identified task row is marked not exposed.
pub fn exempt_unexposed_reminder(
    fields: &mut serde_json::Map<String, Value>,
    row_present: bool,
) -> bool {
    if row_present && fields.get("notify_before").and_then(Value::as_str) == Some("unavailable") {
        fields.insert("notify_before".into(), Value::from(REMINDER_NOT_EXPOSED));
        return true;
    }
    false
}

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

/// `READBACK_DELAYS`: seconds slept before each of the four observations.
pub const READBACK_DELAYS: [f64; 4] = [0.0, 0.5, 1.5, 3.0];

type SleepHook = Option<Box<dyn Fn(f64)>>;

thread_local! {
    static READBACK_SLEEP: std::cell::RefCell<SleepHook> =
        const { std::cell::RefCell::new(None) };
}

/// Replaces the bounded-readback sleep on this thread while `body` runs
/// (Python tests monkeypatch `readback._sleep`).
pub fn with_readback_sleep<R>(sleep: impl Fn(f64) + 'static, body: impl FnOnce() -> R) -> R {
    struct Reset(SleepHook);
    impl Drop for Reset {
        fn drop(&mut self) {
            let prior = self.0.take();
            READBACK_SLEEP.with(|s| *s.borrow_mut() = prior);
        }
    }
    let prior = READBACK_SLEEP.with(|s| s.borrow_mut().replace(Box::new(sleep)));
    let _reset = Reset(prior);
    body()
}

fn readback_sleep(seconds: f64) {
    let hooked = READBACK_SLEEP.with(|s| s.borrow().as_ref().map(|f| f(seconds)).is_some());
    if !hooked && !cfg!(test) {
        std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
    }
}

/// Port of fsv.readback.bounded_readback: re-runs only the read-only
/// verifier, at most four observations over five seconds, and keeps the
/// final observation. The caller's journal still forbids any resend.
pub fn bounded_readback(
    verify: impl Fn() -> Result<serde_json::Map<String, Value>>,
) -> serde_json::Map<String, Value> {
    let mut result = serde_json::Map::new();
    result.insert("verified".into(), Value::Bool(false));
    for (attempt, delay) in READBACK_DELAYS.iter().enumerate() {
        if *delay != 0.0 {
            readback_sleep(*delay);
        }
        result = verify().unwrap_or_else(|_| {
            let mut m = serde_json::Map::new();
            m.insert("verified".into(), Value::Bool(false));
            m.insert(
                "warning".into(),
                Value::from("Independent readback unavailable; do not resend."),
            );
            m
        });
        result.insert("readback_attempts".into(), Value::from(attempt + 1));
        if result.get("verified") == Some(&Value::Bool(true)) {
            break;
        }
    }
    result
}

/// Port of fsv.readback.duration_seconds: `HH:MM` or `HH:MM:SS` with
/// unbounded hours; never wraps at 24h.
pub fn duration_seconds(value: &Value) -> Result<i64> {
    // Python `\d` is a Unicode decimal digit. `[0-5]` stays ASCII.
    static RE: LazyLock<Regex> =
        // `\z`, not `$`: Python `fullmatch` rejects a trailing newline.
        LazyLock::new(|| Regex::new(r"^(\d{2,}):([0-5]\d)(?::([0-5]\d))?\z").expect("regex"));
    let bad = || Error::value("duration must be HH:MM or HH:MM:SS");
    let caps = value
        .as_str()
        .and_then(|s| RE.captures(s))
        .ok_or_else(bad)?;
    let num = |i: usize| -> Result<i64> {
        caps.get(i).map_or(Ok(0), |m| {
            fsv_core::pystr::parse_decimal(m.as_str()).ok_or_else(bad)
        })
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

/// Port of fsv.readback.resolution_text: internal details wrap `notes_html`;
/// v2 ticket details return plain text; `resolution_notes_html` is the alias.
pub fn resolution_text(record: &Value) -> Option<String> {
    let mut value = record.get("resolution_notes");
    if let Some(Value::Object(m)) = value {
        value = m.get("notes_html");
    }
    if let Some(Value::String(s)) = value {
        return Some(s.clone());
    }
    match record.get("resolution_notes_html") {
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
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

/// Port of fsv.readback.custom_fields_match: every submitted custom key,
/// unwrapping a native lookup `{id, value}` to its id for a scalar
/// submission. A lookup's display value never stands in for its identity.
pub fn custom_fields_match(actual: &Value, expected: &Value) -> bool {
    let (Some(actual), Some(expected)) = (actual.as_object(), expected.as_object()) else {
        return false;
    };
    expected.iter().all(|(key, value)| {
        let Some(mut observed) = actual.get(key) else {
            return false;
        };
        if let Value::Object(wrapper) = observed
            && wrapper.len() == 2
            && wrapper.contains_key("id")
            && wrapper.contains_key("value")
            && !matches!(
                value,
                Value::Object(_) | Value::Array(_) | Value::Bool(_) | Value::Null
            )
        {
            observed = &wrapper["id"];
        }
        submitted_value_matches(observed, value)
    })
}

/// Port of fsv.readback.tag_names: tag names from a readback list of strings
/// or native `{id, name}` objects; `None` for anything else.
pub fn tag_names(value: &Value) -> Option<Vec<String>> {
    let mut names = Vec::new();
    for item in value.as_array()? {
        let item = match item {
            Value::Object(m) => m.get("name").unwrap_or(&Value::Null),
            other => other,
        };
        let text = item.as_str().map(fsv_core::pystr::strip)?;
        if text.is_empty() {
            return None;
        }
        names.push(text.to_string());
    }
    Some(names)
}

/// Port of fsv.readback.same_tags: tag lists compared by name,
/// case-insensitively and order-independently.
pub fn same_tags(actual: &Value, expected: &Value) -> bool {
    let (Some(got), Some(want)) = (tag_names(actual), tag_names(expected)) else {
        return false;
    };
    let fold = |v: &[String]| {
        v.iter()
            .map(|t| fsv_core::pystr::casefold(t))
            .collect::<std::collections::BTreeSet<String>>()
    };
    let got_set = fold(&got);
    got_set == fold(&want) && got_set.len() == got.len()
}

#[cfg(test)]
mod tests {
    /// `test_ticket_properties.py::test_same_tags_helper_edges` (7f1e537).
    #[test]
    fn same_tags_helper_edges() {
        use super::{same_tags, tag_names};
        use serde_json::json;
        assert_eq!(
            tag_names(&json!([{"id": 1, "name": " x "}, "y"])),
            Some(vec!["x".to_string(), "y".to_string()])
        );
        assert_eq!(tag_names(&json!("x")), None);
        assert_eq!(tag_names(&json!([1])), None);
        assert!(!same_tags(&json!([]), &json!(["x"])));
        assert!(same_tags(&json!([]), &json!([])));
        assert!(!same_tags(&json!(["a", "A"]), &json!(["a"])));
    }

    use super::*;

    #[test]
    fn bounded_readback_keeps_last_failed_observation() {
        let calls = std::cell::Cell::new(0);
        let sleeps = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let seen = std::rc::Rc::clone(&sleeps);
        let got = with_readback_sleep(
            move |s| seen.borrow_mut().push(s),
            || {
                bounded_readback(|| {
                    calls.set(calls.get() + 1);
                    Err(Error::other("read failed"))
                })
            },
        );
        assert_eq!(calls.get(), 4);
        assert_eq!(*sleeps.borrow(), vec![0.5, 1.5, 3.0]);
        assert_eq!(
            Value::Object(got),
            serde_json::json!({"verified": false,
                "warning": "Independent readback unavailable; do not resend.",
                "readback_attempts": 4})
        );
    }

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
            duration_seconds(&json!("१२:05")).unwrap(),
            12 * 3600 + 5 * 60
        );
        assert_eq!(
            duration_seconds(&json!("10:0३:09")).unwrap(),
            10 * 3600 + 3 * 60 + 9
        );
        assert!(same_duration(&json!("१२:05"), &json!("12:05")));
        assert!(duration_seconds(&json!("१२:३०")).is_err());
        assert_eq!(
            duration_seconds(&json!("1:00")).unwrap_err().to_string(),
            "duration must be HH:MM or HH:MM:SS"
        );
        assert_eq!(
            duration_seconds(&json!("00:05\n")).unwrap_err().to_string(),
            "duration must be HH:MM or HH:MM:SS"
        );
        assert!(!same_duration(&json!("00:05\n"), &json!("00:05\n")));
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
