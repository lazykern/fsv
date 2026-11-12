use serde_json::Value;
use std::fmt::Write;

use crate::float::float_repr_py;

/// Python `str(value)` for a JSON-decoded value.
///
/// Strings are returned as-is; `None`/`True`/`False` and containers use
/// Python spelling (`{'a': 1}`), which several error messages expose.
pub fn py_str(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => py_repr(other),
    }
}

/// Python `repr(value)` for a JSON-decoded value.
pub fn py_repr(value: &Value) -> String {
    let mut out = String::new();
    write_repr(&mut out, value);
    out
}

fn write_repr(out: &mut String, value: &Value) {
    match value {
        Value::Null => out.push_str("None"),
        Value::Bool(true) => out.push_str("True"),
        Value::Bool(false) => out.push_str("False"),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                let _ = write!(out, "{i}");
            } else if let Some(u) = n.as_u64() {
                let _ = write!(out, "{u}");
            } else {
                out.push_str(&float_repr_py(n.as_f64().unwrap_or(f64::NAN)));
            }
        }
        Value::String(s) => out.push_str(&py_repr_str(s)),
        Value::Array(items) => {
            out.push('[');
            for (i, v) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_repr(out, v);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            for (i, (k, v)) in map.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&py_repr_str(k));
                out.push_str(": ");
                write_repr(out, v);
            }
            out.push('}');
        }
    }
}

/// Python `repr(str)`: single quotes unless the text contains `'` and no `"`.
pub fn py_repr_str(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if !is_printable(c) => {
                let cp = c as u32;
                if cp < 0x100 {
                    let _ = write!(out, "\\x{cp:02x}");
                } else if cp < 0x10000 {
                    let _ = write!(out, "\\u{cp:04x}");
                } else {
                    let _ = write!(out, "\\U{cp:08x}");
                }
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// Approximation of `str.isprintable()` for a single character: control
/// characters, separators other than ASCII space, and unassigned/format
/// code points are escaped by CPython.
fn is_printable(c: char) -> bool {
    let cp = c as u32;
    if c == ' ' {
        return true;
    }
    if cp < 0x20 || (0x7f..0xa0).contains(&cp) {
        return false;
    }
    !matches!(
        cp,
        0xad | 0x2028 | 0x2029 | 0x2000..=0x200f | 0x202a..=0x202f | 0x205f..=0x2064
            | 0x2066..=0x206f | 0x3000 | 0xa0 | 0x1680 | 0xfeff | 0xfff9..=0xfffb
            | 0xd800..=0xdfff | 0xe000..=0xf8ff | 0xf0000..=0x10ffff
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn repr_like_python() {
        assert_eq!(
            py_repr(&json!({"a": [1, null, true, 1.5, "x"]})),
            "{'a': [1, None, True, 1.5, 'x']}"
        );
        assert_eq!(py_repr_str("it's"), "\"it's\"");
        assert_eq!(py_repr_str("a'b\"c"), "'a\\'b\"c'");
        assert_eq!(py_repr_str("tab\there"), "'tab\\there'");
        assert_eq!(py_repr_str("é"), "'é'");
        assert_eq!(py_str(&json!("plain")), "plain");
        assert_eq!(py_str(&json!(null)), "None");
        assert_eq!(py_str(&json!(2.0)), "2.0");
    }
}
