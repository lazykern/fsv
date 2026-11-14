//! `fsv form` (`form_cli.py`).

use std::sync::Arc;

use clap::ArgMatches;
use serde_json::{Map, Value, json};

use fsv_api::formmeta;
use fsv_core::term::{Column, Overflow, Table, err, escape, out};
use fsv_core::{Error, Result};

use crate::cmd::parse;
use crate::generated::*;
use crate::util::{CmdResult, client, fail, fail_exc, uncaught};

const SNAPSHOT_VERSION: i64 = 1;

/// `form_cli._fail`: the message is plain text.
fn form_fail(msg: &str) -> Error {
    fail(&escape(msg), 2)
}

const SR_ALIASES: [&str; 7] = [
    "sr",
    "srs",
    "service_request",
    "service_requests",
    "service-request",
    "service-requests",
    "catalog",
];
const SR: &str = "service_request";

/// `form_cli._module`.
fn module(text: &str) -> Result<String> {
    if SR_ALIASES.contains(&fsv_core::pystr::strip(text).to_lowercase().as_str()) {
        return Ok(SR.into());
    }
    formmeta::module_name(text).map_err(|e| form_fail(&e.to_string()))
}

/// What `fsv form` reads: a module's form, or a catalog item's form (`sr`).
enum Meta {
    Form(Box<formmeta::FormMeta>),
    Item(Box<fsv_api::catalog::ItemMeta>),
}

impl Meta {
    fn rules(&self) -> &Value {
        match self {
            Meta::Form(m) => &m.rules,
            Meta::Item(m) => &m.rules,
        }
    }
    fn spec(&self) -> Arc<fsv_forms::FormSpec> {
        match self {
            Meta::Form(m) => m.spec(),
            Meta::Item(m) => m.spec(),
        }
    }
    fn state(
        &self,
        mode: &str,
        resolver: Option<Arc<dyn fsv_forms::LookupResolver>>,
        existing: Option<fsv_forms::Values>,
    ) -> Result<fsv_forms::FormState> {
        match self {
            Meta::Form(m) => m.state(mode, resolver, existing),
            Meta::Item(m) => m.state(mode, resolver, existing),
        }
    }
    fn as_of(&self) -> Map<String, Value> {
        match self {
            Meta::Form(m) => m.as_of(),
            Meta::Item(m) => m.as_of(),
        }
    }
    fn form_notes(&self) -> Vec<String> {
        match self {
            Meta::Form(m) => m.form_notes(),
            Meta::Item(_) => Vec::new(),
        }
    }
}

pub fn dispatch(sub: &str, m: &ArgMatches) -> CmdResult {
    match sub {
        "explain" => explain(parse(m)?),
        "fields" => fields(parse(m)?),
        "dump" => dump(parse(m)?),
        "diff" => diff(parse(m)?),
        _ => Err(Error::other(format!("unknown command: form {sub}"))),
    }
}

/// `fsv form dump`.
fn dump(a: FormDumpArgs) -> CmdResult {
    let module = module(&a.module)?;
    if a.mode != "new" && a.mode != "edit" {
        return Err(form_fail("--mode must be new or edit"));
    }
    let c = client()?;
    let meta = load_parts(
        &c,
        &module,
        a.item.as_deref(),
        &a.mode,
        true,
        &Value::Null,
        &Value::Null,
    )?;
    let mut snap = Map::new();
    snap.insert("fsv_form_snapshot".into(), json!(SNAPSHOT_VERSION));
    snap.insert("module".into(), json!(module));
    if let Parts::Item(m) = &meta {
        snap.insert("item".into(), m.display_id());
    }
    snap.insert("form_type".into(), json!(meta.form_type()));
    snap.insert("workspace_id".into(), meta.workspace_id().clone());
    snap.insert(
        "taken_at".into(),
        json!(
            jiff::Zoned::now()
                .strftime("%Y-%m-%dT%H:%M:%S%z")
                .to_string()
        ),
    );
    snap.insert("fields".into(), Value::Array(meta.raw_fields()));
    snap.insert("business_rules".into(), rule_list(meta.rules()));
    print_json(&Value::Object(snap));
    Ok(())
}

