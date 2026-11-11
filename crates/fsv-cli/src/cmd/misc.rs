//! Top-level commands: help, search, setup, tui.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};

use fsv_core::config;
use fsv_core::pystr;
use fsv_core::session;
use fsv_core::term::{self, Column, Justify, Table, err, out};

use crate::generated::{HelpArgs, SearchArgs, TuiArgs};
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

const SETUP_ITEMS: [(&str, &str); 6] = [
    ("Domain", "Freshservice instance URL"),
    ("Authentication", "Login with session cookies"),
    ("Shell completion", "Install tab completion"),
    ("Network completion", "Remote requester/agent lookup"),
    ("Default filters", "Default --where per resource"),
    ("All", "Run full setup"),
];

/// `fsv setup`.
pub fn setup(prog: &str) -> CmdResult {
    if !stdin_tty() {
        return Err(usage("setup requires an interactive terminal"));
    }
    setup_show_status();
    out().blank();
    let mut t = Table::new().show_header(false);
    t.add_column(Column::new("#").justify(Justify::Right).style("bold cyan"));
    t.add_column(Column::new("Item").style("bold"));
    t.add_column(Column::new("Description"));
    for (i, (name, desc)) in SETUP_ITEMS.iter().enumerate() {
        t.add_row([(i + 1).to_string(), name.to_string(), desc.to_string()]);
    }
    out().print_table(&t);
    let choice = loop {
        let answer = term::prompt("Choose", Some("6"), true)?;
        let answer = pystr::strip(&answer);
        if let Some(n) = pystr::parse_decimal(answer)
            && (1..=SETUP_ITEMS.len() as i64).contains(&n)
        {
            break n;
        }
        err().print(&format!("[red]error[/red]: choose 1-{}", SETUP_ITEMS.len()));
    };
    let all = choice == 6;
    if choice == 1 || all {
        out().print("\n[bold]── Domain ──[/bold]");
        crate::cmd::auth::resolve_domain(false)?;
        out().print(&format!("domain set to [bold]{}[/bold]", config::domain()));
    }
    if choice == 2 || all {
        setup_auth()?;
    }
    if choice == 3 || all {
        setup_completion_install(prog)?;
    }
    if choice == 4 || all {
        setup_network_completion()?;
    }
    if choice == 5 || all {
        setup_defaults()?;
    }
    out().print("\n[green]✓[/green] setup complete");
    Ok(())
}

/// `_setup_show_status`.
fn setup_show_status() {
    let mut t = Table::new().title("Current status").show_header(false);
    t.add_column(Column::new("Setting").style("bold"));
    t.add_column(Column::new("Value"));
    let domain = config::domain();
    t.add_row([
        "domain".to_string(),
        if domain.is_empty() {
            "[dim](not set)[/dim]".to_string()
        } else {
            domain
        },
    ]);
    let auth = match session::session_age_hours() {
        Some(age) => format!(
            "[green]✓[/green] {}, {age:.1}h ago",
            session::current_backend().as_str()
        ),
        None => "[red]✗[/red] not logged in".to_string(),
    };
    t.add_row(["auth".to_string(), auth]);
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default());
    let mut shells: Vec<&str> = Vec::new();
    if home.join(".config/fish/completions/fsv.fish").exists() {
        shells.push("fish");
    }
    for sh in ["bash", "zsh"] {
        if home.join(format!(".fsv-complete.{sh}")).exists() {
            shells.push(sh);
        }
    }
    t.add_row([
        "completion".to_string(),
        if shells.is_empty() {
            "[red]✗[/red] not installed".to_string()
        } else {
            format!("[green]✓[/green] {}", shells.join(", "))
        },
    ]);
    let cfg = config::load_config();
    let network = cfg
        .get("completion")
        .filter(|v| fsv_py::truthy(v))
        .and_then(|c| c.get("network"))
        .map(fsv_py::py_str)
        .unwrap_or_else(|| "[dim](unset)[/dim]".to_string());
    t.add_row(["network completion".to_string(), network]);
    for r in fsv_core::resource::REGISTRY {
        let exprs = config::get_default_where(r.name);
        let val = if exprs.is_empty() {
            "[dim](none)[/dim]".to_string()
        } else {
            exprs.join(", ")
        };
        t.add_row([format!("defaults.{}", r.name), val]);
    }
    out().print_table(&t);
}

/// `_setup_auth`.
fn setup_auth() -> CmdResult {
    out().print("\n[bold]── Authentication ──[/bold]");
    if config::domain().is_empty() {
        out().print("domain required first");
        crate::cmd::auth::resolve_domain(false)?;
    }
    let cookies = session::login_interactive().map_err(crate::util::fail_exc)?;
    crate::cmd::auth::validate_new_cookies(&cookies)?;
    let store = crate::cmd::auth::choose_store()?;
    let backend = session::save_cookies(&cookies, Some(store))?;
    let target = if backend == session::Backend::Keychain {
        "Keychain".to_string()
    } else {
        config::display_path(&config::session_file())
    };
    out().print(&format!("saved {} cookies to {target}", cookies.len()));
    session::clear_cache();
    fsv_core::client::reset_client();
    match fsv_core::get_client().and_then(|c| c.check_session()) {
        Ok(agent) => {
            let s = |k: &str| agent.get(k).map(fsv_py::py_str).unwrap_or_default();
            let name = format!("{} {}", s("first_name"), s("last_name"))
                .trim()
                .to_string();
            if agent.as_object().is_some_and(|m| !m.is_empty()) {
                out().print(&format!(
                    "logged in as: [bold]{name}[/bold] <{}>",
                    s("email")
                ));
            } else {
                out().print("logged in (identity unavailable: agent profile is not readable)");
            }
            err().print_plain("warming cache in background...");
            crate::cmd::cache::refresh_background();
        }
        Err(e) => err().print(&format!(
            "[yellow]warning[/yellow]: saved but could not verify: {e}"
        )),
    }
    Ok(())
}

