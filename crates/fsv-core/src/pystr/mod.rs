//! Python `str`/`html`/`urllib.parse` semantics that fsv output depends on.
//!
//! Python's notion of whitespace, digits, HTML entity decoding and URL
//! quoting differ in small ways from Rust's std; messages, previews and
//! hashes must match the Python build, so these helpers reproduce CPython.

use std::borrow::Cow;

/// `str.isspace()` for one character (Rust's `is_whitespace` misses U+001C..U+001F).
pub fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// `s.split()` (no argument).
pub fn split_ws(s: &str) -> Vec<&str> {
    s.split(is_space).filter(|p| !p.is_empty()).collect()
}

/// `" ".join(s.split())`.
pub fn collapse_ws(s: &str) -> String {
    split_ws(s).join(" ")
}

/// `s.strip()`.
pub fn strip(s: &str) -> &str {
    s.trim_matches(is_space)
}

/// `s.rstrip()`.
pub fn rstrip(s: &str) -> &str {
    s.trim_end_matches(is_space)
}

/// `s.lstrip()`.
pub fn lstrip(s: &str) -> &str {
    s.trim_start_matches(is_space)
}

/// `str.isdigit()` for one character (decimal digits plus super/subscript digits).
pub fn is_digit_char(c: char) -> bool {
    if c.is_ascii_digit() {
        return true;
    }
    if matches!(c, '\u{b2}' | '\u{b3}' | '\u{b9}' | '\u{2070}' | '\u{2074}'..='\u{2079}' | '\u{2080}'..='\u{2089}')
    {
        return true;
    }
    !c.is_ascii() && decimal_value(c).is_some()
}

/// `s.isdigit()`.
pub fn is_digit(s: &str) -> bool {
    !s.is_empty() && s.chars().all(is_digit_char)
}

/// Value of a Unicode decimal digit (`\d` / category Nd), as Python's `int()` accepts.
pub fn decimal_value(c: char) -> Option<u32> {
    if let Some(d) = c.to_digit(10) {
        return Some(d);
    }
    // Every Nd block is a contiguous run of ten code points starting at a
    // code point whose value is 0; these are the zero digits.
    const ZEROS: &[u32] = &[
        0x660, 0x6F0, 0x7C0, 0x966, 0x9E6, 0xA66, 0xAE6, 0xB66, 0xBE6, 0xC66, 0xCE6, 0xD66, 0xDE6,
        0xE50, 0xED0, 0xF20, 0x1040, 0x1090, 0x17E0, 0x1810, 0x1946, 0x19D0, 0x1A80, 0x1A90,
        0x1B50, 0x1BB0, 0x1C40, 0x1C50, 0xA620, 0xA8D0, 0xA900, 0xA9D0, 0xA9F0, 0xAA50, 0xABF0,
        0xFF10, 0x104A0, 0x10D30, 0x11066, 0x110F0, 0x11136, 0x111D0, 0x112F0, 0x11450, 0x114D0,
        0x11650, 0x116C0, 0x11730, 0x118E0, 0x11950, 0x11C50, 0x11D50, 0x11DA0, 0x11F50, 0x16A60,
        0x16AC0, 0x16B50, 0x1D7CE, 0x1D7D8, 0x1D7E2, 0x1D7EC, 0x1D7F6, 0x1E140, 0x1E2F0, 0x1E4F0,
        0x1E950, 0x1FBF0,
    ];
    let cp = c as u32;
    ZEROS
        .iter()
        .find(|&&z| cp >= z && cp < z + 10)
        .map(|z| cp - z)
}

/// `int(s)` for a string of `\d` characters (no sign); `None` on overflow.
pub fn parse_decimal(s: &str) -> Option<i64> {
    let mut n: i64 = 0;
    for c in s.chars() {
        let d = decimal_value(c)?;
        n = n.checked_mul(10)?.checked_add(i64::from(d))?;
    }
    Some(n)
}

