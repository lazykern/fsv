//! `tickets`-only commands.

use clap::ArgMatches;
use serde_json::{Value, json};

use fsv_api::associations::SendOpts;
use fsv_api::notes::{self, BodyFormat};
use fsv_api::service::DiskJournal;
use fsv_core::resource::TICKETS;
use fsv_core::{Error, Result, config, term};

use crate::cmd::changes::form_tty;
use crate::cmd::{parse, resource};
use crate::generated::*;
use crate::pending;
use crate::types::OutputFormat;
use crate::util::{CmdResult, cid, client, fail_exc, interactive, no_input, plain_fail, uncaught};

pub fn dispatch(sub: &str, m: &ArgMatches) -> CmdResult {
    match sub {
        "conversations" => resource::history(
            &TICKETS,
            &parse::<TicketsConversationsArgs>(m)?.id_,
            "conversations",
        ),
        "reply" => reply(parse(m)?),
        "create" => create(parse(m)?),
        "approvals" => {
            let a: TicketsApprovalsArgs = parse(m)?;
            let id = cid(&a.id_, &TICKETS)?;
            let c = client()?;
            pending::ticket_approvals(&c, id).map_err(fail_exc)
        }
        "associations" => {
            let a: TicketsAssociationsArgs = parse(m)?;
            let id = cid(&a.id_, &TICKETS)?;
            let c = client()?;
            pending::ticket_associations(&c, id).map_err(fail_exc)
        }
        _ => Err(Error::other(format!("unknown command: tickets {sub}"))),
    }
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

/// Reviewed `tickets add-note` (`ticket_notes.register`).
pub fn add_note(a: TicketsAddNoteArgs) -> CmdResult {
    add_note_inner(&a).map_err(note_error)
}

fn add_note_inner(a: &TicketsAddNoteArgs) -> Result<()> {
    if a.set_status.is_some() {
        if a.read_only && !a.dry_run {
            return Err(Error::exit(2, "read-only mode: no note was sent"));
        }
        let _c = fsv_core::get_client()?;
        return pending::ticket_review("ticket_conversation.build_plan");
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
        &a.notify,
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
    let c = fsv_core::get_client()?;
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
        println!(
            "Note {} submitted on ticket {}; exact-ID read-back: {}",
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
    if a.cc.is_empty() && a.set_status.is_none() && a.expect_plan.is_none() {
        return resource::reply(&TICKETS, &a.id_, &a.body, a.dry_run, a.yes, a.no_input);
    }
    let _c = client()?;
    pending::ticket_review("ticket_conversation.build_plan").map_err(|e| match e {
        Error::Value(m) => plain_fail(&m, 2),
        other => fail_exc(other),
    })
}

fn create(a: TicketsCreateArgs) -> CmdResult {
    let local_no_input = no_input(a.no_input);
    let _as_json = a.json_out || a.format_ == OutputFormat::Json;
    let tty = interactive() && !local_no_input;
    let inputs = a.from_.is_some() || !a.set_.is_empty() || a.preset.is_some() || a.edit;
    let form = !inputs && tty && form_tty();
    let template_only = !inputs && !form && a.dry;
    if !(inputs || form || template_only) {
        return Err(Error::exit(
            2,
            "pass --from or --set, or open the interactive form in a terminal",
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
    let _c = client()?;
    pending::load_meta("ticket", "new").map_err(fail_exc)?;
    pending::ticket_create().map_err(fail_exc)
}
