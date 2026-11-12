//! YAML for drafts, presets and hints: strict, predictable scalars (`yamlio.py`).
//!
//! YAML 1.1 loaders turn `No` into false, `22:30` into 1350 and
//! `2030-01-01` into a date. Form answers are labels, so every plain scalar
//! stays a string except `null`/`~`/empty and `true`/`false` (three
//! spellings each). Quoted scalars are always strings.

use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value};
use yaml_rust2::parser::{Event, MarkedEventReceiver, Parser, Tag};
use yaml_rust2::scanner::{Marker, TScalarStyle};

use crate::error::{Error, Result};
use crate::pystr;

static NULL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:~|null|Null|NULL|)$").expect("regex"));
static BOOL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:true|True|TRUE|false|False|FALSE)$").expect("regex"));

enum Frame {
    Seq(Vec<Value>, usize),
    Map(Map<String, Value>, Option<String>, usize),
}

#[derive(Default)]
struct Builder {
    stack: Vec<Frame>,
    docs: Vec<Value>,
    anchors: HashMap<usize, Value>,
    error: Option<String>,
}

fn tag_suffix(tag: &Option<Tag>) -> Option<&str> {
    let t = tag.as_ref()?;
    if t.handle == "!!" || t.handle == "tag:yaml.org,2002:" {
        Some(t.suffix.as_str())
    } else {
        None
    }
}

fn tag_text(tag: &Tag) -> String {
    if tag.handle == "!!" || tag.handle == "tag:yaml.org,2002:" {
        format!("tag:yaml.org,2002:{}", tag.suffix)
    } else {
        format!("{}{}", tag.handle, tag.suffix)
    }
}

fn resolve_scalar(
    text: String,
    style: TScalarStyle,
    tag: &Option<Tag>,
) -> std::result::Result<Value, String> {
    if let Some(t) = tag {
        return match tag_suffix(tag) {
            Some("str") => Ok(Value::String(text)),
            Some("null") => Ok(Value::Null),
            Some("bool") => match text.to_lowercase().as_str() {
                "true" | "yes" | "on" => Ok(Value::Bool(true)),
                "false" | "no" | "off" => Ok(Value::Bool(false)),
                _ => Err(format!("invalid boolean value {text:?}")),
            },
            Some("int") => {
                let cleaned = text.replace('_', "");
                cleaned
                    .parse::<i64>()
                    .map(Value::from)
                    .map_err(|_| format!("invalid int value {text:?}"))
            }
            Some("float") => {
                let cleaned = text.replace('_', "").to_lowercase();
                let f = match cleaned.as_str() {
                    ".inf" | "+.inf" => Some(f64::INFINITY),
                    "-.inf" => Some(f64::NEG_INFINITY),
                    ".nan" => Some(f64::NAN),
                    s => s.parse::<f64>().ok(),
                };
                f.and_then(serde_json::Number::from_f64)
                    .map(Value::Number)
                    .ok_or_else(|| format!("invalid float value {text:?}"))
            }
            _ => Err(format!(
                "could not determine a constructor for the tag '{}'",
                tag_text(t)
            )),
        };
    }
    if matches!(style, TScalarStyle::Plain) {
        if NULL_RE.is_match(&text) {
            return Ok(Value::Null);
        }
        if BOOL_RE.is_match(&text) {
            return Ok(Value::Bool(text.starts_with(['t', 'T'])));
        }
    }
    Ok(Value::String(text))
}

/// A mapping key as Python's callers see it after `str(k)`: a plain `true:`
/// is `"True"`, `null:` / `~:` is `"None"`, a quoted `"true":` stays `"true"`.
fn key_text(v: &Value) -> std::result::Result<String, String> {
    match v {
        Value::String(_) | Value::Null | Value::Bool(_) | Value::Number(_) => Ok(fsv_py::py_str(v)),
        _ => Err("found unhashable key".into()),
    }
}

impl Builder {
    fn push_value(&mut self, v: Value, anchor: usize) {
        if anchor > 0 {
            self.anchors.insert(anchor, v.clone());
        }
        match self.stack.last_mut() {
            None => self.docs.push(v),
            Some(Frame::Seq(items, _)) => items.push(v),
            Some(Frame::Map(map, pending, _)) => match pending.take() {
                None => match key_text(&v) {
                    Ok(k) => *pending = Some(k),
                    Err(e) => self.error = Some(e),
                },
                Some(k) => {
                    map.insert(k, v);
                }
            },
        }
    }
}

