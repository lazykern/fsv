//! `tickets`-only commands.

use clap::ArgMatches;
use serde_json::{Map, Value, json};

use fsv_api::associations::SendOpts;
use fsv_api::attachments::{self, ConversationAttach};
use fsv_api::conversations::{self, ConversationArgs};
use fsv_api::formmeta::DiskStore;
use fsv_api::notes::{self, BodyFormat};
use fsv_api::service::DiskJournal;
use fsv_core::resource::{CHANGES, PROBLEMS, TICKETS};
use fsv_core::term::{Column, Table, err, escape, out};
use fsv_core::{Error, Result, config, term};

use crate::cmd::attachments::finish;
use crate::cmd::changes::form_tty;
use crate::cmd::{formflow, parse, resource};
use crate::generated::*;
use crate::types::{OutputFormat, effective};
use crate::util::{
    CmdResult, cid, client, emit_fmt, fail_exc, first_str, interactive, no_input, obj, plain_fail,
    uncaught, usage,
};

pub fn dispatch(sub: &str, m: &ArgMatches) -> CmdResult {
    match sub {
        "conversations" => {
            let a: TicketsConversationsArgs = parse(m)?;
            resource::history(
                &TICKETS,
                resource::HistoryArgs {
                    id_: &a.id_,
                    page: a.page,
                    per_page: a.per_page,
                    all_pages: a.all_pages,
                    n_pages: a.n_pages,
                    fmt: effective(a.format_, a.json_out),
                },
            )
        }
        "reply" => reply(parse(m)?),
        "forward" => forward(parse(m)?),
        "create" => create(parse(m)?),
        "approvals" => approvals(parse(m)?),
        "associations" => associations(parse(m)?),
        _ => Err(Error::other(format!("unknown command: tickets {sub}"))),
    }
}

/// `str(d.get(k, "-"))`.
fn get_dash(v: &Value, key: &str) -> String {
    v.get(key).map_or_else(|| "-".into(), fsv_py::py_str)
}

/// `ticket_approvals_resource`.
fn approvals(a: TicketsApprovalsArgs) -> CmdResult {
    let id = cid(&a.id_, &TICKETS)?;
    let c = client()?;
    let items = match fsv_api::service::get_ticket_approvals(c.as_ref(), &TICKETS, id) {
        Ok(items) => items,
        Err(e) if e.status() == Some(404) => {
            let prefix = match a.id_.split_once('-') {
                Some((p, _)) => p.to_uppercase(),
                None => "ticket".into(),
            };
            let mut hint = "ticket approvals not available for this ticket".to_string();
            if prefix == "INC" {
                hint.push_str("; INC tickets often do not expose approvals, try SR or another approval-capable ticket");
            }
            return Err(usage(&hint));
        }
        Err(e @ Error::Api(_)) => return Err(fail_exc(e)),
        Err(e) => return Err(uncaught(e)),
    };
    use fsv_api::approval_model;
    let flat: Vec<Map<String, Value>> = items
        .iter()
        .map(|x| {
            let row = approval_model::flat_row(x);
            approval_model::FLAT_KEYS
                .iter()
                .map(|k| (k.to_string(), row[*k].clone()))
                .collect()
        })
        .collect();
    if emit_fmt(
        &Value::Array(approval_model::with_flat(&items)),
        &flat,
        effective(a.format_, a.json_out),
    )? {
        return Ok(());
    }
    out().print_table(&crate::cmd::inventory::flat_approval_table(&items));
    Ok(())
}

/// `schema.PRIORITY.get(item.get("priority") or 0, "-")`.
fn priority_label(item: &Value) -> String {
    let p = item.get("priority").filter(|v| fsv_py::truthy(v));
    let n = match p {
        None => Some(0),
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::Bool(b)) => Some(i64::from(*b)),
        _ => None,
    };
    n.and_then(|n| fsv_api::PRIORITY.iter().find(|(k, _)| *k == n))
        .map_or_else(|| "-".into(), |(_, v)| v.to_string())
}

