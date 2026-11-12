use serde_json::Value;
use std::fmt::Write;

use crate::float::float_repr;

/// Options mirroring `json.dumps(obj, ...)`.
///
/// Defaults equal Python's defaults: `ensure_ascii=True`, no indent,
/// separators `(", ", ": ")`, insertion order.
#[derive(Clone, Debug)]
pub struct Dumps {
    pub sort_keys: bool,
    pub ensure_ascii: bool,
    pub indent: Option<usize>,
    /// `None` means Python's default for the indent mode: `(", ", ": ")`
    /// without indent and `(",", ": ")` with one.
    pub separators: Option<(&'static str, &'static str)>,
}

impl Default for Dumps {
    fn default() -> Self {
        Self {
            sort_keys: false,
            ensure_ascii: true,
            indent: None,
            separators: None,
        }
    }
}

impl Dumps {
    pub fn sort_keys(mut self, yes: bool) -> Self {
        self.sort_keys = yes;
        self
    }
    pub fn ensure_ascii(mut self, yes: bool) -> Self {
        self.ensure_ascii = yes;
        self
    }
    pub fn indent(mut self, n: usize) -> Self {
        self.indent = Some(n);
        self
    }
    pub fn separators(mut self, item: &'static str, key: &'static str) -> Self {
        self.separators = Some((item, key));
        self
    }
    pub fn dump(&self, value: &Value) -> String {
        let (item, key) = self.separators.unwrap_or(if self.indent.is_some() {
            (",", ": ")
        } else {
            (", ", ": ")
        });
        let mut out = String::new();
        write_value(&mut out, value, self, item, key, 0);
        out
    }
}

/// `json.dumps(value, **opts)`.
pub fn dumps(value: &Value, opts: &Dumps) -> String {
    opts.dump(value)
}

/// `json.dumps(v, sort_keys=True, separators=(",", ":"), ensure_ascii=False)`:
/// the canonical form hashed by plans and the journal.
pub fn canonical(value: &Value) -> String {
    Dumps::default()
        .sort_keys(true)
        .separators(",", ":")
        .ensure_ascii(false)
        .dump(value)
}

/// `json.dumps(v, ensure_ascii=False, separators=(",", ":"))`.
pub fn compact(value: &Value) -> String {
    Dumps::default()
        .separators(",", ":")
        .ensure_ascii(false)
        .dump(value)
}

/// `json.dumps(v, indent=2)` (ensure_ascii stays True, like Python's default).
pub fn pretty(value: &Value) -> String {
    Dumps::default().indent(2).dump(value)
}

fn newline(out: &mut String, opts: &Dumps, level: usize) {
    if let Some(n) = opts.indent {
        out.push('\n');
        for _ in 0..n * level {
            out.push(' ');
        }
    }
}

fn write_value(out: &mut String, value: &Value, opts: &Dumps, item: &str, key: &str, level: usize) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => write_number(out, n),
        Value::String(s) => write_str(out, s, opts.ensure_ascii),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            newline(out, opts, level + 1);
            for (i, v) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(item);
                    newline(out, opts, level + 1);
                }
                write_value(out, v, opts, item, key, level + 1);
            }
            newline(out, opts, level);
            out.push(']');
        }
        Value::Object(map) => {
            if map.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            newline(out, opts, level + 1);
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            if opts.sort_keys {
                entries.sort_by(|a, b| a.0.cmp(b.0));
            }
            for (i, (k, v)) in entries.into_iter().enumerate() {
                if i > 0 {
                    out.push_str(item);
                    newline(out, opts, level + 1);
                }
                write_str(out, k, opts.ensure_ascii);
                out.push_str(key);
                write_value(out, v, opts, item, key, level + 1);
            }
            newline(out, opts, level);
            out.push('}');
        }
    }
}

fn write_number(out: &mut String, n: &serde_json::Number) {
    if let Some(i) = n.as_i64() {
        let _ = write!(out, "{i}");
    } else if let Some(u) = n.as_u64() {
        let _ = write!(out, "{u}");
    } else if let Some(f) = n.as_f64() {
        out.push_str(&float_repr(f));
    } else {
        out.push_str(&n.to_string());
    }
}

/// JSON string literal exactly as CPython's encoder writes it.
pub(crate) fn write_str(out: &mut String, s: &str, ensure_ascii: bool) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c if ensure_ascii && (c as u32) > 0x7e => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    let _ = write!(out, "\\u{unit:04x}");
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn default_separators_and_ascii() {
        let v = json!({"b": 1, "a": [1, 2.5, null, true], "é": "ü\n😀"});
        assert_eq!(
            dumps(&v, &Dumps::default()),
            r#"{"b": 1, "a": [1, 2.5, null, true], "\u00e9": "\u00fc\n\ud83d\ude00"}"#
        );
    }

    #[test]
    fn canonical_sorts_and_keeps_unicode() {
        let v = json!({"b": {"y": 1, "x": "é"}, "a": []});
        assert_eq!(canonical(&v), r#"{"a":[],"b":{"x":"é","y":1}}"#);
    }

    #[test]
    fn indent_matches_python() {
        let v = json!({"a": [1, {}], "b": {"c": null}});
        assert_eq!(
            pretty(&v),
            "{\n  \"a\": [\n    1,\n    {}\n  ],\n  \"b\": {\n    \"c\": null\n  }\n}"
        );
    }

    #[test]
    fn del_is_not_escaped_but_non_ascii_is() {
        assert_eq!(dumps(&json!("\u{7f}"), &Dumps::default()), "\"\\u007f\"");
        assert_eq!(compact(&json!("\u{7f}")), "\"\u{7f}\"");
    }
}