impl MarkedEventReceiver for Builder {
    fn on_event(&mut self, ev: Event, mark: Marker) {
        if self.error.is_some() {
            return;
        }
        match ev {
            Event::Scalar(text, style, anchor, tag) => match resolve_scalar(text, style, &tag) {
                Ok(v) => self.push_value(v, anchor),
                Err(e) => {
                    self.error = Some(format!(
                        "{e} at line {} column {}",
                        mark.line(),
                        mark.col() + 1
                    ))
                }
            },
            Event::SequenceStart(anchor, _) => self.stack.push(Frame::Seq(Vec::new(), anchor)),
            Event::MappingStart(anchor, _) => self.stack.push(Frame::Map(Map::new(), None, anchor)),
            Event::SequenceEnd => {
                if let Some(Frame::Seq(items, anchor)) = self.stack.pop() {
                    self.push_value(Value::Array(items), anchor);
                }
            }
            Event::MappingEnd => {
                if let Some(Frame::Map(map, _, anchor)) = self.stack.pop() {
                    self.push_value(Value::Object(map), anchor);
                }
            }
            Event::Alias(id) => match self.anchors.get(&id).cloned() {
                Some(v) => self.push_value(v, 0),
                None => self.error = Some("found undefined alias".into()),
            },
            _ => {}
        }
    }
}

/// Parse YAML; `Error::Value("YAML error: ...")` (one line) on syntax errors.
pub fn loads(text: &str) -> Result<Value> {
    let mut builder = Builder::default();
    let mut parser = Parser::new_from_str(text);
    if let Err(e) = parser.load(&mut builder, true) {
        let m = e.marker();
        let msg = format!(
            "{} in \"<unicode string>\", line {}, column {}",
            e.info(),
            m.line(),
            m.col() + 1
        );
        return Err(Error::value(format!(
            "YAML error: {}",
            pystr::collapse_ws(&msg)
        )));
    }
    if let Some(e) = builder.error {
        return Err(Error::value(format!(
            "YAML error: {}",
            pystr::collapse_ws(&e)
        )));
    }
    match builder.docs.len() {
        0 => Ok(Value::Null),
        1 => Ok(builder.docs.pop().unwrap_or(Value::Null)),
        _ => Err(Error::value(
            "YAML error: expected a single document in the stream but found another document",
        )),
    }
}

/// Read and parse a YAML file.
pub fn load_file(path: &Path) -> Result<Value> {
    loads(&std::fs::read_to_string(path)?)
}

static PLAIN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^[^\s#:\-?\[\]{},&*!|>'"%@`][^#\n]*$"#).expect("regex"));
static RESERVED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:~|null|Null|NULL|true|True|TRUE|false|False|FALSE)$").expect("regex")
});
static FLOW_PLAIN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_][A-Za-z0-9_.@/ +()-]*$").expect("regex"));

fn json_str(s: &str) -> String {
    fsv_py::Dumps::default()
        .ensure_ascii(false)
        .dump(&Value::String(s.to_string()))
}

/// One YAML scalar that reads back as the same value with [`loads`].
pub fn scalar(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        Value::Number(_) => fsv_py::py_str(value),
        other => scalar_str(&fsv_py::py_str(other)),
    }
}

/// [`scalar`] for a string.
pub fn scalar_str(s: &str) -> String {
    if PLAIN.is_match(s)
        && !RESERVED.is_match(s)
        && s == pystr::strip(s)
        && !s.contains(": ")
        && !s.ends_with(':')
    {
        s.to_string()
    } else {
        json_str(s)
    }
}

/// A mapping key (labels may contain ':' or '#').
pub fn key(text: &str) -> String {
    scalar_str(text)
}

/// `[a, "b: c", 3]` flow sequence.
pub fn flow_list(values: &[Value]) -> String {
    let items: Vec<String> = values
        .iter()
        .map(|v| match v {
            Value::String(s)
                if !(FLOW_PLAIN.is_match(s) && s == pystr::strip(s) && !RESERVED.is_match(s)) =>
            {
                json_str(s)
            }
            other => scalar(other),
        })
        .collect();
    format!("[{}]", items.join(", "))
}