/// `ticket_associations_resource`.
fn associations(a: TicketsAssociationsArgs) -> CmdResult {
    let id = cid(&a.id_, &TICKETS)?;
    let c = client()?;
    let tabs = c
        .int_get(&format!("tickets/{id}/tabs"), None)
        .map_err(fail_exc)?;
    let mut modules: Vec<String> = Vec::new();
    for tab in tabs.as_array().into_iter().flatten() {
        if tab.get("name").and_then(Value::as_str) == Some("associations") {
            for m in tab
                .get("associated_modules")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                modules.push(fsv_py::py_str(m));
            }
        }
    }
    let wants = |m: &str| modules.is_empty() || modules.iter().any(|x| x == m);
    let associated = |key: &str, params: Option<Map<String, Value>>| -> Result<Vec<Value>> {
        let got = fsv_api::service::collect_pages(
            |p, q| c.int_get(p, q),
            &format!("tickets/{id}/{key}"),
            key,
            params.as_ref(),
            key,
            fsv_api::service::MAX_DETAIL_PAGES,
        )
        .map_err(fail_exc)?;
        if let Some(p) = got.meta.get("partial").filter(|v| fsv_py::truthy(v)) {
            err().print(&escape(&fsv_py::py_str(p)));
        }
        if got.missing.is_some() && !got.meta.get("pages").is_some_and(fsv_py::truthy) {
            return Err(Error::exit(1, ""));
        }
        Ok(got.rows)
    };
    let change_type = |t: &str| Some(obj(vec![("change_type", json!(t))]));
    let changes = if wants("change") {
        associated("changes", change_type("change"))?
    } else {
        Vec::new()
    };
    let causes = if wants("change_cause") {
        associated("changes", change_type("change_cause"))?
    } else {
        Vec::new()
    };
    let problems = if wants("problem") {
        associated("problems", None)?
    } else {
        Vec::new()
    };
    let id_of = |item: &Value, res: &fsv_core::resource::Resource| match item
        .get("human_display_id")
        .filter(|v| fsv_py::truthy(v))
    {
        Some(h) => fsv_py::py_str(h),
        None => fsv_core::resource::format_id(item, res),
    };
    let row = |kind: &str, item: &Value| {
        let res = if kind == "problems" {
            &PROBLEMS
        } else {
            &CHANGES
        };
        [
            id_of(item, res),
            first_str(item, &["subject"], "-"),
            match item.get("status_name").filter(|v| fsv_py::truthy(v)) {
                Some(s) => fsv_py::py_str(s),
                None => get_dash(item, "status"),
            },
            match item.get("priority_name").filter(|v| fsv_py::truthy(v)) {
                Some(s) => fsv_py::py_str(s),
                None => priority_label(item),
            },
        ]
    };
    let payload = json!({"changes": changes, "change_causes": causes, "problems": problems});
    let mut flat = Vec::new();
    for (kind, items) in [
        ("changes", &changes),
        ("change_causes", &causes),
        ("problems", &problems),
    ] {
        for x in items {
            let [i, s, st, p] = row(kind, x);
            flat.push(obj(vec![
                ("type", json!(kind)),
                ("id", json!(i)),
                ("subject", json!(s)),
                ("status", json!(st)),
                ("priority", json!(p)),
            ]));
        }
    }
    if emit_fmt(&payload, &flat, effective(a.format_, a.json_out))? {
        return Ok(());
    }
    if changes.is_empty() && causes.is_empty() && problems.is_empty() {
        out().print("no associations");
        return Ok(());
    }
    for (title, kind, items) in [
        ("Changes", "changes", &changes),
        ("Caused by changes", "change_causes", &causes),
        ("Problems", "problems", &problems),
    ] {
        if items.is_empty() {
            continue;
        }
        let mut t = Table::new().title(title);
        t.add_column(Column::new("ID").style("cyan"));
        t.add_column(Column::new("Subject"));
        t.add_column(Column::new("Status"));
        if kind != "problems" {
            t.add_column(Column::new("Priority"));
        }
        for x in items {
            let [i, s, st, p] = row(kind, x);
            if kind == "problems" {
                t.add_row([i, s, st]);
            } else {
                t.add_row([i, s, st, p]);
            }
        }
        out().print_table(&t);
    }
    Ok(())
}

fn pretty(v: &Value) -> String {
    fsv_py::Dumps::default()
        .indent(2)
        .ensure_ascii(false)
        .dump(v)
}

