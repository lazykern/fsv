//! The target tenant is impossible to miss: one stderr line per tenant
//! command, tenant/config_dir in write JSON, tenant-bound plan hashes, a named
//! refusal for a plan reviewed on another tenant, and `--yes` writes that need
//! the tenant confirmed (Python `tests/test_tenant_visibility.py`,
//! `test_tenant_write_guard.py`, `test_tenant_identity.py`).

mod common;
use common::{DOMAIN, Env, Run, fixture};
use serde_json::{Value, json};

const GLOBEX: &str = "globex.freshservice.com";

fn spam_env() -> Env {
    Env::new(fixture("ticket_spam"))
}

fn dry_hash(env: &Env, extra: &[&str]) -> String {
    let r = env.run(&[&["tickets", "spam", "32", "--dry-run"][..], extra].concat());
    assert_eq!(r.code, 0, "{}", r.err);
    r.json()["plan_hash"].as_str().unwrap().to_string()
}

fn send(env: &Env, hash: &str, extra: &[&str]) -> Run {
    env.run(
        &[
            &[
                "tickets",
                "spam",
                "32",
                "--yes",
                "--expect-plan",
                hash,
                "--confirm-destructive",
            ][..],
            extra,
        ]
        .concat(),
    )
}

fn with_vars(mut env: Env, vars: &[(&str, &str)]) -> Env {
    for (k, v) in vars {
        env.vars.push((k.to_string(), v.to_string()));
    }
    env
}

fn banners(r: &Run) -> Vec<String> {
    r.err
        .lines()
        .filter(|l| l.starts_with("fsv: tenant"))
        .map(str::to_string)
        .collect()
}

