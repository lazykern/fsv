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

/// Click `INT` / `IntRange` params convert with Python `int()` and the same
/// range (`fsv_cli`'s py-int parser), not clap's ASCII `i64` parser.
fn check_int(p: &Value, cmd: &Command, path: &str, problems: &mut Vec<String>) {
    let ty = p["type"].as_str().unwrap_or_default();
    if ty != "integer" && ty != "integer range" {
        return;
    }
    let name = p["name"].as_str().unwrap();
    let long = p["opts"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|o| o.as_str()?.strip_prefix("--"))
        .next();
    let arg = cmd.get_arguments().find(|a| match long {
        Some(l) if p["kind"] != "argument" => a.get_long() == Some(l),
        _ => a.is_positional() && a.get_id().as_str() == name,
    });
    let Some(arg) = arg else {
        problems.push(format!("{path}: no clap arg for integer {name}"));
        return;
    };
    let parse = |v: &str| {
        let mut one = arg.clone().required(false);
        let mut argv = vec!["t"];
        if arg.is_positional() {
            one = one.index(1);
        } else {
            argv.push("--x");
            one = one.long("x");
        }
        argv.push(v);
        Command::new("t")
            .arg(one)
            .try_get_matches_from(argv)
            .map(|m| {
                *m.get_many::<i64>(arg.get_id().as_str())
                    .unwrap()
                    .next()
                    .unwrap()
            })
            .map_err(|e| e.to_string())
    };
    let min = p["min"].as_i64();
    let max = p["max"].as_i64();
    let mut want = vec![(" 1_2 ", Ok(12)), ("\u{661}\u{662}", Ok(12))];
    let kind = if ty == "integer" {
        "integer"
    } else {
        "integer range"
    };
    want.push(("x", Err(format!("'x' is not a valid {kind}."))));
    if min == Some(1) {
        want.push(("0", Err("0 is not in the range ".to_string())));
    } else {
        want.push(("+0", Ok(0)));
    }
    if let Some(hi) = max {
        let over = (hi + 1).to_string();
        want.push((
            "101",
            Err(format!("{over} is not in the range 1<=x<={hi}.")),
        ));
    }
    for (v, w) in want {
        let got = parse(v);
        let ok = match (&got, &w) {
            (Ok(a), Ok(b)) => a == b,
            (Err(a), Err(b)) => a.contains(b.as_str()),
            _ => false,
        };
        if !ok {
            problems.push(format!(
                "{path}: {name} parses {v:?} as {got:?}, want {w:?}"
            ));
        }
    }
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
        check_int(p, cmd, &path, problems);
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
        // Option help: compare when Python sets plain text (rich markup is
        // not reproduced by clap, so `[`-bearing help is skipped).
        if let Some(want) = p["help"]
            .as_str()
            .filter(|h| !h.is_empty() && !h.contains('['))
        {
            // Match by long flag: Python's param names (`sets`) need not equal
            // the Rust field ids (`set_`).
            let flag = p["opts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|o| o.as_str().unwrap())
                .find(|o| o.starts_with("--"))
                .unwrap_or_else(|| panic!("{path}: no long flag for {p}"))
                .trim_start_matches("--");
            let got = cmd
                .get_arguments()
                .find(|a| a.get_long() == Some(flag))
                .and_then(|a| a.get_help().map(|h| h.to_string()));
            match got {
                Some(got) if collapse(&got) == collapse(want) => {}
                other => problems.push(format!(
                    "{path}: option --{flag} help {other:?} != {want:?}"
                )),
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
    assert_eq!(seen.len(), 263);
}

/// Fixtures still stamped with an older Python commit. Each one must be
/// regenerated against `port/UPSTREAM_BASE` before its stamp moves; until
/// then the debt is named here instead of hidden behind a false stamp.
const STAMP_DEBT: &[(&str, &str)] = &[];

/// Every JSON oracle records the Python commit it was dumped from.
#[test]
fn fixtures_record_the_python_commit() {
    let pinned = include_str!("../../../port/UPSTREAM_BASE").trim();
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
        match STAMP_DEBT.iter().find(|(n, _)| *n == name) {
            Some((_, old)) => assert_eq!(commit, *old, "{name}: stale stamp list out of date"),
            None => assert_eq!(commit, pinned, "{name}: fixture predates UPSTREAM_BASE"),
        }
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
    assert_eq!(all.len(), 262);
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
