//! Top-level commands: help, search, setup, tui.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};

use fsv_core::pystr;
use fsv_core::term::{Column, Table, err, out};

use crate::generated::{HelpArgs, SearchArgs, TuiArgs};
use crate::pending;
use crate::types::{OutputFormat, effective};
use crate::util::{CmdResult, client, first_str, head, stdin_tty, usage};

pub const HELP_TOPICS: &[(&str, &str)] = &[
    (
        "auth",
        "Login: fsv auth login --browser --domain acme.freshservice.com; fsv auth status. Scripts: use fsv auth login --domain ... --header ... --store file.",
    ),
    (
        "workflow",
        "Daily flow: fsv changes ls --where status=Open; fsv changes get CHN-1234 --internal; fsv changes update CHN-1234 --set 'field=value'; fsv changes download CHN-1234 --all.",
    ),
    (
        "fields",
        "Discover fields with fsv changes fields, fsv changes fields --choices status, and fsv changes lookup requester alice@example.com. Prefer dedicated flags for default fields; use --set for custom fields.",
    ),
    (
        "scripting",
        "Use --json for create/update/clone/download. Use --output json/csv/tsv for list/search commands. Use --no-input in CI. Use --dry-run before updates. Do not parse rich table output.",
    ),
    (
        "comments",
        "Tickets support reply and reviewed add-note (see docs/ticket-notes.md). Changes/problems use add-note. Use --public only when notes should be visible outside private workflow.",
    ),
];

/// `fsv help [topic]`.
pub fn help(a: HelpArgs) -> CmdResult {
    let Some(topic) = a.topic else {
        let mut t = Table::new().title("help topics");
        t.add_column(Column::new("Topic").style("cyan"));
        t.add_column(Column::new("Use"));
        for (name, _) in HELP_TOPICS {
            t.add_row([name.to_string(), format!("fsv help {name}")]);
        }
        out().print_table(&t);
        out().print("Use `fsv COMMAND --help` for command reference.");
        return Ok(());
    };
    let key = pystr::casefold(&topic);
    match HELP_TOPICS.iter().find(|(k, _)| *k == key) {
        Some((_, text)) => {
            out().print(text);
            Ok(())
        }
        None => {
            let names: Vec<&str> = HELP_TOPICS.iter().map(|(k, _)| *k).collect();
            Err(usage(&format!(
                "unknown help topic {}; choices: {}",
                fsv_py::py_repr_str(&topic),
                names.join(", ")
            )))
        }
    }
}

const SEARCH_TYPE_LABEL: &[(&str, &str)] = &[
    ("helpdesk_ticket", "Ticket"),
    ("itil_problem", "Problem"),
    ("itil_change", "Change"),
    ("itil_task", "Task"),
    ("cmdb_config_item", "Asset"),
    ("solution_article", "Solution"),
];

fn search_clean(s: &str) -> String {
    static TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").expect("regex"));
    let stripped = TAGS.replace_all(s, "");
    pystr::strip(&pystr::html_unescape(&stripped)).to_string()
}

/// `_search_extract`: `(type, id, title, status, group)`.
pub fn search_extract(r: &Value) -> [String; 5] {
    let rt = first_str(r, &["result_type"], "");
    let label = SEARCH_TYPE_LABEL
        .iter()
        .find(|(k, _)| *k == rt)
        .map_or_else(
            || {
                if rt.is_empty() {
                    "-".to_string()
                } else {
                    rt.clone()
                }
            },
            |(_, v)| v.to_string(),
        );
    let did_keys = [
        "ticket_display_id",
        "itil_module_display_id",
        "task_display_id",
        "ci_display_id",
        "display_id",
    ];
    let did = if did_keys
        .iter()
        .any(|k| r.get(*k).is_some_and(fsv_py::truthy))
    {
        first_str(r, &did_keys, "")
    } else {
        r.get("id").map_or_else(|| "-".to_string(), fsv_py::py_str)
    };
    let title = search_clean(&first_str(r, &["subject", "name", "title"], ""));
    let status = first_str(
        r,
        &[
            "ticket_status",
            "itil_module_status",
            "task_status",
            "ci_type",
        ],
        "-",
    );
    let grp = first_str(
        r,
        &[
            "ticket_group",
            "itil_module_group",
            "task_group",
            "owner_name",
            "folder_name",
        ],
        "-",
    );
    [label, did, title, status, grp]
}

