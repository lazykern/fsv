//! Every command that can really write goes through the shared tenant guard
//! (Python `tests/test_tenant_write_guard.py::test_every_yes_command_routes_through_the_tenant_guard`)
//! and no command prints JSON without the tenant-aware serializer
//! (`test_tenant_identity.py::test_command_json_goes_through_the_tenant_serializer`).

use clap::Command;
use fsv_core::tenant::{self, CommandInfo};

fn leaves(cmd: &Command, prefix: &str, out: &mut Vec<(String, Vec<String>)>) {
    for sub in cmd.get_subcommands() {
        let path = if prefix.is_empty() {
            sub.get_name().to_string()
        } else {
            format!("{prefix} {}", sub.get_name())
        };
        if sub.has_subcommands() {
            leaves(sub, &path, out);
        } else {
            let longs = sub
                .get_arguments()
                .filter_map(|a| a.get_long().map(str::to_string))
                .collect();
            out.push((path, longs));
        }
    }
}

#[test]
fn every_yes_command_is_refused_on_an_unconfirmed_tenant() {
    let dir = tempfile::tempdir().unwrap();
    fsv_core::config::use_for_tests(fsv_core::Paths::under(dir.path()), "guard.example.com");
    assert!(
        !std::env::var("FSV_ALLOW_TENANT").is_ok_and(|v| v == "guard.example.com"),
        "FSV_ALLOW_TENANT must not confirm the test tenant"
    );
    let mut all = Vec::new();
    leaves(&fsv_cli::command(), "", &mut all);
    let mut guarded = Vec::new();
    for (path, longs) in &all {
        let has = |f: &str| longs.iter().any(|l| l == f);
        assert!(
            !(has("dry-run") || has("expect-plan")) || has("yes"),
            "{path}: reviewed command without --yes"
        );
        if !has("yes") {
            continue;
        }
        let info = CommandInfo {
            path: path.clone(),
            write_capable: true,
            yes: true,
            ..CommandInfo::default()
        };
        if tenant::LOCAL_YES_COMMANDS.contains(&path.as_str()) {
            assert!(tenant::check(&info).is_ok(), "{path}");
            continue;
        }
        let refusal = tenant::check(&info).expect_err(path);
        assert!(
            refusal
                .0
                .starts_with("refusing to write: tenant guard.example.com"),
            "{path}"
        );
        guarded.push(path.clone());
    }
    assert!(guarded.len() > 60, "{}", guarded.len());
    for want in [
        "tickets bulk",
        "changes delete",
        "tickets spam",
        "problems delete",
    ] {
        assert!(guarded.iter().any(|p| p == want), "{want}");
    }
    for p in tenant::LOCAL_YES_COMMANDS {
        assert!(all.iter().any(|(path, _)| path == p), "{p}");
    }
}

/// A printed JSON document goes through `tenant::annotate` (inside the
/// output helpers); a bare `println!("{}", Dumps...)` of a command result
/// would silently drop `tenant`/`config_dir`.
#[test]
fn command_json_goes_through_the_tenant_serializer() {
    // Where Python prints without tenant_json too: local commands, Rich's
    // `print_json` of a review, and request-body construction.
    let allowed: &[(&str, &str)] = &[
        ("cmd/journal.rs", "local command"),
        ("cmd/links.rs", "err.print_json review"),
    ];
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cmd");
    let mut offenders = Vec::new();
    for entry in std::fs::read_dir(&src).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let rel = format!("cmd/{}", path.file_name().unwrap().to_string_lossy());
        if allowed.iter().any(|(f, _)| *f == rel) {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let mut rest = text.as_str();
        while let Some(i) = rest.find("println!(") {
            let stmt_end = rest[i..].find(");").map_or(rest.len(), |e| i + e);
            let stmt = &rest[i..stmt_end];
            let bare = stmt
                .split_once(',')
                .is_some_and(|(_, args)| args.trim_start().starts_with("fsv_py::Dumps"));
            if bare && !stmt.contains("annotate") {
                offenders.push(format!("{rel}: {}", stmt.lines().next().unwrap_or("")));
            }
            rest = &rest[i + "println!(".len()..];
        }
    }
    assert!(offenders.is_empty(), "{offenders:#?}");
}
