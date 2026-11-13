//! Command handlers, dispatched by command path.

pub mod attachments;
pub mod auth;
pub mod cache;
pub mod changes;
pub mod completion;
pub mod config;
pub mod download;
pub mod form;
pub mod formflow;
pub mod inventory;
pub mod journal;
pub mod links;
pub mod misc;
pub mod native;
pub mod resource;
pub mod review;
pub mod reviewed;
pub mod sr;
pub mod ticket_links;
pub mod tickets;
pub mod windows;

use clap::{ArgMatches, FromArgMatches};

use fsv_core::Error;
use fsv_core::resource::{CHANGES, PROBLEMS, Resource, TICKETS};

use crate::generated::*;
use crate::util::CmdResult;

fn parse<T: FromArgMatches>(m: &ArgMatches) -> fsv_core::Result<T> {
    T::from_arg_matches(m).map_err(|e| Error::other(e.to_string()))
}

fn res_of(path: &str) -> &'static Resource {
    match path.split(' ').next() {
        Some("tickets") => &TICKETS,
        Some("problems") => &PROBLEMS,
        _ => &CHANGES,
    }
}

/// Run the leaf command at `path` (without the program name).
pub fn dispatch(path: &str, m: &ArgMatches, prog: &str) -> CmdResult {
    let res = res_of(path);
    let sub = path.split_once(' ').map_or("", |(_, s)| s);
    match path {
        "help" => misc::help(parse(m)?),
        "search" => misc::search(parse(m)?),
        "setup" => misc::setup(prog),
        "tui" => misc::tui(parse(m)?),
        "auth login" => auth::login(parse(m)?),
        "auth logout" => auth::logout(parse(m)?),
        "auth status" => auth::status(),
        "completion show" => completion::show(parse::<CompletionShowArgs>(m)?.shell, prog),
        "completion install" => completion::install(parse::<CompletionInstallArgs>(m)?.shell, prog),
        "completion refresh" | "cache refresh" => cache::refresh(),
        "completion doctor" => completion::doctor(prog),
        "config list" => config::list(),
        "config get" => config::get(parse(m)?),
        "config set" => config::set(parse(m)?),
        "config defaults show" => config::defaults_show(parse(m)?),
        "config defaults set" => config::defaults_set(parse(m)?),
        "config defaults clear" => config::defaults_clear(parse(m)?),
        "cache status" => cache::status(),
        "cache clear" => cache::clear(parse(m)?),
        "journal ls" => journal::ls(parse(m)?),
        "journal tail" => journal::tail(parse(m)?),
        "journal path" => journal::path(),
        "attachments delete" => attachments::delete(parse(m)?),
        "assets search" => {
            let a: AssetsSearchArgs = parse(m)?;
            inventory::search(&a.query, &a.kind, a.page)
        }
        "solutions create" => inventory::solutions_create(parse(m)?),
        "sr items list" => reviewed::sr_items_list(parse(m)?),
        "sr items write" => reviewed::sr_items_write(parse(m)?),
        // batch windows: ticket templates and change maintenance windows
        "templates list" => windows::templates_list(parse(m)?),
        "templates show" => windows::templates_show(parse(m)?),
        "changes windows list" => windows::windows_list(parse(m)?),
        "changes windows create" => windows::windows_create(parse(m)?),
        "changes windows delete" => windows::windows_delete(parse(m)?),
        _ if path.starts_with("form ") => form::dispatch(sub, m),
        _ if path.starts_with("sr ") => {
            inventory::dispatch("sr", sub, m).unwrap_or_else(|| sr::dispatch(sub, m))
        }
        _ => resource_dispatch(res, sub, m),
    }
}

