//! CPython string semantics the engine depends on: `str.split()` /
//! `str.strip()` whitespace, `str.casefold()`, Unicode decimal digits,
//! `html.unescape` / `html.escape`, `os.path` helpers and JSON truthiness.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value as Json;

use crate::tables::{casefold::CASEFOLD, digits::DIGIT_ZEROS};
use fsv_py::html_entities as html5;

/// `str.isspace()` for one character.
pub fn is_py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// `s.split()` (no argument).
pub fn split_ws(s: &str) -> impl Iterator<Item = &str> {
    s.split(is_py_space).filter(|p| !p.is_empty())
}

/// `" ".join(s.split())`.
pub fn squash_ws(s: &str) -> String {
    split_ws(s).collect::<Vec<_>>().join(" ")
}

/// `s.strip()` (no argument).
pub fn strip(s: &str) -> &str {
    s.trim_matches(is_py_space)
}

/// `str.casefold()`.
pub fn casefold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match CASEFOLD.binary_search_by(|(k, _)| k.cmp(&c)) {
            Ok(i) => out.push_str(CASEFOLD[i].1),
            Err(_) => out.push(c),
        }
    }
    out
}

/// Value of a Unicode decimal digit (`unicodedata.decimal`), as `int()` and
/// regex `\d` accept them.
pub fn decimal_value(c: char) -> Option<u32> {
    let cp = c as u32;
    let i = DIGIT_ZEROS.partition_point(|z| *z <= cp);
    if i == 0 {
        return None;
    }
    let zero = DIGIT_ZEROS[i - 1];
    (cp - zero < 10).then_some(cp - zero)
}

/// `int(s)` for a string of Unicode decimal digits with an optional sign
/// (no surrounding whitespace, no underscores). `None` when not digits or
/// out of `i64` range.
pub fn parse_int(s: &str) -> Option<i64> {
    let (neg, digits) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    if digits.is_empty() {
        return None;
    }
    let mut n: i64 = 0;
    for c in digits.chars() {
        let d = decimal_value(c)? as i64;
        n = n.checked_mul(10)?.checked_add(d)?;
    }
    Some(if neg { -n } else { n })
}

/// `s.isdigit()` restricted to ASCII and decimal digits (non-decimal
/// digits such as superscripts are not accepted by `int()` anyway).
pub fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| decimal_value(c).is_some())
}

/// Python truthiness of a JSON value (`bool(x)`).
pub fn truthy(v: &Json) -> bool {
    match v {
        Json::Null => false,
        Json::Bool(b) => *b,
        Json::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Json::String(s) => !s.is_empty(),
        Json::Array(a) => !a.is_empty(),
        Json::Object(o) => !o.is_empty(),
    }
}

/// `str(x)` of a JSON value.
pub fn jstr(v: &Json) -> String {
    fsv_py::py_str(v)
}

/// `repr(x)` of a JSON value.
pub fn jrepr(v: &Json) -> String {
    fsv_py::py_repr(v)
}

/// `d.get(key)` treating a missing key and JSON null alike.
pub fn get<'a>(obj: &'a Json, key: &str) -> Option<&'a Json> {
    obj.as_object()
        .and_then(|m| m.get(key))
        .filter(|v| !v.is_null())
}

/// `d.get(key) or default` truthiness: the value when truthy.
pub fn get_truthy<'a>(obj: &'a Json, key: &str) -> Option<&'a Json> {
    get(obj, key).filter(|v| truthy(v))
}

/// `str(d.get(key) or "")`.
pub fn get_str(obj: &Json, key: &str) -> String {
    get_truthy(obj, key).map(jstr).unwrap_or_default()
}

/// Whether the object has `key` (even when its value is null).
pub fn has(obj: &Json, key: &str) -> bool {
    obj.as_object().is_some_and(|m| m.contains_key(key))
}

static CHARREF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"&(#[0-9]+;?|#[xX][0-9a-fA-F]+;?|[^\t\n\x0c <&#;]{1,32};?)").expect("charref regex")
});

fn html5_lookup(name: &str) -> Option<&'static str> {
    html5::HTML5
        .binary_search_by(|(k, _)| (*k).cmp(name))
        .ok()
        .map(|i| html5::HTML5[i].1)
}

