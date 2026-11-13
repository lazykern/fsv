//! File inputs: the checks the form runs on typed paths, sizes for the
//! file list, and path completion for the file editor.

use std::path::{Component, Path, PathBuf};

use fsv_forms::Value;
use serde_json::{Map, Value as Json};

/// Where `~` and relative paths point. The caller takes them from the
/// process (`HOME`, the working directory) at the binary edge.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Paths {
    pub home: Option<PathBuf>,
    pub cwd: PathBuf,
}

impl Paths {
    pub fn new(home: Option<PathBuf>, cwd: impl Into<PathBuf>) -> Self {
        Paths {
            home: home.filter(|h| !h.as_os_str().is_empty()),
            cwd: cwd.into(),
        }
    }

    /// `os.path.expanduser`: a leading `~` or `~/` becomes the home
    /// directory.
    pub fn expand_user(&self, p: &str) -> String {
        match &self.home {
            Some(home) if p == "~" || p.starts_with("~/") => {
                format!("{}{}", home.to_string_lossy(), &p[1..])
            }
            _ => p.to_string(),
        }
    }

    /// `os.path.abspath(os.path.expanduser(p))`: absolute against the
    /// working directory, `.` and `..` removed lexically (symlinks are not
    /// resolved).
    pub fn abspath(&self, p: &str) -> PathBuf {
        let expanded = self.expand_user(p);
        let path = Path::new(&expanded);
        let joined = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.cwd.join(path)
        };
        let mut out = PathBuf::new();
        for c in joined.components() {
            match c {
                Component::CurDir => {}
                Component::ParentDir => {
                    out.pop();
                }
                other => out.push(other.as_os_str()),
            }
        }
        out
    }

    /// `os.path.relpath(path)` (lexical).
    pub fn relpath(&self, path: &Path) -> String {
        let p: Vec<Component<'_>> = path.components().collect();
        let b: Vec<Component<'_>> = self.cwd.components().collect();
        let common = p.iter().zip(&b).take_while(|(x, y)| x == y).count();
        if common == 0 {
            return path.display().to_string();
        }
        let mut out = PathBuf::new();
        for _ in common..b.len() {
            out.push("..");
        }
        for c in &p[common..] {
            out.push(c);
        }
        if out.as_os_str().is_empty() {
            ".".into()
        } else {
            out.display().to_string()
        }
    }
}

fn basename(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn split_raw(raw: &Value) -> Vec<String> {
    match raw {
        Value::List(items) => items.iter().map(Value::py_str).collect(),
        Value::Json(Json::Array(items)) => items.iter().map(fsv_py::py_str).collect(),
        Value::Null => vec![],
        other => other
            .py_str()
            .replace('\n', ";")
            .split(';')
            .map(str::to_string)
            .collect(),
    }
}

/// Rules a file field puts on its paths.
#[derive(Clone, Debug, Default)]
pub struct FileRules<'a> {
    pub allowed: &'a [String],
    pub max_files: Option<i64>,
    pub max_bytes: Option<i64>,
}

/// Paths from `a.pdf; b.png` (or a list) -> (absolute paths, error).
pub fn check_files(raw: &Value, rules: &FileRules<'_>, where_: &Paths) -> (Vec<String>, String) {
    let paths: Vec<PathBuf> = split_raw(raw)
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .map(|p| where_.abspath(p))
        .collect();
    let strs: Vec<String> = paths
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    if let Some(max) = rules.max_files
        && paths.len() as i64 > max
    {
        return (strs, format!("at most {max} file(s)"));
    }
    let mut types: Vec<String> = rules
        .allowed
        .iter()
        .map(|t| {
            let t = t.to_lowercase();
            if t.starts_with('.') {
                t
            } else {
                format!(".{t}")
            }
        })
        .collect();
    types.sort();
    types.dedup();
    for p in &paths {
        let name = basename(p);
        let Ok(meta) = std::fs::metadata(p) else {
            return (strs, format!("{name}: file not found"));
        };
        if !meta.is_file() {
            return (strs, format!("{name}: not a file"));
        }
        if !types.is_empty() && !types.contains(&ext_lower(p)) {
            return (
                strs,
                format!("{name}: allowed file types are {}", types.join(", ")),
            );
        }
        let size = meta.len();
        if size == 0 {
            return (strs, format!("{name}: the file is empty"));
        }
        if let Some(max) = rules.max_bytes
            && i128::from(size) > i128::from(max)
        {
            return (
                strs,
                format!(
                    "{name}: {:.1} MB is over the {:.0} MB limit",
                    size as f64 / 1e6,
                    max as f64 / 1e6
                ),
            );
        }
    }
    (strs, String::new())
}

/// `os.path.splitext(p)[1].lower()` (a leading dot is not an extension).
fn ext_lower(p: &Path) -> String {
    let name = basename(p);
    let trimmed = name.trim_start_matches('.');
    match trimmed.rfind('.') {
        Some(i) => trimmed[i..].to_lowercase(),
        None => String::new(),
    }
}