#[test]
fn banner_once_on_stderr_and_stdout_stays_pure_json() {
    let env = with_vars(spam_env(), &[("FSV_QUIET", "")]);
    let r = env.run(&["tickets", "spam", "32", "--dry-run"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        banners(&r),
        [format!("fsv: tenant {DOMAIN} (config ~/config)")]
    );
    let review = r.json();
    assert_eq!(review["tenant"], json!(DOMAIN));
    assert_eq!(review["config_dir"], json!("<CONFIG_DIR>"));
    assert!(!r.out.contains("fsv: tenant"));
}

#[test]
fn quiet_flag_and_env_suppress_the_banner() {
    for (args, env) in [
        (vec!["--quiet", "tickets", "spam", "32", "--dry-run"], ""),
        (vec!["tickets", "spam", "32", "--dry-run", "--quiet"], ""),
        (vec!["tickets", "spam", "32", "--dry-run"], "1"),
    ] {
        let env = with_vars(spam_env(), &[("FSV_QUIET", env)]);
        let r = env.run(&args);
        assert_eq!(r.code, 0, "{}", r.err);
        assert!(banners(&r).is_empty(), "{}", r.err);
        r.json();
    }
}

#[test]
fn local_commands_print_no_banner() {
    for args in [
        vec!["config", "list"],
        vec!["journal", "path"],
        vec!["--help"],
        vec!["tickets", "spam", "--help"],
        vec!["--version"],
        vec!["completion", "show", "bash"],
    ] {
        let env = with_vars(spam_env(), &[("FSV_QUIET", "")]);
        let r = env.run(&args);
        assert!(banners(&r).is_empty(), "{args:?}: {}", r.err);
    }
}

#[test]
fn result_json_names_tenant_and_config_dir() {
    let env = spam_env();
    let hash = dry_hash(&env, &[]);
    let done = send(&env, &hash, &[]);
    assert_eq!(done.code, 0, "{}", done.err);
    let result = done.json();
    assert_eq!(result["tenant"], json!(DOMAIN));
    assert_eq!(result["config_dir"], json!("<CONFIG_DIR>"));
    assert_eq!(done.write_paths(), ["PUT /api/_/tickets/32/spam"]);
}

#[test]
fn read_json_is_not_annotated() {
    let r = Env::new(fixture("ls")).run(&["changes", "ls", "--json"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(!r.out.contains("\"config_dir\""), "{}", r.out);
}

/// A plan reviewed on one tenant is refused by name on another, even in a new
/// process (the on-disk registry under `$XDG_STATE_HOME`).
#[test]
fn plan_reviewed_on_another_tenant_is_refused_by_name() {
    let acme = spam_env();
    let digest = dry_hash(&acme, &[]);
    let globex = spam_env();
    std::fs::create_dir_all(globex.config_dir()).unwrap();
    std::fs::write(
        globex.config_dir().join("config.json"),
        json!({"domain": GLOBEX}).to_string(),
    )
    .unwrap();
    let globex = with_vars(
        globex,
        &[(
            "XDG_STATE_HOME",
            acme.home().join("state").to_str().unwrap(),
        )],
    );
    let r = send(&globex, &digest, &["--tenant", GLOBEX]);
    assert_eq!(r.code, 2, "{}", r.err);
    assert!(
        r.err.contains(&format!(
            "plan {} was reviewed on tenant {DOMAIN}",
            &digest[..12]
        )),
        "{}",
        r.err
    );
    assert!(r.err.contains(GLOBEX));
    assert!(r.writes().is_empty());
}

#[test]
fn tenant_option_must_match_configured_domain() {
    let env = spam_env();
    let r = env.run(&["tickets", "spam", "32", "--dry-run", "--tenant", GLOBEX]);
    assert_eq!(r.code, 2, "{}", r.err);
    assert!(
        r.err.contains(&format!(
            "--tenant {GLOBEX} does not match the configured tenant {DOMAIN}"
        )),
        "{}",
        r.err
    );
    let ok = env.run(&[
        "tickets",
        "spam",
        "32",
        "--dry-run",
        &format!("--tenant={DOMAIN}"),
    ]);
    assert_eq!(ok.code, 0, "{}", ok.err);
}

#[test]
fn write_refused_without_confirmation_and_dry_run_never_needs_it() {
    let env = with_vars(spam_env(), &[("FSV_ALLOW_TENANT", "")]);
    assert_eq!(env.run(&["tickets", "spam", "32", "--dry-run"]).code, 0);
    assert_eq!(
        env.run(&["tickets", "spam", "32", "--dry-run", "--yes"])
            .code,
        0
    );
    let hash = dry_hash(&env, &[]);
    let r = send(&env, &hash, &[]);
    assert_eq!(r.code, 2, "{}", r.err);
    assert!(r.writes().is_empty());
    assert!(
        r.err
            .contains(&format!("refusing to write: tenant {DOMAIN}")),
        "{}",
        r.err
    );
    assert!(r.err.contains(&format!("--tenant {DOMAIN}")));
    assert!(r.err.contains(&format!("FSV_ALLOW_TENANT={DOMAIN}")));
    assert!(
        r.err
            .contains(&format!("fsv config set tenant.confirmed {DOMAIN}"))
    );
}

#[test]
fn success_with_tenant_option() {
    let env = with_vars(spam_env(), &[("FSV_ALLOW_TENANT", "")]);
    let hash = dry_hash(&env, &[]);
    let r = send(&env, &hash, &["--tenant", DOMAIN]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/32/spam"]);
}

#[test]
fn env_confirmation_and_mismatch() {
    let env = with_vars(spam_env(), &[("FSV_ALLOW_TENANT", GLOBEX)]);
    let hash = dry_hash(&env, &[]);
    let r = send(&env, &hash, &[]);
    assert_eq!(r.code, 2, "{}", r.err);
    assert!(
        r.err
            .contains(&format!("FSV_ALLOW_TENANT={GLOBEX} does not match")),
        "{}",
        r.err
    );
    assert!(r.writes().is_empty());
    let env = with_vars(spam_env(), &[("FSV_ALLOW_TENANT", &DOMAIN.to_uppercase())]);
    let r = send(&env, &hash, &[]);
    assert_eq!(r.code, 0, "{}", r.err);
}

#[test]
fn config_confirmation_set_once() {
    let env = with_vars(spam_env(), &[("FSV_ALLOW_TENANT", "")]);
    let bad = env.run(&["config", "set", "tenant.confirmed", GLOBEX]);
    assert_eq!(bad.code, 2, "{}", bad.err);
    assert!(
        bad.err.contains(&format!("configured tenant is {DOMAIN}")),
        "{}",
        bad.err
    );
    let ok = env.run(&["config", "set", "tenant.confirmed", &DOMAIN.to_uppercase()]);
    assert_eq!(ok.code, 0, "{}", ok.err);
    assert_eq!(ok.out, format!("tenant.confirmed = {DOMAIN}\n"));
    let hash = dry_hash(&env, &[]);
    let r = send(&env, &hash, &[]);
    assert_eq!(r.code, 0, "{}", r.err);
    let bad = env.run(&["config", "set", "tenant.confirmed", "not a domain"]);
    assert_eq!(bad.code, 2, "{}", bad.err);
}

// ---------------------------------------------------------------- identity --

#[test]
fn whoami_text_and_json() {
    let env = Env::new(fixture("auth"));
    let r = env.run(&["whoami"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(
        r.out.contains("Me Agent") && r.out.contains("me@acme.test"),
        "{}",
        r.out
    );
    assert!(
        r.out
            .contains(&format!("tenant:      {DOMAIN}\nconfig dir:  ~/config\n")),
        "{}",
        r.out
    );
    let data = env.run(&["whoami", "--json"]).json();
    assert_eq!(
        data,
        json!({"name": "Me Agent", "email": "me@acme.test", "tenant": DOMAIN,
               "config_dir": "<CONFIG_DIR>"})
    );
}

#[test]
fn whoami_prints_the_banner() {
    let env = with_vars(Env::new(fixture("auth")), &[("FSV_QUIET", "")]);
    let r = env.run(&["whoami"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(
        r.err.starts_with(&format!("fsv: tenant {DOMAIN} (config ")),
        "{}",
        r.err
    );
}

#[test]
fn config_show_names_tenant_dir_and_confirmation() {
    let env = with_vars(Env::new(vec![]), &[("FSV_ALLOW_TENANT", "")]);
    let r = env.run(&["config", "show"]);
    assert_eq!(r.code, 0, "{}", r.err);
    let lines: Vec<&str> = r.out.lines().collect();
    assert_eq!(lines[0], format!("tenant:      {DOMAIN}"));
    assert_eq!(lines[1], "config dir:  ~/config");
    assert!(
        lines[2].starts_with("writes:      not confirmed"),
        "{}",
        r.out
    );
    assert_eq!(
        env.run(&["config", "set", "tenant.confirmed", &DOMAIN.to_uppercase()])
            .code,
        0
    );
    let data: Value = env.run(&["config", "show", "--json"]).json();
    assert_eq!(data["tenant"], json!(DOMAIN));
    assert_eq!(data["config_dir"], json!("<CONFIG_DIR>"));
    assert_eq!(data["writes_confirmed_by"], json!("tenant.confirmed"));
    assert_eq!(data["values"]["tenant.confirmed"], json!(DOMAIN));
    assert!(data["values"].as_object().unwrap().len() >= 11);
}

#[test]
fn auth_status_shows_tenant_and_config_dir() {
    let env = Env::new(fixture("auth"));
    let r = env.run(&["auth", "status"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(
        r.out.contains(&format!("tenant:      {DOMAIN}")),
        "{}",
        r.out
    );
    assert!(r.out.contains("config dir:  ~/config"), "{}", r.out);
    let mut env = Env::new(vec![]);
    env.session = false;
    let missing = env.run(&["auth", "status"]);
    assert_eq!(missing.code, 2, "{}", missing.err);
    assert!(
        missing
            .err
            .contains(&format!("no session for tenant {DOMAIN}")),
        "{}",
        missing.err
    );
}

/// Commands added after the tenant guard (problem and change notes, plain
/// ticket reply, tickets/problems/changes update, due-by, planning-text) take
/// the same refusal on an unconfirmed tenant, send nothing, and their dry-run
/// JSON names the tenant and config dir with a tenant-bound plan hash.
#[test]
fn reviewed_commands_added_later_are_tenant_guarded_and_annotated() {
    let cases: &[(&str, &[&str])] = &[
        (
            "problem_note_reviewed",
            &["problems", "add-note", "PRB-12", "Acme evidence"],
        ),
        (
            "problem_note_reviewed",
            &["changes", "add-note", "CHN-3", "Acme change note"],
        ),
        (
            "ticket_reply_plain",
            &["tickets", "reply", "INC-123", "Acme is working on it"],
        ),
        (
            "change_update_clean",
            &["changes", "update", "12", "--priority", "High"],
        ),
        (
            "change_helpers",
            &[
                "changes",
                "update",
                "5",
                "--due-by",
                "2026-06-09T18:00:00+07:00",
            ],
        ),
        (
            "planning_edit",
            &[
                "changes",
                "planning-text",
                "126",
                "--section",
                "Rollout Plan",
                "--clear",
            ],
        ),
    ];
    for (route, args) in cases {
        let env = with_vars(Env::new(fixture(route)), &[("FSV_ALLOW_TENANT", "")]);
        let dry = env.run(&[&args[..], &["--dry-run", "--json"]].concat());
        if dry.code == 0 {
            let doc = dry.json();
            if doc["status"] != "nothing_to_change" {
                let hash = doc["plan_hash"].as_str().unwrap_or_default();
                assert_eq!(hash.len(), 64, "{args:?}: {}", dry.out);
            }
            assert_eq!(doc["tenant"], json!(DOMAIN), "{args:?}: {}", dry.out);
            assert!(doc["config_dir"].is_string(), "{args:?}: {}", dry.out);
        }
        let zeros = "0".repeat(64);
        let r = env.run(&[&args[..], &["--yes", "--expect-plan", &zeros]].concat());
        assert_eq!(r.code, 2, "{args:?}: {}", r.err);
        assert!(r.writes().is_empty(), "{args:?}");
        assert!(
            r.err
                .contains(&format!("refusing to write: tenant {DOMAIN}")),
            "{args:?}: {}",
            r.err
        );
    }
}
