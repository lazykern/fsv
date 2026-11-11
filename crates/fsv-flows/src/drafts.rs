//! Change drafts: YAML files, presets, `--set` answers, the annotated
//! editor template, and the local hints file (`drafts.py`).
//!
//! Draft file (`--from`), every key optional:
//!
//! ```yaml
//! preset: acme-normal          # name in <config>/presets/ or a path
//! requester: jane@example.com
//! answers:                     # label or internal name -> value
//!   Change Type: Normal
//!   Services: [Billing, Search]
//! description_md: |            # or description_html: "<p>...</p>"
//!   Rotate the **billing** keys.
//! attachments: [evidence/plan.pdf]
//! planning:                    # planning field label -> files/text
//!   Rollout Plan:
//!     description_md: Steps are in the attached file.
//!     files: [evidence/rollout.xlsx]
//! ack: [all]                   # acknowledge blocking rule messages
//! ```
//!
//! Presets have the same shape; they are partial answers replayed through
//! the engine (never payload fragments), so they keep working when the form
//! changes and are validated like any other answer.
//!
//! User files live in `$FSV_CONFIG_DIR` if set, else
//! `$XDG_CONFIG_HOME/fsv`, else `~/.config/fsv`: `presets/*.yaml` and
//! `hints.yaml`.

use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use serde_json::{Map, Value as Json};

use fsv_core::error::{Error, Result};
use fsv_core::pystr::{split_ws, strip};
use fsv_core::safety::{norm, norm_value};
use fsv_core::yaml;
use fsv_forms::spec::plain_text;
use fsv_forms::{Field, FormSpec, FormState, Kind, Value};
use fsv_py::truthy;

use crate::meta::{form_err, is_description, is_requester, is_status};

pub const DRAFT_KEYS: [&str; 8] = [
    "preset",
    "requester",
    "answers",
    "description_md",
    "description_html",
    "attachments",
    "planning",
    "ack",
];
pub const MAX_CHOICES_SHOWN: usize = 12;

/// `user_dir()` from its inputs: `FSV_CONFIG_DIR`, else
/// `XDG_CONFIG_HOME/fsv`, else `~/.config/fsv`.
pub fn user_dir_from(
    fsv_config_dir: Option<&str>,
    xdg_config_home: Option<&str>,
    home: &Path,
) -> PathBuf {
    if let Some(d) = fsv_config_dir.filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    if let Some(x) = xdg_config_home.filter(|x| !x.is_empty()) {
        return PathBuf::from(x).join("fsv");
    }
    home.join(".config").join("fsv")
}

/// Where presets and `hints.yaml` live. Unlike `config_dir`, this honors
/// `XDG_CONFIG_HOME` (Python reads the environment here).
pub fn user_dir() -> PathBuf {
    let var = |k: &str| std::env::var(k).ok();
    user_dir_from(
        var("FSV_CONFIG_DIR").as_deref(),
        var("XDG_CONFIG_HOME").as_deref(),
        &fsv_core::config::home_dir(),
    )
}

pub fn presets_dir() -> PathBuf {
    user_dir().join("presets")
}

pub fn hints_path() -> PathBuf {
    user_dir().join("hints.yaml")
}

/// A planning field entry of a draft: files to attach and/or text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlanningSpec {
    pub files: Vec<String>,
    pub description_html: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Draft {
    pub answers: Map<String, Json>,
    pub requester: Option<String>,
    pub description_html: Option<String>,
    pub attachments: Vec<String>,
    pub planning: IndexMap<String, PlanningSpec>,
    pub ack: Vec<String>,
    pub preset: Option<String>,
    /// Relative file paths resolve against this.
    pub base_dir: Option<PathBuf>,
}

fn or_str(a: &Option<String>, b: &Option<String>) -> Option<String> {
    match a {
        Some(s) if !s.is_empty() => a.clone(),
        _ => b.clone(),
    }
}

impl Draft {
    /// `other` wins key by key (answers are merged, not replaced).
    pub fn merged(&self, other: &Draft) -> Draft {
        let mut answers = self.answers.clone();
        for (k, v) in &other.answers {
            answers.insert(k.clone(), v.clone());
        }
        let mut planning = self.planning.clone();
        for (k, v) in &other.planning {
            planning.insert(k.clone(), v.clone());
        }
        Draft {
            answers,
            requester: or_str(&other.requester, &self.requester),
            description_html: other
                .description_html
                .clone()
                .or_else(|| self.description_html.clone()),
            attachments: if other.attachments.is_empty() {
                self.attachments.clone()
            } else {
                other.attachments.clone()
            },
            planning,
            ack: self.ack.iter().chain(&other.ack).cloned().collect(),
            preset: or_str(&other.preset, &self.preset),
            base_dir: other.base_dir.clone().or_else(|| self.base_dir.clone()),
        }
    }
}

/// `os.path.expanduser` for `~` and `~/...`.
pub(crate) fn expanduser(s: &str) -> String {
    if s == "~" || s.starts_with("~/") {
        let home = fsv_core::config::home_dir();
        let home = home.to_string_lossy();
        let home = home.trim_end_matches('/');
        return format!("{}{}", if home.is_empty() { "/" } else { home }, &s[1..]);
    }
    s.to_string()
}

/// `str(PurePosixPath(p))`: collapse `//` and `.` segments, keep `..`.
pub(crate) fn pathlib_str(p: &str) -> String {
    let root = if p.starts_with("//") && !p.starts_with("///") {
        "//"
    } else if p.starts_with('/') {
        "/"
    } else {
        ""
    };
    let parts: Vec<&str> = p
        .split('/')
        .filter(|c| !c.is_empty() && *c != ".")
        .collect();
    let out = format!("{root}{}", parts.join("/"));
    if out.is_empty() { ".".to_string() } else { out }
}

/// `posixpath.normpath`.
pub(crate) fn normpath(p: &str) -> String {
    if p.is_empty() {
        return ".".to_string();
    }
    let initial = if p.starts_with("//") && !p.starts_with("///") {
        2
    } else if p.starts_with('/') {
        1
    } else {
        0
    };
    let mut comps: Vec<&str> = Vec::new();
    for c in p.split('/') {
        if c.is_empty() || c == "." {
            continue;
        }
        if c != ".." || (initial == 0 && comps.is_empty()) || comps.last() == Some(&"..") {
            comps.push(c);
        } else if !comps.is_empty() {
            comps.pop();
        }
    }
    let out = format!("{}{}", "/".repeat(initial), comps.join("/"));
    if out.is_empty() { ".".to_string() } else { out }
}

