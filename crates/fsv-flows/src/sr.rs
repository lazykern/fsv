//! Service-request drafts (`cli_sr.py`): the annotated YAML template for a
//! catalog item, its parser, and preset < file < `--set` < `--attach`.
//!
//! A service-request draft is a change draft plus `requested_for:` (other
//! draft readers reject that key), so it is an [`SrDraft`] wrapping a
//! [`Draft`]. [`SrDraft::catalog_draft`] is what `build_sr_plan` reads.

use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use serde_json::{Map, Value as Json, json};

use fsv_api::catalog::{self, SRPlan};
use fsv_core::error::{Error, Result};
use fsv_core::pystr::{split_ws, strip};
use fsv_core::safety::norm;
use fsv_core::yaml;
use fsv_forms::{Field, FormSpec, Kind, Value};
use fsv_py::{py_repr_str, py_str};

use crate::drafts::{
    Draft, MAX_CHOICES_SHOWN, abspath, answer_kv, draft_from_data, expanduser, kind_text, kv,
    label_unique, one_line, parse_sets, preset_path_in, presets_dir, relpath, strerror,
    value_for_yaml,
};
use crate::meta::form_err;

/// The keys of a service-request draft: [`DRAFT_KEYS`] and `requested_for`.
pub const SR_DRAFT_KEYS: [&str; 9] = [
    "preset",
    "requester",
    "answers",
    "description_md",
    "description_html",
    "attachments",
    "planning",
    "ack",
    "requested_for",
];

/// A draft for `fsv sr create` (`fsv.drafts.Draft` read with
/// `allow_requested_for=True`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SrDraft {
    pub draft: Draft,
    /// Exact email or name of the person receiving the request.
    pub requested_for: Option<String>,
}

impl SrDraft {
    /// `other` wins key by key; `requested_for` when it is set at all.
    pub fn merged(&self, other: &SrDraft) -> SrDraft {
        SrDraft {
            draft: self.draft.merged(&other.draft),
            requested_for: other
                .requested_for
                .clone()
                .or_else(|| self.requested_for.clone()),
        }
    }

    /// The parts `catalog::build_sr_plan` reads.
    pub fn catalog_draft(&self) -> catalog::Draft {
        let d = &self.draft;
        catalog::Draft {
            answers: d.answers.clone(),
            description_html: d.description_html.clone(),
            attachments: d.attachments.clone(),
            planning: d
                .planning
                .iter()
                .map(|(k, p)| {
                    (
                        k.clone(),
                        json!({"files": p.files, "description_html": p.description_html}),
                    )
                })
                .collect(),
            ack: d.ack.clone(),
        }
    }
}

/// `draft_from_data(data, base_dir, where, allow_requested_for=True)`.
pub fn sr_draft_from_data(data: &Json, base_dir: Option<&Path>, where_: &str) -> Result<SrDraft> {
    let Json::Object(obj) = data else {
        return Ok(SrDraft {
            draft: draft_from_data(data, base_dir, where_)?,
            requested_for: None,
        });
    };
    let unknown: Vec<&str> = obj
        .keys()
        .map(String::as_str)
        .filter(|k| !SR_DRAFT_KEYS.contains(k))
        .collect();
    if !unknown.is_empty() {
        let mut keys = SR_DRAFT_KEYS.to_vec();
        keys.sort_unstable();
        return Err(Error::value(format!(
            "{where_}: unknown key(s) {}; expected: {}",
            unknown.join(", "),
            keys.join(", ")
        )));
    }
    let requested_for = match obj.get("requested_for") {
        None | Some(Json::Null) => None,
        Some(Json::String(s)) => Some(strip(s).to_string()).filter(|s| !s.is_empty()),
        Some(_) => {
            return Err(Error::value(format!(
                "{where_}: requested_for must be an email or exact name"
            )));
        }
    };
    let mut rest = obj.clone();
    rest.shift_remove("requested_for");
    Ok(SrDraft {
        draft: draft_from_data(&Json::Object(rest), base_dir, where_)?,
        requested_for,
    })
}