fn resource_dispatch(res: &'static Resource, sub: &str, m: &ArgMatches) -> CmdResult {
    if let Some(r) = inventory::dispatch(res.name, sub, m) {
        return r;
    }
    // batch links: tickets problems|changes|children list|candidates|link|unlink
    if res.name == "tickets"
        && let Some(r) = ticket_links::dispatch(sub, m)
    {
        return r;
    }
    match sub {
        "ls" => resource::ls(res, parse(m)?),
        "get" => resource::get(res, parse(m)?),
        "activity" => resource::activity(res, parse(m)?),
        "tasks" => resource::tasks(res, parse(m)?),
        "fields" => resource::fields(res, parse(m)?),
        "lookup" => resource::lookup(res, parse(m)?),
        "views" => resource::views(res, parse(m)?),
        "search" => resource::search(res, parse(m)?),
        "url" => resource::url(res, parse(m)?),
        "notes" => resource::notes(res, parse(m)?),
        "add-note" if res.name == "tickets" => tickets::add_note(parse(m)?),
        "add-note" => resource::add_note(res, parse(m)?),
        "update" if res.name == "changes" => changes::update(parse(m)?),
        "update" => resource::update(res, parse(m)?),
        "download" if res.name == "changes" => changes::download(parse(m)?),
        "download" => resource::download(res, parse(m)?),
        "watchers list" => reviewed::watchers_list(res, parse(m)?),
        "watchers set" => reviewed::watchers_set(res, parse(m)?),
        "planning-text" => reviewed::planning_text(res, parse(m)?),
        "asset-picker" => reviewed::asset_picker(parse(m)?),
        "mark-known-error" => reviewed::mark_known_error(parse(m)?),
        "time list" if res.name == "changes" => {
            reviewed::module_time_list("changes", &parse::<ChangesTimeListArgs>(m)?.id_)
        }
        "time list" if res.name == "problems" => {
            reviewed::module_time_list("problems", &parse::<ProblemsTimeListArgs>(m)?.id_)
        }
        "time write" if res.name == "changes" => reviewed::changes_time_write(parse(m)?),
        "time write" if res.name == "problems" => reviewed::problems_time_write(parse(m)?),
        "time list" => reviewed::time_list(parse(m)?),
        "time write" => reviewed::time_write(parse(m)?),
        "task-write" => reviewed::problem_task_write(parse(m)?),
        "bulk-priority" => reviewed::bulk_priority(parse(m)?),
        "bulk" => reviewed::tickets_bulk(parse(m)?),
        "relationships list" => reviewed::relationships_list(parse(m)?),
        "relationships write" => reviewed::relationships_write(parse(m)?),
        "status" if res.name == "problems" => reviewed::problem_status(parse(m)?),
        "status" => reviewed::status(parse(m)?),
        "resolve" => reviewed::resolve(parse(m)?),
        "close" => reviewed::close(parse(m)?),
        "reopen" => reviewed::reopen(parse(m)?),
        "merge" => reviewed::merge(parse(m)?),
        "assign" => reviewed::assign(parse(m)?),
        "pick-up" => reviewed::pick_up(parse(m)?),
        "note-edit" => reviewed::note_edit(parse(m)?),
        "note-delete" => reviewed::note_delete(parse(m)?),
        "tasks list" => reviewed::tasks_list(parse(m)?),
        "tasks add" => reviewed::tasks_add(parse(m)?),
        "tasks update" => reviewed::tasks_update(parse(m)?),
        "tasks status" => reviewed::tasks_status(parse(m)?),
        "tasks delete" => reviewed::tasks_delete(parse(m)?),
        "tasks-legacy" => resource::tasks(res, parse(m)?),
        "link-incident" => reviewed::link_incident(parse(m)?),
        "unlink-incident" => reviewed::unlink_incident(parse(m)?),
        "note" => attachments::problems_note(parse(m)?),
        "canned folders" => native::canned_folders(parse(m)?),
        "canned list" => native::canned_list(parse(m)?),
        "canned show" => native::canned_show(parse(m)?),
        "canned reply" => native::canned_reply(parse(m)?),
        "scenarios list" => native::scenarios_list(parse(m)?),
        "scenarios show" => native::scenarios_show(parse(m)?),
        "scenarios apply" => native::scenarios_apply(parse(m)?),
        // batch views: private views (tickets, changes)
        "private-views list" => native::private_views_list(res.name),
        "private-views show" if res.name == "changes" => {
            native::private_views_show(res.name, parse::<ChangesPrivateViewsShowArgs>(m)?.id_)
        }
        "private-views show" => native::private_views_show(res.name, parse::<PositiveIdArgs>(m)?.id_),
        "private-views write" if res.name == "changes" => native::private_views_write(
            res.name,
            parse::<ChangesPrivateViewsWriteArgs>(m)?.into(),
        ),
        "private-views write" => native::private_views_write(
            res.name,
            parse::<TicketsPrivateViewsWriteArgs>(m)?.into(),
        ),
        "columns list" => native::columns_list(res, parse(m)?),
        "columns set" => native::columns_set(res, parse(m)?),
        "export" => native::export(res, parse(m)?),
        "create" if res.name == "problems" => native::problems_create(parse(m)?),
        "write-child" => native::problems_write_child(parse(m)?),
        _ if res.name == "changes" => changes::dispatch(sub, m),
        _ => tickets::dispatch(sub, m),
    }
}
