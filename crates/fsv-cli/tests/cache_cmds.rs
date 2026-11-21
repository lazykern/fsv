//! `fsv cache clear|status` on a scratch cache directory
//! (`tests/test_cache_cli.py`, `tests/test_schema_freshness.py`).

use std::path::Path;
use std::process::{Command, Output, Stdio};

struct Run {
    code: i32,
    out: String,
    err: String,
}

fn fsv(home: &Path, args: &[&str]) -> Run {
    let Output {
        status,
        stdout,
        stderr,
    } = Command::new(env!("CARGO_BIN_EXE_fsv"))
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", home)
        .env("FSV_CONFIG_DIR", home.join("config"))
        .env("FSV_CACHE_DIR", home.join("cache"))
        .env("XDG_STATE_HOME", home.join("state"))
        .env("NO_COLOR", "1")
        .env("COLUMNS", "200")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    Run {
        code: status.code().unwrap_or(-1),
        out: String::from_utf8_lossy(&stdout).into_owned(),
        err: String::from_utf8_lossy(&stderr).into_owned(),
    }
}

fn seed(home: &Path) {
    for sub in ["schema", "forms"] {
        let dir = home.join("cache").join(sub);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("acme--tickets.json"), "{}").unwrap();
    }
}

/// `test_cache_clear_targets_are_precise`.
#[test]
fn cache_clear_targets_are_precise() {
    let tmp = tempfile::tempdir().unwrap();
    let h = tmp.path();
    let cache = h.join("cache");
    seed(h);
    assert_eq!(fsv(h, &["cache", "clear", "schema"]).code, 0);
    assert!(!cache.join("schema").exists());
    assert!(cache.join("forms").join("acme--tickets.json").exists());
    seed(h);
    let r = fsv(h, &["cache", "clear", "forms"]);
    assert_eq!(r.code, 0);
    assert!(r.out.contains("forms cache cleared"), "{}", r.out);
    assert!(cache.join("schema").join("acme--tickets.json").exists());
    assert!(!cache.join("forms").exists());
    seed(h);
    assert_eq!(fsv(h, &["cache", "clear", "all"]).code, 0);
    assert!(!cache.join("schema").exists() && !cache.join("forms").exists());
}

#[test]
fn cache_clear_help_and_rejection_name_forms() {
    let tmp = tempfile::tempdir().unwrap();
    let r = fsv(tmp.path(), &["cache", "clear", "bogus"]);
    assert_eq!(r.code, 2);
    assert!(
        r.err.contains("choose schema|forms|views|groups|all"),
        "{}",
        r.err
    );
    let help = fsv(tmp.path(), &["cache", "clear", "--help"]);
    assert_eq!(help.code, 0);
    assert!(
        help.out.contains("schema|forms|views|groups|all"),
        "{}",
        help.out
    );
}

/// `test_cache_status_prints_age_and_ttl_per_cache`, with the form-cache rows
/// and the caption naming the clear targets.
#[test]
fn cache_status_labels_form_caches_with_age_and_ttl() {
    let tmp = tempfile::tempdir().unwrap();
    let h = tmp.path();
    let forms = h.join("cache").join("forms");
    std::fs::create_dir_all(&forms).unwrap();
    let saved = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        - 120.0;
    for (name, kind) in [
        ("acme--ticket-fields.json", "schema"),
        ("acme--ticket-rules.json", "form_rules"),
    ] {
        let doc = format!(r#"{{"_kind": "{kind}", "saved_at": {saved}, "data": {{}}}}"#);
        std::fs::write(forms.join(name), doc).unwrap();
    }
    let r = fsv(h, &["cache", "status"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.out.contains("forms/ticket-fields"), "{}", r.out);
    assert!(
        r.out.contains("form fields") && r.out.contains("form rules"),
        "{}",
        r.out
    );
    assert!(r.out.contains("2m") && r.out.contains("15m"), "{}", r.out);
    assert!(r.out.contains("clear: fsv cache clear schema"), "{}", r.out);
}
