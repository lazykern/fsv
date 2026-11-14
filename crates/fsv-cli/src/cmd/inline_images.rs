//! `--inline-image` on `tickets reply` and `tickets add-note`
//! (`cli.py`, `ticket_notes.py`, `inline_images.finish`).

use serde_json::{Map, Value, json};

use fsv_api::attachments::ConversationAttach;
use fsv_api::formmeta::DiskStore;
use fsv_api::inline_images::{self, InlinePlan};
use fsv_api::service::{Api, DiskJournal};
use fsv_core::{Error, Result};

use super::reviewed::{dumps, print_verified, require_full_hash};
use crate::generated::*;

/// Port of inline_images.finish.
fn finish(
    plan: &InlinePlan,
    c: &dyn Api,
    dry_run: bool,
    yes: bool,
    expected: Option<&str>,
    rebuild: &dyn Fn() -> Result<InlinePlan>,
) -> Result<()> {
    if dry_run {
        let mut out = Map::new();
        out.insert("status".into(), json!("dry_run"));
        out.extend(plan.review());
        println!("{}", dumps(&Value::Object(out)));
        return Ok(());
    }
    let hash = plan.hash();
    require_full_hash(yes, expected, &hash)?;
    let result = inline_images::send(plan, c, &DiskJournal, Some(&hash), rebuild)?;
    print_verified(&result)
}

/// `tickets reply --inline-image`.
pub(crate) fn reply(a: &TicketsReplyArgs) -> Result<()> {
    if !a.attach.is_empty()
        || !a.cc.is_empty()
        || !a.bcc.is_empty()
        || a.set_status.as_deref().is_some_and(|s| !s.is_empty())
    {
        return Err(Error::exit(
            2,
            "--inline-image with --attach/--cc/--bcc/--set-status is not a captured combination; nothing was sent",
        ));
    }
    let c = crate::util::client()?;
    let store = DiskStore::current();
    let args = ConversationAttach {
        kind: "reply",
        sender: a.sender.as_deref(),
        max_bytes: a.max_bytes,
        ..Default::default()
    };
    let rebuild =
        || inline_images::plan(c.as_ref(), &store, &a.id_, &a.body, &a.inline_image, &args);
    finish(
        &rebuild()?,
        c.as_ref(),
        a.dry_run,
        a.yes,
        a.expect_plan.as_deref(),
        &rebuild,
    )
}

/// `tickets add-note --inline-image`.
pub(crate) fn add_note(a: &TicketsAddNoteArgs, notify: &[String]) -> Result<()> {
    if a.read_only && !a.dry_run {
        return Err(Error::exit(2, "read-only mode: nothing was sent"));
    }
    if !a.attach.is_empty() || a.set_status.is_some() {
        return Err(Error::exit(
            2,
            "--inline-image with --attach/--set-status is not a captured combination; nothing was sent",
        ));
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
        notify,
        max_bytes: a.max_bytes,
        ..Default::default()
    };
    let rebuild =
        || inline_images::plan(c.as_ref(), &store, &a.id_, &a.body, &a.inline_image, &args);
    finish(
        &rebuild()?,
        c.as_ref(),
        a.dry_run,
        a.yes,
        a.expected.as_deref(),
        &rebuild,
    )
}