/// `ValueError`/`NoteError` print plain and exit 2; anything else is uncaught.
fn note_error(e: Error) -> Error {
    match e {
        Error::Value(m) => plain_fail(&m, 2),
        Error::Exit { code: 2, message } if !message.is_empty() => plain_fail(&message, 2),
        other => uncaught(other),
    }
}

/// `--notify-email` plus one `Name <email>` entry per `--notify-agent`.
fn notify_with_agents(a: &TicketsAddNoteArgs) -> Result<Vec<String>> {
    let mut notify = a.notify.clone();
    if !a.notify_agent.is_empty() {
        let c = crate::util::shared_client()?;
        for name in &a.notify_agent {
            notify.push(fsv_api::templates::notify_entry(c.as_ref(), name)?);
        }
    }
    Ok(notify)
}

/// Reviewed `tickets add-note` (`ticket_notes.register`).
pub fn add_note(a: TicketsAddNoteArgs) -> CmdResult {
    add_note_inner(&a).map_err(note_error)
}

fn add_note_inner(a: &TicketsAddNoteArgs) -> Result<()> {
    let notify = notify_with_agents(a)?;
    if !a.attach.is_empty() {
        if a.read_only && !a.dry_run {
            return Err(Error::exit(2, "read-only mode: no upload attempted"));
        }
        if a.set_status.is_some() {
            return Err(Error::exit(2, "combined attachment/status wire not proven"));
        }
        if a.public && !a.confirm_public && !a.dry_run {
            return Err(Error::exit(2, "public note requires --confirm-public"));
        }
        let c = crate::util::shared_client()?;
        let store = DiskStore::current();
        let args = ConversationAttach {
            kind: "note",
            public: a.public,
            html_body: a.html_body,
            notify: &notify,
            max_bytes: a.max_bytes,
            ..Default::default()
        };
        let rebuild = || {
            attachments::conversation_plan(c.as_ref(), &store, &a.id_, &a.body, &a.attach, &args)
        };
        let expected = a.expected.as_deref();
        return finish(
            &rebuild()?,
            c.as_ref(),
            a.dry_run,
            a.yes,
            expected,
            &rebuild,
        );
    }
    if a.set_status.is_some() {
        if a.read_only && !a.dry_run {
            return Err(Error::exit(2, "read-only mode: no note was sent"));
        }
        let c = crate::util::shared_client()?;
        let store = DiskStore::current();
        let format = if a.html_body {
            BodyFormat::Html
        } else {
            BodyFormat::Plain
        };
        let args = ConversationArgs {
            kind: "note",
            public: a.public,
            notify: &notify,
            format,
            set_status: a.set_status.as_deref(),
            ..Default::default()
        };
        let plan = conversations::build_plan(c.as_ref(), &store, &a.id_, &a.body, &args)?;
        let review = plan.review();
        if a.dry_run {
            let mut out = serde_json::Map::new();
            out.insert("status".into(), json!("dry_run"));
            out.extend(review);
            println!("{}", pretty(&Value::Object(out)));
            return Ok(());
        }
        eprintln!("{}", pretty(&Value::Object(review)));
        let hash = plan.hash();
        if a.expected.as_deref().is_some_and(|e| e != hash) {
            return Err(Error::exit(
                2,
                "note review hash does not match; review again",
            ));
        }
        if a.yes {
            if a.expected.as_deref() != Some(hash.as_str()) {
                return Err(Error::exit(
                    2,
                    "--yes requires the full --expect-plan from a prior review",
                ));
            }
        } else if a.no_input {
            return Err(Error::exit(
                2,
                "sending requires --yes and --expect-plan in no-input mode",
            ));
        } else if !term::confirm("Add this exact reviewed note?", Some(false), false)? {
            println!("Cancelled; no note was sent.");
            return Ok(());
        }
        let mut confirm_public = a.confirm_public;
        if a.public && !confirm_public {
            if a.yes || a.no_input {
                return Err(Error::exit(2, "--public also requires --confirm-public"));
            }
            if !term::confirm(
                "Confirm PUBLIC note visible to the customer?",
                Some(false),
                false,
            )? {
                println!("Cancelled; no note was sent.");
                return Ok(());
            }
            confirm_public = true;
        }
        let result = conversations::send(
            &plan,
            c.as_ref(),
            &DiskJournal,
            &store,
            Some(&hash),
            confirm_public,
        )?;
        println!("{}", pretty(&result));
        return Ok(());
    }
    let format = if a.html_body {
        BodyFormat::Html
    } else {
        BodyFormat::Plain
    };
    let plan = notes::build_plan(
        &a.id_,
        &a.body,
        a.public,
        &notify,
        format,
        &config::domain(),
    )?;
    let mut review = Value::Object(plan.review());
    if a.dry_run {
        let mut out = serde_json::Map::new();
        out.insert("status".into(), json!("dry_run"));
        if let Value::Object(m) = &mut review {
            out.extend(std::mem::take(m));
        }
        println!("{}", pretty(&Value::Object(out)));
        return Ok(());
    }
    eprintln!("{}", pretty(&review));
    if a.read_only {
        return Err(Error::exit(2, "read-only mode: no note was sent"));
    }
    let hash = plan.hash();
    if a.expected.as_deref().is_some_and(|e| e != hash) {
        return Err(Error::exit(
            2,
            "note review hash does not match; review the exact body again",
        ));
    }
    if a.yes {
        if a.expected.as_deref() != Some(hash.as_str()) {
            return Err(Error::exit(
                2,
                "--yes requires the full --expect-plan from a prior body review",
            ));
        }
    } else if a.no_input {
        return Err(Error::exit(
            2,
            "sending requires --yes and --expect-plan in no-input mode",
        ));
    } else if !term::confirm("Add this exact reviewed note?", Some(false), false)? {
        println!("Cancelled; no note was sent.");
        return Ok(());
    }
    let mut confirm_public = a.confirm_public;
    if a.public && !confirm_public {
        if a.yes || a.no_input {
            return Err(Error::exit(2, "--public also requires --confirm-public"));
        }
        if !term::confirm(
            "Confirm PUBLIC note visible to the customer (not a reply)?",
            Some(false),
            false,
        )? {
            println!("Cancelled; no note was sent.");
            return Ok(());
        }
        confirm_public = true;
    }
    let c = crate::util::shared_client()?;
    let opts = SendOpts {
        reviewed_hash: Some(&hash),
        read_only: false,
        dry_run: false,
    };
    let result = notes::send(&plan, c.as_ref(), &DiskJournal, opts, confirm_public)?;
    if a.json_out {
        println!("{}", pretty(&result));
    } else {
        let s = |k: &str| result.get(k).map_or("None".to_string(), fsv_py::py_str);
        let identity = if result.get("identity_verified").is_some() {
            s("identity_verified")
        } else {
            s("verified")
        };
        println!(
            "Note {} submitted on ticket {}; exact-ID read-back: {identity}; all fields verified: {}",
            s("note_id"),
            plan.ticket_id,
            s("verified")
        );
        if let Some(w) = result.get("warning").filter(|w| fsv_py::truthy(w)) {
            eprintln!("{}", fsv_py::py_str(w));
        }
    }
    Ok(())
}

