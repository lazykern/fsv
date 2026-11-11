//! `fsv sr` (`cli_sr.py`).

use clap::ArgMatches;
use serde_json::{Map, Value, json};

use fsv_api::catalog as cat;
use fsv_core::Error;
use fsv_core::resource::{TICKETS, parse_id};
use fsv_core::term::{Column, Overflow, Table, err, escape, out};
use fsv_flows::drafts::one_line;
use fsv_flows::sr::{
    SrDraft, SrTemplateOptions, parse_sr_template, render_sr_template, resolve_sr_draft,
};

use crate::cmd::changes::form_tty;
use crate::cmd::{formflow, parse};
use crate::generated::*;
use crate::util::{CmdResult, client, fail, fail_exc, interactive};

pub fn dispatch(sub: &str, m: &ArgMatches) -> CmdResult {
    match sub {
        "catalog" => catalog(parse(m)?),
        "fields" => fields(parse(m)?),
        "create" => create(parse(m)?),
        "get" => get(parse(m)?),
        _ => Err(Error::other(format!("unknown command: sr {sub}"))),
    }
}

fn create(a: SrCreateArgs) -> CmdResult {
    let tty = interactive() && !a.no_input;
    let has_input =
        a.from_.is_some() || !a.set_.is_empty() || a.preset.is_some() || !a.attach.is_empty();
    let mut edit = a.edit;
    let (mut form, mut template_only) = (false, false);
    if !has_input && !edit {
        if tty && form_tty() {
            form = true;
        } else if a.dry_run {
            template_only = true;
        } else if tty {
            edit = true;
        } else {
            return Err(Error::exit(
                2,
                "nothing to request: pass --from FILE or --set LABEL=VALUE (the interactive form and --edit need a terminal); start with `fsv sr create ITEM --dry-run` to get a draft",
            ));
        }
    }
    if (form || template_only) && (a.requested_for.is_some() || !a.cc.is_empty()) {
        return Err(Error::exit(
            2,
            "--requested-for and --cc require --from, --set, --preset or --edit; the interactive form and template cannot retain them",
        ));
    }
    if edit && !tty {
        return Err(Error::exit(
            2,
            "--edit needs a terminal (no TTY or --no-input); use --from FILE",
        ));
    }
    let c = client()?;
    let modes = SrModes {
        tty,
        edit,
        form,
        template_only,
    };
    sr_create(&a, c, modes).map_err(guard)
}

struct SrModes {
    tty: bool,
    edit: bool,
    form: bool,
    template_only: bool,
}

/// `_valid_recipient(e) and _EMAIL.fullmatch(e)`.
fn plain_email(e: &str) -> bool {
    static EMAIL: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^[^\s@<>,;]+@[^\s@<>,;]+\.[^\s@<>,;]+$").expect("email regex")
    });
    !e.chars().any(|c| (c as u32) < 32 || c as u32 == 127) && EMAIL.is_match(e)
}

enum SrReviewed {
    Flow(Box<cat::SRPlan>),
    Form(Box<fsv_app::Submitted>),
}

impl SrReviewed {
    fn ok(&self) -> bool {
        self.errors().is_empty()
    }
    fn errors(&self) -> &[String] {
        match self {
            SrReviewed::Flow(p) => &p.errors,
            SrReviewed::Form(s) => &s.plan.errors,
        }
    }
    fn hash(&self) -> &str {
        match self {
            SrReviewed::Flow(p) => &p.hash,
            SrReviewed::Form(s) => &s.plan.hash,
        }
    }
    fn review(&self) -> &fsv_core::review::Review {
        match self {
            SrReviewed::Flow(p) => &p.review,
            SrReviewed::Form(s) => &s.plan.review,
        }
    }
    fn payload(&self) -> Option<Value> {
        match self {
            SrReviewed::Flow(p) => p.payload.clone().map(Value::Object),
            SrReviewed::Form(s) => s.plan.payload.clone(),
        }
    }
    fn to_json(&self) -> Value {
        match self {
            SrReviewed::Flow(p) => p.to_json(),
            SrReviewed::Form(s) => match formflow::carried::<cat::SRPlan>(&s.plan) {
                Some(p) => p.to_json(),
                None => json!({
                    "plan_hash": if s.plan.hash.is_empty() { Value::Null } else { json!(s.plan.hash) },
                    "payload": s.plan.payload,
                    "review": s.plan.review.to_dict(),
                    "errors": s.plan.errors,
                }),
            },
        }
    }
}

/// `cli_sr._print_plan`.
fn sr_print_plan(plan: &SrReviewed, as_json: bool, display_id: &Value) {
    if as_json {
        print_json(&plan.to_json());
        return;
    }
    let o = out();
    plan.review().render(o);
    o.print(&format!("[bold red]⚠ {}[/bold red]", escape(cat::SAFETY)));
    match plan.payload().filter(|_| plan.ok()) {
        None => o.print(&format!("[dim]{}[/dim]", fsv_core::review::NO_PAYLOAD)),
        Some(payload) => {
            o.print(&format!(
                "[bold]Payload[/bold] (POST /api/_/catalog/service-items/{}/place-request; <upload N: name> becomes the attachment id after upload)",
                fsv_py::py_str(display_id)
            ));
            print_json(&payload);
        }
    }
    let h = plan.hash();
    if !h.is_empty() {
        o.print(&format!("plan_hash: {h}"));
        o.print(&format!(
            "[dim]send exactly this with: --yes --expect-plan {}[/dim]",
            &h[..h.len().min(16)]
        ));
    }
}