fn replace_charref(s: &str) -> String {
    if let Some(num) = s.strip_prefix('#') {
        let parsed = if let Some(hex) = num.strip_prefix(['x', 'X']) {
            u128::from_str_radix(hex.trim_end_matches(';'), 16).unwrap_or(u128::MAX)
        } else {
            num.trim_end_matches(';')
                .parse::<u128>()
                .unwrap_or(u128::MAX)
        };
        if let Ok(small) = u32::try_from(parsed)
            && let Ok(i) = html5::INVALID_CHARREFS.binary_search_by(|(k, _)| k.cmp(&small))
        {
            return html5::INVALID_CHARREFS[i].1.to_string();
        }
        if (0xD800..=0xDFFF).contains(&parsed) || parsed > 0x10FFFF {
            return "\u{FFFD}".to_string();
        }
        let cp = parsed as u32;
        if html5::INVALID_CODEPOINTS.binary_search(&cp).is_ok() {
            return String::new();
        }
        return char::from_u32(cp)
            .map(String::from)
            .unwrap_or_else(|| "\u{FFFD}".to_string());
    }
    if let Some(v) = html5_lookup(s) {
        return v.to_string();
    }
    let idx: Vec<usize> = s.char_indices().map(|(i, _)| i).collect();
    let n = idx.len();
    let mut x = n.saturating_sub(1);
    while x > 1 {
        let cut = idx[x];
        if let Some(v) = html5_lookup(&s[..cut]) {
            return format!("{v}{}", &s[cut..]);
        }
        x -= 1;
    }
    format!("&{s}")
}

/// `html.unescape`.
pub fn html_unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    CHARREF
        .replace_all(s, |c: &regex::Captures<'_>| replace_charref(&c[1]))
        .into_owned()
}

/// `html.escape(s, quote=True)`.
pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// `os.path.basename` (POSIX).
pub fn basename(p: &str) -> &str {
    match p.rfind('/') {
        Some(i) => &p[i + 1..],
        None => p,
    }
}

/// `os.path.splitext(p)[1]` (POSIX).
pub fn splitext_ext(p: &str) -> &str {
    let sep = p.rfind('/').map(|i| i as isize).unwrap_or(-1);
    let Some(dot) = p.rfind('.') else { return "" };
    if (dot as isize) <= sep {
        return "";
    }
    let start = (sep + 1) as usize;
    if p[start..dot].chars().any(|c| c != '.') {
        return &p[dot..];
    }
    ""
}

/// First `n` characters (Python `s[:n]`).
pub fn take_chars(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// `repr(list_of_str)`.
pub fn repr_str_list<S: AsRef<str>>(items: &[S]) -> String {
    let parts: Vec<String> = items
        .iter()
        .map(|s| fsv_py::py_repr_str(s.as_ref()))
        .collect();
    format!("[{}]", parts.join(", "))
}

/// `repr(tuple_of_str)`.
pub fn repr_str_tuple<S: AsRef<str>>(items: &[S]) -> String {
    let parts: Vec<String> = items
        .iter()
        .map(|s| fsv_py::py_repr_str(s.as_ref()))
        .collect();
    if parts.len() == 1 {
        format!("({},)", parts[0])
    } else {
        format!("({})", parts.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn casefold_specials() {
        assert_eq!(casefold("Straße ÄÖ"), "strasse äö");
        assert_eq!(casefold("ΣΑΣ"), "σασ");
    }

    #[test]
    fn unescape_named_numeric_and_prefix() {
        assert_eq!(html_unescape("&lt;p&gt;a&amp;b&#39;&#x27;"), "<p>a&b''");
        assert_eq!(
            html_unescape("&ampx &notit; &#0; &#x110000;"),
            "&x ¬it; \u{FFFD} \u{FFFD}"
        );
        assert_eq!(html_unescape("&bogus;"), "&bogus;");
    }

    #[test]
    fn digits_and_paths() {
        assert_eq!(parse_int("١٢"), Some(12));
        assert_eq!(parse_int("+7"), Some(7));
        assert_eq!(splitext_ext("/tmp/deck.PPTX"), ".PPTX");
        assert_eq!(splitext_ext(".bashrc"), "");
        assert_eq!(splitext_ext("a/..b.c"), ".c");
        assert_eq!(basename("/tmp/deck.pptx"), "deck.pptx");
    }
}