pub fn group_thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `1,000 B`, `1.5 KB`, `2.0 MB` (1024 steps).
pub fn human_size(n: u64) -> String {
    if n < 1024 {
        return format!("{} B", group_thousands(n));
    }
    let mut v = n as f64;
    for unit in ["KB", "MB", "GB"] {
        v /= 1024.0;
        if v < 1024.0 || unit == "GB" {
            return format!("{v:.1} {unit}");
        }
    }
    unreachable!("GB is the last unit")
}

/// `_max_bytes`: `forms.max_file_mb` from the config, in bytes. A negative
/// limit is kept, so every non-empty file is over it.
pub fn max_file_bytes(config: &Map<String, Json>) -> Option<i64> {
    let v = config.get("forms")?.get("max_file_mb")?;
    let mb = match v {
        Json::Number(n) => n.as_f64()?,
        Json::String(s) if !s.is_empty() => s.trim().parse::<f64>().ok()?,
        Json::Bool(b) => f64::from(u8::from(*b)),
        _ => return None,
    };
    let bytes = (mb * 1_000_000.0).trunc();
    bytes.is_finite().then_some(bytes as i64)
}

/// Complete the last path of `a.pdf; b.pn` from the file system: the
/// completed text, or `None` when there is nothing to add.
pub fn complete_path(value: &str, where_: &Paths) -> Option<String> {
    let last = value.rsplit_once(';').map_or(value, |(_, l)| l);
    let frag = last.trim_start();
    if frag.is_empty() {
        return None;
    }
    let exp = where_.expand_user(frag);
    let (dir, base) = match exp.rfind('/') {
        Some(i) => (&exp[..=i], &exp[i + 1..]),
        None => ("", exp.as_str()),
    };
    let dir_trimmed = if dir.len() > 1 {
        dir.trim_end_matches('/')
    } else {
        dir
    };
    let listing = where_
        .cwd
        .join(if dir.is_empty() { "." } else { dir_trimmed });
    let mut names: Vec<String> = std::fs::read_dir(&listing)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let hits: Vec<&String> = names
        .iter()
        .filter(|n| n.starts_with(base) && (base.starts_with('.') || !n.starts_with('.')))
        .collect();
    let first = hits.first()?;
    let mut common: Vec<char> = first.chars().collect();
    for h in &hits[1..] {
        let n = common
            .iter()
            .zip(h.chars())
            .take_while(|(a, b)| **a == *b)
            .count();
        common.truncate(n);
    }
    let mut common: String = common.into_iter().collect();
    if hits.len() == 1 && listing.join(hits[0]).is_dir() {
        common.push('/');
    }
    if common == base {
        return None;
    }
    let suffix: String = common.chars().skip(base.chars().count()).collect();
    Some(format!("{value}{suffix}"))
}