fn reply(a: TicketsReplyArgs) -> CmdResult {
    if !a.attach.is_empty() {
        return reply_attached(&a).map_err(crate::cmd::reviewed::refusal);
    }
    if a.cc.is_empty()
        && a.bcc.is_empty()
        && a.sender.is_none()
        && a.set_status.is_none()
        && a.expect_plan.is_none()
    {
        return resource::reply(&TICKETS, &a.id_, &a.body, a.dry_run, a.yes, a.no_input);
    }
    let c = client()?;
    reply_reviewed(&a, &c).map_err(note_error)
}

/// `tickets reply --attach`: one reviewed two-step reply, without
/// recipients or a status change.
fn reply_attached(a: &TicketsReplyArgs) -> Result<()> {
    if !a.cc.is_empty() || !a.bcc.is_empty() || a.set_status.is_some() {
        return Err(Error::exit(
            2,
            "combined attachment/recipient/status variant not proven",
        ));
    }
    let c = client()?;
    let store = DiskStore::current();
    let args = ConversationAttach {
        kind: "reply",
        sender: a.sender.as_deref(),
        max_bytes: a.max_bytes,
        ..Default::default()
    };
    let rebuild =
        || attachments::conversation_plan(c.as_ref(), &store, &a.id_, &a.body, &a.attach, &args);
    let expected = a.expect_plan.as_deref();
    finish(
        &rebuild()?,
        c.as_ref(),
        a.dry_run,
        a.yes,
        expected,
        &rebuild,
    )
}