/// `load_draft_file(path, allow_requested_for=True)`.
pub fn load_sr_draft_file(path: &str) -> Result<SrDraft> {
    let p = PathBuf::from(expanduser(path));
    let text = std::fs::read_to_string(&p)
        .map_err(|e| Error::value(format!("cannot read {path}: {}", strerror(&e))))?;
    let data = yaml::loads(&text)?;
    let parent = p
        .parent()
        .map(Path::to_path_buf)
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("."));
    sr_draft_from_data(&data, Some(&parent), path)
}

/// [`load_sr_preset`] with an explicit presets directory.
pub fn load_sr_preset_in(name: &str, presets: &Path) -> Result<SrDraft> {
    let d = load_sr_draft_file(&preset_path_in(name, presets)?.to_string_lossy())?;
    if d.draft.preset.is_some() {
        return Err(Error::value(format!(
            "preset {}: presets cannot reference other presets",
            py_repr_str(name)
        )));
    }
    Ok(d)
}

/// `load_preset(name, allow_requested_for=True)`.
pub fn load_sr_preset(name: &str) -> Result<SrDraft> {
    load_sr_preset_in(name, &presets_dir())
}

/// `parse_sr_template`: the edited template as a draft (paths relative to
/// `base_dir`, default the current directory).
pub fn parse_sr_template(text: &str, base_dir: Option<&Path>) -> Result<SrDraft> {
    let data = yaml::loads(text)?;
    let base = match base_dir {
        Some(b) => b.to_path_buf(),
        None => std::env::current_dir()?,
    };
    sr_draft_from_data(&data, Some(&base), "draft")
}

fn file_answers(
    spec: &FormSpec,
    answers: &Map<String, Json>,
    base: Option<&Path>,
) -> Map<String, Json> {
    let base = base.map(|b| b.to_string_lossy().into_owned());
    catalog::file_answers(spec, answers, base.as_deref())
}

/// [`resolve_sr_draft`] with explicit presets and current directories.
pub fn resolve_sr_draft_in(
    spec: &FormSpec,
    file: Option<&str>,
    preset: Option<&str>,
    sets: &[String],
    attach: &[String],
    presets: &Path,
    cwd: &Path,
) -> Result<SrDraft> {
    let mut d = match file {
        Some(f) if !f.is_empty() => load_sr_draft_file(f)?,
        _ => SrDraft::default(),
    };
    let name = preset
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .or_else(|| d.draft.preset.clone());
    let mut base = match name.as_deref().filter(|n| !n.is_empty()) {
        Some(n) => load_sr_preset_in(n, presets)?,
        None => SrDraft::default(),
    };
    base.draft.answers = file_answers(spec, &base.draft.answers, base.draft.base_dir.as_deref());
    d.draft.answers = file_answers(spec, &d.draft.answers, d.draft.base_dir.as_deref());
    let sets_d = SrDraft {
        draft: Draft {
            answers: file_answers(spec, &parse_sets(sets)?, Some(cwd)),
            ..Draft::default()
        },
        requested_for: None,
    };
    let mut out = base.merged(&d).merged(&sets_d);
    let cwd = cwd.to_string_lossy();
    out.draft
        .attachments
        .extend(attach.iter().map(|p| abspath(&expanduser(p), &cwd)));
    out.draft.preset = name;
    Ok(out)
}

/// `resolve_sr_draft`: preset < file < `--set` < `--attach`. File-field
/// paths resolve against the draft (or preset) file's folder, `--set` paths
/// against the current directory.
pub fn resolve_sr_draft(
    spec: &FormSpec,
    file: Option<&str>,
    preset: Option<&str>,
    sets: &[String],
    attach: &[String],
) -> Result<SrDraft> {
    resolve_sr_draft_in(
        spec,
        file,
        preset,
        sets,
        attach,
        &presets_dir(),
        &std::env::current_dir()?,
    )
}

/// `_reveals`: normalised choice value -> labels of the fields it reveals.
pub fn reveals(spec: &FormSpec, f: &Field) -> IndexMap<String, Vec<String>> {
    let mut out: IndexMap<String, Vec<String>> = IndexMap::new();
    for s in &f.sections {
        for ch in &s.choices {
            out.entry(ch.clone()).or_default().extend(
                s.fields
                    .iter()
                    .filter_map(|n| spec.get(n))
                    .map(|g| g.label.clone()),
            );
        }
    }
    out
}

