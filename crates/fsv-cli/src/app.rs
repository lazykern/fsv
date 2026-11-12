//! Process edge: argv rewriting, clap parsing, dispatch, exit codes.

use clap::error::{ContextKind, ContextValue, ErrorKind};
use clap::{Arg, ArgAction, ArgMatches, Command};

use fsv_core::resource::{self, CHANGES};
use fsv_core::term::err;

use crate::cmd;
use crate::generated::CLICK_USAGE;
use crate::util::report;

/// `_ROOT_FLAGS_ANYWHERE`.
pub const ROOT_FLAGS_ANYWHERE: &[&str] = &["-v", "--verbose"];

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

fn global_verbose() -> Arg {
    Arg::new("fsv_verbose")
        .long("verbose")
        .short('v')
        .action(ArgAction::SetTrue)
        .help("print debug info to stderr")
        .help_heading("Global options")
}

fn finish(mut cmd: Command, depth: usize) -> Command {
    let names: Vec<String> = cmd
        .get_subcommands()
        .map(|s| s.get_name().to_string())
        .collect();
    for name in &names {
        cmd = cmd.mut_subcommand(name, |s| finish(s, depth + 1));
    }
    cmd = cmd.disable_help_subcommand(true);
    if !names.is_empty() {
        cmd = cmd.arg_required_else_help(true);
        if depth > 0 {
            cmd = cmd.subcommand_required(true).arg(global_verbose());
        }
    }
    cmd
}

/// The clap tree with fsv's process-wide settings.
pub fn command() -> Command {
    let root = crate::generated::build()
        .disable_version_flag(true)
        .bin_name("fsv");
    finish(root, 0)
}

fn takes_value(arg: &Arg) -> bool {
    !matches!(
        arg.get_action(),
        ArgAction::SetTrue
            | ArgAction::SetFalse
            | ArgAction::Count
            | ArgAction::Help
            | ArgAction::HelpShort
            | ArgAction::HelpLong
            | ArgAction::Version
    )
}

/// `_option_arity`.
fn option_arity(cmd: &Command, token: &str) -> usize {
    if token.contains('=') {
        return 0;
    }
    let found = cmd.get_arguments().find(|a| {
        if let Some(long) = token.strip_prefix("--") {
            a.get_long() == Some(long) || a.get_all_aliases().is_some_and(|al| al.contains(&long))
        } else if let Some(short) = token.strip_prefix('-').filter(|s| s.chars().count() == 1) {
            let c = short.chars().next();
            a.get_short() == c
                || a.get_all_short_aliases()
                    .is_some_and(|al| c.is_some_and(|c| al.contains(&c)))
        } else {
            false
        }
    });
    found.map_or(0, |a| usize::from(takes_value(a)))
}

/// `_reorder_root_flags` plus `ChangeLegacyGroup` routing. Returns the new
/// argv (without the program name) and the deprecation notice to print.
/// Port of fsv.cli.FSVGroup.parse_args, fsv.cli.ChangeLegacyGroup.parse_args.
pub fn rewrite(root: &Command, args: &[String]) -> (Vec<String>, Option<&'static str>) {
    let mut moved = Vec::new();
    let mut kept: Vec<String> = Vec::new();
    let mut current = root;
    let mut path: Vec<&str> = Vec::new();
    let mut started = false;
    let mut skip = 0usize;
    let mut passthrough = false;
    let mut notice = None;
    let mut legacy_check = false;
    for token in args {
        if legacy_check {
            legacy_check = false;
            if !token.starts_with('-')
                && current.find_subcommand(token).is_none()
                && resource::parse_id(token, &CHANGES).is_ok()
            {
                if path.last() == Some(&"associations") {
                    notice = Some(
                        "[yellow]deprecated[/]: use `fsv changes associations <ls|search|add|remove|pick> ...`",
                    );
                }
                kept.push("legacy".into());
                current = current.find_subcommand("legacy").unwrap_or(current);
            }
        }
        if passthrough {
            kept.push(token.clone());
            continue;
        }
        if skip > 0 {
            kept.push(token.clone());
            skip -= 1;
            continue;
        }
        if token == "--" {
            passthrough = true;
            kept.push(token.clone());
            continue;
        }
        if started && ROOT_FLAGS_ANYWHERE.contains(&token.as_str()) {
            moved.push(token.clone());
            continue;
        }
        if token.starts_with('-') {
            skip = option_arity(current, token);
            kept.push(token.clone());
            continue;
        }
        if current.has_subcommands()
            && let Some(sub) = current.find_subcommand(token)
        {
            started = true;
            current = sub;
            path.push(sub.get_name());
            legacy_check = matches!(path[..], [_, "assets"] | ["changes", "associations"]);
        }
        kept.push(token.clone());
    }
    moved.extend(kept);
    (moved, notice)
}