/// `str.casefold()`.
pub fn casefold(s: &str) -> String {
    caseless::default_case_fold_str(s)
}

/// Python's `round(x)` (ties to even).
pub fn round_half_even(x: f64) -> f64 {
    let r = x.round();
    if (x - x.trunc()).abs() == 0.5 {
        2.0 * (x / 2.0).round()
    } else {
        r
    }
}

/// `format(x, "g")`.
pub fn format_g(x: f64) -> String {
    if x.is_nan() {
        return "nan".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if x == 0.0 {
        return if x.is_sign_negative() {
            "-0".into()
        } else {
            "0".into()
        };
    }
    let sci = format!("{:.5e}", x);
    let (mant, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    if !(-4..6).contains(&exp) {
        let mant = trim_fraction(mant);
        let sign = if exp < 0 { '-' } else { '+' };
        return format!("{mant}e{sign}{:02}", exp.abs());
    }
    let decimals = (5 - exp).max(0) as usize;
    trim_fraction(&format!("{:.*}", decimals, x)).to_string()
}

fn trim_fraction(s: &str) -> &str {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        s
    }
}

// ---------------------------------------------------------------------------
// html

/// `html.escape(s, quote)`.
pub fn html_escape(s: &str, quote: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if quote => out.push_str("&quot;"),
            '\'' if quote => out.push_str("&#x27;"),
            c => out.push(c),
        }
    }
    out
}

fn html5_lookup(name: &str) -> Option<&'static str> {
    fsv_py::html_entities::HTML5
        .binary_search_by(|(k, _)| (*k).cmp(name))
        .ok()
        .map(|i| fsv_py::html_entities::HTML5[i].1)
}