/// `fsv form diff`.
fn diff(a: FormDiffArgs) -> CmdResult {
    let old = read_snapshot(&a.snapshot)?;
    let new = if let Some(other) = a.against.as_deref().filter(|s| !s.is_empty()) {
        read_snapshot(other)?
    } else {
        let c = client()?;
        let mode = if old.get("form_type").and_then(Value::as_str) == Some("edit_form") {
            "edit"
        } else {
            "new"
        };
        let module = module(
            &old.get("module")
                .filter(|v| fsv_py::truthy(v))
                .map_or_else(|| "change".to_string(), fsv_py::py_str),
        )?;
        let item = old
            .get("item")
            .filter(|v| module == SR && !v.is_null())
            .map(fsv_py::py_str);
        if module == SR && item.is_none() {
            return Err(form_fail(&format!(
                "{} is an sr snapshot without its catalog item; dump it again with --item",
                a.snapshot
            )));
        }
        let meta = load_parts(
            &c,
            &module,
            item.as_deref(),
            mode,
            module != SR,
            &Value::Null,
            &Value::Null,
        )?;
        json!({"fields": meta.raw_fields(), "business_rules": rule_list(meta.rules())})
    };
    let changes = diff_snapshots(&old, &new);
    if a.output == "json" {
        print_json(&Value::Array(changes));
        return Ok(());
    }
    if changes.is_empty() {
        out().print("no changes");
        return Ok(());
    }
    for ch in &changes {
        let s = |k: &str| ch.get(k).filter(|v| fsv_py::truthy(v)).map(fsv_py::py_str);
        let ident = s("label").or_else(|| s("name")).unwrap_or_default();
        let kind = fsv_py::py_str(&ch["kind"]);
        let r = if kind == "field" { "name" } else { "id" };
        let reference = fsv_py::py_str(ch.get(r).unwrap_or(&Value::Null));
        let sign = match ch["change"].as_str() {
            Some("added") => "[green]+[/green]",
            Some("removed") => "[red]-[/red]",
            _ => "[yellow]~[/yellow]",
        };
        let mut line = format!(
            "{sign} {kind} {} [dim]({})[/dim]",
            escape(&ident),
            escape(&reference)
        );
        if let Some(what) = ch
            .get("what")
            .and_then(Value::as_array)
            .filter(|w| !w.is_empty())
        {
            let parts: Vec<String> = what.iter().map(fsv_py::py_str).collect();
            line.push_str(&format!(": {}", escape(&parts.join("; "))));
        }
        out().print(&line);
    }
    out().print(&format!("[dim]{} difference(s)[/dim]", changes.len()));
    Ok(())
}

/// `_guard`: form errors exit 2, the rest by their exit code.
fn guard(e: Error) -> Error {
    match e {
        Error::Exit { code, message } if !message.is_empty() => fail(&escape(&message), code),
        Error::Exit { .. } | Error::Aborted => fail_exc(e),
        other => fail(&escape(&other.to_string()), other.exit_code()),
    }
}

fn print_json(v: &Value) {
    println!(
        "{}",
        fsv_py::Dumps::default()
            .indent(2)
            .ensure_ascii(false)
            .dump(v)
    );
}

fn rule_list(rules: &Value) -> Value {
    rules
        .get("business_rules")
        .filter(|v| fsv_py::truthy(v))
        .cloned()
        .unwrap_or_else(|| json!([]))
}

/// `_load`: form metadata for a module, or a catalog item's form for `sr`
/// (the same item form and rules `fsv sr fields` uses).
#[allow(clippy::too_many_arguments)]
fn load(
    c: &Arc<fsv_core::Client>,
    module: &str,
    item: Option<&str>,
    mode: &str,
    fresh: bool,
    module_id: &Value,
    record: &Value,
) -> Result<Meta> {
    match load_parts(c, module, item, mode, fresh, module_id, record)? {
        Parts::Form(p) => p.build().map(|m| Meta::Form(Box::new(m))),
        Parts::Item(p) => p.build().map(|m| Meta::Item(Box::new(m))),
    }
    .map_err(guard)
}