fn reply_reviewed(a: &TicketsReplyArgs, c: &fsv_core::Client) -> Result<()> {
    let store = DiskStore::current();
    conversations::recipients(c, &a.bcc)?;
    let cc = conversations::recipients(c, &a.cc)?;
    let args = ConversationArgs {
        kind: "reply",
        cc: &cc,
        set_status: a.set_status.as_deref(),
        ..Default::default()
    };
    let mail = conversations::MailArgs {
        bcc: &a.bcc,
        sender: a.sender.as_deref(),
    };
    let plan = conversations::build_plan_mail(c, &store, &a.id_, &a.body, &args, &mail)?;
    let review = plan.review();
    if a.dry_run {
        let mut out = serde_json::Map::new();
        out.insert("status".into(), json!("dry_run"));
        out.extend(review);
        println!("{}", pretty(&Value::Object(out)));
        return Ok(());
    }
    eprintln!("{}", pretty(&Value::Object(review)));
    let hash = plan.hash();
    if a.expect_plan.as_deref().is_some_and(|e| e != hash) {
        return Err(Error::exit(2, "review hash changed; review again"));
    }
    if a.yes {
        if a.expect_plan.as_deref() != Some(hash.as_str()) {
            return Err(Error::exit(
                2,
                "--yes requires the full --expect-plan from a prior reply review",
            ));
        }
    } else if no_input(a.no_input) {
        return Err(Error::exit(
            2,
            "sending requires --yes and --expect-plan in no-input mode",
        ));
    } else if !term::confirm("Send this exact PUBLIC reply?", Some(false), false)? {
        println!("Cancelled; no reply sent.");
        return Ok(());
    }
    let result = conversations::send(&plan, c, &DiskJournal, &store, Some(&hash), true)?;
    println!("{}", pretty(&result));
    Ok(())
}

/// `tickets forward`: a whole-ticket or exact-conversation forward, JSON
/// review only.
fn forward(a: TicketsForwardArgs) -> CmdResult {
    use crate::cmd::reviewed::{dumps, print_dry_run, refusal, require_review};
    let c = client()?;
    let run = || -> Result<()> {
        let store = DiskStore::current();
        conversations::recipients(c.as_ref(), &a.to)?;
        let args = ConversationArgs {
            kind: "forward",
            notify: &a.to,
            ..Default::default()
        };
        let mail = conversations::MailArgs {
            bcc: &[],
            sender: Some(&a.sender),
        };
        let plan = match a.conversation {
            Some(id) => conversations::build_plan_forward_source(
                c.as_ref(),
                &store,
                &a.id_,
                &a.body,
                &args,
                &mail,
                id,
            )?,
            None => {
                conversations::build_plan_mail(c.as_ref(), &store, &a.id_, &a.body, &args, &mail)?
            }
        };
        if a.dry_run {
            print_dry_run(plan.review());
            return Ok(());
        }
        let hash = plan.hash();
        require_review(a.yes, a.expect_plan.as_deref(), &hash)?;
        let result =
            conversations::send(&plan, c.as_ref(), &DiskJournal, &store, Some(&hash), true)?;
        println!("{}", dumps(&result));
        Ok(())
    };
    run().map_err(refusal)
}

fn create(a: TicketsCreateArgs) -> CmdResult {
    let local_no_input = no_input(a.no_input);
    let _as_json = a.json_out || a.format_ == OutputFormat::Json;
    let tty = interactive() && !local_no_input;
    let prefill = match a.template.as_deref().filter(|t| !t.is_empty()) {
        Some(name) => Some(template_prefill(name)?),
        None => None,
    };
    let inputs = a.from_.is_some()
        || !a.set_.is_empty()
        || a.preset.is_some()
        || a.edit
        || prefill.is_some();
    let form = !inputs && tty && form_tty();
    let template_only = !inputs && !form && a.dry;
    if !(inputs || form || template_only) {
        return Err(Error::exit(
            2,
            "pass --from or --set, or open the interactive form in a terminal",
        ));
    }
    if !a.attach.is_empty() && (form || template_only || a.allow_duplicate) {
        return Err(Error::exit(
            2,
            "--attach requires explicit form answers and does not allow duplicate override",
        ));
    }
    if (form || template_only) && !a.cc.is_empty() {
        return Err(Error::exit(
            2,
            "--cc requires --from, --set, --preset or --edit; the interactive form and template cannot retain it",
        ));
    }
    if a.edit && !tty {
        return Err(Error::exit(
            2,
            "--edit needs a terminal; use --from or --set",
        ));
    }
    let c = client()?;
    ticket_create(&a, c, tty, form, template_only, prefill).map_err(fail_exc)
}

