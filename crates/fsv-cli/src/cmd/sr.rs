//! `fsv sr` (`cli_sr.py`).

use clap::ArgMatches;

use fsv_core::Error;
use fsv_core::resource::{TICKETS, parse_id};

use crate::cmd::changes::form_tty;
use crate::cmd::parse;
use crate::generated::*;
use crate::pending;
use crate::util::{CmdResult, client, fail_exc, interactive};

pub fn dispatch(sub: &str, m: &ArgMatches) -> CmdResult {
    match sub {
        "catalog" => {
            let _a: SrCatalogArgs = parse(m)?;
            let _c = client()?;
            pending::catalog("catalog.list_categories").map_err(fail_exc)
        }
        "fields" => {
            let _a: SrFieldsArgs = parse(m)?;
            let _c = client()?;
            pending::catalog("catalog.load_item_meta").map_err(fail_exc)
        }
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
    let _c = client()?;
    pending::catalog("catalog.load_item_meta").map_err(fail_exc)
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
    let tid = parse_id(&a.sr_id, &TICKETS).map_err(fail_exc)?;
    let c = client()?;
    pending::sr_get(&c, tid).map_err(fail_exc)
}