/// Keyword arguments of [`render_sr_template`].
#[derive(Clone, Copy, Default)]
pub struct SrTemplateOptions<'a> {
    /// Shown as `# ERROR:` lines at the top.
    pub errors: &'a [String],
    /// `--requester`: wins over the plan's requester and the draft's.
    pub requester: Option<&'a str>,
    /// The draft the plan was built from, for `requester:` and
    /// `requested_for:` (answers, attachments and `ack` come from
    /// `plan.draft`).
    pub draft: Option<&'a SrDraft>,
    /// Where the draft will be saved; file paths are written relative to
    /// it (default: the current directory).
    pub base_dir: Option<&'a str>,
}

fn non_empty(s: Option<&str>) -> Option<&str> {
    s.filter(|s| !s.is_empty())
}

/// `render_sr_template`: the annotated YAML draft for an item: visible
/// fields in web order with `# REQUIRED · type · help` and choices above
/// each; file fields take paths (relative to `base_dir`); general
/// attachments and the requester at the top level.
pub fn render_sr_template(plan: &SRPlan, opts: &SrTemplateOptions<'_>) -> Result<String> {
    let state = &plan.state;
    let draft = &plan.draft;
    let rel = |p: &str| relpath(p, opts.base_dir);
    let spec = state.spec.clone();
    let item = spec
        .item
        .as_ref()
        .ok_or_else(|| Error::other("not a service-item form"))?;
    let ev = &state.evaluated;
    let r = py_str(&plan.meta.display_id());
    let mut lines = vec![format!(
        "# New service request: {} (catalog item {r})",
        item.name
    )];
    for e in opts.errors {
        lines.push(format!("# ERROR: {}", split_ws(e).join(" ")));
    }
    lines.extend([
        "# Fill in values after the colons. Lines starting with # are ignored.".to_string(),
        "# Fields appear with your answers, like the web form; after saving, new".to_string(),
        "# required fields are added and the file reopens if needed.".to_string(),
        format!("# Full item form: fsv sr fields {r}"),
        "# Save and close to continue; close without changes to abort.".to_string(),
        format!("# {}", catalog::SAFETY),
        format!("# {}", catalog::REQUESTED_FOR_NOTE),
        String::new(),
        "# REQUIRED · email or exact name of the requester (approvals route via their manager)"
            .to_string(),
    ]);
    let sr = opts.draft;
    let who = non_empty(opts.requester)
        .or_else(|| non_empty(plan.requester.as_ref().and_then(|r| r.email.as_deref())))
        .or_else(|| non_empty(sr.and_then(|d| d.draft.requester.as_deref())));
    lines.push(match who {
        Some(w) => format!("requester: {}", yaml::scalar_str(w)),
        None => "requester:".to_string(),
    });
    lines.push("# Optional · exact email or name of the person receiving this request".to_string());
    lines.push(
        match non_empty(sr.and_then(|d| d.requested_for.as_deref())) {
            Some(w) => format!("requested_for: {}", yaml::scalar_str(w)),
            None => "requested_for:".to_string(),
        },
    );
    lines.push("answers:".to_string());
    let mut shown = false;
    for name in &ev.visible {
        let Some(f) = spec.get(name) else { continue };
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
        let key = if label_unique(&spec, f) {
            &f.label
        } else {
            &f.api_name
        };
        let mut meta_bits = vec![
            if ev.required.contains(name) {
                "REQUIRED"
            } else {
                "optional"
            }
            .to_string(),
        ];
        if f.kind == Kind::Attachment {
            meta_bits.push("file(s): paths relative to this draft".to_string());
            if !f.allowed_file_types.is_empty() {
                meta_bits.push(format!("types {}", f.allowed_file_types.join(", ")));
            }
            if let Some(n) = f.max_files.filter(|n| *n != 0) {
                meta_bits.push(format!("at most {n} file(s)"));
            }
            lines.push(format!("  # {}", meta_bits.join(" · ")));
            let have: Vec<Json> = match ev.values.get(name) {
                Some(Value::List(items)) => items.iter().map(|p| json!(rel(&p.py_str()))).collect(),
                Some(v) if v.truthy() => vec![json!(rel(&v.py_str()))],
                _ => Vec::new(),
            };
            lines.push(format!("  {}: {}", yaml::key(key), yaml::flow_list(&have)));
            shown = true;
            continue;
        }
        meta_bits.push(kind_text(f));
        if !f.help().is_empty() {
            meta_bits.push(one_line(f.help(), 160));
        }
        if ev.disabled.contains(name) {
            meta_bits.push("read-only (set by a rule)".to_string());
        }
        lines.push(format!("  # {}", meta_bits.join(" · ")));
        if !f.choices.is_empty() {
            let pool: Vec<_> = state
                .choices(name)
                .map_err(form_err)?
                .into_iter()
                .filter(|c| c.visible)
                .collect();
            let labels: Vec<&str> = pool.iter().map(|c| c.label.as_str()).collect();
            let more = if labels.len() > MAX_CHOICES_SHOWN {
                format!(" | … ({} total; fsv sr fields {r})", labels.len())
            } else {
                String::new()
            };
            lines.push(format!(
                "  # choices: {}{more}",
                labels[..labels.len().min(MAX_CHOICES_SHOWN)].join(" | ")
            ));
            for c in pool.iter().take(MAX_CHOICES_SHOWN) {
                if !c.description.is_empty() {
                    lines.push(format!(
                        "  #   {} — {}",
                        c.label,
                        one_line(&c.description, 120)
                    ));
                }
            }
            let rv = reveals(&spec, f);
            for c in pool.iter().take(MAX_CHOICES_SHOWN) {
                if let Some(adds) = rv.get(&norm(&c.value)).filter(|a| !a.is_empty()) {
                    lines.push(format!("  #   {} → adds: {}", c.label, adds.join(", ")));
                }
            }
        }
        if !f.placeholder.is_empty() && f.choices.is_empty() {
            lines.push(format!("  # e.g. {}", one_line(&f.placeholder, 80)));
        }
        lines.push(kv(key, value_for_yaml(state, f)?.as_ref()));
        shown = true;
    }
    for (k, v) in &draft.answers {
        match spec.find(k) {
            None => {
                lines.push(
                    "  # not a field on this item: fix the label or delete the line".to_string(),
                );
                lines.push(answer_kv(k, v));
                shown = true;
            }
            Some(f) if !ev.visible.contains(&f.api_name) && f.kind != Kind::Notice => {
                lines.push(
                    "  # hidden now (its section's choice is not selected): not sent".to_string(),
                );
                lines.push(answer_kv(k, v));
                shown = true;
            }
            Some(_) => {}
        }
    }
    if !shown {
        lines.push("  {}".to_string());
    }
    lines.push(String::new());
    if item.allow_attachments {
        let mandatory = ev.attachments_required.unwrap_or(item.attachment_mandatory);
        lines.push(format!(
            "# {} · general attachments for the request (uploaded only after you confirm)",
            if mandatory { "REQUIRED" } else { "optional" }
        ));
        let files: Vec<Json> = draft.attachments.iter().map(|p| json!(rel(p))).collect();
        lines.push(format!("attachments: {}", yaml::flow_list(&files)));
    } else {
        lines.push("# This item does not accept general attachments.".to_string());
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

/// The form session's "save draft" for a service request: the template
/// with paths relative to the file's folder. Returns the path written.
pub fn save_sr_draft(plan: &SRPlan, draft: &SrDraft, path: &str) -> Result<String> {
    let p = PathBuf::from(expanduser(path));
    let resolved = fsv_core::journal::resolve_path(&p);
    let parent = resolved
        .parent()
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/".into());
    let text = render_sr_template(
        plan,
        &SrTemplateOptions {
            draft: Some(draft),
            base_dir: Some(&parent),
            ..SrTemplateOptions::default()
        },
    )?;
    std::fs::write(&p, text)?;
    Ok(p.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests;