/// `_setup_completion_install`.
fn setup_completion_install(prog: &str) -> CmdResult {
    out().print("\n[bold]── Shell completion ──[/bold]");
    let Some(shell) = crate::cmd::completion::shell_from_env() else {
        err().print("[yellow]could not detect shell; skipping[/yellow]");
        return Ok(());
    };
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default());
    let comp_path = match shell.as_str() {
        "fish" => Some(home.join(format!(".config/fish/completions/{prog}.fish"))),
        "bash" | "zsh" => Some(home.join(format!(".{prog}-complete.{shell}"))),
        _ => None,
    };
    match &comp_path {
        Some(p) if p.exists() => {
            out().print(&format!("already installed: {}", p.display()));
            if !term::confirm("Reinstall?", Some(false), false)? {
                return Ok(());
            }
        }
        _ => {
            if !term::confirm(&format!("Install {shell} completion?"), Some(true), false)? {
                return Ok(());
            }
        }
    }
    let script = crate::cmd::completion::build_script(&shell, prog)?;
    if shell == "fish" {
        let dir = home.join(".config/fish/completions");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{prog}.fish"));
        std::fs::write(&path, &script)?;
        out().print(&format!("installed: {}", path.display()));
    } else if shell == "bash" || shell == "zsh" {
        let script_path = home.join(format!(".{prog}-complete.{shell}"));
        std::fs::write(&script_path, &script)?;
        let rc = home.join(if shell == "bash" { ".bashrc" } else { ".zshrc" });
        let existing = std::fs::read_to_string(&rc).unwrap_or_default();
        let shown = script_path.display().to_string();
        if !existing.contains(&shown) {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&rc)?;
            write!(f, "\n. {shown}\n")?;
        }
        out().print(&format!("installed: {shown}"));
        out().print(&format!("sourced from: {}", rc.display()));
    }
    out().print("restart shell: exec $SHELL");
    Ok(())
}

/// `_setup_network_completion`.
fn setup_network_completion() -> CmdResult {
    out().print("\n[bold]── Network completion ──[/bold]");
    let mut cfg = config::load_config();
    let current = cfg
        .get("completion")
        .filter(|v| fsv_py::truthy(v))
        .and_then(|c| c.get("network"))
        .map(fsv_py::py_str)
        .unwrap_or_else(|| "(unset)".to_string());
    out().print(&format!("current: {current}"));
    out().print("when on, TAB fetches live requester/agent names");
    let answer = term::prompt("Enable? [on/off]", Some("on"), true)?;
    let answer = pystr::strip(&answer).to_lowercase();
    if answer != "on" && answer != "off" {
        err().print("[yellow]expected on/off; skipping[/yellow]");
        return Ok(());
    }
    let entry = cfg.entry("completion").or_insert_with(|| json!({}));
    if !entry.is_object() {
        *entry = json!({});
    }
    if let Some(m) = entry.as_object_mut() {
        m.insert("network".into(), json!(answer));
    }
    config::save_config(&cfg)?;
    out().print(&format!("completion.network = {answer}"));
    Ok(())
}

/// `_setup_defaults`.
fn setup_defaults() -> CmdResult {
    out().print("\n[bold]── Default filters ──[/bold]");
    out().print("set default --where filters for every `ls` query");
    out().print("examples: status=Open, agent=alice@example.com, requester=john@example.com");
    for r in fsv_core::resource::REGISTRY {
        let current = config::get_default_where(r.name);
        let hint = if current.is_empty() {
            String::new()
        } else {
            format!(" [{}]", current.join(", "))
        };
        let answer = term::prompt(
            &format!("\n{} --where (comma-sep, enter to skip){hint}", r.name),
            Some(""),
            false,
        )?;
        let answer = pystr::strip(&answer);
        if answer.is_empty() {
            continue;
        }
        let exprs: Vec<String> = answer
            .split(',')
            .map(|e| pystr::strip(e).to_string())
            .filter(|e| !e.is_empty())
            .collect();
        if !exprs.is_empty() {
            config::set_default_where(r.name, &exprs)?;
            let shown: Vec<Value> = exprs.iter().map(|e| json!(e)).collect();
            out().print(&format!(
                "  defaults.{}.where = {}",
                r.name,
                fsv_core::term::escape(&fsv_py::py_repr(&Value::Array(shown)))
            ));
        }
    }
    Ok(())
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
    fsv_app::run_tui(tui_read_only(a.read_only)).map_err(crate::util::uncaught)
}
