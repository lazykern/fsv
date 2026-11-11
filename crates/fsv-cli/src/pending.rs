//! Python functions the Rust port does not have yet. Each returns the error
//! the command reports (exit 1) after the handler's local validation ran.

use std::sync::Arc;

use serde_json::Value;

use fsv_core::resource::Resource;
use fsv_core::{Client, Error, Result};

fn not_ported(what: &str) -> Error {
    Error::other(format!("{what} is not ported to Rust yet"))
}

/// `fsv.schema.load(res, client, force)`.
pub fn schema_load(_res: &Resource, _c: &Arc<Client>, _force: bool) -> Result<Value> {
    Err(not_ported("schema.load"))
}

/// `cli.list_resource` (saved views, query hash, list pages and table).
pub fn list_resource(_res: &Resource, _c: &Arc<Client>) -> Result<()> {
    Err(not_ported("list_resource (fsv ls)"))
}

/// `cli.get_resource` (record detail page, tabs, evidence).
pub fn get_resource(_res: &Resource, _c: &Arc<Client>, _cid: i64) -> Result<()> {
    Err(not_ported("get_resource (fsv get)"))
}

/// `cli.activity_resource` / `cli.tasks_resource` (detail tab rows).
pub fn tab_resource(_res: &Resource, _c: &Arc<Client>, _cid: i64, tab: &str) -> Result<()> {
    Err(not_ported(&format!("{tab}_resource")))
}

/// `cli._history_rows` (notes / conversations pages).
pub fn history_rows(_res: &Resource, _c: &Arc<Client>, _cid: i64, kind: &str) -> Result<()> {
    Err(not_ported(&format!("_history_rows ({kind})")))
}

/// `cli._find_schema_field` + choice listing for `fields` and `lookup`.
pub fn fields_resource(_res: &Resource, _schema: &Value) -> Result<()> {
    Err(not_ported("fields_resource"))
}

/// `cli.update_resource` body resolution (`_resolve_update_choice`, `_resolve_set_field`).
pub fn update_resource(_res: &Resource, _c: &Arc<Client>, _cid: i64) -> Result<()> {
    Err(not_ported("update_resource"))
}

/// `cli.download_generic` / changes `download`.
pub fn download(_res: &Resource, _c: &Arc<Client>, _cid: i64) -> Result<()> {
    Err(not_ported("download (create.download_attachment)"))
}

/// `cli.ticket_approvals_resource`.
pub fn ticket_approvals(_c: &Arc<Client>, _cid: i64) -> Result<()> {
    Err(not_ported("ticket_approvals_resource"))
}

/// `cli.ticket_associations_resource`.
pub fn ticket_associations(_c: &Arc<Client>, _cid: i64) -> Result<()> {
    Err(not_ported("ticket_associations_resource"))
}

/// `create.delete_task`.
pub fn delete_task(_c: &Arc<Client>, _cid: i64, _task_id: i64) -> Result<()> {
    Err(not_ported("create.delete_task"))
}

/// `change_tasks` / `change_approvals` / `change_approval_actions` plans.
pub fn change_review_plan(what: &str) -> Result<()> {
    Err(not_ported(what))
}

/// `create.*` change helpers (tasks-update, approvals, state, clone, update quick path).
pub fn change_helper(what: &str) -> Result<()> {
    Err(not_ported(what))
}

/// `change_cli.create_command` / `update_command` (form engine path).
pub fn change_form_flow(what: &str) -> Result<()> {
    Err(not_ported(what))
}

/// `cli.assets_resource` / `cli.change_associations_resource`.
pub fn change_links(what: &str) -> Result<()> {
    Err(not_ported(what))
}

/// `ticket_cli.create_command`.
pub fn ticket_create() -> Result<()> {
    Err(not_ported("ticket_cli.create_command"))
}

/// `ticket_notes` reviewed add-note and `ticket_conversation` reviewed reply.
pub fn ticket_review(what: &str) -> Result<()> {
    Err(not_ported(what))
}

/// `formmeta.load_meta` (form explain/fields/dump/diff).
pub fn load_meta(_module: &str, _mode: &str) -> Result<Value> {
    Err(not_ported("formmeta.load_meta"))
}

/// `form explain --id`: the edit-form record read (`int_get changes/ID`) and `existing_from_record`.
pub fn form_record() -> Result<()> {
    Err(not_ported("form explain --id (existing_from_record)"))
}

/// `form_cli.diff_snapshots`.
pub fn form_diff() -> Result<()> {
    Err(not_ported("form_cli.diff_snapshots"))
}

/// `catalog.*` (sr catalog/fields/create).
pub fn catalog(what: &str) -> Result<()> {
    Err(not_ported(what))
}

/// `cli_sr.get_cmd` reads (requested items, attachment rows, downloads).
pub fn sr_get(_c: &Arc<Client>, _sr_id: i64) -> Result<()> {
    Err(not_ported("cli_sr.get_cmd"))
}

/// `cli._refresh_cache` schema warmers need `schema.load`.
pub fn refresh_schema(res: &Resource, c: &Arc<Client>) -> Result<(String, usize)> {
    let sch = schema_load(res, c, true)?;
    let n = sch
        .get("fields")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    Ok((res.name.to_string(), n))
}

/// `fsv.tui.launch`.
pub fn tui_launch(_read_only: bool) -> Result<()> {
    Err(not_ported("tui.launch"))
}

/// `cli.setup` steps after the TTY check.
pub fn setup_wizard() -> Result<()> {
    Err(not_ported("setup"))
}