/// [`load`] before the fields are parsed: Python's `FormMeta.spec` is lazy,
/// so `form dump` and `form diff` read fields the spec would refuse.
enum Parts {
    Form(Box<formmeta::MetaParts>),
    Item(Box<fsv_api::catalog::ItemParts>),
}

impl Parts {
    fn rules(&self) -> &Value {
        match self {
            Parts::Form(p) => &p.rules,
            Parts::Item(p) => &p.rules,
        }
    }
    fn raw_fields(&self) -> Vec<Value> {
        match self {
            Parts::Form(p) => p.raw_fields(),
            Parts::Item(p) => p.raw_fields(),
        }
    }
    /// An item form is always a new form.
    fn form_type(&self) -> &str {
        match self {
            Parts::Form(p) => p.form_type,
            Parts::Item(_) => "new_form",
        }
    }
    fn workspace_id(&self) -> &Value {
        match self {
            Parts::Form(p) => &p.workspace_id,
            Parts::Item(p) => &p.workspace_id,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn load_parts(
    c: &Arc<fsv_core::Client>,
    module: &str,
    item: Option<&str>,
    mode: &str,
    fresh: bool,
    module_id: &Value,
    record: &Value,
) -> Result<Parts> {
    let store = formmeta::DiskStore::current();
    let item = item.filter(|s| !s.is_empty());
    if module == SR {
        let Some(item) = item else {
            return Err(form_fail(
                "sr forms are per catalog item: pass --item ID (the display id or exact name)",
            ));
        };
        if mode != "new" || !record.is_null() {
            return Err(form_fail(
                "sr item forms have no edit form here; use --item without --id/--mode edit",
            ));
        }
        return fsv_api::catalog::load_item_parts(c.as_ref(), &store, item, None)
            .map(|p| Parts::Item(Box::new(p)))
            .map_err(guard);
    }
    if item.is_some() {
        return Err(form_fail("--item applies to sr forms only"));
    }
    formmeta::load_parts_for_record(c.as_ref(), &store, module, mode, fresh, module_id, record)
        .map(|p| Parts::Form(Box::new(p)))
        .map_err(guard)
}

/// `_note_rules`.
fn note_rules(meta: &Meta) {
    if let Some(note) = formmeta::rules_note(meta.rules()) {
        err().print(&format!("[dim]{}[/dim]", escape(&note)));
    }
    for form_note in meta.form_notes() {
        err().print(&format!("[yellow]note[/yellow]: {}", escape(&form_note)));
    }
}

/// `form_cli._one_line` (default limit 100).
fn one_line(text: &str, limit: usize) -> String {
    let s = fsv_core::pystr::split_ws(&fsv_forms::spec::plain_text(text)).join(" ");
    if s.chars().count() <= limit {
        s
    } else {
        let mut out: String = s.chars().take(limit.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn fold_table(title: String, cols: &[&str]) -> Table {
    let mut t = Table::new().title(title);
    for c in cols {
        t.add_column(Column::new(*c).overflow(Overflow::Fold));
    }
    t
}

fn form_err(e: fsv_forms::FormError) -> Error {
    fail(&escape(&e.to_string()), 2)
}

fn explain(a: FormExplainArgs) -> CmdResult {
    let module = module(&a.module)?;
    let c = client()?;
    let mut record: Option<Map<String, Value>> = None;
    if a.id_.as_deref().is_some_and(|s| !s.is_empty()) && module == SR {
        return Err(form_fail(
            "--id does not apply to sr item forms; use --item",
        ));
    }
    if let Some(id) = a.id_.as_deref().filter(|s| !s.is_empty()) {
        let res = fsv_core::resource::lookup(&format!("{module}s"))
            .ok_or_else(|| Error::other(format!("unknown form resource: {module}")))?;
        let cid = fsv_core::resource::parse_id(id, res).map_err(uncaught)?;
        let data = c
            .int_get(&format!("{}/{cid}", res.api_path), None)
            .map_err(guard)?;
        let rec = match data.get(res.item_key) {
            Some(v) => v.clone(),
            None => data.clone(),
        };
        record = Some(rec.as_object().cloned().unwrap_or_default()).filter(|m| !m.is_empty());
    }
    let edit = record.is_some();
    let module_id = record.as_ref().map_or(Value::Null, |r| {
        r.get("display_id")
            .filter(|v| fsv_py::truthy(v))
            .or_else(|| r.get("id"))
            .cloned()
            .unwrap_or(Value::Null)
    });
    let meta = load(
        &c,
        &module,
        a.item.as_deref(),
        if edit { "edit" } else { "new" },
        false,
        &module_id,
        &record.clone().map_or(Value::Null, Value::Object),
    )?;
    note_rules(&meta);
    let draft =
        fsv_flows::drafts::resolve_draft(a.from_.as_deref(), a.preset.as_deref(), &a.set_, None)
            .map_err(guard)?;
    let spec = meta.spec();
    let api: Arc<dyn fsv_api::service::Api + Send + Sync> = c.clone();
    let existing = record.as_ref().map(|r| {
        let resolver = formmeta::ClientResolver::new(api.clone());
        formmeta::name_existing(
            formmeta::existing_from_record(&spec, r),
            &spec,
            r,
            Some(&resolver),
        )
    });
    let resolver: Arc<dyn fsv_forms::LookupResolver> = Arc::new(formmeta::ClientResolver::new(api));
    let mut state = meta
        .state(if edit { "edit" } else { "new" }, Some(resolver), existing)
        .map_err(guard)?;
    let errors = formmeta::apply_answers(&mut state, &draft.answers, None).map_err(guard)?;
    let ev = state.evaluated.clone();
    let explained = state.explain(None).map_err(form_err)?;
    let mut rows: Vec<Value> = Vec::new();
    for f in spec.inputs() {
        let xs: Vec<&fsv_forms::rules::Explanation> =
            explained.iter().filter(|x| x.field == f.api_name).collect();
        let vis = ev.visible.contains(&f.api_name);
        if !vis && !a.all_ && !xs.iter().any(|x| x.source == "rule") {
            continue;
        }
        let why: Vec<String> = xs
            .iter()
            .filter(|x| x.source != "metadata")
            .map(|x| {
                let text = x.to_string();
                match text.split_once(": ") {
                    Some((_, rest)) => rest.to_string(),
                    None => text,
                }
            })
            .collect();
        let value = if vis {
            state.display(&f.api_name, None).map_err(form_err)?
        } else {
            String::new()
        };
        rows.push(json!({
            "field": f.api_name, "label": f.label, "visible": vis,
            "required": ev.required.contains(&f.api_name),
            "disabled": ev.disabled.contains(&f.api_name), "value": value,
            "populated_by": ev.populated.get(&f.api_name).cloned().unwrap_or(Value::Null),
            "why": why,
        }));
    }
    let messages: Vec<Value> = ev
        .messages
        .iter()
        .map(|m| json!({"text": m.text, "blocking": m.blocking, "rule": m.rule_name}))
        .collect();
    let conflicts: Vec<String> = fsv_flows::change::real_conflicts(&ev)
        .iter()
        .map(|c| c.describe())
        .collect();
    if a.output == "json" {
        print_json(
            &json!({"rows": rows, "messages": messages, "conflicts": conflicts,
                           "errors": errors, "metadata": Value::Object(meta.as_of()),
                           "form_notes": meta.form_notes()}),
        );
        return Ok(());
    }
    let mut t = fold_table(
        format!("{module} form{}", if edit { " (edit)" } else { " (new)" }),
        &["Field", "State", "Value", "Why"],
    );
    for r in &rows {
        let flag = |k: &str| r[k].as_bool().unwrap_or(false);
        let state_s = format!(
            "{}{}{}",
            if flag("visible") {
                "visible"
            } else {
                "[dim]hidden[/dim]"
            },
            if flag("required") {
                ", [bold]required[/bold]"
            } else {
                ""
            },
            if flag("disabled") { ", read-only" } else { "" }
        );
        let mut val = escape(r["value"].as_str().unwrap_or(""));
        if let Some(p) = r.get("populated_by").filter(|v| fsv_py::truthy(v)) {
            val.push_str(&format!(" [dim](rule {})[/dim]", fsv_py::py_str(p)));
        }
        let why: Vec<String> = r["why"]
            .as_array()
            .into_iter()
            .flatten()
            .map(fsv_py::py_str)
            .collect();
        let why = escape(&why.join("; "));
        let why = if why.is_empty() {
            "[dim]form metadata[/dim]".to_string()
        } else {
            why
        };
        t.add_row([escape(&fsv_py::py_str(&r["label"])), state_s, val, why]);
    }
    out().print_table(&t);
    for m in &ev.messages {
        out().print(&format!(
            "[magenta]{}message[/] {} [dim](rule {})[/dim]",
            if m.blocking { "BLOCKING " } else { "" },
            escape(&m.text),
            escape(&m.rule_name)
        ));
    }
    for cf in &conflicts {
        out().print(&format!("[yellow]conflict[/] {}", escape(cf)));
    }
    for e in &errors {
        out().print(&format!("[red]answer error[/] {}", escape(e)));
    }
    if !errors.is_empty() {
        return Err(Error::exit(2, ""));
    }
    Ok(())
}

fn fields(a: FormFieldsArgs) -> CmdResult {
    let module = module(&a.module)?;
    let c = client()?;
    let meta = load(
        &c,
        &module,
        a.item.as_deref(),
        "new",
        false,
        &Value::Null,
        &Value::Null,
    )?;
    note_rules(&meta);
    let spec = meta.spec();
    let state = meta.state("new", None, None).map_err(guard)?;
    let ev = &state.evaluated;
    if let Some(key) = a.choices.as_deref().filter(|s| !s.is_empty()) {
        let f = formmeta::find_field(&spec, key).map_err(form_err)?;
        let pool: Vec<&fsv_forms::Choice> = if f.is_nested_level() {
            spec.choices_for(f, &ev.values)
                .map_err(form_err)?
                .iter()
                .filter(|ch| ch.visible)
                .collect()
        } else {
            f.choices.iter().filter(|ch| ch.visible).collect()
        };
        if f.is_nested_level() && pool.is_empty() {
            return Err(form_fail(&format!(
                "{}: choose the upper level first (its choices depend on it)",
                f.label
            )));
        }
        let data: Vec<Value> = pool
            .iter()
            .map(|ch| json!({"label": ch.label, "value": ch.value, "id": ch.id, "description": ch.description}))
            .collect();
        if a.output == "json" {
            print_json(&json!({"field": f.api_name, "label": f.label, "choices": data}));
            return Ok(());
        }
        let mut t = fold_table(
            format!("{} ({}): {} choices", f.label, f.api_name, data.len()),
            &["Choice", "Meaning"],
        );
        for ch in &pool {
            t.add_row([escape(&ch.label), escape(&one_line(&ch.description, 200))]);
        }
        out().print_table(&t);
        if !f.help().is_empty() {
            out().print(&format!("[dim]{}[/dim]", escape(&one_line(f.help(), 400))));
        }
        return Ok(());
    }
    let rows: Vec<Value> = spec
        .inputs()
        .iter()
        .map(|f| {
            let descs: Vec<Value> = f
                .choices
                .iter()
                .filter(|c| c.visible && !c.description.is_empty())
                .map(|c| json!({"label": c.label, "description": c.description}))
                .collect();
            json!({
                "label": f.label, "name": f.api_name, "kind": f.kind.as_str(), "custom": f.is_custom,
                "required_static": f.required_base, "visible_on_load": ev.visible.contains(&f.api_name),
                "required_on_load": ev.required.contains(&f.api_name), "help": one_line(f.help(), 300),
                "choices": f.choices.len(), "choice_descriptions": descs,
            })
        })
        .collect();
    if a.output == "json" {
        print_json(&Value::Array(rows));
        return Ok(());
    }
    let mut t = fold_table(
        format!("{module} form fields (web order; state on a new form)"),
        &["Label", "Name", "Type", "On load", "Help"],
    );
    for r in &rows {
        let flag = |k: &str| r[k].as_bool().unwrap_or(false);
        let load = format!(
            "{}{}",
            if flag("visible_on_load") {
                "visible"
            } else {
                "[dim]hidden[/dim]"
            },
            if flag("required_on_load") {
                ", [bold]required[/bold]"
            } else {
                ""
            }
        );
        let n = r["choices"].as_u64().unwrap_or(0);
        let mut kind = fsv_py::py_str(&r["kind"]);
        if n > 0 {
            kind.push_str(&format!(" ({n})"));
        }
        let mut help_cell = one_line(r["help"].as_str().unwrap_or(""), 80);
        let extra: Vec<String> = r["choice_descriptions"]
            .as_array()
            .into_iter()
            .flatten()
            .take(8)
            .map(|d| {
                format!(
                    "• {} — {}",
                    fsv_py::py_str(&d["label"]),
                    one_line(d["description"].as_str().unwrap_or(""), 80)
                )
            })
            .collect();
        if !extra.is_empty() {
            let mut lines = if help_cell.is_empty() {
                Vec::new()
            } else {
                vec![help_cell]
            };
            lines.extend(extra);
            help_cell = lines.join("\n");
        }
        t.add_row([
            escape(&fsv_py::py_str(&r["label"])),
            escape(&fsv_py::py_str(&r["name"])),
            kind,
            load,
            escape(&help_cell),
        ]);
    }
    out().print_table(&t);
    let notes: Vec<&fsv_forms::Field> = spec
        .ordered()
        .into_iter()
        .filter(|f| !f.is_input() && ev.visible.contains(&f.api_name))
        .collect();
    if !notes.is_empty() {
        let shown: Vec<String> = notes
            .iter()
            .take(5)
            .map(|f| {
                one_line(
                    if f.content.is_empty() {
                        &f.label
                    } else {
                        &f.content
                    },
                    60,
                )
            })
            .collect();
        out().print(&format!(
            "[dim]{} information block(s) on the form, not inputs: {}[/dim]",
            notes.len(),
            escape(&shown.join("; "))
        ));
    }
    let hint = if module == SR {
        format!("sr --item {}", a.item.as_deref().unwrap_or(""))
    } else {
        format!("{module}s")
    };
    out().print(&format!(
        "[dim]choices: fsv form fields {} --choices LABEL[/dim]",
        escape(&hint)
    ));
    Ok(())
}

/// `_choice_labels`.
fn choice_labels(f: &Value) -> Vec<String> {
    f.get("choices")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|ch| match ch {
            Value::Object(_) => ["value", "label", "name"]
                .iter()
                .find_map(|k| ch.get(*k).filter(|v| fsv_py::truthy(v)))
                .map_or_else(
                    || fsv_py::py_str(ch.get("id").unwrap_or(&Value::Null)),
                    fsv_py::py_str,
                ),
            other => fsv_py::py_str(other),
        })
        .collect()
}

/// `diff_snapshots`.
fn diff_snapshots(old: &Value, new: &Value) -> Vec<Value> {
    let by = |snap: &Value, key: &dyn Fn(&Value) -> String, list: &str| -> Vec<(String, Value)> {
        let mut out: Vec<(String, Value)> = Vec::new();
        for f in snap
            .get(list)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let k = key(f);
            match out.iter_mut().find(|(n, _)| *n == k) {
                Some(slot) => slot.1 = f.clone(),
                None => out.push((k, f.clone())),
            }
        }
        out
    };
    let get = |v: &Value, k: &str| v.get(k).cloned().unwrap_or(Value::Null);
    let field_key = |f: &Value| fsv_py::py_str(f.get("name").unwrap_or(&Value::Null));
    let of = by(old, &field_key, "fields");
    let nf = by(new, &field_key, "fields");
    let find = |list: &[(String, Value)], k: &str| {
        list.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())
    };
    let mut out = Vec::new();
    for (name, f) in &nf {
        if find(&of, name).is_none() {
            out.push(
                json!({"kind": "field", "change": "added", "name": name, "label": get(f, "label")}),
            );
        }
    }
    for (name, f) in &of {
        if find(&nf, name).is_none() {
            out.push(json!({"kind": "field", "change": "removed", "name": name, "label": get(f, "label")}));
        }
    }
    let mut common: Vec<&String> = of
        .iter()
        .map(|(n, _)| n)
        .filter(|n| find(&nf, n).is_some())
        .collect();
    common.sort();
    for name in common {
        let (a, b) = (
            find(&of, name).unwrap_or_default(),
            find(&nf, name).unwrap_or_default(),
        );
        let mut what: Vec<String> = Vec::new();
        for k in [
            "label",
            "field_type",
            "required",
            "position",
            "helptext",
            "displayed_to_agents",
        ] {
            if !fsv_api::readback::py_equal(&get(&a, k), &get(&b, k)) {
                what.push(format!(
                    "{k}: {} -> {}",
                    fsv_py::py_repr(&get(&a, k)),
                    fsv_py::py_repr(&get(&b, k))
                ));
            }
        }
        let (ca, cb) = (choice_labels(&a), choice_labels(&b));
        if ca != cb {
            let added: Vec<String> = cb.iter().filter(|x| !ca.contains(x)).cloned().collect();
            let removed: Vec<String> = ca.iter().filter(|x| !cb.contains(x)).cloned().collect();
            let mut s = "choices".to_string();
            if !added.is_empty() {
                s.push_str(&format!(" +{}", fsv_py::py_repr(&json!(added))));
            }
            if !removed.is_empty() {
                s.push_str(&format!(" -{}", fsv_py::py_repr(&json!(removed))));
            }
            if added.is_empty() && removed.is_empty() {
                s.push_str(" reordered");
            }
            what.push(s);
        }
        if !what.is_empty() {
            out.push(json!({"kind": "field", "change": "changed", "name": name, "label": get(&b, "label"), "what": what}));
        }
    }
    let rule_key = |r: &Value| {
        format!(
            "{}/{}",
            fsv_py::py_str(r.get("id").unwrap_or(&Value::Null)),
            if r.get("is_reversible").is_some_and(fsv_py::truthy) {
                "rev"
            } else {
                "fixed"
            }
        )
    };
    let orules = by(old, &rule_key, "business_rules");
    let nrules = by(new, &rule_key, "business_rules");
    let sorted_keys = |list: &[(String, Value)], other: &[(String, Value)], present: bool| {
        let mut ks: Vec<String> = list
            .iter()
            .map(|(k, _)| k.clone())
            .filter(|k| find(other, k).is_some() == present)
            .collect();
        ks.sort();
        ks
    };
    for k in sorted_keys(&nrules, &orules, false) {
        let r = find(&nrules, &k).unwrap_or_default();
        out.push(json!({"kind": "rule", "change": "added", "id": k, "name": get(&r, "name")}));
    }
    for k in sorted_keys(&orules, &nrules, false) {
        let r = find(&orules, &k).unwrap_or_default();
        out.push(json!({"kind": "rule", "change": "removed", "id": k, "name": get(&r, "name")}));
    }
    for k in sorted_keys(&orules, &nrules, true) {
        let (a, b) = (
            find(&orules, &k).unwrap_or_default(),
            find(&nrules, &k).unwrap_or_default(),
        );
        let mut keys: Vec<String> = a
            .as_object()
            .into_iter()
            .chain(b.as_object())
            .flat_map(|m| m.keys().cloned())
            .collect();
        keys.sort();
        keys.dedup();
        let what: Vec<String> = keys
            .into_iter()
            .filter(|f| f != "updated_at" && f != "created_at")
            .filter(|f| {
                fsv_forms::plan::canonical_json(&get(&a, f))
                    != fsv_forms::plan::canonical_json(&get(&b, f))
            })
            .collect();
        if !what.is_empty() {
            out.push(json!({"kind": "rule", "change": "changed", "id": k, "name": get(&b, "name"), "what": what}));
        }
    }
    out
}

fn read_snapshot(path: &str) -> Result<Value> {
    let fail = |why: String| form_fail(&format!("cannot read snapshot {path}: {why}"));
    let bytes = std::fs::read(path).map_err(|e| fail(fsv_py::os_error(&e, path)))?;
    let text = std::str::from_utf8(&bytes).map_err(|e| fail(fsv_py::utf8_error(&bytes, &e)))?;
    let data = crate::util::json_loads(text).map_err(fail)?;
    let version = data.get("fsv_form_snapshot").unwrap_or(&Value::Null);
    if !data.is_object() || !fsv_api::readback::py_equal(version, &json!(SNAPSHOT_VERSION)) {
        return Err(form_fail(&format!(
            "{path} is not an fsv form snapshot (make one with `fsv form dump`)"
        )));
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Values from `form_cli._module`, `_one_line`, `_choice_labels` and
    /// `diff_snapshots`.
    #[test]
    fn module_names() {
        assert_eq!(module("Changes ").unwrap(), "change");
        assert_eq!(module("TICKETS").unwrap(), "ticket");
        assert_eq!(module("changess").unwrap(), "change");
    }

    #[test]
    fn one_line_collapses_and_truncates() {
        assert_eq!(one_line("<p>a  b</p>\n c", 100), "a b c");
        assert_eq!(one_line("xxxxx", 4), "xxx…");
        assert_eq!(one_line("abcd", 4), "abcd");
    }

    #[test]
    fn choice_labels_fallbacks() {
        let f = json!({"choices": [{"value": "A"}, {"name": "B"}, {"id": 3}, "C", {"value": ""}]});
        assert_eq!(choice_labels(&f), ["A", "B", "3", "C", "None"]);
    }

    #[test]
    fn diff_snapshots_fields_and_rules() {
        let old = json!({
            "fields": [
                {"name": "a", "label": "A", "choices": [{"value": "x"}, {"value": "y"}]},
                {"name": "gone", "label": "Gone"},
                {"name": "r", "label": "R", "required": false, "choices": [{"value": "p"}, {"value": "q"}]},
            ],
            "business_rules": [
                {"id": 1, "name": "One", "x": 1, "updated_at": "t"},
                {"id": 2, "is_reversible": true, "name": "Two"},
            ],
        });
        let new = json!({
            "fields": [
                {"name": "a", "label": "A2", "choices": [{"value": "y"}, {"value": "z"}]},
                {"name": "new", "label": "New"},
                {"name": "r", "label": "R", "required": true, "choices": [{"value": "q"}, {"value": "p"}]},
            ],
            "business_rules": [
                {"id": 1, "name": "One", "x": 2, "updated_at": "u"},
                {"id": 3, "name": "Three"},
            ],
        });
        assert_eq!(
            Value::Array(diff_snapshots(&old, &new)),
            json!([
                {"kind": "field", "change": "added", "name": "new", "label": "New"},
                {"kind": "field", "change": "removed", "name": "gone", "label": "Gone"},
                {"kind": "field", "change": "changed", "name": "a", "label": "A2", "what": ["label: 'A' -> 'A2'", "choices +['z'] -['x']"]},
                {"kind": "field", "change": "changed", "name": "r", "label": "R", "what": ["required: False -> True", "choices reordered"]},
                {"kind": "rule", "change": "added", "id": "3/fixed", "name": "Three"},
                {"kind": "rule", "change": "removed", "id": "2/rev", "name": "Two"},
                {"kind": "rule", "change": "changed", "id": "1/fixed", "name": "One", "what": ["x"]},
            ])
        );
    }
}