pub(crate) fn abspath(p: &str, cwd: &str) -> String {
    if p.starts_with('/') {
        normpath(p)
    } else {
        normpath(&format!("{cwd}/{p}"))
    }
}

/// `os.path.relpath(path, start)` (`start`: the current directory when `None`).
pub fn relpath(path: &str, start: Option<&str>) -> String {
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "/".into());
    let start = start.unwrap_or(&cwd);
    let s = abspath(start, &cwd);
    let p = abspath(path, &cwd);
    let sl: Vec<&str> = s.split('/').filter(|x| !x.is_empty()).collect();
    let pl: Vec<&str> = p.split('/').filter(|x| !x.is_empty()).collect();
    let i = sl.iter().zip(&pl).take_while(|(a, b)| a == b).count();
    let mut rel: Vec<&str> = vec![".."; sl.len() - i];
    rel.extend(&pl[i..]);
    if rel.is_empty() {
        ".".to_string()
    } else {
        rel.join("/")
    }
}

/// `_path`: a draft file path, `~` expanded, relative paths joined to `base`.
fn draft_path(p: &str, base: Option<&Path>) -> String {
    let s = expanduser(p);
    match base {
        Some(b) if !s.starts_with('/') => pathlib_str(&format!("{}/{s}", b.to_string_lossy())),
        _ => s,
    }
}

/// `str(x)` for the scalars a list of strings may hold (`str`, `int`, `bool`).
fn list_item(v: &Json) -> Option<String> {
    match v {
        Json::String(s) => Some(s.clone()),
        Json::Bool(_) => Some(fsv_py::py_str(v)),
        Json::Number(n) if n.is_i64() || n.is_u64() => Some(n.to_string()),
        _ => None,
    }
}

fn str_list(v: Option<&Json>, what: &str) -> Result<Vec<String>> {
    let bad = || Error::value(format!("{what}: expected a list of strings"));
    match v {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(Json::Array(items)) => items.iter().map(|x| list_item(x).ok_or_else(bad)).collect(),
        Some(other) => list_item(other).map(|s| vec![s]).ok_or_else(bad),
    }
}

/// A draft (or preset) from parsed YAML.
pub fn draft_from_data(data: &Json, base_dir: Option<&Path>, where_: &str) -> Result<Draft> {
    let base = base_dir.map(Path::to_path_buf);
    let obj = match data {
        Json::Null => {
            return Ok(Draft {
                base_dir: base,
                ..Draft::default()
            });
        }
        Json::Object(m) => m,
        _ => {
            return Err(Error::value(format!(
                "{where_}: top level must be a mapping (preset/answers/description_md/...)"
            )));
        }
    };
    let unknown: Vec<&str> = obj
        .keys()
        .map(String::as_str)
        .filter(|k| !DRAFT_KEYS.contains(k))
        .collect();
    if !unknown.is_empty() {
        let mut keys = DRAFT_KEYS.to_vec();
        keys.sort_unstable();
        return Err(Error::value(format!(
            "{where_}: unknown key(s) {}; expected: {}",
            unknown.join(", "),
            keys.join(", ")
        )));
    }
    let get = |k: &str| obj.get(k).filter(|v| !v.is_null());
    let answers = match obj.get("answers").filter(|v| truthy(v)) {
        None => Map::new(),
        Some(Json::Object(m)) => m.clone(),
        Some(_) => {
            return Err(Error::value(format!(
                "{where_}: answers must be a mapping of field label -> value"
            )));
        }
    };
    if get("description_md").is_some() && get("description_html").is_some() {
        return Err(Error::value(format!(
            "{where_}: use description_md or description_html, not both"
        )));
    }
    let mut desc = None;
    if let Some(md) = get("description_md") {
        let md = fsv_py::py_str(md);
        desc = (!strip(&md).is_empty()).then(|| fsv_core::mdlite::to_html(&md));
    } else if let Some(h) = get("description_html") {
        let h = fsv_py::py_str(h);
        if !strip(&h).is_empty() {
            desc = Some(h);
        }
    }
    let mut planning = IndexMap::new();
    let raw_planning = match obj.get("planning").filter(|v| truthy(v)) {
        None => Map::new(),
        Some(Json::Object(m)) => m.clone(),
        Some(_) => {
            return Err(Error::value(format!(
                "{where_}: planning must map a planning field label to files or text"
            )));
        }
    };
    for (label, spec) in &raw_planning {
        let mut entry = PlanningSpec::default();
        let paths = |v: Option<&Json>, what: String| -> Result<Vec<String>> {
            Ok(str_list(v, &what)?
                .iter()
                .map(|p| draft_path(p, base_dir))
                .collect())
        };
        match spec {
            Json::String(s) if s.contains('\n') => {
                return Err(Error::value(format!(
                    "{where_}: planning.{label}: expected files or a mapping"
                )));
            }
            Json::String(_) | Json::Array(_) => {
                entry.files = paths(Some(spec), format!("planning.{label}"))?
            }
            Json::Object(m) => {
                let bad: Vec<&str> = m
                    .keys()
                    .map(String::as_str)
                    .filter(|k| !["files", "description_md", "description_html"].contains(k))
                    .collect();
                if !bad.is_empty() {
                    return Err(Error::value(format!(
                        "{where_}: planning.{label}: unknown key(s) {}",
                        bad.join(", ")
                    )));
                }
                entry.files = paths(m.get("files"), format!("planning.{label}.files"))?;
                if let Some(md) = m.get("description_md").filter(|v| !v.is_null()) {
                    entry.description_html = Some(fsv_core::mdlite::to_html(&fsv_py::py_str(md)));
                } else if let Some(h) = m.get("description_html").filter(|v| !v.is_null()) {
                    entry.description_html = Some(fsv_py::py_str(h));
                }
            }
            _ => {
                return Err(Error::value(format!(
                    "{where_}: planning.{label}: expected files or a mapping"
                )));
            }
        }
        planning.insert(label.clone(), entry);
    }
    let requester = match obj.get("requester") {
        None | Some(Json::Null) => None,
        Some(Json::String(s)) if s.is_empty() => None,
        Some(v) => Some(fsv_py::py_str(v)),
    };
    Ok(Draft {
        answers,
        requester,
        description_html: desc,
        attachments: str_list(obj.get("attachments"), "attachments")?
            .iter()
            .map(|p| draft_path(p, base_dir))
            .collect(),
        planning,
        ack: str_list(obj.get("ack"), "ack")?,
        preset: obj.get("preset").filter(|v| truthy(v)).map(fsv_py::py_str),
        base_dir: base,
    })
}