/// What the root's eager options ask for before any subcommand runs.
enum Eager {
    Version,
    ShowCompletion,
    InstallCompletion,
}

fn eager(root: &Command, args: &[String]) -> Option<Eager> {
    let mut found = None;
    for token in args {
        if token == "--" {
            break;
        }
        if !token.starts_with('-') {
            if root.find_subcommand(token).is_none() && matches!(found, Some(Eager::Version)) {
                return None;
            }
            break;
        }
        match token.as_str() {
            "-V" | "--version" => found = found.or(Some(Eager::Version)),
            "--show-completion" => return Some(Eager::ShowCompletion),
            "--install-completion" => return Some(Eager::InstallCompletion),
            _ => {}
        }
    }
    found
}

/// The leaf command path and its matches.
fn leaf(m: &ArgMatches) -> (Vec<String>, &ArgMatches) {
    let mut path = Vec::new();
    let mut cur = m;
    while let Some((name, sub)) = cur.subcommand() {
        path.push(name.to_string());
        cur = sub;
    }
    (path, cur)
}

/// Run fsv with `argv` (program name first); returns the exit code.
/// Port of fsv.cli.main.
pub fn run(argv: Vec<String>) -> i32 {
    let args: Vec<String> = argv.iter().skip(1).cloned().collect();
    if args.first().map(String::as_str) == Some("_complete")
        && std::env::var(fsv_completion::THIN_COMPLETE_ENV).is_ok_and(|v| !v.is_empty())
    {
        print!(
            "{}",
            fsv_completion::run(&args[1..], cmd::completion::network_enabled())
        );
        return 0;
    }
    let mut root = command();
    let prog = argv
        .first()
        .and_then(|a| std::path::Path::new(a).file_name())
        .map_or("fsv".to_string(), |n| n.to_string_lossy().into_owned());
    match eager(&root, &args) {
        Some(Eager::Version) => {
            fsv_core::term::out().print(&format!("fsv {VERSION}"));
            return 0;
        }
        Some(Eager::ShowCompletion) => return report(cmd::completion::show_eager(&prog)),
        Some(Eager::InstallCompletion) => return report(cmd::completion::install_eager(&prog)),
        None => {}
    }
    let (args, notice) = rewrite(&root, &args);
    let mut full = vec![prog.clone()];
    full.extend(args);
    let matches = match root.try_get_matches_from_mut(full) {
        Ok(m) => m,
        Err(e) => return clap_error(e),
    };
    if let Some(n) = notice {
        err().print(n);
    }
    fsv_core::runtime::set_no_input(matches.get_flag("no_input"));
    fsv_core::runtime::set_verbose(matches.get_flag("verbose"));
    let (path, m) = leaf(&matches);
    if path.is_empty() {
        let _ = root.print_help();
        return 0;
    }
    fsv_core::runtime::set_command_path(Some(&format!("{prog} {}", path.join(" "))));
    let run = || cmd::dispatch(&path.join(" "), m, &prog);
    // Canned routes answer instantly, so an unverified bulk, assignment,
    // merge or priority send must not sleep through the bounded readback
    // (Python's tests patch `readback._sleep` the same way).
    #[cfg(debug_assertions)]
    if std::env::var_os("FSV_TEST_ROUTES").is_some_and(|v| !v.is_empty()) {
        return report(fsv_api::readback::with_readback_sleep(|_| {}, run));
    }
    report(run())
}

fn clap_error(e: clap::Error) -> i32 {
    match e.kind() {
        ErrorKind::DisplayHelp => {
            let _ = e.print();
            0
        }
        ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
            print!("{}", e.render());
            2
        }
        ErrorKind::MissingRequiredArgument if missing_parameter(&e) => 2,
        _ => {
            let _ = e.print();
            2
        }
    }
}