/// `cli_sr.edit_loop`.
fn sr_edit_loop(
    build: &dyn Fn(&SrDraft) -> Result<cat::SRPlan, Error>,
    draft: SrDraft,
) -> Result<(SrDraft, cat::SRPlan), Error> {
    use fsv_flows::change::yaml_norm;
    let mut draft = draft;
    let mut plan = build(&draft)?;
    let mut errors: Vec<String> = Vec::new();
    for _ in 0..20 {
        let opts = SrTemplateOptions {
            errors: &errors,
            draft: Some(&draft),
            ..SrTemplateOptions::default()
        };
        let text = render_sr_template(&plan, &opts)?;
        let baseline = parse_sr_template(&text, None)?.draft.answers;
        let eo = fsv_core::editor::EditOptions {
            suffix: ".yaml",
            comment_prefix: "#",
            max_rounds: 20,
        };
        let new = fsv_core::editor::edit_text(&text, |t| parse_sr_template(t, None), &eo)?;
        let answers: Map<String, Value> = new
            .draft
            .answers
            .iter()
            .filter(|(k, v)| {
                draft.draft.answers.contains_key(*k)
                    || yaml_norm(Some(v)) != yaml_norm(baseline.get(*k))
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let base = new
            .draft
            .base_dir
            .as_ref()
            .map(|b| b.to_string_lossy().into_owned());
        let answers = cat::file_answers(&plan.state.spec, &answers, base.as_deref());
        draft = SrDraft {
            draft: fsv_flows::drafts::Draft {
                answers,
                preset: draft.draft.preset.clone(),
                ..new.draft
            },
            requested_for: new.requested_for,
        };
        plan = build(&draft)?;
        if plan.ok() {
            return Ok((draft, plan));
        }
        errors = plan.errors.clone();
    }
    Err(Error::EditorAbort(
        "aborted: too many rounds with errors".into(),
    ))
}

/// `cli_sr._confirm`: [y] place request / [e] edit / [s] save draft / [n] cancel.
fn sr_confirm(
    mut plan: cat::SRPlan,
    mut draft: SrDraft,
    build: &dyn Fn(&SrDraft, bool) -> Result<cat::SRPlan, Error>,
) -> Result<(cat::SRPlan, SrDraft), Error> {
    use crate::cmd::changes::menu;
    let mut ack = false;
    loop {
        plan.review.render(err());
        err().print(&format!("[bold red]⚠ {}[/bold red]", escape(cat::SAFETY)));
        let choice = if plan.ok() {
            menu("[y] place request  [e] edit  [s] save draft  [n] cancel")?
        } else {
            let c = menu("Problems must be fixed first: [e] edit  [s] save draft  [n] cancel")?;
            if c == 'y' {
                continue;
            }
            c
        };
        match choice {
            'y' => {
                let blocking = plan.review.blocking_unacked();
                if !blocking.is_empty() {
                    for m in &blocking {
                        err().print(&format!("[magenta]blocking rule message:[/] {}", escape(m)));
                    }
                    if !fsv_core::term::confirm("Acknowledge and continue?", Some(false), false)? {
                        continue;
                    }
                    ack = true;
                    plan = build(&draft, ack)?;
                    if !plan.ok() {
                        continue;
                    }
                }
                return Ok((plan, draft));
            }
            'e' => match sr_edit_loop(&|d| build(d, ack), draft.clone()) {
                Ok((d, p)) => {
                    draft = d;
                    plan = p;
                }
                Err(Error::EditorAbort(m)) => {
                    err().print(&format!(
                        "[yellow]{}[/yellow]; keeping the previous draft",
                        escape(&m)
                    ));
                }
                Err(e) => return Err(e),
            },
            's' => {
                let path = fsv_core::term::prompt("Save draft to", Some("sr-draft.yaml"), true)?;
                if std::path::Path::new(&path).exists()
                    && !fsv_core::term::confirm(
                        &format!("{path} exists; overwrite?"),
                        Some(false),
                        false,
                    )?
                {
                    continue;
                }
                let dest = std::env::current_dir()?.join(expanduser(&path));
                let parent = dest
                    .parent()
                    .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()))
                    .unwrap_or_default();
                let parent = parent.to_string_lossy();
                let opts = SrTemplateOptions {
                    draft: Some(&draft),
                    base_dir: Some(&parent),
                    ..SrTemplateOptions::default()
                };
                std::fs::write(&dest, render_sr_template(&plan, &opts)?)?;
                err().print(&format!(
                    "[green]saved[/green] {}; continue later with --from {}",
                    escape(&path),
                    escape(&path)
                ));
                return Err(Error::exit(0, ""));
            }
            _ => return Err(Error::exit(1, "cancelled; nothing was uploaded or sent")),
        }
    }
}

struct SrSend<'a> {
    tty: bool,
    yes: bool,
    expect: Option<&'a str>,
    allow_duplicate: bool,
}

/// `cli_sr._send`.
fn sr_send(
    plan: cat::SRPlan,
    deps: &fsv_app::FormDeps,
    rebuild: &mut dyn FnMut(cat::ItemMeta) -> Result<cat::SRPlan, Error>,
    o: &SrSend<'_>,
) -> Result<Value, Error> {
    let journal = deps.journal.as_ref();
    cat::duplicate_check(&plan, o.allow_duplicate, journal)?;
    if !plan.identity.is_empty() {
        let checked = rebuild(plan.meta.clone())?;
        if !checked.ok() || checked.hash != plan.hash {
            return Err(Error::exit(
                4,
                "recipient identities changed since review; review again",
            ));
        }
    }
    let (new, diff) = cat::freshen(plan, deps.api.as_ref(), deps.store.as_ref(), rebuild)?;
    if let Some(diff) = diff {
        err().print("[yellow]the catalog item or its rules changed since the review; the plan is now different:[/yellow]");
        for line in diff {
            err().print(&format!("  {}", escape(&line)));
        }
        if !new.ok() {
            new.review.render(err());
            return Err(Error::exit(
                2,
                format!(
                    "the updated form needs more answers: {}",
                    new.errors.join("; ")
                ),
            ));
        }
        if o.expect.is_some_and(|e| !e.is_empty()) || o.yes || !o.tty {
            let h = if new.hash.is_empty() {
                "(incomplete)"
            } else {
                &new.hash
            };
            return Err(Error::exit(
                4,
                format!("plan changed to {h}; review it again with --dry-run"),
            ));
        }
        new.review.render(err());
        if !fsv_core::term::confirm("Place the updated request?", Some(false), false)? {
            return Err(Error::exit(1, "cancelled; nothing was uploaded or sent"));
        }
        cat::duplicate_check(&new, o.allow_duplicate, journal)?;
    }
    formflow::expect_hash(&new.hash, &new.errors, o.expect)?;
    cat::execute(
        &new,
        deps.api.as_ref(),
        journal,
        &cat::upload,
        o.allow_duplicate,
    )
}