/// `|-` block scalar for multi-line text (reads back without a final newline).
pub fn block(text: &str, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let normalized = text.replace("\r\n", "\n");
    let lines: Vec<String> = normalized
        .trim_end_matches('\n')
        .split('\n')
        .map(|ln| {
            if ln.is_empty() {
                String::new()
            } else {
                format!("{pad}{ln}")
            }
        })
        .collect();
    format!("|-\n{}", lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn no_stays_a_label_not_a_bool() {
        assert_eq!(
            loads("a: No\nb: 22:30\nc: 2030-01-01\nd: true").unwrap(),
            json!({"a": "No", "b": "22:30", "c": "2030-01-01", "d": true})
        );
        for v in [
            "No",
            "true",
            "a: b",
            "# x",
            "22:30",
            "Reports, Exports",
            "-5",
        ] {
            let v = json!(v);
            assert_eq!(
                loads(&format!("k: {}", scalar(&v))).unwrap(),
                json!({"k": v})
            );
            assert_eq!(
                loads(&format!("k: {}", flow_list(std::slice::from_ref(&v)))).unwrap(),
                json!({"k": [v]})
            );
        }
    }

    #[test]
    fn typed_keys_read_as_python_str() {
        for (text, want) in [
            ("true: 1", json!({"True": "1"})),
            ("True: 1", json!({"True": "1"})),
            ("FALSE: 1", json!({"False": "1"})),
            ("null: 1", json!({"None": "1"})),
            ("~: 1", json!({"None": "1"})),
            ("? \n: 1", json!({"None": "1"})),
            ("\"true\": 1", json!({"true": "1"})),
            ("'null': 1", json!({"null": "1"})),
            ("!!int 7: a", json!({"7": "a"})),
            ("!!float 1: a", json!({"1.0": "a"})),
            (
                "a: {false: x, Null: y}",
                json!({"a": {"False": "x", "None": "y"}}),
            ),
            ("true: 1\nTrue: 2\n\"True\": 3", json!({"True": "3"})),
        ] {
            assert_eq!(loads(text).unwrap(), want, "{text:?}");
        }
    }

    #[test]
    fn errors_are_one_line_value_errors() {
        let e = loads("a: [1\nb: 2").unwrap_err();
        assert!(
            matches!(&e, Error::Value(m) if m.starts_with("YAML error: ") && !m.contains('\n')),
            "{e}"
        );
    }

    #[test]
    fn key_quotes_only_when_needed() {
        for (k, want) in [
            ("Status", "Status"),
            ("Type: Normal", "\"Type: Normal\""),
            ("# hash", "\"# hash\""),
            ("a#b", "\"a#b\""),
            ("true", "\"true\""),
            ("", "\"\""),
            (" lead", "\" lead\""),
            ("x:", "\"x:\""),
            ("Reports, Exports", "Reports, Exports"),
            ("-x", "\"-x\""),
            ("1", "1"),
            ("é ok", "é ok"),
        ] {
            assert_eq!(key(k), want, "{k:?}");
        }
    }

    #[test]
    fn block_literal_indent() {
        assert_eq!(block("a\nb", 2), "|-\n  a\n  b");
        assert_eq!(block("line\r\n\r\nnext\n\n", 4), "|-\n    line\n\n    next");
        assert_eq!(block("", 2), "|-\n");
        assert_eq!(block("one", 0), "|-\none");
    }

    #[test]
    fn load_file_reads_and_reports_one_line_errors() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.yaml");
        std::fs::write(&p, "a: No\nb: [1, x]\nc:\n").unwrap();
        assert_eq!(
            load_file(&p).unwrap(),
            json!({"a": "No", "b": ["1", "x"], "c": null})
        );
        std::fs::write(&p, "a: [1\n").unwrap();
        match load_file(&p) {
            Err(Error::Value(m)) => {
                assert!(m.starts_with("YAML error: ") && !m.contains('\n'), "{m}")
            }
            other => panic!("{other:?}"),
        }
    }
}