/// Click's `MissingParameter` for the first missing parameter in declaration
/// order. clap's usage line names the command (`Usage: fsv problems create
/// --requester <REQUESTER>`); false if the leaf has no `CLICK_USAGE` row.
fn missing_parameter(e: &clap::Error) -> bool {
    let (Some(ContextValue::StyledStr(usage)), Some(ContextValue::Strings(missing))) =
        (e.get(ContextKind::Usage), e.get(ContextKind::InvalidArg))
    else {
        return false;
    };
    let usage = usage.to_string();
    let mut words = usage.strip_prefix("Usage: ").unwrap_or(&usage).split(' ');
    let prog = words.next().unwrap_or("fsv");
    let path: Vec<&str> = words
        .take_while(|w| !w.starts_with(['-', '<', '[']))
        .collect();
    let path = path.join(" ");
    let tokens: Vec<&str> = missing
        .iter()
        .filter_map(|m| m.split(' ').next())
        .map(|t| t.trim_end_matches("..."))
        .collect();
    let Some((_, pieces, params)) = CLICK_USAGE.iter().find(|(p, ..)| *p == path) else {
        return false;
    };
    let Some((_, message)) = params.iter().find(|(t, _)| tokens.contains(t)) else {
        return false;
    };
    cmd::reviewed::usage_error(&format!("{prog} {path}"), pieces, message);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Python `cli.main` (the root callback): `--version`/`-V` prints
    /// `fsv 2026.5.29` (the fsv 3432bea package version) and exits 0.
    #[test]
    fn run_matches_python_main() {
        for flag in ["--version", "-V"] {
            let (code, text) = fsv_core::term::capture(|| run(argv(&["fsv", flag])));
            assert_eq!((code, text.as_str()), (0, "fsv 2026.5.29\n"));
        }
    }

    fn argv(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    /// Expected argv from `cli._reorder_root_flags` on the Typer tree.
    #[test]
    fn rewrite_moves_verbose_to_the_front() {
        let root = command();
        for (input, want) in [
            (&["changes", "ls", "-v"][..], &["-v", "changes", "ls"][..]),
            (&["-v", "changes", "ls"], &["-v", "changes", "ls"]),
            (
                &["changes", "--json", "ls", "--where", "x", "--no-input"],
                &["changes", "--json", "ls", "--where", "x", "--no-input"],
            ),
            (
                &["changes", "ls", "--where", "-v", "--verbose"],
                &["--verbose", "changes", "ls", "--where", "-v"],
            ),
            (
                &["changes", "ls", "--where=-v", "-v"],
                &["-v", "changes", "ls", "--where=-v"],
            ),
            (
                &["changes", "ls", "--", "-v"],
                &["changes", "ls", "--", "-v"],
            ),
            (
                &["tickets", "get", "1", "--verbose", "--format", "-v"],
                &["--verbose", "-v", "tickets", "get", "1", "--format"],
            ),
            (
                &["config", "set", "-v", "k", "v"],
                &["-v", "config", "set", "k", "v"],
            ),
            (&["changes", "assets", "ls"], &["changes", "assets", "ls"]),
        ] {
            assert_eq!(
                rewrite(&root, &argv(input)),
                (argv(want), None),
                "{input:?}"
            );
        }
    }

    /// Port of fsv.cli.ChangeLegacyGroup.parse_args: an ID after
    /// `<module> assets` or `changes associations` routes to the hidden
    /// `legacy` command; only associations print a group-level notice.
    #[test]
    fn rewrite_routes_legacy_change_ids() {
        let root = command();
        let (args, notice) = rewrite(&root, &argv(&["changes", "assets", "CHN-5"]));
        assert_eq!(args, argv(&["changes", "assets", "legacy", "CHN-5"]));
        assert_eq!(notice, None);
        let (args, notice) = rewrite(&root, &argv(&["tickets", "assets", "5"]));
        assert_eq!(args, argv(&["tickets", "assets", "legacy", "5"]));
        assert_eq!(notice, None);
        let (args, notice) = rewrite(&root, &argv(&["changes", "associations", "12", "ls"]));
        assert_eq!(
            args,
            argv(&["changes", "associations", "legacy", "12", "ls"])
        );
        assert!(
            notice
                .unwrap()
                .contains("fsv changes associations <ls|search|add|remove|pick>")
        );
        let (args, notice) = rewrite(&root, &argv(&["changes", "assets", "nope"]));
        assert_eq!((args, notice), (argv(&["changes", "assets", "nope"]), None));
    }

    /// Values from `cli._option_arity` on `changes ls`.
    #[test]
    fn option_arity_counts_value_options() {
        let root = command();
        let ls = root
            .find_subcommand("changes")
            .and_then(|c| c.find_subcommand("ls"))
            .unwrap();
        for (token, want) in [
            ("--where", 1),
            ("-w", 1),
            ("--where=x", 0),
            ("--json", 0),
            ("--limit", 0),
            ("-n", 1),
            ("--nope", 0),
            ("x", 0),
        ] {
            assert_eq!(option_arity(ls, token), want, "{token}");
        }
    }
}
