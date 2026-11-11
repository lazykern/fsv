//! `$VISUAL`/`$EDITOR` round-trips with re-open on parse errors (`editor.py`).

use std::path::Path;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// Marker on the error lines written at the top of the file.
pub const ERROR_MARK: &str = "ERROR:";

/// `shlex.split(s)` (POSIX mode, no comments).
pub fn shlex_split(s: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_token = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_token = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(ch) => cur.push(ch),
                        None => return Err(Error::value("No closing quotation")),
                    }
                }
            }
            '"' => {
                in_token = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(ch @ ('"' | '\\')) => cur.push(ch),
                            Some(ch) => {
                                cur.push('\\');
                                cur.push(ch);
                            }
                            None => return Err(Error::value("No closing quotation")),
                        },
                        Some(ch) => cur.push(ch),
                        None => return Err(Error::value("No closing quotation")),
                    }
                }
            }
            '\\' => {
                in_token = true;
                match chars.next() {
                    Some(ch) => cur.push(ch),
                    None => return Err(Error::value("No escaped character")),
                }
            }
            c if matches!(c, ' ' | '\t' | '\r' | '\n') => {
                if in_token {
                    out.push(std::mem::take(&mut cur));
                    in_token = false;
                }
            }
            c => {
                in_token = true;
                cur.push(c);
            }
        }
    }
    if in_token {
        out.push(cur);
    }
    Ok(out)
}

/// Editor command: `$VISUAL`, else `$EDITOR`, else `vi`.
pub fn editor_command() -> Result<Vec<String>> {
    let editor = ["VISUAL", "EDITOR"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .unwrap_or_else(|| "vi".into());
    shlex_split(&editor)
}

fn digest(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
}

/// Drop the error lines a previous round put at the top.
fn strip_error_header(text: &str, prefix: &str) -> String {
    let marker = format!("{prefix} {ERROR_MARK}");
    let kept: Vec<&str> = py_splitlines(text).into_iter().filter(|ln| !crate::pystr::lstrip(ln).starts_with(&marker)).collect();
    kept.join("\n") + if text.ends_with('\n') { "\n" } else { "" }
}

fn py_splitlines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut it = text.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        let brk = matches!(
            c,
            '\n' | '\r' | '\u{0b}' | '\u{0c}' | '\u{1c}' | '\u{1d}' | '\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}'
        );
        if brk {
            out.push(&text[start..i]);
            let mut end = i + c.len_utf8();
            if c == '\r' {
                if let Some(&(j, '\n')) = it.peek() {
                    it.next();
                    end = j + 1;
                }
            }
            start = end;
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// Options for [`edit_text`].
#[derive(Clone, Debug)]
pub struct EditOptions<'a> {
    pub suffix: &'a str,
    pub comment_prefix: &'a str,
    pub max_rounds: usize,
}

impl Default for EditOptions<'_> {
    fn default() -> Self {
        Self { suffix: ".txt", comment_prefix: "#", max_rounds: 20 }
    }
}

fn run_editor(path: &Path) -> Result<()> {
    let argv = editor_command()?;
    let Some((prog, args)) = argv.split_first() else {
        return Err(Error::exit(1, ""));
    };
    match std::process::Command::new(prog).args(args).arg(path).status() {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("error: editor not found: {prog}; set $VISUAL or $EDITOR");
            Err(Error::exit(1, ""))
        }
        Err(e) => Err(e.into()),
    }
}

/// Open the editor on `initial` and return `parse(edited_text)`.
///
/// When `parse` fails with [`Error::Value`] (JSON/YAML errors), the editor
/// reopens on the same file with the error at the top, so edits are not
/// lost. "Unchanged" is decided by content hash, not mtime. Fails with
/// [`Error::EditorAbort`] on unchanged/too many bad rounds and
/// `Error::Exit { code: 1 }` (already reported) when the editor cannot start.
pub fn edit_text<T>(initial: &str, parse: impl FnMut(&str) -> Result<T>, opts: &EditOptions<'_>) -> Result<T> {
    edit_text_with(initial, parse, opts, &mut |p: &Path| run_editor(p))
}

/// [`edit_text`] with an injectable editor launcher (tests, TUI).
pub fn edit_text_with<T>(
    initial: &str,
    mut parse: impl FnMut(&str) -> Result<T>,
    opts: &EditOptions<'_>,
    launch: &mut dyn FnMut(&Path) -> Result<()>,
) -> Result<T> {
    let tmp = tempfile::Builder::new().prefix("tmp").suffix(opts.suffix).tempfile()?;
    std::fs::write(tmp.path(), initial)?;
    let path = tmp.into_temp_path();
    let mut shown = initial.to_string();
    for _ in 0..opts.max_rounds {
        launch(&path)?;
        let text = std::fs::read_to_string(&path)?;
        if digest(&text) == digest(&shown) {
            return Err(Error::EditorAbort("aborted: file unchanged".into()));
        }
        let body = strip_error_header(&text, opts.comment_prefix);
        match parse(&body) {
            Ok(v) => return Ok(v),
            Err(Error::Value(msg)) => {
                let msg = crate::pystr::collapse_ws(&msg);
                let p = opts.comment_prefix;
                shown = format!("{p} {ERROR_MARK} {msg}\n{p} {ERROR_MARK} fix and save, or leave unchanged to abort\n{body}");
                std::fs::write(&path, &shown)?;
            }
            Err(e) => return Err(e),
        }
    }
    Err(Error::EditorAbort("aborted: too many invalid edits".into()))
}

