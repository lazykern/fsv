//! The clap tree matches the Python/Typer tree dumped by `tests/golden/dump_cli_tree.py`.

use clap::Command;
use serde_json::Value;

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/cli_tree.json")).unwrap()
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn has_flag(cmd: &Command, flag: &str) -> bool {
    cmd.get_arguments().any(|a| {
        if let Some(long) = flag.strip_prefix("--") {
            a.get_long() == Some(long) || a.get_all_aliases().is_some_and(|al| al.contains(&long))
        } else {
            let c = flag.chars().nth(1).unwrap();
            a.get_short() == Some(c) || a.get_all_short_aliases().is_some_and(|al| al.contains(&c))
        }
    })
}

fn walk(node: &Value, cmd: &Command, seen: &mut Vec<String>, problems: &mut Vec<String>) {
    let path = node["path"].as_str().unwrap().to_string();
    seen.push(path.clone());
    let is_group = node["group"].as_bool().unwrap();
    if cmd.is_hide_set() != node["hidden"].as_bool().unwrap_or(false) {
        problems.push(format!("{path}: hidden mismatch"));
    }
    let help = node["help"].as_str().unwrap_or("");
    if !help.is_empty() && !help.contains('[') {
        let about = cmd.get_about().map(|s| s.to_string()).unwrap_or_default();
        if collapse(&about) != collapse(help) {
            problems.push(format!("{path}: help {about:?} != {help:?}"));
        }
    }
    let mut positionals = 0;
    for p in node["params"].as_array().unwrap() {
        if p["kind"] == "argument" {
            positionals += 1;
            continue;
        }
        let opts = p["opts"]
            .as_array()
            .unwrap()
            .iter()
            .chain(p["secondary_opts"].as_array().unwrap());
        for o in opts {
            let o = o.as_str().unwrap();
            if !has_flag(cmd, o) {
                problems.push(format!("{path}: missing option {o}"));
            }
        }
    }
    let clap_positionals = cmd.get_positionals().count();
    if !is_group && clap_positionals != positionals {
        problems.push(format!(
            "{path}: {clap_positionals} positionals, Python has {positionals}"
        ));
    }
    let subs: Vec<&Value> = node["commands"]
        .as_array()
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    if is_group {
        let mut want: Vec<String> = subs
            .iter()
            .map(|s| {
                s["path"]
                    .as_str()
                    .unwrap()
                    .rsplit(' ')
                    .next()
                    .unwrap()
                    .to_string()
            })
            .collect();
        let mut have: Vec<String> = cmd
            .get_subcommands()
            .map(|c| c.get_name().to_string())
            .collect();
        want.sort();
        have.sort();
        if want != have {
            problems.push(format!("{path}: subcommands {have:?} != {want:?}"));
        }
    }
    for s in subs {
        let name = s["path"].as_str().unwrap().rsplit(' ').next().unwrap();
        match cmd.find_subcommand(name) {
            Some(c) => walk(s, c, seen, problems),
            None => problems.push(format!("{path}: no subcommand {name}")),
        }
    }
}

#[test]
fn tree_matches_python() {
    let tree = fixture();
    let mut cmd = fsv_cli::command();
    cmd.build();
    let mut seen = Vec::new();
    let mut problems = Vec::new();
    walk(&tree, &cmd, &mut seen, &mut problems);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    assert_eq!(seen.len(), 223);
}

/// The dumps record the Python commit they came from (`tests/golden/pyrev.py`).
#[test]
fn fixtures_record_the_python_commit() {
    let completion: Value =
        serde_json::from_str(include_str!("fixtures/completion_tree.json")).unwrap();
    for (name, tree) in [("cli_tree", fixture()), ("completion_tree", completion)] {
        let rev = &tree["_python"];
        let commit = rev["commit"].as_str().unwrap_or_default();
        assert!(
            commit.len() == 40 && commit.chars().all(|c| c.is_ascii_hexdigit()),
            "{name}: _python.commit {rev}"
        );
        assert!(rev["dirty"].is_boolean(), "{name}: _python.dirty {rev}");
    }
}

#[test]
fn every_command_path_is_reachable() {
    fn paths(c: &Command, prefix: &str, out: &mut Vec<String>) {
        for s in c.get_subcommands() {
            let p = format!("{prefix} {}", s.get_name());
            out.push(p.clone());
            paths(s, &p, out);
        }
    }
    let mut all = vec![];
    paths(&fsv_cli::command(), "fsv", &mut all);
    assert_eq!(all.len(), 222);
    for p in [
        "fsv changes assets legacy",
        "fsv changes relationships write",
        "fsv tickets bulk",
        "fsv sr approvals remind",
        "fsv solutions create",
        "fsv changes time write",
        "fsv problems time list",
        "fsv problems task-write",
        "fsv sr items write",
        "fsv changes delete",
        "fsv changes approval-remind",
        "fsv changes assets candidates",
        "fsv sr assets candidates",
        "fsv changes approval-request",
        "fsv sr get",
        "fsv form diff",
        "fsv tickets add-note",
    ] {
        assert!(all.iter().any(|a| a == p), "{p}");
    }
}

#[test]
fn rewrite_moves_root_flags_and_routes_legacy() {
    let root = fsv_cli::command();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let (args, notice) = fsv_cli::rewrite(&root, &s(&["changes", "ls", "-v"]));
    assert_eq!(args.first().map(String::as_str), Some("-v"));
    assert!(notice.is_none());
    let (args, notice) = fsv_cli::rewrite(&root, &s(&["changes", "assets", "5"]));
    assert_eq!(args, s(&["changes", "assets", "legacy", "5"]));
    assert!(notice.is_none());
    let (args, notice) = fsv_cli::rewrite(&root, &s(&["changes", "associations", "5"]));
    assert_eq!(args, s(&["changes", "associations", "legacy", "5"]));
    assert!(notice.is_some());
}