fn results_with_type(data: &Value) -> Vec<Value> {
    data.get("results")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter(|r| r.get("result_type").is_some_and(fsv_py::truthy))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// `fsv search QUERY` (`global_search`).
pub fn search(a: SearchArgs) -> CmdResult {
    let c = client()?;
    let sort = Some(a.sort.as_str());
    let items = if a.n_pages.is_some() || a.all_pages {
        let mut all = Vec::new();
        let mut p = a.page;
        let mut fetched = 0;
        loop {
            let batch = results_with_type(&c.fulltext_search("all", &a.query, p, sort)?);
            let n = batch.len();
            all.extend(batch);
            fetched += 1;
            err().print_plain(&format!("  fetched {}...", all.len()));
            if n < 30 || a.n_pages.is_some_and(|limit| fetched >= limit) {
                break;
            }
            p += 1;
        }
        all
    } else {
        results_with_type(&c.fulltext_search("all", &a.query, a.page, sort)?)
    };
    let fmt = effective(a.format_, a.json_out);
    if fmt == OutputFormat::Json {
        fsv_core::output::emit_json(&Value::Array(items));
        return Ok(());
    }
    let rows: Vec<Map<String, Value>> = items
        .iter()
        .map(|r| {
            let [t, id, title, status, group] = search_extract(r);
            let mut m = Map::new();
            for (k, v) in [
                ("type", t),
                ("id", id),
                ("title", title),
                ("status", status),
                ("group", group),
            ] {
                m.insert(k.into(), json!(v));
            }
            m
        })
        .collect();
    if matches!(fmt, OutputFormat::Csv | OutputFormat::Tsv) {
        let delim = if fmt == OutputFormat::Csv { ',' } else { '\t' };
        if rows.is_empty() {
            print!("\r\n");
        } else {
            fsv_core::output::emit_delimited(&rows, delim)?;
        }
        return Ok(());
    }
    let mut t = Table::new();
    t.add_column(Column::new("Type").style("magenta").no_wrap());
    t.add_column(Column::new("ID").style("cyan").no_wrap());
    t.add_column(Column::new("Title"));
    t.add_column(Column::new("Status").style("green"));
    t.add_column(Column::new("Group").style("dim"));
    let s = |m: &Map<String, Value>, k: &str| {
        m.get(k).and_then(Value::as_str).unwrap_or("").to_string()
    };
    for r in &rows {
        t.add_row([
            s(r, "type"),
            s(r, "id"),
            head(&s(r, "title"), 70),
            s(r, "status"),
            s(r, "group"),
        ]);
    }
    out().print_table(&t);
    err().print_plain(&format!("{} rows", items.len()));
    Ok(())
}

/// `fsv setup`.
pub fn setup() -> CmdResult {
    if !stdin_tty() {
        return Err(usage("setup requires an interactive terminal"));
    }
    pending::setup_wizard()
}

/// `tui/actions.py resolve_read_only`.
pub fn tui_read_only(flag: bool) -> bool {
    if flag {
        return true;
    }
    let cfg = fsv_core::config::load_config();
    let v = cfg.get("tui").and_then(|t| t.get("read_only"));
    v.is_some_and(|v| {
        let s = pystr::casefold(pystr::strip(&fsv_py::py_str(v)));
        matches!(s.as_str(), "on" | "true" | "1" | "yes")
    })
}

/// `fsv tui`.
pub fn tui(a: TuiArgs) -> CmdResult {
    pending::tui_launch(tui_read_only(a.read_only))
}