/// `cli_sr.create_command` after the input checks.
fn sr_create(a: &SrCreateArgs, c: std::sync::Arc<fsv_core::Client>, m: SrModes) -> CmdResult {
    use std::sync::Arc;

    let as_json = a.output == "json";
    let tty = m.tty;
    let deps = fsv_app::FormDeps::live(c)?;
    let meta = cat::load_item_meta(deps.api.as_ref(), deps.store.as_ref(), &a.item, None)?;
    let display_id = meta.display_id();
    let draft = resolve_sr_draft(
        &meta.spec(),
        a.from_.as_deref(),
        a.preset.as_deref(),
        &a.set_,
        &a.attach,
    )
    .map_err(formflow::draft_error)?;
    let who = formflow::Requester::sr(deps.api.as_ref(), a.requester.clone(), tty);
    if tty && !m.form && !m.edit && !m.template_only && who.value(&draft.draft).is_none() {
        who.ask_sr()?;
    }
    let resolver: Arc<dyn fsv_forms::LookupResolver> = deps.resolver.clone();
    let domain = deps.api.domain();
    let api = deps.api.as_ref();
    let build = |d: &SrDraft,
                 ack_all: bool,
                 mt: Option<cat::ItemMeta>|
     -> Result<cat::SRPlan, Error> {
        let (req, problem) = who.resolve(&d.draft)?;
        let beneficiary = a.requested_for.clone().or_else(|| d.requested_for.clone());
        let target = match beneficiary {
            Some(b) => match fsv_api::formmeta::resolve_requester(api, &b, None) {
                Ok(t) => Some(t),
                Err(Error::Value(e)) => return Err(Error::exit(2, format!("requested-for: {e}"))),
                Err(e) => return Err(e),
            },
            None => None,
        };
        let (mut cc_emails, mut cc_ids) = (Vec::new(), Vec::new());
        for email in &a.cc {
            if !plain_email(email) {
                return Err(Error::exit(2, "CC must be a plain email address"));
            }
            let exact = match fsv_api::formmeta::resolve_requester(api, email, None) {
                Ok(p) => p
                    .email
                    .clone()
                    .filter(|e| e.to_lowercase() == email.to_lowercase())
                    .map(|e| (e, p.id.clone())),
                Err(Error::Value(_)) => None,
                Err(e) => return Err(e),
            };
            let Some((e, id)) = exact else {
                return Err(Error::exit(
                    2,
                    "each CC email must resolve to exactly one non-deleted user",
                ));
            };
            cc_emails.push(e);
            cc_ids.push(id);
        }
        let who = cat::Recipients {
            requested_for: target.as_ref(),
            cc_emails: &cc_emails,
            cc_ids: &cc_ids,
        };
        let mut plan = cat::build_sr_plan(
            mt.as_ref().unwrap_or(&meta),
            &d.catalog_draft(),
            Some(resolver.clone()),
            req.as_ref(),
            ack_all,
            &who,
            &domain,
        )?;
        if let Some(p) = problem {
            let mut errors = vec![p];
            errors.extend(
                plan.errors
                    .drain(..)
                    .filter(|e| e != cat::REQUESTER_MISSING),
            );
            plan.errors = errors;
            plan.review.errors = plan.errors.clone();
            plan.hash.clear();
            plan.review.plan_hash.clear();
        }
        Ok(plan)
    };

    if m.template_only {
        let plan = build(&draft, a.ack, None)?;
        let opts = SrTemplateOptions {
            draft: Some(&draft),
            ..SrTemplateOptions::default()
        };
        print!("{}", render_sr_template(&plan, &opts)?);
        err().print(&format!(
            "[dim]fill this in, then: fsv sr create {} --from FILE --dry-run[/dim]",
            fsv_py::py_str(&display_id)
        ));
        return Ok(());
    }
    let (plan, draft) = if m.form {
        let mut req = None;
        if a.requester.as_deref().is_some_and(|r| !r.is_empty()) {
            let (r, problem) = who.resolve(&draft.draft)?;
            req = r;
            if let Some(p) = problem {
                err().print(&format!("[yellow]--requester[/yellow]: {}", escape(&p)));
            }
        }
        let session = fsv_app::sr_session(&deps, &a.item, Some(meta.clone()), req, a.dry_run)?;
        let s = formflow::run_form(session).map_err(|e| match e {
            Error::Exit { code: 1, .. } => {
                Error::exit(1, "cancelled; nothing was uploaded or sent")
            }
            other => other,
        })?;
        (SrReviewed::Form(Box::new(s)), draft)
    } else if m.edit {
        let (d, p) =
            sr_edit_loop(&|d| build(d, a.ack, None), draft).map_err(formflow::editor_abort)?;
        (SrReviewed::Flow(Box::new(p)), d)
    } else {
        let p = build(&draft, a.ack, None)?;
        (SrReviewed::Flow(Box::new(p)), draft)
    };

    let expect = a.expect_plan.as_deref();
    if a.dry_run {
        sr_print_plan(&plan, as_json, &display_id);
        if !plan.ok() {
            return Err(Error::exit(2, ""));
        }
        return formflow::expect_hash(plan.hash(), plan.errors(), expect);
    }
    let result = match plan {
        SrReviewed::Form(s) => {
            formflow::expect_hash(&s.plan.hash, &s.plan.errors, expect)?;
            s.session.send(&s.plan, a.allow_duplicate)?
        }
        SrReviewed::Flow(p) => {
            let mut plan = *p;
            let mut draft = draft;
            if tty && !a.yes {
                let rebuild = |d: &SrDraft, ack: bool| build(d, ack || a.ack, None);
                (plan, draft) = sr_confirm(plan, draft, &rebuild)?;
            } else {
                if !tty && !a.yes {
                    plan.review.render(err());
                    return Err(Error::exit(
                        2,
                        "not a terminal: review with --dry-run, then send with --yes --expect-plan HASH",
                    ));
                }
                if !plan.ok() {
                    plan.review.render(err());
                    return Err(Error::exit(
                        2,
                        format!("cannot place the request: {}", plan.errors.join("; ")),
                    ));
                }
                let blocking = plan.review.blocking_unacked();
                if !blocking.is_empty() {
                    plan.review.render(err());
                    return Err(Error::exit(
                        2,
                        format!(
                            "blocking rule message(s) need --ack: {}",
                            blocking.join("; ")
                        ),
                    ));
                }
                formflow::expect_hash(&plan.hash, &plan.errors, expect)?;
            }
            let ack_now = a.ack || !plan.state.acknowledged.is_empty();
            let mut rebuild = |mt: cat::ItemMeta| build(&draft, ack_now, Some(mt));
            let send = SrSend {
                tty,
                yes: a.yes,
                expect,
                allow_duplicate: a.allow_duplicate,
            };
            sr_send(plan, &deps, &mut rebuild, &send)?
        }
    };
    let get = |k: &str| result.get(k).cloned().unwrap_or(Value::Null);
    let created = get("created");
    if as_json {
        let mut o = Map::new();
        for (k, src) in [
            ("created", "created"),
            ("result", "result"),
            ("plan_hash", "plan_hash"),
            ("uploaded", "uploaded"),
            ("steps", "steps"),
            ("verification", "verification"),
        ] {
            o.insert(k.into(), get(src));
        }
        print_json(&Value::Object(o));
        return Ok(());
    }
    let url = match created.get("id").filter(|v| !v.is_null()) {
        Some(id) => cat::domain_url(&format!("/a/tickets/{}", fsv_py::py_str(id)), &domain),
        None => String::new(),
    };
    for u in get("uploaded").as_array().into_iter().flatten() {
        let s = |k: &str| fsv_py::py_str(u.get(k).unwrap_or(&Value::Null));
        out().print(&format!(
            "  [green]ok[/green] uploaded {} -> {} ({})",
            escape(&s("name")),
            s("id"),
            escape(&s("target"))
        ));
    }
    out().print(&format!(
        "[green]placed[/green] {}  {url}",
        escape(&fsv_py::py_str(
            created.get("display").unwrap_or(&Value::Null)
        ))
    ));
    if let Some(w) = result
        .get("verification")
        .and_then(|v| v.get("warning"))
        .filter(|w| fsv_py::truthy(w))
    {
        err().print(&fsv_py::py_str(w));
    }
    Ok(())
}