/// `--template NAME`: the template's prefill draft and the keys it applies.
fn template_prefill(name: &str) -> Result<(fsv_flows::drafts::Draft, Vec<String>)> {
    let c = client()?;
    let row = fsv_api::templates::show(c.as_ref(), name).map_err(|e| match e {
        Error::Value(m) => Error::exit(2, m),
        other => fail_exc(other),
    })?;
    let p = fsv_api::templates::to_draft(&row);
    let shown = row
        .get("name")
        .map_or_else(|| "None".to_string(), fsv_py::py_repr);
    let applied = if p.applied.is_empty() {
        "nothing".to_string()
    } else {
        p.applied.join(", ")
    };
    let ignored = if p.ignored.is_empty() {
        String::new()
    } else {
        format!("; not applied: {}", p.ignored.join(", "))
    };
    err().print_plain(&format!("template {shown} prefills: {applied}{ignored}"));
    let draft = fsv_flows::drafts::Draft {
        answers: p.answers,
        description_html: p.description_html,
        ..Default::default()
    };
    Ok((draft, p.applied))
}

/// `templates.prefill_changes` after a template create, reported on stderr.
fn template_readback(
    api: &dyn fsv_api::service::Api,
    id: &Value,
    payload: &Map<String, Value>,
    prefilled: &[String],
) -> Value {
    let check = fsv_api::templates::prefill_changes(api, id, payload, prefilled);
    for change in check["changes"].as_array().into_iter().flatten() {
        err().print_plain(&format!(
            "server changed {} from {} to {}",
            fsv_py::py_str(&change["field"]),
            fsv_py::py_str(&change["sent"]),
            fsv_py::py_str(&change["server"])
        ));
    }
    if let Some(w) = check.get("warning").filter(|w| fsv_py::truthy(w)) {
        err().print_plain(&fsv_py::py_str(w));
    } else if let Some(missing) = check
        .get("unavailable")
        .and_then(Value::as_array)
        .filter(|u| !u.is_empty())
    {
        let names: Vec<String> = missing.iter().map(fsv_py::py_str).collect();
        err().print_plain(&format!(
            "not in the ticket read-back: {}",
            names.join(", ")
        ));
    }
    check
}