/// `html.unescape(s)` (full HTML5 entity table, numeric reference fix-ups).
pub fn html_unescape(s: &str) -> Cow<'_, str> {
    if !s.contains('&') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 1..];
        match charref_len(after) {
            Some(len) => {
                out.push_str(&replace_charref(&after[..len]));
                rest = &after[len..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// Length of the match of `#[0-9]+;?|#[xX][0-9a-fA-F]+;?|[^\t\n\f <&#;]{1,32};?` at the start.
fn charref_len(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    if b.first() == Some(&b'#') {
        let (start, hex) = if matches!(b.get(1), Some(b'x' | b'X')) {
            (2, true)
        } else {
            (1, false)
        };
        let mut i = start;
        while i < b.len()
            && (if hex {
                b[i].is_ascii_hexdigit()
            } else {
                b[i].is_ascii_digit()
            })
        {
            i += 1;
        }
        if i == start {
            return None;
        }
        if b.get(i) == Some(&b';') {
            i += 1;
        }
        return Some(i);
    }
    let mut chars = 0;
    let mut end = 0;
    for (i, c) in s.char_indices() {
        if matches!(c, '\t' | '\n' | '\x0c' | ' ' | '<' | '&' | '#' | ';') || chars == 32 {
            break;
        }
        chars += 1;
        end = i + c.len_utf8();
    }
    if chars == 0 {
        return None;
    }
    if s[end..].starts_with(';') {
        end += 1;
    }
    Some(end)
}

fn replace_charref(s: &str) -> String {
    if let Some(num) = s.strip_prefix('#') {
        let (digits, radix) = match num.strip_prefix(['x', 'X']) {
            Some(h) => (h, 16),
            None => (num, 10),
        };
        let digits = digits.trim_end_matches(';');
        let n = u64::from_str_radix(digits, radix).unwrap_or(u64::MAX);
        if let Ok(n32) = u32::try_from(n)
            && let Ok(i) =
                fsv_py::html_entities::INVALID_CHARREFS.binary_search_by(|(k, _)| k.cmp(&n32))
        {
            return fsv_py::html_entities::INVALID_CHARREFS[i].1.to_string();
        }
        if (0xD800..=0xDFFF).contains(&n) || n > 0x10FFFF {
            return "\u{FFFD}".into();
        }
        let n32 = n as u32;
        if fsv_py::html_entities::INVALID_CODEPOINTS
            .binary_search(&n32)
            .is_ok()
        {
            return String::new();
        }
        return char::from_u32(n32)
            .map(String::from)
            .unwrap_or_else(|| "\u{FFFD}".into());
    }
    if let Some(v) = html5_lookup(s) {
        return v.to_string();
    }
    let idx: Vec<usize> = s
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(s.len()))
        .collect();
    // Longest prefix of at least two characters that is a known entity.
    for x in (2..idx.len() - 1).rev() {
        let prefix = &s[..idx[x]];
        if let Some(v) = html5_lookup(prefix) {
            return format!("{v}{}", &s[idx[x]..]);
        }
    }
    format!("&{s}")
}

// ---------------------------------------------------------------------------
// urllib.parse

const ALWAYS_SAFE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_.-~";

/// `urllib.parse.quote(s, safe=safe)`.
pub fn quote(s: &str, safe: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if ALWAYS_SAFE.contains(&b) || (b.is_ascii() && safe.as_bytes().contains(&b)) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `urllib.parse.quote_plus(s)` (safe="").
pub fn quote_plus(s: &str) -> String {
    if s.contains(' ') {
        quote(s, " ").replace(' ', "+")
    } else {
        quote(s, "")
    }
}

/// `urllib.parse.urlencode(pairs)`.
pub fn urlencode<K: AsRef<str>, V: AsRef<str>>(pairs: &[(K, V)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", quote_plus(k.as_ref()), quote_plus(v.as_ref())))
        .collect::<Vec<_>>()
        .join("&")
}

/// `urllib.parse.unquote(s)` (utf-8, errors="replace").
pub fn unquote(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut pending: Vec<u8> = Vec::new();
    let mut i = 0;
    let mut lit_start = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && b[i + 1].is_ascii_hexdigit()
            && b[i + 2].is_ascii_hexdigit()
        {
            if lit_start < i {
                out.push_str(&String::from_utf8_lossy(&pending));
                pending.clear();
                out.push_str(&s[lit_start..i]);
            }
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("00");
            pending.push(u8::from_str_radix(hex, 16).unwrap_or(0));
            i += 3;
            lit_start = i;
        } else {
            i += 1;
        }
    }
    out.push_str(&String::from_utf8_lossy(&pending));
    out.push_str(&s[lit_start..]);
    out
}

/// `urllib.parse.parse_qsl(qs, keep_blank_values=keep_blank)`.
pub fn parse_qsl(qs: &str, keep_blank: bool) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if qs.is_empty() {
        return out;
    }
    for part in qs.split('&') {
        if part.is_empty() {
            continue;
        }
        let (name, value) = match part.split_once('=') {
            Some((n, v)) => (n, v),
            None if keep_blank => (part, ""),
            None => continue,
        };
        if !value.is_empty() || keep_blank {
            out.push((
                unquote(&name.replace('+', " ")),
                unquote(&value.replace('+', " ")),
            ));
        }
    }
    out
}

/// `urllib.parse.parse_qs(qs)` reduced to the first value per key (insertion ordered).
pub fn parse_qs_first(qs: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (k, v) in parse_qsl(qs, false) {
        if !out.iter().any(|(ek, _)| *ek == k) {
            out.push((k, v));
        }
    }
    out
}

/// Result of `urllib.parse.urlsplit`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SplitUrl {
    pub scheme: String,
    pub netloc: String,
    pub path: String,
    pub query: String,
    pub fragment: String,
}

impl SplitUrl {
    /// `.hostname`: netloc without userinfo/port, lowercased.
    pub fn hostname(&self) -> Option<String> {
        let host = self
            .netloc
            .rsplit_once('@')
            .map_or(self.netloc.as_str(), |(_, h)| h);
        let host = if let Some(rest) = host.strip_prefix('[') {
            rest.split_once(']').map_or(rest, |(h, _)| h)
        } else {
            host.split_once(':').map_or(host, |(h, _)| h)
        };
        if host.is_empty() {
            None
        } else {
            Some(host.to_lowercase())
        }
    }
}

/// `urllib.parse.urlsplit(url)`; `Err` carries Python's ValueError text.
pub fn urlsplit(url: &str) -> Result<SplitUrl, String> {
    let cleaned: String = url
        .chars()
        .filter(|c| !matches!(c, '\t' | '\r' | '\n'))
        .collect();
    let mut rest: &str = cleaned.trim_start_matches(|c: char| c <= ' ');
    let mut out = SplitUrl::default();
    if let Some(i) = rest.find(':') {
        let first = rest.chars().next();
        if i > 0
            && first.is_some_and(|c| c.is_ascii_alphabetic())
            && rest[..i]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        {
            out.scheme = rest[..i].to_ascii_lowercase();
            rest = &rest[i + 1..];
        }
    }
    if let Some(after) = rest.strip_prefix("//") {
        let end = after.find(['/', '?', '#']).unwrap_or(after.len());
        out.netloc = after[..end].to_string();
        rest = &after[end..];
        let (open, close) = (out.netloc.contains('['), out.netloc.contains(']'));
        if open != close {
            return Err("Invalid IPv6 URL".into());
        }
    }
    let mut rest = rest.to_string();
    if let Some((u, f)) = rest.split_once('#') {
        out.fragment = f.to_string();
        rest = u.to_string();
    }
    if let Some((u, q)) = rest.split_once('?') {
        out.query = q.to_string();
        rest = u.to_string();
    }
    out.path = rest;
    Ok(out)
}

/// Path of `urllib.parse.urlparse(url)` (the `;params` part of the last segment removed).
pub fn urlparse_path(path: &str) -> &str {
    if !path.contains(';') {
        return path;
    }
    let from = path.rfind('/').unwrap_or(0);
    match path[from..].find(';') {
        Some(i) => &path[..from + i],
        None => path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescape_matches_python() {
        assert_eq!(html_unescape("a&#43;b&amp;c"), "a+b&c");
        assert_eq!(
            html_unescape("&amp &ampx &notit; &#x80; &#0; &bogus;"),
            "& &x ¬it; \u{20ac} \u{fffd} &bogus;"
        );
        assert_eq!(html_unescape("&#xD800;&#1114112;&#11;"), "\u{fffd}\u{fffd}");
    }

    #[test]
    fn g_format() {
        assert_eq!(format_g(0.5), "0.5");
        assert_eq!(format_g(1.5), "1.5");
        assert_eq!(format_g(10.0), "10");
        assert_eq!(format_g(2.25), "2.25");
        assert_eq!(format_g(1e-5), "1e-05");
        assert_eq!(format_g(1234567.0), "1.23457e+06");
    }

    #[test]
    fn quoting() {
        assert_eq!(quote("a b/é", "-._~,/:@"), "a%20b/%C3%A9");
        assert_eq!(urlencode(&[("q", "a b+c")]), "q=a+b%2Bc");
        assert_eq!(
            parse_qsl("a=1&b&c=&d=%C3%A9+x", true),
            vec![
                ("a".into(), "1".into()),
                ("b".into(), "".into()),
                ("c".into(), "".into()),
                ("d".into(), "é x".into())
            ]
        );
        assert_eq!(unquote("%E2%82%ACx%zz%41"), "€x%zzA");
    }

    #[test]
    fn split() {
        let u = urlsplit("https://ACME.com:8/a/b?x=1#f").unwrap();
        assert_eq!(
            (u.scheme.as_str(), u.netloc.as_str(), u.path.as_str()),
            ("https", "ACME.com:8", "/a/b")
        );
        assert_eq!((u.query.as_str(), u.fragment.as_str()), ("x=1", "f"));
        assert_eq!(urlsplit("search/y").unwrap().path, "search/y");
    }
}