fn get(a: SrGetArgs) -> CmdResult {
    if a.download.as_deref().is_some_and(|d| d.starts_with('-')) {
        eprintln!("Usage: fsv sr get [OPTIONS] SR-ID");
        eprintln!("Try 'fsv sr get --help' for help.");
        eprintln!();
        eprintln!(
            "Error: Invalid value for '--download': expected a directory, not an option; use ./ for names starting with -"
        );
        return Err(Error::exit(2, ""));
    }
    let (mut items, mut attachments) = (a.items, a.attachments);
    if !items && !attachments && a.download.is_none() {
        items = true;
        attachments = true;
    }
    let tid = parse_id(&a.sr_id, &TICKETS).map_err(guard)?;
    let c = client()?;
    let raw = c.int_get(&format!("tickets/{tid}"), None).map_err(guard)?;
    let ticket = match &raw {
        Value::Object(m) => m.get("ticket").cloned().unwrap_or_else(|| raw.clone()),
        _ => json!({}),
    };
    let reqs = fsv_api::service::get_requested_items(c.as_ref(), tid).map_err(guard)?;
    let mut labels = Vec::new();
    for it in &reqs {
        labels.push(cat::item_labels(c.as_ref(), it).map_err(guard)?);
    }
    let ticket_atts: Vec<Value> = ticket
        .get("attachments")
        .filter(|v| fsv_py::truthy(v))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let names: Vec<(String, String)> = ticket_atts
        .iter()
        .filter(|x| x.get("name").is_some_and(fsv_py::truthy))
        .map(|x| (py_get(x, "id"), py_get(x, "name")))
        .collect();
    let name_of = |key: &str| {
        names
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    };
    let mut out_items: Vec<Value> = Vec::new();
    for (it, lab) in reqs.iter().zip(&labels) {
        let meta = it
            .get("item")
            .filter(|v| fsv_py::truthy(v))
            .cloned()
            .unwrap_or(json!({}));
        let mut vals = Vec::new();
        for (name, v) in custom_fields(it) {
            if is_blank(&v) {
                continue;
            }
            let text = if cat::is_file_value(&v) {
                v.as_array()
                    .into_iter()
                    .flatten()
                    .map(|x| match x {
                        Value::Object(_) => match x.get("name").filter(|n| fsv_py::truthy(n)) {
                            Some(n) => fsv_py::py_str(n),
                            None => py_get(x, "id"),
                        },
                        other => {
                            let k = fsv_py::py_str(other);
                            name_of(&k).unwrap_or_else(|| format!("attachment {k}"))
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            } else {
                cat::value_text(&v)
            };
            let label = lab.get(&name).cloned().unwrap_or_else(|| name.clone());
            vals.push(json!({"label": label, "name": name, "value": text, "raw": v}));
        }
        let stage = match it.get("stage") {
            Some(Value::Object(m)) => m.get("name").cloned().unwrap_or(Value::Null),
            Some(other) => other.clone(),
            None => Value::Null,
        };
        out_items.push(
            json!({"item": meta.get("name").cloned().unwrap_or(Value::Null),
                              "display_id": meta.get("display_id").cloned().unwrap_or(Value::Null),
                              "stage": stage, "values": vals}),
        );
    }
    let want_atts = attachments || a.download.is_some();
    let att_rows = if want_atts {
        attachment_rows(&ticket_atts, &reqs, &labels)
    } else {
        Vec::new()
    };
    let mut downloads: Vec<Value> = Vec::new();
    if let Some(dir) = &a.download {
        for att in &att_rows {
            downloads.push(download_one(&c, att, dir, a.force).map_err(guard)?);
        }
    }
    let shown_id = match ticket.get("human_display_id").filter(|v| fsv_py::truthy(v)) {
        Some(h) => h.clone(),
        None => {
            let did = ticket
                .get("display_id")
                .filter(|v| fsv_py::truthy(v))
                .map_or_else(|| tid.to_string(), fsv_py::py_str);
            json!(format!("SR-{did}"))
        }
    };
    let mut doc = Map::new();
    doc.insert("id".into(), shown_id.clone());
    doc.insert(
        "subject".into(),
        ticket.get("subject").cloned().unwrap_or(Value::Null),
    );
    if items {
        doc.insert("items".into(), Value::Array(out_items.clone()));
    }
    let att_doc: Vec<Value> = att_rows
        .iter()
        .map(|x| {
            let size = x
                .get("size")
                .filter(|v| fsv_py::truthy(v))
                .or_else(|| x.get("file_size"))
                .cloned()
                .unwrap_or(Value::Null);
            json!({"id": x.get("id").cloned().unwrap_or(Value::Null),
                   "name": x.get("name").cloned().unwrap_or(Value::Null),
                   "size": size, "source": x.get("source").cloned().unwrap_or(Value::Null)})
        })
        .collect();
    if want_atts {
        doc.insert("attachments".into(), Value::Array(att_doc.clone()));
    }
    if a.download.is_some() {
        doc.insert("downloads".into(), Value::Array(downloads.clone()));
    }
    let status = |d: &Value| {
        d.get("status")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let bad = downloads
        .iter()
        .any(|d| matches!(status(d).as_str(), "rejected" | "error"));
    if a.output == "json" {
        if let Some(Value::Array(list)) = doc.get_mut("items") {
            for it in list {
                if let Some(Value::Array(vals)) = it.get_mut("values") {
                    for v in vals {
                        if let Value::Object(m) = v {
                            m.shift_remove("raw");
                        }
                    }
                }
            }
        }
        print_json(&Value::Object(doc));
        return if bad { Err(Error::exit(2, "")) } else { Ok(()) };
    }
    out().rule(&format!(
        "[bold cyan]{}[/]  {}",
        escape(&fsv_py::py_str(&shown_id)),
        escape(&text_of(ticket.get("subject")))
    ));
    if items {
        if out_items.is_empty() {
            out().print("[yellow]no requested items (not a service request?)[/yellow]");
        }
        for it in &out_items {
            let name = it
                .get("item")
                .filter(|v| fsv_py::truthy(v))
                .map_or_else(|| "requested item".into(), fsv_py::py_str);
            let stage = it
                .get("stage")
                .filter(|v| fsv_py::truthy(v))
                .map_or_else(|| "-".into(), fsv_py::py_str);
            out().print(&format!(
                "[bold]{}[/bold]  [dim]#{} · stage {}[/dim]",
                escape(&name),
                fsv_py::py_str(&it["display_id"]),
                escape(&stage)
            ));
            let mut t = Table::new().show_header(false).no_box().pad_edge(false);
            t.add_column(Column::new("").style("cyan").no_wrap());
            t.add_column(Column::new("").overflow(Overflow::Fold));
            for v in it["values"].as_array().into_iter().flatten() {
                t.add_row([
                    escape(&fsv_py::py_str(&v["label"])),
                    escape(&fsv_py::py_str(&v["value"])),
                ]);
            }
            out().print_table(&t);
        }
    }
    if want_atts {
        out().print("[bold]Attachments[/bold]");
        if att_rows.is_empty() {
            out().print("  (none)");
        }
        for x in &att_doc {
            let size = match &x["size"] {
                Value::Number(n) if n.is_i64() || n.is_u64() => {
                    format!("{} B", thousands(&n.to_string()))
                }
                Value::Bool(b) => format!("{} B", i32::from(*b)),
                _ => String::new(),
            };
            out().print(&format!(
                "  {}  {size}  [dim]id {} · {}[/dim]",
                escape(&fsv_py::py_str(&x["name"])),
                fsv_py::py_str(&x["id"]),
                escape(&fsv_py::py_str(&x["source"]))
            ));
        }
    }
    for d in &downloads {
        let st = status(d);
        let style = match st.as_str() {
            "downloaded" => "green",
            "exists" => "yellow",
            _ => "red",
        };
        let extra = d
            .get("error")
            .filter(|v| fsv_py::truthy(v))
            .map(fsv_py::py_str)
            .unwrap_or_default();
        let line = format!(
            "  [{style}]{st}[/{style}] {}  {}",
            escape(&fsv_py::py_str(&d["name"])),
            escape(&extra)
        );
        out().print(line.trim_end());
    }
    if let Some(dir) = &a.download
        && downloads
            .iter()
            .any(|d| matches!(status(d).as_str(), "downloaded" | "exists"))
    {
        out().print(&format!("Saved to {}", escape(dir)));
    }
    if bad {
        return Err(Error::exit(2, ""));
    }
    Ok(())
}

fn py_get(x: &Value, k: &str) -> String {
    fsv_py::py_str(x.get(k).unwrap_or(&Value::Null))
}

fn custom_fields(it: &Value) -> Vec<(String, Value)> {
    it.get("custom_fields")
        .and_then(Value::as_object)
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}

/// `v in (None, "", [], False)` (`0 == False` too).
fn is_blank(v: &Value) -> bool {
    match v {
        Value::Null | Value::Bool(false) => true,
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Number(n) => n.as_f64() == Some(0.0),
        _ => false,
    }
}

fn thousands(digits: &str) -> String {
    let (sign, body) = digits.strip_prefix('-').map_or(("", digits), |b| ("-", b));
    let mut out = String::new();
    for (i, ch) in body.chars().enumerate() {
        if i > 0 && (body.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    format!("{sign}{out}")
}

/// `_attachment_rows`.
fn attachment_rows(
    ticket_atts: &[Value],
    items: &[Value],
    labels: &[indexmap::IndexMap<String, String>],
) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut by_id: Vec<(String, usize)> = Vec::new();
    let set = |by_id: &mut Vec<(String, usize)>, key: String, at: usize| match by_id
        .iter_mut()
        .find(|(k, _)| *k == key)
    {
        Some(slot) => slot.1 = at,
        None => by_id.push((key, at)),
    };
    for x in ticket_atts {
        if let Value::Object(m) = x {
            let mut row = m.clone();
            row.insert("source".into(), json!("request"));
            out.push(Value::Object(row));
            if m.get("id").is_some_and(|v| !v.is_null()) {
                set(&mut by_id, py_get(x, "id"), out.len() - 1);
            }
        }
    }
    for (it, lab) in items.iter().zip(labels) {
        for (name, v) in custom_fields(it) {
            if !cat::is_file_value(&v) {
                continue;
            }
            for x in v.as_array().into_iter().flatten() {
                let key = match x {
                    Value::Object(_) => py_get(x, "id"),
                    other => fsv_py::py_str(other),
                };
                let src = lab.get(&name).cloned().unwrap_or_else(|| name.clone());
                if let Some((_, at)) = by_id.iter().find(|(k, _)| *k == key) {
                    if let Value::Object(m) = &mut out[*at] {
                        m.insert("source".into(), json!(src));
                    }
                    continue;
                }
                let mut row = match x {
                    Value::Object(m) => m.clone(),
                    other => {
                        let mut m = Map::new();
                        m.insert("id".into(), other.clone());
                        m.insert(
                            "name".into(),
                            json!(format!("attachment-{}", fsv_py::py_str(other))),
                        );
                        m
                    }
                };
                row.insert("source".into(), json!(src));
                out.push(Value::Object(row));
                set(&mut by_id, key, out.len() - 1);
            }
        }
    }
    out
}

/// `_download`.
fn download_one(
    c: &fsv_core::Client,
    att: &Value,
    directory: &str,
    force: bool,
) -> fsv_core::Result<Value> {
    use fsv_api::service::Api;
    let name = match att.get("name").filter(|v| fsv_py::truthy(v)) {
        Some(n) => fsv_py::py_str(n),
        None => format!(
            "attachment-{}",
            att.get("id").map(fsv_py::py_str).unwrap_or_default()
        ),
    };
    let mut row = Map::new();
    row.insert("id".into(), att.get("id").cloned().unwrap_or(Value::Null));
    row.insert("name".into(), json!(name));
    row.insert(
        "source".into(),
        att.get("source").cloned().unwrap_or(Value::Null),
    );
    let with = |mut row: Map<String, Value>, extra: Value| {
        if let Value::Object(m) = extra {
            row.extend(m);
        }
        Value::Object(row)
    };
    let target = match cat::safe_target(directory, &name) {
        Ok(t) => t,
        Err(e) => {
            return Ok(with(
                row,
                json!({"status": "rejected", "error": e.to_string()}),
            ));
        }
    };
    let domain = Api::domain(c);
    let mut url = fsv_api::create::attachment_url(att, &domain);
    if url.is_empty() && att.get("id").is_some_and(|v| !v.is_null()) {
        url = cat::domain_url(
            &format!("/helpdesk/attachments/{}?download=true", py_get(att, "id")),
            &domain,
        );
    }
    if url.is_empty() {
        return Ok(with(
            row,
            json!({"status": "error", "error": "no download URL"}),
        ));
    }
    let path = target.to_string_lossy().to_string();
    if target.exists() && !force {
        return Ok(with(row, json!({"path": path, "status": "exists"})));
    }
    let r = Api::fetch(c, &url)?;
    if r.status >= 400 {
        return Ok(with(
            row,
            json!({"status": "error", "error": format!("HTTP {}", r.status)}),
        ));
    }
    let dir = std::path::PathBuf::from(expanduser(directory));
    std::fs::create_dir_all(&dir).map_err(|e| Error::other(e.to_string()))?;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true);
    if force {
        opts.create(true).truncate(true);
    } else {
        opts.create_new(true);
    }
    match opts.open(&target) {
        Ok(mut fh) => {
            use std::io::Write;
            fh.write_all(&r.body)
                .map_err(|e| Error::other(e.to_string()))?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Ok(with(row, json!({"path": path, "status": "exists"})));
        }
        Err(e) => return Err(Error::other(e.to_string())),
    }
    Ok(with(
        row,
        json!({"path": path, "size": r.body.len(), "status": "downloaded"}),
    ))
}

/// `_guard`: form, session, API and value errors print `error: ...`.
fn guard(e: Error) -> Error {
    match e {
        Error::Exit { .. } | Error::Aborted => fail_exc(e),
        other => {
            let msg = fsv_core::pystr::split_ws(&other.to_string()).join(" ");
            fail(&escape(&msg), other.exit_code())
        }
    }
}

/// `_json`.
fn print_json(v: &Value) {
    println!(
        "{}",
        fsv_py::Dumps::default()
            .indent(2)
            .ensure_ascii(false)
            .dump(v)
    );
}

fn text_of(v: Option<&Value>) -> String {
    v.filter(|x| fsv_py::truthy(x))
        .map(fsv_py::py_str)
        .unwrap_or_default()
}

fn fold_table(title: Option<String>, cols: &[&str]) -> Table {
    let mut t = Table::new();
    if let Some(title) = title {
        t = t.title(title);
    }
    for c in cols {
        t.add_column(Column::new(*c).overflow(Overflow::Fold));
    }
    t
}

/// `catalog_cmd`.
fn catalog(a: SrCatalogArgs) -> CmdResult {
    let c = client()?;
    let cats = cat::list_categories(c.as_ref()).map_err(guard)?;
    if a.categories_only {
        if a.output == "json" {
            print_json(&Value::Array(cats));
            return Ok(());
        }
        let mut t = fold_table(
            Some("Service catalog categories".into()),
            &["ID", "Category", "Description"],
        );
        for x in &cats {
            t.add_row([
                fsv_py::py_str(x.get("id").unwrap_or(&Value::Null)),
                escape(&text_of(x.get("name"))),
                escape(&one_line(&text_of(x.get("description")), 160)),
            ]);
        }
        out().print_table(&t);
        return Ok(());
    }
    let items = cat::list_items(c.as_ref(), a.popular, cat::MAX_PAGES).map_err(guard)?;
    let ids: Option<Vec<Value>> = match a.category.as_deref().filter(|s| !s.is_empty()) {
        Some(text) => Some(
            cat::resolve_category(&cats, text)
                .map_err(guard)?
                .iter()
                .map(|x| x.get("id").cloned().unwrap_or(Value::Null))
                .collect(),
        ),
        None => None,
    };
    let search = a.search.as_deref().filter(|s| !s.is_empty());
    let mut rows = cat::filter_items(&items, ids.as_deref(), search, &cats);
    for r in &mut rows {
        if let Value::Object(m) = r
            && !m.contains_key("category_name")
        {
            let cid = m.get("category_id").cloned().unwrap_or(Value::Null);
            let name = cats
                .iter()
                .rev()
                .find(|x| x.get("id").cloned().unwrap_or(Value::Null) == cid)
                .map(|x| text_of(x.get("name")))
                .unwrap_or_default();
            m.insert("category_name".into(), json!(name));
        }
    }
    let key = |r: &Value| {
        (
            cat::norm(&json!(text_of(r.get("category_name")))),
            cat::norm(&json!(text_of(r.get("name")))),
        )
    };
    rows.sort_by_key(key);
    let get = |r: &Value, k: &str| r.get(k).cloned().unwrap_or(Value::Null);
    if a.output == "json" {
        let doc: Vec<Value> = rows
            .iter()
            .map(|r| {
                json!({"display_id": get(r, "display_id"), "id": get(r, "id"), "name": get(r, "name"),
                       "short_description": get(r, "short_description"), "category_id": get(r, "category_id"),
                       "category": get(r, "category_name")})
            })
            .collect();
        print_json(&Value::Array(doc));
        return Ok(());
    }
    let title = format!(
        "Service catalog{}",
        if a.popular {
            " (frequently requested)"
        } else {
            ""
        }
    );
    let mut t = fold_table(
        Some(title),
        &["ID", "Item", "Category", "Short description"],
    );
    for r in &rows {
        t.add_row([
            fsv_py::py_str(&get(r, "display_id")),
            escape(&text_of(r.get("name"))),
            escape(&text_of(r.get("category_name"))),
            escape(&one_line(&text_of(r.get("short_description")), 160)),
        ]);
    }
    out().print_table(&t);
    if rows.is_empty() {
        err().print("[yellow]no items match[/yellow]");
    } else {
        err().print(&format!(
            "[dim]{} item(s); see one with: fsv sr fields ID[/dim]",
            rows.len()
        ));
    }
    Ok(())
}

/// `_attachment_rule`.
fn attachment_rule(item: &fsv_forms::spec::ServiceItem) -> &'static str {
    if !item.allow_attachments {
        "not accepted"
    } else if item.attachment_mandatory {
        "required: at least one general file"
    } else {
        "optional"
    }
}

/// `fields_doc`.
fn fields_doc(meta: &cat::ItemMeta) -> fsv_core::Result<Value> {
    let form = |e: fsv_forms::FormError| Error::exit(2, e.to_string());
    let spec = meta.spec();
    let item = spec
        .item
        .clone()
        .ok_or_else(|| Error::other("catalog item form has no item"))?;
    let engine = meta.engine()?;
    let n_rules = engine.rules.len();
    let state = fsv_forms::FormState::new(
        spec.clone(),
        Some(engine),
        "new",
        None,
        fsv_forms::StateOptions::default(),
    )
    .map_err(form)?;
    let ev = &state.evaluated;
    let (diagnostics, blockers) = cat::rule_diagnostics(meta, &state);
    let uncertain =
        !blockers.is_empty() || !fsv_api::formmeta::rules_warnings(&meta.rules).is_empty();
    let effective =
        (!uncertain).then(|| ev.attachments_required.unwrap_or(item.attachment_mandatory));
    let mut rule = attachment_rule(&item).to_string();
    if item.allow_attachments && uncertain {
        rule.push_str(" (base; effective requirement unknown)");
    } else if item.allow_attachments && ev.attachments_required.is_some() {
        rule.push_str(&format!(
            "; effective at load: {} (business rule)",
            if effective == Some(true) {
                "required"
            } else {
                "optional"
            }
        ));
    }
    let norm = |s: &str| cat::norm(&json!(s));
    let opt = |s: &str| if s.is_empty() { Value::Null } else { json!(s) };
    let mut rows = Vec::new();
    for f in spec.ordered() {
        let visible = ev.visible.contains(&f.api_name);
        let d = ev.decision(&f.api_name, "visible");
        let hidden = match d {
            Some(d) if !visible && d.action == "hide" => {
                if d.rule_name.is_empty() {
                    d.rule_id.clone()
                } else {
                    json!(d.rule_name)
                }
            }
            _ => Value::Null,
        };
        let mut row = Map::new();
        row.insert("name".into(), json!(f.api_name));
        row.insert("label".into(), json!(f.label));
        row.insert("kind".into(), json!(f.kind.as_str()));
        let required = if visible {
            ev.required.contains(&f.api_name)
        } else {
            f.required_base
        };
        row.insert("required".into(), json!(required));
        row.insert("visible_at_load".into(), json!(visible));
        row.insert("hidden_by_rule".into(), hidden);
        row.insert("placeholder".into(), opt(&f.placeholder));
        row.insert("help".into(), opt(f.help()));
        if f.kind.as_str() == "notice" {
            let content =
                fsv_core::pystr::split_ws(&fsv_forms::spec::plain_text(&f.content)).join(" ");
            row.insert("content".into(), json!(content));
            row.insert("required".into(), json!(false));
            row.insert("input".into(), json!(false));
        }
        if let Some((parent, choices)) = spec.section_parent(&f.api_name) {
            let wanted: Vec<String> = choices.iter().map(|x| norm(x)).collect();
            let labels: Vec<String> = parent
                .choices
                .iter()
                .filter(|c| wanted.contains(&norm(&c.value)))
                .map(|c| c.label.clone())
                .collect();
            let shown = if labels.is_empty() {
                choices.clone()
            } else {
                labels
            };
            row.insert(
                "shown_when".into(),
                json!({"field": parent.label, "choices": shown}),
            );
        }
        if !f.choices.is_empty() {
            let mut reveals: Vec<(String, Vec<String>)> = Vec::new();
            for s in &f.sections {
                for ch in &s.choices {
                    let labels = s
                        .fields
                        .iter()
                        .filter_map(|n| spec.get(n))
                        .map(|x| x.label.clone());
                    match reveals.iter_mut().find(|(k, _)| k == ch) {
                        Some((_, v)) => v.extend(labels),
                        None => reveals.push((ch.clone(), labels.collect())),
                    }
                }
            }
            let list: Vec<Value> = f
                .choices
                .iter()
                .filter(|c| c.visible)
                .map(|c| {
                    let key = norm(&c.value);
                    let rv = reveals.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone()).unwrap_or_default();
                    json!({"value": c.value, "label": c.label, "description": opt(&c.description), "reveals": rv})
                })
                .collect();
            row.insert("choices".into(), Value::Array(list));
        }
        if f.kind.as_str() == "attachment" {
            row.insert("allowed_file_types".into(), json!(f.allowed_file_types));
            row.insert("max_files".into(), json!(f.max_files));
        }
        rows.push(Value::Object(row));
    }
    let mut warnings: Vec<String> = spec.warnings.clone();
    warnings.extend(diagnostics);
    warnings.extend(blockers);
    warnings.push(cat::REQUESTED_FOR_NOTE.to_string());
    Ok(json!({
        "item": {"id": item.id, "display_id": item.display_id, "name": item.name,
                 "short_description": item.short_description, "category": item.category_name,
                 "description": fsv_forms::spec::plain_text(&item.description_html),
                 "attachments": {"allowed": item.allow_attachments, "mandatory": item.attachment_mandatory,
                                 "effective_mandatory": if item.allow_attachments { json!(effective) } else { json!(false) },
                                 "rule": rule},
                 "subject_template": opt(&item.subject_template)},
        "fields": rows,
        "rules": n_rules,
        "warnings": warnings,
        "metadata": Value::Object(meta.as_of()),
    }))
}

/// `fields_cmd`.
fn fields(a: SrFieldsArgs) -> CmdResult {
    let c = client()?;
    let store = fsv_api::formmeta::DiskStore::current();
    let meta = cat::load_item_meta(c.as_ref(), &store, &a.item, None).map_err(guard)?;
    let doc = fields_doc(&meta).map_err(guard)?;
    if a.output == "json" {
        print_json(&doc);
        return Ok(());
    }
    let it = &doc["item"];
    let s = |v: &Value| fsv_py::py_str(v);
    let category = text_of(it.get("category"));
    out().print(&format!(
        "[bold]{}[/bold]  [dim]#{}{}[/dim]",
        escape(&s(&it["name"])),
        s(&it["display_id"]),
        if category.is_empty() {
            String::new()
        } else {
            format!(" · {}", escape(&category))
        }
    ));
    let short = text_of(it.get("short_description"));
    if !short.is_empty() {
        out().print(&escape(&short));
    }
    let desc = text_of(it.get("description"));
    if !desc.is_empty() {
        out().print(&format!("[dim]{}[/dim]", escape(&one_line(&desc, 600))));
    }
    out().print(&format!(
        "Attachments: {}",
        escape(&s(&it["attachments"]["rule"]))
    ));
    let mut t = fold_table(None, &["Field", "Required", "Type", "Details"]);
    for r in doc["fields"].as_array().into_iter().flatten() {
        let label = s(&r["label"]);
        let kind = s(&r["kind"]);
        if kind == "notice" {
            t.add_row([
                format!("[dim]{}[/dim]", escape(&label)),
                String::new(),
                "note".into(),
                escape(&one_line(&text_of(r.get("content")), 400)),
            ]);
            continue;
        }
        let shown = r.get("shown_when").filter(|v| fsv_py::truthy(v));
        let required = fsv_py::truthy(&r["required"]);
        let mut req = if required {
            "[bold]yes[/bold]".to_string()
        } else {
            "no".to_string()
        };
        if shown.is_some() && required {
            req.push_str(" [dim](when shown)[/dim]");
        }
        let kind_text = if kind == "attachment" {
            "file(s)".to_string()
        } else {
            kind.clone()
        };
        let mut details: Vec<String> = Vec::new();
        if let Some(sw) = shown {
            let choices: Vec<String> = sw["choices"]
                .as_array()
                .into_iter()
                .flatten()
                .map(fsv_py::py_str)
                .collect();
            details.push(format!(
                "shown when {} = {}",
                s(&sw["field"]),
                choices.join(" / ")
            ));
        }
        if let Some(h) = r.get("hidden_by_rule").filter(|v| fsv_py::truthy(v)) {
            details.push(format!("hidden at load by rule {}", s(h)));
        }
        let help = text_of(r.get("help"));
        if !help.is_empty() {
            details.push(one_line(&help, 160));
        }
        let ph = text_of(r.get("placeholder"));
        if !ph.is_empty() {
            details.push(format!("e.g. {}", one_line(&ph, 80)));
        }
        for ch in r
            .get("choices")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let mut line = format!("• {}", s(&ch["label"]));
            let d = text_of(ch.get("description"));
            if !d.is_empty() {
                line.push_str(&format!(" — {d}"));
            }
            let rv: Vec<String> = ch["reveals"]
                .as_array()
                .into_iter()
                .flatten()
                .map(fsv_py::py_str)
                .collect();
            if !rv.is_empty() {
                line.push_str(&format!(" → reveals: {}", rv.join(", ")));
            }
            details.push(line);
        }
        if kind == "attachment" {
            let types: Vec<String> = r
                .get("allowed_file_types")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(fsv_py::py_str)
                .collect();
            let types = if types.is_empty() {
                "any type".to_string()
            } else {
                types.join(", ")
            };
            let mx = r.get("max_files").filter(|v| fsv_py::truthy(v));
            details.push(format!(
                "types: {types}{}",
                mx.map(|m| format!(" · at most {} file(s)", s(m)))
                    .unwrap_or_default()
            ));
        }
        let indent = if shown.is_some() { "  ↳ " } else { "" };
        t.add_row([
            escape(&format!("{indent}{label}")),
            req,
            kind_text,
            escape(&details.join("\n")),
        ]);
    }
    out().print_table(&t);
    for w in doc["warnings"].as_array().into_iter().flatten() {
        err().print(&format!("[yellow]warning[/yellow]: {}", escape(&s(w))));
    }
    let md = &doc["metadata"];
    out().print(&format!(
        "[dim]item form as of {}; {} business rule(s) apply. Create with: fsv sr create {} (interactive form)[/dim]",
        escape(&s(md.get("updated_at").unwrap_or(&Value::Null))),
        s(&doc["rules"]),
        s(&it["display_id"])
    ));
    Ok(())
}

/// `Path.expanduser`.
pub(crate) fn expanduser(s: &str) -> String {
    let home = || std::env::var("HOME").unwrap_or_default();
    if s == "~" {
        home()
    } else if let Some(rest) = s.strip_prefix("~/") {
        format!("{}/{rest}", home().trim_end_matches('/'))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Values from `cli_sr._attachment_rule`.
    #[test]
    fn attachment_rules() {
        let item = |allow: bool, mandatory: bool| fsv_forms::spec::ServiceItem {
            id: Value::Null,
            display_id: Value::Null,
            name: String::new(),
            short_description: String::new(),
            category_name: String::new(),
            description_html: String::new(),
            allow_attachments: allow,
            attachment_mandatory: mandatory,
            allow_quantity: false,
            subject_template: String::new(),
        };
        assert_eq!(attachment_rule(&item(false, true)), "not accepted");
        assert_eq!(
            attachment_rule(&item(true, true)),
            "required: at least one general file"
        );
        assert_eq!(attachment_rule(&item(true, false)), "optional");
    }
}