/// `OSError.strerror` from a Rust I/O error.
pub(crate) fn strerror(e: &std::io::Error) -> String {
    let s = e.to_string();
    match s.rfind(" (os error ") {
        Some(i) => s[..i].to_string(),
        None => s,
    }
}

pub fn load_draft_file(path: &str) -> Result<Draft> {
    let p = PathBuf::from(expanduser(path));
    let text = std::fs::read_to_string(&p)
        .map_err(|e| Error::value(format!("cannot read {path}: {}", strerror(&e))))?;
    let data = yaml::loads(&text)?;
    let parent = p
        .parent()
        .map(Path::to_path_buf)
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("."));
    draft_from_data(&data, Some(&parent), path)
}

/// `Path(name).suffix`.
fn suffix(name: &str) -> &str {
    let base = name.rsplit('/').next().unwrap_or(name);
    match base.rfind('.') {
        Some(i) if i > 0 && i + 1 < base.len() => &base[i..],
        _ => "",
    }
}

/// [`preset_path`] with an explicit presets directory.
pub fn preset_path_in(name: &str, presets: &Path) -> Result<PathBuf> {
    let cand = PathBuf::from(expanduser(name));
    if [".yaml", ".yml"].contains(&suffix(&cand.to_string_lossy()))
        || name.contains('/')
        || cand.exists()
    {
        if !cand.exists() {
            return Err(Error::value(format!("preset file not found: {name}")));
        }
        return Ok(cand);
    }
    for ext in [".yaml", ".yml"] {
        let p = presets.join(format!("{name}{ext}"));
        if p.exists() {
            return Ok(p);
        }
    }
    let mut have: Vec<String> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(presets) {
        let rx = regex::Regex::new(r"^.*\.y.*ml$").expect("glob regex");
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if rx.is_match(&n) {
                let sfx = suffix(&n).len();
                have.push(n[..n.len() - sfx].to_string());
            }
        }
    }
    have.sort();
    let hint = if have.is_empty() {
        format!("; put presets in {}", presets.display())
    } else {
        format!("; available: {}", have.join(", "))
    };
    Err(Error::value(format!(
        "preset {} not found{hint}",
        fsv_py::py_repr_str(name)
    )))
}

/// A preset by name (`<config>/presets/<name>.yaml`) or by path.
pub fn preset_path(name: &str) -> Result<PathBuf> {
    preset_path_in(name, &presets_dir())
}

/// [`load_preset`] with an explicit presets directory.
pub fn load_preset_in(name: &str, presets: &Path) -> Result<Draft> {
    let d = load_draft_file(&preset_path_in(name, presets)?.to_string_lossy())?;
    if d.preset.is_some() {
        return Err(Error::value(format!(
            "preset {}: presets cannot reference other presets",
            fsv_py::py_repr_str(name)
        )));
    }
    Ok(d)
}

pub fn load_preset(name: &str) -> Result<Draft> {
    load_preset_in(name, &presets_dir())
}

/// `--set LABEL=VALUE` expressions as answers.
pub fn parse_sets(exprs: &[String]) -> Result<Map<String, Json>> {
    let mut out = Map::new();
    for expr in exprs {
        let bad = || {
            Error::value(format!(
                "--set needs LABEL=VALUE, got {}",
                fsv_py::py_repr_str(expr)
            ))
        };
        let (k, v) = expr.split_once('=').ok_or_else(bad)?;
        if strip(k).is_empty() {
            return Err(bad());
        }
        out.insert(strip(k).to_string(), Json::String(v.to_string()));
    }
    Ok(out)
}

/// [`resolve_draft`] with an explicit presets directory.
pub fn resolve_draft_in(
    file: Option<&str>,
    preset: Option<&str>,
    sets: &[String],
    extra: Option<&Draft>,
    presets: &Path,
) -> Result<Draft> {
    let d = match file {
        Some(f) if !f.is_empty() => load_draft_file(f)?,
        _ => Draft::default(),
    };
    let name = preset
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .or_else(|| d.preset.clone());
    let base = match name.as_deref().filter(|n| !n.is_empty()) {
        Some(n) => load_preset_in(n, presets)?,
        None => Draft::default(),
    };
    let mut out = base.merged(&d);
    out = out.merged(&Draft {
        answers: parse_sets(sets)?,
        ..Draft::default()
    });
    if let Some(e) = extra {
        out = out.merged(e);
    }
    out.preset = name;
    Ok(out)
}

/// preset < file < `--set` < explicit flags (`extra`).
pub fn resolve_draft(
    file: Option<&str>,
    preset: Option<&str>,
    sets: &[String],
    extra: Option<&Draft>,
) -> Result<Draft> {
    resolve_draft_in(file, preset, sets, extra, &presets_dir())
}

// ---------------------------------------------------------------------------
// annotated YAML template

pub fn kind_text(f: &Field) -> String {
    match f.kind {
        Kind::Datetime => "date-time (YYYY-MM-DD HH:MM, optional +HH:MM)".into(),
        Kind::Date => "date (YYYY-MM-DD)".into(),
        Kind::Checkbox => "yes/no".into(),
        Kind::MultiSelect => "multi-select (list)".into(),
        k if k.as_str().starts_with("lookup") => "lookup (name or email)".into(),
        k => k.as_str().into(),
    }
}