/// `e.strerror or e`: the OS message without Rust's `(os error N)` suffix.
pub fn strerror(e: &std::io::Error) -> String {
    let s = e.to_string();
    match s.rfind(" (os error ") {
        Some(i) if s.ends_with(')') => s[..i].to_string(),
        _ => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(raw: &Value, rules: &FileRules<'_>) -> (Vec<String>, String) {
        check_files(raw, rules, &Paths::default())
    }

    fn complete(value: &str) -> Option<String> {
        complete_path(value, &Paths::default())
    }

    fn s(v: &str) -> Value {
        Value::str(v)
    }

    #[test]
    fn check_files_types_count_and_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let deck = dir.path().join("deck.pptx");
        let pdf = dir.path().join("brief.pdf");
        let empty = dir.path().join("empty.pptx");
        std::fs::write(&deck, b"deck").expect("write");
        std::fs::write(&pdf, b"pdf").expect("write");
        std::fs::write(&empty, b"").expect("write");
        let pptx = vec![".pptx".to_string()];
        let d = deck.to_string_lossy().into_owned();
        let one = FileRules {
            allowed: &pptx,
            max_files: Some(1),
            max_bytes: None,
        };
        let (paths, e) = check(&s(&d), &one);
        assert_eq!(paths, vec![d.clone()]);
        assert_eq!(e, "");
        let types = FileRules {
            allowed: &pptx,
            ..Default::default()
        };
        assert!(
            check(&s(&pdf.to_string_lossy()), &types)
                .1
                .contains("allowed file types are .pptx")
        );
        assert!(
            check(&s(&format!("{d}; {d}")), &one)
                .1
                .contains("at most 1 file")
        );
        let none = FileRules::default();
        let nope = dir.path().join("nope.pptx");
        assert!(
            check(&s(&nope.to_string_lossy()), &none)
                .1
                .contains("not found")
        );
        assert!(
            check(&s(&empty.to_string_lossy()), &none)
                .1
                .contains("empty")
        );
        let big = dir.path().join("big.bin");
        std::fs::write(&big, vec![b'x'; 2_000_001]).expect("write");
        let cap = FileRules {
            max_bytes: Some(2_000_000),
            ..Default::default()
        };
        assert!(
            check(&s(&big.to_string_lossy()), &cap)
                .1
                .contains("2.0 MB is over the 2 MB limit")
        );
        assert!(
            check(&s(&dir.path().to_string_lossy()), &none)
                .1
                .ends_with(": not a file")
        );
    }

    #[test]
    fn list_input_and_blank_parts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = dir.path().join("a.txt");
        std::fs::write(&a, b"a").expect("write");
        let a = a.to_string_lossy().into_owned();
        let (paths, e) = check(&Value::list([s(&a), s("  ")]), &FileRules::default());
        assert_eq!((paths, e), (vec![a.clone()], String::new()));
        let (paths, _) = check(&s(&format!("{a}\n\n;")), &FileRules::default());
        assert_eq!(paths, vec![a]);
    }

    #[test]
    fn path_completion() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("rollout-plan.xlsx"), b"x").expect("write");
        std::fs::write(dir.path().join("rollback.md"), b"x").expect("write");
        std::fs::create_dir(dir.path().join("evidence")).expect("mkdir");
        let base = dir.path().to_string_lossy().into_owned();
        assert_eq!(
            complete(&format!("{base}/rol")),
            Some(format!("{base}/roll"))
        );
        assert_eq!(
            complete(&format!("{base}/rollo")),
            Some(format!("{base}/rollout-plan.xlsx"))
        );
        assert_eq!(
            complete(&format!("a.pdf; {base}/ev")),
            Some(format!("a.pdf; {base}/evidence/"))
        );
        assert_eq!(complete(&format!("{base}/zzz")), None);
        assert_eq!(complete("a.pdf; "), None);
    }

    #[test]
    fn sizes_and_config_limit() {
        assert_eq!(human_size(1000), "1,000 B");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(3 * 1024 * 1024), "3.0 MB");
        let cfg: Map<String, Json> =
            serde_json::from_str(r#"{"forms": {"max_file_mb": "2.5"}}"#).expect("json");
        assert_eq!(max_file_bytes(&cfg), Some(2_500_000));
        let cfg: Map<String, Json> =
            serde_json::from_str(r#"{"forms": {"max_file_mb": "lots"}}"#).expect("json");
        assert_eq!(max_file_bytes(&cfg), None);
        assert_eq!(max_file_bytes(&Map::new()), None);
        // `_max_bytes()` with each `max_file_mb` in the config.
        for (raw, want) in [
            (r#""-1""#, Some(-1_000_000)),
            ("-0.0000001", Some(0)),
            (r#""0""#, Some(0)),
            ("true", Some(1_000_000)),
            (r#"" 2.5 ""#, Some(2_500_000)),
            (r#""nan""#, None),
            ("[1]", None),
            ("1.9999999", Some(1_999_999)),
        ] {
            let cfg: Map<String, Json> =
                serde_json::from_str(&format!(r#"{{"forms": {{"max_file_mb": {raw}}}}}"#))
                    .expect("json");
            assert_eq!(max_file_bytes(&cfg), want, "{raw}");
        }
    }

    #[test]
    fn negative_size_limit_rejects_every_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let one = dir.path().join("one.txt");
        std::fs::write(&one, b"x").expect("write");
        let rules = FileRules {
            max_bytes: Some(-1_000_000),
            ..Default::default()
        };
        assert_eq!(
            check(&s(&one.to_string_lossy()), &rules).1,
            "one.txt: 0.0 MB is over the -1 MB limit"
        );
    }

    #[test]
    fn paths_expand_and_resolve_against_the_given_dirs() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("report.pdf"), b"x").expect("write");
        let p = Paths::new(Some("/home/robin".into()), dir.path());
        assert_eq!(p.expand_user("~/a.pdf"), "/home/robin/a.pdf");
        assert_eq!(p.expand_user("~robin/a.pdf"), "~robin/a.pdf");
        assert_eq!(
            p.abspath("~/x/../a.pdf"),
            PathBuf::from("/home/robin/a.pdf")
        );
        assert_eq!(p.abspath("./report.pdf"), dir.path().join("report.pdf"));
        assert_eq!(p.relpath(&dir.path().join("d/x.yaml")), "d/x.yaml");
        assert_eq!(complete_path("rep", &p), Some("report.pdf".into()));
        let (files, err) = check_files(&Value::str("report.pdf"), &FileRules::default(), &p);
        assert_eq!(err, "");
        assert_eq!(
            files,
            vec![dir.path().join("report.pdf").to_string_lossy().into_owned()]
        );
        assert_eq!(Paths::new(None, "/").expand_user("~/a"), "~/a");
    }

    #[test]
    fn strerror_drops_the_os_error_suffix() {
        let e = std::io::Error::from_raw_os_error(13);
        assert_eq!(strerror(&e), "Permission denied");
    }
}
