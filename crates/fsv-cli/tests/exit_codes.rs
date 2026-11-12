//! Exit codes and messages of the `fsv` binary on paths that need no network.

use std::process::{Command, Output, Stdio};

struct Run {
    code: i32,
    out: String,
    err: String,
}

fn fsv(args: &[&str]) -> Run {
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    let Output {
        status,
        stdout,
        stderr,
    } = Command::new(env!("CARGO_BIN_EXE_fsv"))
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", h)
        .env("FSV_CONFIG_DIR", h.join("config"))
        .env("FSV_CACHE_DIR", h.join("cache"))
        .env("XDG_STATE_HOME", h.join("state"))
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    Run {
        code: status.code().unwrap_or(-1),
        out: String::from_utf8_lossy(&stdout).into_owned(),
        err: String::from_utf8_lossy(&stderr).into_owned(),
    }
}

#[test]
fn bare_root_prints_help_and_exits_2() {
    let r = fsv(&[]);
    assert_eq!(r.code, 2);
    assert!(r.out.contains("Usage: fsv"), "{}", r.out);
}

#[test]
fn bare_group_prints_help_and_exits_2() {
    let r = fsv(&["changes"]);
    assert_eq!(r.code, 2);
    assert!(r.out.contains("Usage: fsv changes"), "{}", r.out);
}

#[test]
fn root_flag_alone_prints_help_and_exits_0() {
    assert_eq!(fsv(&["-v"]).code, 0);
}

#[test]
fn version() {
    let r = fsv(&["-V"]);
    assert_eq!(r.code, 0);
    assert_eq!(r.out.trim(), format!("fsv {}", env!("CARGO_PKG_VERSION")));
}

#[test]
fn help_topic() {
    let r = fsv(&["help", "nope"]);
    assert_eq!(r.code, 2);
    assert!(r.err.contains("unknown help topic 'nope'"), "{}", r.err);
    assert_eq!(fsv(&["help", "auth"]).code, 0);
}

#[test]
fn usage_errors_exit_2() {
    assert_eq!(fsv(&["changes", "ls", "-o", "xml"]).code, 2);
    assert_eq!(fsv(&["nope"]).code, 2);
    assert_eq!(fsv(&["changes", "get"]).code, 2);
}

#[test]
fn config_unknown_key() {
    let r = fsv(&["config", "get", "bad"]);
    assert_eq!(r.code, 2);
    assert!(r.err.contains("unknown key: bad"), "{}", r.err);
}

#[test]
fn cache_clear_needs_target() {
    let r = fsv(&["cache", "clear"]);
    assert_eq!(r.code, 2);
    assert!(
        r.err.contains("choose schema|views|groups|all"),
        "{}",
        r.err
    );
}

#[test]
fn no_session_exits_3() {
    let r = fsv(&["changes", "ls"]);
    assert_eq!(r.code, 3);
    assert!(r.err.contains("no session"), "{}", r.err);
    assert_eq!(fsv(&["changes", "ls", "-v"]).code, 3);
}

#[test]
fn url_without_domain_exits_1() {
    assert_eq!(fsv(&["changes", "url", "5"]).code, 1);
}

#[test]
fn bad_id_exits_2() {
    let r = fsv(&["changes", "get", "abc"]);
    assert_eq!(r.code, 2);
    assert!(r.err.contains("invalid id"), "{}", r.err);
}

#[test]
fn task_delete_dry_run_is_local() {
    let r = fsv(&["changes", "tasks-delete", "CHN-1", "2", "--dry-run"]);
    assert_eq!(r.code, 0, "{}", r.err);
    let v: serde_json::Value = serde_json::from_str(&r.out).unwrap();
    assert_eq!(v["change_id"], 1);
    assert_eq!(v["task_id"], 2);
}

#[test]
fn form_module_is_checked_first() {
    let r = fsv(&["form", "dump", "nope"]);
    assert_eq!(r.code, 2);
    assert!(
        r.err
            .contains("unsupported form 'nope'; supported: changes, tickets, problems"),
        "{}",
        r.err
    );
}

#[test]
fn sr_download_rejects_option_like_dir() {
    assert_eq!(fsv(&["sr", "get", "1", "--download", "-x"]).code, 2);
}

#[test]
fn ticket_note_dry_run_is_local() {
    let r = fsv(&["tickets", "add-note", "5", "-b", "hi", "--dry-run"]);
    assert_eq!(r.code, 0, "{}", r.err);
    let v: serde_json::Value = serde_json::from_str(&r.out).unwrap();
    assert_eq!(v["status"], "dry_run");
    assert_eq!(v["ticket_id"], 5);
}

#[test]
fn legacy_assets_route_prints_notice() {
    let r = fsv(&["changes", "assets", "5"]);
    assert_eq!(r.code, 3);
    assert!(
        r.err.contains("deprecated: use `fsv changes assets"),
        "{}",
        r.err
    );
}

#[test]
fn form_diff_unreadable_snapshot() {
    let r = fsv(&["form", "diff", "/nonexistent.json"]);
    assert_eq!(r.code, 2);
    assert!(
        r.err.contains("cannot read snapshot /nonexistent.json"),
        "{}",
        r.err
    );
}

#[test]
fn identical_snapshots_exit_0() {
    let dir = tempfile::tempdir().unwrap();
    let snap = dir.path().join("s.json");
    std::fs::write(&snap, r#"{"fsv_form_snapshot": 1, "fields": []}"#).unwrap();
    let s = snap.to_str().unwrap();
    let r = fsv(&["form", "diff", s, "--against", s]);
    assert_eq!(r.code, 0);
    assert_eq!(r.out, "no changes\n");
}