pub fn one_line(text: &str, limit: usize) -> String {
    let s = split_ws(&plain_text(text)).join(" ");
    if s.chars().count() <= limit {
        s
    } else {
        let mut out: String = s.chars().take(limit.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn lookup_text(v: &fsv_forms::LookupValue) -> String {
    match &v.email {
        Some(e) if !e.is_empty() => e.clone(),
        _ if !v.display.is_empty() => v.display.clone(),
        _ => fsv_py::py_str(&v.id),
    }
}

/// `isoformat(timespec="minutes")`.
pub(crate) fn iso_minutes(d: &fsv_forms::pydt::DateTimeValue) -> String {
    let l = d.local;
    format!(
        "{}T{:02}:{:02}{}",
        fsv_forms::pydt::date_iso(l.date()),
        l.hour(),
        l.minute(),
        d.offset_text()
    )
}

/// The value to write after `Label:` (`None`: leave it blank).
pub fn value_for_yaml(state: &FormState, f: &Field) -> Result<Option<Json>> {
    let Some(v) = state.evaluated.values.get(&f.api_name) else {
        return Ok(None);
    };
    if matches!(v, Value::Null)
        || v.as_str() == Some("")
        || matches!(v, Value::List(l) if l.is_empty())
    {
        return Ok(None);
    }
    if f.kind == Kind::MultiSelect {
        let items: Vec<Value> = v
            .as_list()
            .map_or_else(|| vec![v.clone()], <[Value]>::to_vec);
        let mut out = Vec::new();
        for x in &items {
            let ch = f.choice(x).map_err(form_err)?;
            out.push(Json::String(
                ch.map_or_else(|| x.py_str(), |c| c.label.clone()),
            ));
        }
        return Ok(Some(Json::Array(out)));
    }
    if matches!(f.kind, Kind::Dropdown | Kind::Department | Kind::Nested) {
        return Ok(Some(Json::String(
            state.display(&f.api_name, None).map_err(form_err)?,
        )));
    }
    if f.kind == Kind::Checkbox {
        return Ok(Some(Json::Bool(v.truthy())));
    }
    Ok(Some(match v {
        Value::Lookup(lv) => Json::String(lookup_text(lv)),
        Value::List(items) if matches!(items.first(), Some(Value::Lookup(_))) => Json::Array(
            items
                .iter()
                .map(|x| match x {
                    Value::Lookup(lv) => Json::String(lookup_text(lv)),
                    other => Json::String(other.py_str()),
                })
                .collect(),
        ),
        Value::DateTime(d) if f.kind == Kind::Datetime => Json::String(iso_minutes(d)),
        Value::Date(d) if f.kind == Kind::Date => Json::String(fsv_forms::pydt::date_iso(*d)),
        Value::DateTime(d) if f.kind == Kind::Date => Json::String(d.isoformat()),
        other => other.to_json().map_err(form_err)?,
    }))
}

/// Keyword arguments of [`render_template`].
#[derive(Clone, Copy, Default)]
pub struct TemplateOptions<'a> {
    pub title: &'a str,
    pub errors: &'a [String],
    pub draft: Option<&'a Draft>,
    pub requester: Option<&'a str>,
    /// CLI group named in the hints (`changes` when `None`).
    pub module_cli: Option<&'a str>,
    pub unknown: Option<&'a Map<String, Json>>,
    pub hidden: Option<&'a Map<String, Json>>,
    /// Where the draft will be saved; file paths are written relative to
    /// it (default: the current directory), as `--from` reads them.
    pub base_dir: Option<&'a str>,
}

/// The editor draft: visible fields in web order, `Label: value` with
/// `# REQUIRED · type · help` and `# choices: ...` above each. Status is
/// never listed (the server default is used on create).
pub fn render_template(state: &FormState, opts: &TemplateOptions<'_>) -> Result<String> {
    let empty = Draft::default();
    let draft = opts.draft.unwrap_or(&empty);
    let module_cli = opts.module_cli.unwrap_or("changes");
    let rel = |p: &str| relpath(p, opts.base_dir);
    let spec = state.spec.clone();
    let ev = &state.evaluated;
    let mut lines = vec![format!("# {}", opts.title)];
    for e in opts.errors {
        lines.push(format!("# ERROR: {}", split_ws(e).join(" ")));
    }
    lines.extend([
        "# Fill in values after the colons. Lines starting with # are ignored.".to_string(),
        "# Fields appear and disappear with your answers, like the web form; after".to_string(),
        "# saving, new required fields are added and the file reopens if needed.".to_string(),
        format!("# Full choice lists: fsv form fields {module_cli} --choices LABEL"),
        "# Save and close to continue; close without changes to abort.".to_string(),
        String::new(),
    ]);
    if let Some(p) = draft.preset.as_deref().filter(|p| !p.is_empty()) {
        lines.push(format!(
            "# preset {p} is applied: its answers are filled in below"
        ));
    }
    let has_req_field = spec.fields().iter().any(is_requester);
    let requester = opts.requester.filter(|r| !r.is_empty());
    if has_req_field || requester.is_some() {
        lines.push("# REQUIRED · email or exact name of the requester".to_string());
        let who = requester
            .map(str::to_string)
            .or_else(|| draft.requester.clone())
            .filter(|w| !w.is_empty());
        lines.push(match who {
            Some(w) => format!("requester: {}", yaml::scalar_str(&w)),
            None => "requester:".to_string(),
        });
    }
    lines.push("answers:".to_string());
    let mut desc_field: Option<&Field> = None;
    let mut shown_any = false;
    for name in &ev.visible {
        let Some(f) = spec.get(name) else { continue };
        if is_status(f) || is_requester(f) {
            continue;
        }
        if is_description(f) {
            desc_field = Some(f);
            continue;
        }
        if f.kind == Kind::Notice {
            let text = one_line(
                if f.content.is_empty() {
                    &f.label
                } else {
                    &f.content
                },
                300,
            );
            if !text.is_empty() {
                lines.push(format!("  # NOTE: {text}"));
            }
            continue;
        }
        if f.kind == Kind::Attachment {
            continue;
        }
        let mut meta = vec![
            if ev.required.contains(name) {
                "REQUIRED"
            } else {
                "optional"
            }
            .to_string(),
            kind_text(f),
        ];
        if !f.help().is_empty() {
            meta.push(one_line(f.help(), 160));
        }
        if ev.disabled.contains(name) {
            meta.push("read-only (set by a rule)".to_string());
        }
        if ev.populated.contains_key(name) {
            meta.push("filled in by a rule".to_string());
        }
        let mut prefill: Option<String> = None;
        if let Some(r) = ev.restricted.get(name) {
            let rule = if r.rule_name.is_empty() {
                fsv_py::py_str(&r.rule_id)
            } else {
                r.rule_name.clone()
            };
            let offered: Vec<_> = state
                .choices(name)
                .map_err(form_err)?
                .into_iter()
                .filter(|c| c.visible && !c.label.is_empty() && !c.value.is_empty())
                .collect();
            if offered.len() == 1 {
                meta.push(format!("only option allowed by rule {rule}"));
                if value_for_yaml(state, f)?.is_none() {
                    prefill = Some(offered[0].label.clone());
                }
            } else {
                let shown: Vec<&str> = offered
                    .iter()
                    .take(MAX_CHOICES_SHOWN)
                    .map(|c| c.label.as_str())
                    .collect();
                let shown = shown.join(", ");
                meta.push(if offered.len() <= 5 {
                    format!(
                        "choices limited to {} by rule {rule}",
                        if shown.is_empty() { "(none)" } else { &shown }
                    )
                } else {
                    format!("choices limited by rule {rule}")
                });
            }
        }
        lines.push(format!("  # {}", meta.join(" · ")));
        if !f.choices.is_empty() || f.is_nested_level() {
            let pool: Vec<_> = state
                .choices(&f.api_name)
                .map_err(form_err)?
                .into_iter()
                .filter(|c| c.visible)
                .collect();
            let labels: Vec<&str> = pool.iter().map(|c| c.label.as_str()).collect();
            if !labels.is_empty() {
                let shown = labels[..labels.len().min(MAX_CHOICES_SHOWN)].join(" | ");
                let more = if labels.len() > MAX_CHOICES_SHOWN {
                    format!(
                        " | … ({} total; fsv form fields {module_cli} --choices {})",
                        labels.len(),
                        yaml::scalar_str(&f.label)
                    )
                } else {
                    String::new()
                };
                lines.push(format!("  # choices: {shown}{more}"));
                for c in pool.iter().take(MAX_CHOICES_SHOWN) {
                    if !c.description.is_empty() {
                        lines.push(format!(
                            "  #   {} — {}",
                            c.label,
                            one_line(&c.description, 120)
                        ));
                    }
                }
            }
        }
        if !f.placeholder.is_empty() && f.choices.is_empty() {
            lines.push(format!("  # e.g. {}", one_line(&f.placeholder, 80)));
        }
        let value = value_for_yaml(state, f)?;
        let key = if label_unique(&spec, f) {
            &f.label
        } else {
            &f.api_name
        };
        let shown = prefill.map(Json::String).or(value);
        lines.push(kv(key, shown.as_ref()));
        shown_any = true;
    }
    for (k, v) in opts.hidden.into_iter().flatten() {
        lines.push("  # hidden by a rule now: not sent; kept in case it reappears".to_string());
        lines.push(kv(k, Some(v)));
    }
    for (k, v) in opts.unknown.into_iter().flatten() {
        lines.push("  # not a field on this form: fix the label or delete the line".to_string());
        lines.push(kv(k, Some(v)));
    }
    if !shown_any && opts.unknown.is_none_or(Map::is_empty) && opts.hidden.is_none_or(Map::is_empty)
    {
        lines.push("  {}".to_string());
    }
    lines.push(String::new());
    if let Some(df) = desc_field {
        let req = if ev.required.contains(&df.api_name) {
            "REQUIRED"
        } else {
            "optional"
        };
        lines.push(format!(
            "# {req} · {} · Markdown (or use description_html for raw HTML)",
            df.label
        ));
        let desc_val = ev.values.get(&df.api_name).filter(|v| v.truthy());
        if draft.description_html.is_some() || desc_val.is_some() {
            let html_text = match &draft.description_html {
                Some(h) => h.clone(),
                None => desc_val.map(Value::py_str).unwrap_or_default(),
            };
            lines.push(format!("description_html: {}", yaml::block(&html_text, 2)));
        } else {
            lines.push("description_md: |".to_string());
            lines.push("  ".to_string());
        }
        lines.push(String::new());
    }
    lines.push(
        "# Files to attach after the change is created (uploaded only after you confirm)"
            .to_string(),
    );
    let strs = |v: &[String]| -> Vec<Json> { v.iter().map(|p| Json::String(rel(p))).collect() };
    lines.push(format!(
        "attachments: {}",
        yaml::flow_list(&strs(&draft.attachments))
    ));
    if !draft.planning.is_empty() {
        lines.push("planning:".to_string());
        for (label, entry) in &draft.planning {
            lines.push(format!("  {}:", yaml::key(label)));
            lines.push(format!(
                "    files: {}",
                yaml::flow_list(&strs(&entry.files))
            ));
            if let Some(h) = entry.description_html.as_deref().filter(|h| !h.is_empty()) {
                lines.push(format!("    description_html: {}", yaml::block(h, 6)));
            }
        }
    } else {
        lines.push("# planning:            # planning field label -> files".to_string());
        lines.push("#   Rollout Plan: [rollout.xlsx]".to_string());
    }
    let blocking: Vec<_> = ev.messages.iter().filter(|m| m.blocking).collect();
    if !blocking.is_empty() {
        lines.push(String::new());
        for m in blocking {
            lines.push(format!("# BLOCKING MESSAGE: {}", one_line(&m.text, 160)));
        }
        lines.push("# Acknowledge with: ack: [all]".to_string());
        let acks: Vec<Json> = draft.ack.iter().cloned().map(Json::String).collect();
        lines.push(format!("ack: {}", yaml::flow_list(&acks)));
    }
    Ok(lines.join("\n") + "\n")
}

pub fn label_unique(spec: &FormSpec, f: &Field) -> bool {
    spec.fields()
        .iter()
        .filter(|g| g.on_form && norm(&g.label) == norm(&f.label))
        .count()
        == 1
}

/// One `  key: value` answer line.
pub fn kv(k: &str, v: Option<&Json>) -> String {
    let key = yaml::key(k);
    match v {
        None | Some(Json::Null) => format!("  {key}:"),
        Some(Json::Array(items)) => format!("  {key}: {}", yaml::flow_list(items)),
        Some(Json::Bool(_)) => format!("  {key}: {}", yaml::scalar(v.unwrap_or(&Json::Null))),
        Some(other) => {
            let s = fsv_py::py_str(other);
            if s.contains('\n') {
                format!("  {key}: {}", yaml::block(&s, 4))
            } else {
                format!("  {key}: {}", yaml::scalar(other))
            }
        }
    }
}

/// The edited template as a draft (paths relative to `base_dir`, default
/// the current directory).
pub fn parse_template(text: &str, base_dir: Option<&Path>) -> Result<Draft> {
    let data = yaml::loads(text)?;
    let base = match base_dir {
        Some(b) => b.to_path_buf(),
        None => std::env::current_dir()?,
    };
    draft_from_data(&data, Some(&base), "draft")
}

// ---------------------------------------------------------------------------
// hints (local, tenant-specific knowledge the metadata does not carry)

/// One `hints.yaml` entry.
#[derive(Clone, Debug, PartialEq)]
pub struct Hint {
    pub when: Map<String, Json>,
    pub warn: String,
}

/// Entries of `hints.yaml`:
///
/// ```yaml
/// changes:
///   - when: {"Notify Network Team": "Yes"}
///     warn: creates a task for the Acme network team
///   - when: {"Change Type": Normal, "Environment": [Staging, Live]}
///     warn: the release board reviews this change
/// ```
///
/// `when` maps a field label/internal name to a value, a list of values
/// (any of) or `"*"` (any value). All conditions must hold.
pub fn load_hints(section: &str, path: Option<&Path>) -> Result<Vec<Hint>> {
    let p = path.map_or_else(hints_path, Path::to_path_buf);
    if !p.exists() {
        return Ok(Vec::new());
    }
    let data = yaml::load_file(&p)?;
    let data = if truthy(&data) {
        data
    } else {
        Json::Object(Map::new())
    };
    let Json::Object(doc) = data else {
        return Err(Error::value(format!(
            "{}: top level must map a section (e.g. changes) to a list",
            p.display()
        )));
    };
    let entries = match doc.get(section).filter(|v| truthy(v)) {
        None => Vec::new(),
        Some(Json::Array(a)) => a.clone(),
        Some(_) => {
            return Err(Error::value(format!(
                "{}: {section} must be a list of {{when, warn}} entries",
                p.display()
            )));
        }
    };
    let mut out = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        let when = e.get("when").and_then(Json::as_object);
        let warn = e.get("warn").filter(|w| truthy(w));
        match (e.is_object(), when, warn) {
            (true, Some(when), Some(warn)) => out.push(Hint {
                when: when.clone(),
                warn: fsv_py::py_str(warn),
            }),
            _ => {
                return Err(Error::value(format!(
                    "{}: {section}[{i}] needs 'when' (mapping) and 'warn' (text)",
                    p.display()
                )));
            }
        }
    }
    Ok(out)
}

/// -> (warnings that apply, problems with the hints file).
pub fn matching_hints(state: &FormState, hints: &[Hint]) -> Result<(Vec<String>, Vec<String>)> {
    let sent = state.payload_values();
    let mut out = Vec::new();
    let mut problems = Vec::new();
    for h in hints {
        let mut ok = true;
        for (k, want) in &h.when {
            let Some(f) = state.spec.find(k) else {
                problems.push(format!("hints: unknown field {}", fsv_py::py_repr_str(k)));
                ok = false;
                break;
            };
            if !sent.contains_key(&f.api_name) {
                ok = false;
                break;
            }
            if want.as_str() == Some("*") {
                continue;
            }
            let display = state.display(&f.api_name, None).map_err(form_err)?;
            let mut have: Vec<String> = display.split(", ").map(norm).collect();
            have.push(norm(&display));
            let wants: Vec<Json> = match want {
                Json::Array(a) => a.clone(),
                other => vec![other.clone()],
            };
            let hit = wants.iter().any(|w| {
                let w = match w {
                    Json::Bool(b) => Json::String(if *b { "Yes" } else { "No" }.into()),
                    other => other.clone(),
                };
                have.contains(&norm_value(&w))
            });
            if !hit {
                ok = false;
                break;
            }
        }
        if ok {
            out.push(h.warn.clone());
        }
    }
    Ok((out, problems))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{BASE, acme_meta, golden};
    use serde_json::json;

    fn draft_json(d: &Draft) -> Json {
        let planning: Map<String, Json> = d
            .planning
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    json!({"files": v.files, "description_html": v.description_html}),
                )
            })
            .collect();
        json!({"answers": d.answers, "requester": d.requester, "description_html": d.description_html,
               "attachments": d.attachments, "planning": planning, "ack": d.ack, "preset": d.preset})
    }

    #[test]
    fn golden_draft_cases() {
        for case in golden(&["drafts"]).as_array().unwrap() {
            let text = case["text"].as_str().unwrap();
            if text.contains("~/")
                && fsv_core::config::home_dir().to_string_lossy() != "/Users/phusit.somb"
            {
                continue;
            }
            match parse_template(text, Some(Path::new(BASE))) {
                Ok(d) => {
                    assert_eq!(draft_json(&d), case["draft"], "{text}");
                    assert_eq!(
                        fsv_py::canonical(&draft_json(&d)),
                        fsv_py::canonical(&case["draft"])
                    );
                }
                Err(e) => assert_eq!(
                    e.to_string(),
                    case["error"].as_str().unwrap_or("<ok>"),
                    "{text}"
                ),
            }
        }
    }

    #[test]
    fn plain_bool_and_null_keys_are_python_str() {
        let d = parse_template(
            "answers:\n  true: a\n  null: b\n  \"false\": c\nplanning:\n  TRUE: x.txt\n",
            Some(Path::new(BASE)),
        )
        .unwrap();
        assert_eq!(
            d.answers.keys().collect::<Vec<_>>(),
            ["True", "None", "false"]
        );
        assert_eq!(d.planning.keys().collect::<Vec<_>>(), ["True"]);
        let expected = "ack, answers, attachments, description_html, description_md, planning, preset, requester";
        for (text, key) in [("true: 1", "True"), ("null: 1", "None")] {
            assert_eq!(
                parse_template(text, None).unwrap_err().to_string(),
                format!("draft: unknown key(s) {key}; expected: {expected}")
            );
        }
    }

    #[test]
    fn user_dir_precedence() {
        let home = Path::new("/home/u");
        assert_eq!(
            user_dir_from(Some("/c"), Some("/x"), home),
            PathBuf::from("/c")
        );
        assert_eq!(
            user_dir_from(None, Some("/x"), home),
            PathBuf::from("/x/fsv")
        );
        assert_eq!(
            user_dir_from(Some(""), None, home),
            PathBuf::from("/home/u/.config/fsv")
        );
    }

    #[test]
    fn presets_dir_and_hints_path_under_user_dir() {
        assert_eq!(presets_dir(), user_dir().join("presets"));
        assert_eq!(hints_path(), user_dir().join("hints.yaml"));
    }

    #[test]
    fn merged_other_wins_key_by_key() {
        let mut a = Draft {
            requester: Some("a@x".into()),
            attachments: vec!["a".into()],
            ack: vec!["1".into()],
            ..Draft::default()
        };
        a.answers.insert("A".into(), json!(1));
        a.answers.insert("B".into(), json!(2));
        let mut b = Draft {
            requester: Some(String::new()),
            ack: vec!["2".into()],
            preset: Some("p".into()),
            ..Draft::default()
        };
        b.answers.insert("A".into(), json!(3));
        b.answers.insert("C".into(), json!(4));
        let m = a.merged(&b);
        assert_eq!(
            Json::Object(m.answers.clone()),
            json!({"A": 3, "B": 2, "C": 4})
        );
        assert_eq!(m.answers.keys().collect::<Vec<_>>(), ["A", "B", "C"]);
        assert_eq!(m.requester.as_deref(), Some("a@x"));
        assert_eq!(m.attachments, ["a"]);
        assert_eq!(m.ack, ["1", "2"]);
        assert_eq!(m.preset.as_deref(), Some("p"));
    }

    #[test]
    fn paths_join_and_normalize_like_pathlib() {
        assert_eq!(
            draft_path("./a/../b.pdf", Some(Path::new("/work/drafts"))),
            "/work/drafts/a/../b.pdf"
        );
        assert_eq!(
            draft_path("/abs/c.pdf", Some(Path::new("/w"))),
            "/abs/c.pdf"
        );
        assert_eq!(draft_path("a.pdf", Some(Path::new("."))), "a.pdf");
        assert_eq!(draft_path("x//y", None), "x//y");
        assert_eq!(pathlib_str("//a/./b/"), "//a/b");
        assert_eq!(normpath("/a/../../b"), "/b");
        assert_eq!(normpath("../a/./.."), "..");
    }

    #[test]
    fn str_list_accepts_strings_ints_bools() {
        assert_eq!(
            str_list(Some(&json!(["a", 3, true])), "x").unwrap(),
            ["a", "3", "True"]
        );
        assert_eq!(str_list(Some(&json!("a")), "x").unwrap(), ["a"]);
        assert_eq!(str_list(None, "x").unwrap(), Vec::<String>::new());
        assert_eq!(
            str_list(Some(&json!([1.5])), "ack")
                .unwrap_err()
                .to_string(),
            "ack: expected a list of strings"
        );
        assert!(str_list(Some(&json!({"a": 1})), "x").is_err());
    }

    #[test]
    fn draft_from_data_null_and_where() {
        let d = draft_from_data(&Json::Null, Some(Path::new("/b")), "x.yaml").unwrap();
        assert_eq!(d.base_dir, Some(PathBuf::from("/b")));
        let e = draft_from_data(&json!(["a"]), None, "x.yaml").unwrap_err();
        assert_eq!(
            e.to_string(),
            "x.yaml: top level must be a mapping (preset/answers/description_md/...)"
        );
    }

    #[test]
    fn load_draft_file_resolves_against_its_directory() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("d.yaml");
        std::fs::write(&p, "attachments: [a.pdf]\nanswers: {Subject: x}\n").unwrap();
        let d = load_draft_file(&p.to_string_lossy()).unwrap();
        assert_eq!(d.attachments, [format!("{}/a.pdf", dir.path().display())]);
        assert_eq!(d.base_dir.as_deref(), Some(dir.path()));
        let missing = dir.path().join("nope.yaml");
        let e = load_draft_file(&missing.to_string_lossy()).unwrap_err();
        assert_eq!(
            e.to_string(),
            format!(
                "cannot read {}: No such file or directory",
                missing.display()
            )
        );
        std::fs::write(&p, "colour: x\n").unwrap();
        let e = load_draft_file(&p.to_string_lossy())
            .unwrap_err()
            .to_string();
        assert!(
            e.starts_with(&format!("{}: unknown key(s) colour", p.display())),
            "{e}"
        );
    }

    #[test]
    fn presets_by_name_path_and_errors() {
        let dir = tempfile::tempdir().unwrap();
        let presets = dir.path().join("presets");
        let e = preset_path_in("acme", &presets).unwrap_err().to_string();
        assert_eq!(
            e,
            format!(
                "preset 'acme' not found; put presets in {}",
                presets.display()
            )
        );
        std::fs::create_dir(&presets).unwrap();
        std::fs::write(presets.join("acme.yml"), "answers: {Change Type: Normal}\n").unwrap();
        std::fs::write(presets.join("bad.yaml"), "preset: acme\n").unwrap();
        assert_eq!(
            preset_path_in("acme", &presets).unwrap(),
            presets.join("acme.yml")
        );
        assert_eq!(
            preset_path_in("nope", &presets).unwrap_err().to_string(),
            "preset 'nope' not found; available: acme, bad"
        );
        assert_eq!(
            preset_path_in("x.yaml", &presets).unwrap_err().to_string(),
            "preset file not found: x.yaml"
        );
        let explicit = presets.join("acme.yml");
        assert_eq!(
            preset_path_in(&explicit.to_string_lossy(), &presets).unwrap(),
            explicit
        );
        let d = load_preset_in("acme", &presets).unwrap();
        assert_eq!(d.answers["Change Type"], json!("Normal"));
        assert_eq!(
            load_preset_in("bad", &presets).unwrap_err().to_string(),
            "preset 'bad': presets cannot reference other presets"
        );
        assert!(preset_path("definitely-not-a-preset-name").is_err());
        assert!(load_preset("definitely-not-a-preset-name").is_err());
    }

    #[test]
    fn parse_sets_label_value() {
        let got = parse_sets(&[" A =1=2".into(), "B=".into()]).unwrap();
        assert_eq!(Json::Object(got), json!({"A": "1=2", "B": ""}));
        assert_eq!(
            parse_sets(&["x".into()]).unwrap_err().to_string(),
            "--set needs LABEL=VALUE, got 'x'"
        );
        assert!(parse_sets(&[" =v".into()]).is_err());
    }

    #[test]
    fn resolve_draft_precedence() {
        let dir = tempfile::tempdir().unwrap();
        let presets = dir.path().join("presets");
        std::fs::create_dir(&presets).unwrap();
        std::fs::write(
            presets.join("acme.yaml"),
            "answers: {Change Type: Normal, Risk: Low}\nack: [p]\n",
        )
        .unwrap();
        let f = dir.path().join("d.yaml");
        std::fs::write(&f, "preset: acme\nanswers: {Risk: High, Impact: Low}\n").unwrap();
        let mut extra = Draft::default();
        extra.answers.insert("Impact".into(), json!("High"));
        let d = resolve_draft_in(
            Some(&f.to_string_lossy()),
            None,
            &["Risk=Medium".into()],
            Some(&extra),
            &presets,
        )
        .unwrap();
        assert_eq!(
            Json::Object(d.answers.clone()),
            json!({"Change Type": "Normal", "Risk": "Medium", "Impact": "High"})
        );
        assert_eq!(d.preset.as_deref(), Some("acme"));
        assert_eq!(d.ack, ["p"]);
        let d = resolve_draft(None, None, &["A=1".into()], None).unwrap();
        assert_eq!(Json::Object(d.answers), json!({"A": "1"}));
    }

    #[test]
    fn kind_text_names() {
        let m = acme_meta("change", "new_form");
        let spec = m.spec();
        let k = |n: &str| kind_text(spec.get(n).unwrap());
        assert_eq!(
            k("staging_start"),
            "date-time (YYYY-MM-DD HH:MM, optional +HH:MM)"
        );
        assert_eq!(k("legacy_flag"), "yes/no");
        assert_eq!(k("services"), "multi-select (list)");
        assert_eq!(k("reviewer"), "lookup (name or email)");
        assert_eq!(k("subject"), "text");
    }

    #[test]
    fn one_line_collapses_and_truncates() {
        assert_eq!(one_line("<p>a\n  b</p>", 160), "a b");
        assert_eq!(one_line("abcdef", 4), "abc…");
        assert_eq!(one_line("", 4), "");
    }

    #[test]
    fn golden_relpath_kv_scalar() {
        for c in golden(&["relpath"]).as_array().unwrap() {
            assert_eq!(
                relpath(c[0].as_str().unwrap(), c[1].as_str()),
                c[2].as_str().unwrap()
            );
        }
        for c in golden(&["yaml_kv"]).as_array().unwrap() {
            assert_eq!(
                kv(c[0].as_str().unwrap(), Some(&c[1])),
                c[2].as_str().unwrap(),
                "{c}"
            );
        }
        for c in golden(&["scalar"]).as_array().unwrap() {
            assert_eq!(yaml::scalar(&c[0]), c[1].as_str().unwrap());
        }
    }

    #[test]
    fn value_for_yaml_and_label_unique() {
        let m = acme_meta("change", "new_form");
        let mut st = m.state("new", None, None).unwrap();
        st.set("services", Value::from("Billing, Search")).unwrap();
        st.set("cloud_only", Value::from("No")).unwrap();
        st.set("staging_start", Value::from("2030-11-18 09:00"))
            .unwrap();
        let spec = m.spec();
        let f = |n: &str| spec.get(n).unwrap();
        assert_eq!(
            value_for_yaml(&st, f("services")).unwrap(),
            Some(json!(["Billing", "Search"]))
        );
        assert_eq!(
            value_for_yaml(&st, f("priority")).unwrap(),
            Some(json!("Low"))
        );
        assert_eq!(
            value_for_yaml(&st, f("staging_start")).unwrap(),
            Some(json!("2030-11-18T09:00+09:00"))
        );
        assert_eq!(value_for_yaml(&st, f("subject")).unwrap(), None);
        assert!(label_unique(&spec, f("subject")));
    }

    #[test]
    fn render_template_matches_golden_create() {
        let full = golden(&["create", "full"]);
        let m = acme_meta("change", "new_form");
        let d = parse_template(full["draft"].as_str().unwrap(), Some(Path::new(BASE))).unwrap();
        let mut st = m.state("new", None, None).unwrap();
        crate::meta::apply_answers(&mut st, &d.answers, None).unwrap();
        st.set(
            "description",
            Value::from(d.description_html.clone().unwrap()),
        )
        .unwrap();
        let text = render_template(
            &st,
            &TemplateOptions {
                title: "New change",
                draft: Some(&d),
                requester: Some("jane.doe@example.com"),
                base_dir: Some(BASE),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(text, full["template"].as_str().unwrap());
    }

    #[test]
    fn hints_load_and_match() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("hints.yaml");
        assert!(load_hints("changes", Some(&p)).unwrap().is_empty());
        std::fs::write(&p, golden(&["hints_yaml"]).as_str().unwrap()).unwrap();
        let hints = load_hints("changes", Some(&p)).unwrap();
        assert_eq!(hints.len(), 4);
        assert_eq!(hints[1].when["Services"], json!(["Search", "Mail"]));
        let m = acme_meta("change", "new_form");
        let mut st = m.state("new", None, None).unwrap();
        st.set("cloud_only", Value::from("No")).unwrap();
        st.set("services", Value::from("Mail")).unwrap();
        let (warn, problems) = matching_hints(&st, &hints).unwrap();
        assert_eq!(warn, ["staging runs first", "the search team is paged"]);
        assert_eq!(problems, ["hints: unknown field 'Nope Field'"]);
        std::fs::write(&p, "changes: {a: 1}\n").unwrap();
        assert!(
            load_hints("changes", Some(&p))
                .unwrap_err()
                .to_string()
                .ends_with("changes must be a list of {when, warn} entries")
        );
        std::fs::write(&p, "changes:\n  - when: x\n").unwrap();
        assert!(
            load_hints("changes", Some(&p))
                .unwrap_err()
                .to_string()
                .ends_with("changes[0] needs 'when' (mapping) and 'warn' (text)")
        );
        std::fs::write(&p, "- a\n").unwrap();
        assert!(
            load_hints("changes", Some(&p))
                .unwrap_err()
                .to_string()
                .ends_with("top level must map a section (e.g. changes) to a list")
        );
    }

    #[test]
    fn expanduser_home() {
        let home = fsv_core::config::home_dir().to_string_lossy().into_owned();
        assert_eq!(
            expanduser("~/a"),
            format!("{}/a", home.trim_end_matches('/'))
        );
        assert_eq!(expanduser("a~/b"), "a~/b");
    }

    /// `drafts._path(p, base)`; expected values from Python with
    /// `HOME=/home/u` (here the home dir is substituted).
    #[test]
    fn draft_path_matches_python_path() {
        let home = fsv_core::config::home_dir().to_string_lossy().into_owned();
        let home = home.trim_end_matches('/');
        let w = Some(Path::new("/w"));
        let cases: [(&str, Option<&Path>, String); 7] = [
            ("~/r.pdf", w, format!("{home}/r.pdf")),
            ("~", w, home.to_string()),
            (
                "b/./c//d.txt",
                Some(Path::new("/w/")),
                "/w/b/c/d.txt".into(),
            ),
            ("", w, "/w".into()),
            ("", None, String::new()),
            ("x/", w, "/w/x".into()),
            ("~/a//b", None, format!("{home}/a//b")),
        ];
        for (p, base, want) in cases {
            assert_eq!(draft_path(p, base), want, "{p:?} {base:?}");
        }
    }
}