/// `ticket_cli.create_command` after the input checks.
fn ticket_create(
    a: &TicketsCreateArgs,
    c: std::sync::Arc<fsv_core::Client>,
    tty: bool,
    form: bool,
    template_only: bool,
    prefill: Option<(fsv_flows::drafts::Draft, Vec<String>)>,
) -> CmdResult {
    use formflow::{Requester, Reviewed};
    use fsv_flows::change::{Plan, edit_loop, editor_edit, flow_exit, template_for};
    use fsv_flows::drafts::{Draft, resolve_draft};
    use fsv_flows::meta::{FormMeta, Resolver};
    use fsv_flows::ticket::{self, TicketOptions};
    use std::sync::Arc;

    let as_json = a.json_out || a.format_ == OutputFormat::Json;
    let deps = fsv_app::FormDeps::live(c)?;
    let fresh = a.dry || a.yes || !tty;
    let meta = Arc::new(deps.meta.load_meta("ticket", "new", fresh, &Value::Null)?);
    let draft = resolve_draft(
        a.from_.as_deref(),
        a.preset.as_deref(),
        &a.set_,
        Some(&Draft::default()),
    )
    .map_err(formflow::draft_error)?;
    let (draft, prefilled) = match prefill {
        Some((base, applied)) => (base.merged(&draft), applied),
        None => (draft, Vec::new()),
    };
    let who = Requester::new(deps.api.as_ref(), a.requester.clone(), tty);
    let resolver: Arc<dyn Resolver> = deps.resolver.clone();
    let build = |d: &Draft, ack_all: bool, m: Option<Arc<FormMeta>>| -> Result<Plan> {
        let (req, problem) = who.resolve(d)?;
        let opts = TicketOptions {
            resolver: Some(resolver.clone()),
            requester: req,
            ack_all,
            cc_emails: a.cc.clone(),
            devices: a.device.clone(),
        };
        let mut plan = ticket::build_create_plan(&m.unwrap_or_else(|| meta.clone()), d, &opts)?;
        if let Some(p) = problem {
            plan.errors.insert(0, p);
            plan.hash.clear();
            plan.review.plan_hash.clear();
        }
        Ok(plan)
    };

    if template_only {
        let plan = build(&draft, a.ack, None)?;
        print!(
            "{}",
            template_for(
                &plan,
                &draft,
                "fsv incident draft: fill in, then use tickets create --dry-run",
                &[],
                None,
                None
            )?
        );
        return Ok(());
    }
    let plan = if form {
        let (req, problem) = who.resolve(&draft)?;
        if let Some(p) = problem {
            return Err(flow_exit(2, p));
        }
        let session = fsv_app::ticket_session(&deps, Some(meta.clone()), req, a.dry)?;
        Reviewed::Form(Box::new(formflow::run_form(session)?))
    } else if a.edit {
        let mut b = |d: &Draft| build(d, a.ack, None);
        let mut ed = editor_edit;
        let (_, plan) = edit_loop(&mut b, draft, "New incident", None, &mut ed, 20)
            .map_err(formflow::editor_abort)?;
        Reviewed::Flow(Box::new(plan))
    } else {
        Reviewed::Flow(Box::new(build(&draft, a.ack, None)?))
    };

    if let (false, Reviewed::Flow(p)) = (a.attach.is_empty(), &plan) {
        let api = deps.api.as_ref();
        let attached = |current: &Plan| -> Result<attachments::AttachedPlan> {
            let blocking = current.review.blocking_unacked();
            let Some(payload) = current.payload.as_ref().filter(|_| current.ok()) else {
                return Err(incomplete(&current.errors));
            };
            if !blocking.is_empty() {
                return Err(incomplete(&current.errors));
            }
            let raw = attachments::metadata(api, "tickets")?;
            let context = fsv_core::journal::plan_hash(Some(&json!({
                "plan": current.hash, "fields": current.meta.form_fields,
                "rules": current.meta.rules, "metadata": raw,
            })))
            .unwrap_or_default();
            let base = fsv_api::reviewed_write::WritePlan {
                action: "ticket_create_attachments".into(),
                target: 0,
                tenant: config::domain(),
                method: "POST".into(),
                path: "tickets".into(),
                payload_json: fsv_py::Dumps::default()
                    .sort_keys(true)
                    .dump(&Value::Object(payload.clone())),
                context_hash: context,
            };
            let action = base.action.clone();
            let mut wrapped = attachments::wrap(base, &a.attach, &raw, api, a.max_bytes)?;
            let wrapped_hash = wrapped.hash();
            wrapped.requester_id = current
                .requester
                .as_ref()
                .map(|r| fsv_api::watchers::positive_id(&r.id))
                .transpose()?;
            wrapped.prior_guards = vec![
                (action, wrapped_hash),
                ("ticket_create".into(), current.hash.clone()),
            ];
            Ok(wrapped)
        };
        let ack_now = a.ack || !p.state.acknowledged.is_empty();
        let rebuild = || -> Result<attachments::AttachedPlan> {
            let fresh = Arc::new(deps.meta.load_meta("ticket", "new", true, &Value::Null)?);
            attached(&build(&p.draft, ack_now, Some(fresh))?)
        };
        let expected = a.expect_plan.as_deref();
        return finish(&attached(p)?, api, a.dry, a.yes, expected, &rebuild).map_err(|e| match e {
            Error::Exit { code: 2, message } if !message.is_empty() => flow_exit(2, message),
            other => other,
        });
    }

    if a.dry {
        ticket_print_plan(&plan, as_json);
        if !plan.ok() {
            return Err(flow_exit(2, ""));
        }
        return plan.expect(a.expect_plan.as_deref());
    }
    let is_form = matches!(plan, Reviewed::Form(_));
    if !is_form && tty && !a.yes {
        plan.render(err());
        if !plan.ok() || !plan.review().blocking_unacked().is_empty() {
            return Err(flow_exit(
                2,
                "fix the form or acknowledge blocking messages before creating",
            ));
        }
        if !term::confirm("Create this incident?", Some(false), false)? {
            return Err(flow_exit(1, "cancelled; nothing was sent"));
        }
    } else if !is_form {
        if !a.yes {
            return Err(flow_exit(
                2,
                "review with --dry-run, then use --yes --expect-plan HASH",
            ));
        }
        if !tty && a.expect_plan.as_deref().is_none_or(str::is_empty) {
            return Err(flow_exit(
                2,
                "non-interactive create requires --expect-plan HASH",
            ));
        }
    }
    let blocking = plan.review().blocking_unacked();
    if !plan.ok() || !blocking.is_empty() {
        let why = if plan.errors().is_empty() {
            blocking
        } else {
            plan.errors().to_vec()
        };
        return Err(flow_exit(2, format!("cannot create: {}", why.join("; "))));
    }
    plan.expect(a.expect_plan.as_deref())?;
    let sent_payload = plan.payload();
    let result = match plan {
        Reviewed::Form(s) => {
            let o = fsv_app::SendOptions {
                tty,
                yes: a.yes,
                expect: a.expect_plan.as_deref(),
                allow_duplicate: a.allow_duplicate,
                ack: a.ack,
                duplicate: "append",
            };
            s.send_cli(&o, &mut crate::cmd::changes::send_confirm)?
        }
        Reviewed::Flow(p) => {
            let ack_now = a.ack || !p.state.acknowledged.is_empty();
            let d = p.draft.clone();
            let mut rebuild = |m: Arc<FormMeta>| build(&d, ack_now, Some(m));
            ticket::send(
                *p,
                deps.api.as_ref(),
                deps.meta.as_ref(),
                &mut rebuild,
                deps.plan_journal.as_ref(),
                a.expect_plan.as_deref(),
                a.allow_duplicate,
            )?
        }
    };
    let get = |k: &str| result.get(k).cloned().unwrap_or(Value::Null);
    let template_check = match sent_payload {
        Some(Value::Object(p)) if !prefilled.is_empty() && !p.is_empty() => Some(
            template_readback(deps.api.as_ref(), &get("id"), &p, &prefilled),
        ),
        _ => None,
    };
    if as_json {
        let mut o = Map::new();
        o.insert("created".into(), get("result"));
        o.insert("plan_hash".into(), get("plan_hash"));
        if let Some(v) = template_check {
            o.insert("template_readback".into(), v);
        }
        if let Some(v) = result.get("verification") {
            o.insert("verification".into(), v.clone());
        }
        if let Some(v) = result.get("device_verification") {
            o.insert("device_verification".into(), v.clone());
        }
        println!("{}", fsv_py::Dumps::default().dump(&Value::Object(o)));
    } else {
        out().print(&format!(
            "[green]created[/green] incident {}",
            fsv_py::py_str(&get("id"))
        ));
        if let Some(w) = result
            .get("verification")
            .and_then(|v| v.get("warning"))
            .filter(|w| fsv_py::truthy(w))
        {
            err().print(&fsv_py::py_str(w));
        }
    }
    Ok(())
}

fn incomplete(errors: &[String]) -> Error {
    Error::exit(
        2,
        format!(
            "cannot attach to incomplete incident plan: {}",
            errors.join("; ")
        ),
    )
}

/// `ticket_cli.print_plan`.
fn ticket_print_plan(plan: &formflow::Reviewed, as_json: bool) {
    if as_json {
        println!("{}", formflow::pretty(&plan.to_json("create")));
        return;
    }
    plan.render(out());
    match plan.payload().filter(|_| plan.ok()) {
        Some(payload) => {
            out().print("Payload (POST /api/_/tickets)");
            println!("{}", formflow::pretty(&payload));
            let h = plan.hash();
            out().print(&format!(
                "plan_hash: {h}\nSend with --yes --expect-plan {}",
                &h[..h.len().min(16)]
            ));
        }
        None => out().print("No payload: fix the problems above."),
    }
}