/// Open the editor on `data` as commented JSON and return the parsed result.
///
/// Lines starting with `//` or `#` are ignored. Fails with
/// [`Error::EditorAbort`] on cancel/unchanged/empty.
pub fn edit_json(data: &Value, hint: &str) -> Result<Value> {
    edit_json_with(data, hint, &mut |p: &Path| run_editor(p))
}

/// [`edit_json`] with an injectable editor launcher.
pub fn edit_json_with(data: &Value, hint: &str, launch: &mut dyn FnMut(&Path) -> Result<()>) -> Result<Value> {
    let comment = format!(
        "// {hint}\n// Edit JSON below. Comment lines ignored.\n// Save + close to continue; leave unchanged to abort.\n\n"
    );
    let raw = comment + &fsv_py::Dumps::default().indent(2).ensure_ascii(false).dump(data);
    let opts = EditOptions { suffix: ".jsonc", comment_prefix: "//", ..EditOptions::default() };
    let edited = edit_text_with(&raw, parse_json_text, &opts, launch)?;
    if !crate::value::truthy(&edited) {
        return Err(Error::EditorAbort("aborted: empty file".into()));
    }
    Ok(edited)
}

/// Parse JSON after dropping blank and comment (`#`, `//`) lines; empty → `{}`.
pub fn parse_json_text(text: &str) -> Result<Value> {
    let lines: Vec<&str> = py_splitlines(text)
        .into_iter()
        .filter(|line| {
            let s = crate::pystr::strip(line);
            !(s.is_empty() || s.starts_with('#') || s.starts_with("//"))
        })
        .collect();
    if lines.is_empty() {
        return Ok(Value::Object(serde_json::Map::new()));
    }
    Ok(serde_json::from_str(&lines.join("\n"))?)
}

/// Read a JSON file, stripping comment lines (`#`, `//`) and empty lines.
pub fn read_json_file(path: &Path) -> Result<Value> {
    parse_json_text(&std::fs::read_to_string(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fake(edits: Vec<Box<dyn Fn(&str) -> Option<String>>>) -> (impl FnMut(&Path) -> Result<()>, std::rc::Rc<std::cell::RefCell<Vec<String>>>) {
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let s2 = seen.clone();
        let launch = move |p: &Path| -> Result<()> {
            let text = std::fs::read_to_string(p)?;
            s2.borrow_mut().push(text.clone());
            let n = s2.borrow().len();
            let f = &edits[(n - 1).min(edits.len() - 1)];
            if let Some(new) = f(&text) {
                std::fs::write(p, new)?;
            }
            Ok(())
        };
        (launch, seen)
    }

    #[test]
    fn json_error_reopens_editor_with_error_and_keeps_edits() {
        let (mut launch, seen) = fake(vec![
            Box::new(|t| Some(t.replace("\"a\": 1", "\"a\": 1,, \"keep\": 7"))),
            Box::new(|t| Some(t.replace(",,", ","))),
        ]);
        let out = edit_json_with(&json!({"a": 1}), "", &mut launch).unwrap();
        assert_eq!(out, json!({"a": 1, "keep": 7}));
        let seen = seen.borrow();
        assert_eq!(seen.len(), 2);
        assert!(seen[1].trim_start().starts_with("// ERROR:") && seen[1].contains("\"keep\": 7"));
    }

    #[test]
    fn unchanged_by_content_not_mtime() {
        let (mut launch, _) = fake(vec![Box::new(|t| Some(t.to_string()))]);
        assert!(matches!(edit_json_with(&json!({"a": 1}), "", &mut launch), Err(Error::EditorAbort(_))));
    }

    #[test]
    fn changed_content_is_accepted() {
        let (mut launch, _) = fake(vec![Box::new(|t| Some(t.replace("\"a\": 1", "\"a\": 2")))]);
        assert_eq!(edit_json_with(&json!({"a": 1}), "", &mut launch).unwrap(), json!({"a": 2}));
    }

    #[test]
    fn unchanged_after_error_aborts() {
        let (mut launch, _) = fake(vec![Box::new(|t| Some(format!("{t}{{{{"))), Box::new(|t| Some(t.to_string()))]);
        assert!(matches!(edit_json_with(&json!({"a": 1}), "", &mut launch), Err(Error::EditorAbort(_))));
    }

    #[test]
    fn edit_text_generic_yaml_style_parse_error() {
        let (mut launch, seen) = fake(vec![Box::new(|_| Some("bad\n".into())), Box::new(|_| Some("good\n".into()))]);
        let parse = |t: &str| -> Result<String> {
            if t.contains("bad") { Err(Error::value("line 1: bad value")) } else { Ok(t.trim().to_string()) }
        };
        let opts = EditOptions { suffix: ".yaml", ..EditOptions::default() };
        assert_eq!(edit_text_with("start\n", parse, &opts, &mut launch).unwrap(), "good");
        assert!(seen.borrow()[1].starts_with("# ERROR: line 1: bad value"));
    }

    #[test]
    fn shlex() {
        assert_eq!(shlex_split("vis-editor --wait").unwrap(), vec!["vis-editor", "--wait"]);
        assert_eq!(shlex_split(r#"code -w "a b" 'c d' e\ f"#).unwrap(), vec!["code", "-w", "a b", "c d", "e f"]);
    }
}
