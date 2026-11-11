//! Local commands (auth, config, cache, journal, completion, setup, search)
//! against canned routes. Expected output was captured from the Python fsv
//! run on the same routes (`tests/golden/cmp.py --emit`); tests that print
//! temp paths or race background threads are written by hand from a Python
//! run and say so.

mod common;
use common::{Env, Opts, fixture, run_fixture, run_steps, run_with};

#[test]
fn auth_status_shows_identity() {
    let r = run_fixture("auth", &["auth", "status"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"Me Agent  <me@acme.test>
session:     valid
department:  IT, Ops  (Platform)
session age: 0.0h
rate limit:  4990/5000
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn auth_logout_needs_yes() {
    let r = run_fixture("local", &["auth", "logout"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: pass --yes to confirm non-interactive logout
"#
    );
}

#[test]
fn auth_logout_yes() {
    let r = run_fixture("local", &["auth", "logout", "--yes"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"session cleared
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn auth_status_after_logout() {
    let r = run_steps(
        "local",
        &[&["auth", "logout", "--yes"]],
        &["auth", "status"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: no session; run `fsv auth login`
"#
    );
}

#[test]
fn auth_login_no_input_needs_header() {
    let r = run_fixture("local", &["auth", "login", "--no-input"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: pass --header when using --no-input
"#
    );
}

#[test]
fn auth_login_rejects_unknown_store() {
    let r = run_fixture("local", &["auth", "login", "--store", "vault"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: --store must be 'file', 'argon', or 'keychain'
"#
    );
}

#[test]
fn auth_login_browser_and_header() {
    let r = run_fixture(
        "local",
        &["auth", "login", "--browser", "--header", "x=y"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: use either --browser or --header, not both
"#
    );
}

#[test]
fn auth_login_missing_cookies() {
    let r = run_fixture(
        "local",
        &["auth", "login", "--header", "foo=bar", "--store", "file"],
        None,
    );
    assert_eq!(r.code, 3, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: missing required cookies: ['_x_m', '_x_d', '_x_w', 'fw-session-id']
"#
    );
}

#[test]
fn auth_login_header_from_stdin() {
    let r = run_fixture(
        "auth",
        &[
            "auth", "login", "--header", "-", "--store", "file", "--force",
        ],
        Some("_x_m=1; _x_d=2; _x_w=3; fw-session-id=4"),
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"saved 4 cookies to ~/config/session.json
logged in as: Me Agent <me@acme.test>
"#
    );
    assert_eq!(
        r.err,
        r#"warming cache in background...
"#
    );
}

#[test]
fn auth_login_expired_cookies() {
    let r = run_fixture(
        "local",
        &[
            "auth",
            "login",
            "--header",
            "_x_m=1; _x_d=2; _x_w=3; fw-session-id=4",
            "--store",
            "file",
        ],
        None,
    );
    assert_eq!(r.code, 1, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: could not verify cookies against acme.freshservice.com: HTTP 404 listing me: no route; nothing saved (use --force to save anyway)
"#
    );
}

#[test]
fn config_list_defaults() {
    let r = run_fixture("local", &["config", "list"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"completion.network = (unset)  # enable remote requester/agent completion 
defaults.requester = (unset)  # requester email for `changes create` when --requester is not given
changes.attach_flow = (unset)  # `changes create` attachments: after-create (default) | before-create (unverified vs the web)
forms.workspace_id = (unset)  # workspace whose business rules apply (default: the form metadata, else the account)
defaults.sr_requester = (unset)  # requester email for `sr create` when --requester is not given
tui.read_only = (unset)  # open the TUI without write actions 
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn config_set_rejects_value() {
    let r = run_fixture(
        "local",
        &["config", "set", "completion.network", "maybe"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"completion.network = maybe
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn config_set_then_list() {
    let r = run_steps(
        "local",
        &[&["config", "set", "completion.network", "on"]],
        &["config", "list"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"completion.network = on  # enable remote requester/agent completion 
defaults.requester = (unset)  # requester email for `changes create` when --requester is not given
changes.attach_flow = (unset)  # `changes create` attachments: after-create (default) | before-create (unverified vs the web)
forms.workspace_id = (unset)  # workspace whose business rules apply (default: the form metadata, else the account)
defaults.sr_requester = (unset)  # requester email for `sr create` when --requester is not given
tui.read_only = (unset)  # open the TUI without write actions 
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn config_set_prints_value() {
    let r = run_fixture(
        "local",
        &["config", "set", "completion.network", "off"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"completion.network = off
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn config_defaults_show_empty() {
    let r = run_fixture("local", &["config", "defaults", "show"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"changes: (none)
tickets: (none)
problems: (none)
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn config_defaults_show_one_empty() {
    let r = run_fixture("local", &["config", "defaults", "show", "changes"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"changes: (none)
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn config_defaults_set_prints() {
    let r = run_fixture(
        "local",
        &[
            "config",
            "defaults",
            "set",
            "changes",
            "--where",
            "status=Open",
            "--where",
            "priority=High",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"defaults.changes.where = ['status=Open', 'priority=High']
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn config_defaults_show_after_set() {
    let r = run_steps(
        "local",
        &[&["config", "defaults", "set", "tickets", "-w", "status=Open"]],
        &["config", "defaults", "show"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"changes: (none)
tickets: status=Open
problems: (none)
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn config_defaults_show_unknown_resource() {
    let r = run_fixture("local", &["config", "defaults", "show", "widgets"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"widgets: (none)
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn config_defaults_clear_after_set() {
    let r = run_steps(
        "local",
        &[&["config", "defaults", "set", "tickets", "-w", "status=Open"]],
        &["config", "defaults", "clear", "tickets"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"defaults.tickets.where cleared
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn config_defaults_clear_empty() {
    let r = run_fixture("local", &["config", "defaults", "clear", "problems"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"defaults.problems.where cleared
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn cache_status_empty() {
    let r = run_fixture("local", &["cache", "status"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"            cache status            
┏━━━━━━━┳━━━━━━┳━━━━━┳━━━━━━━┳━━━━━┓
┃ cache ┃ kind ┃ age ┃ state ┃ ttl ┃
┡━━━━━━━╇━━━━━━╇━━━━━╇━━━━━━━╇━━━━━┩
└───────┴──────┴─────┴───────┴─────┘
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn journal_ls_empty() {
    let r = run_fixture("local", &["journal", "ls"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"journal is empty
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn journal_tail_empty() {
    let r = run_fixture("local", &["journal", "tail", "--json"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(r.err, r#""#);
}

#[test]
fn completion_doctor_fresh() {
    let r = run_fixture("local", &["completion", "doctor"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"      completion doctor      
┏━━━━━━━━━━━━━━━━━┳━━━━━━━━━┓
┃ check           ┃ value   ┃
┡━━━━━━━━━━━━━━━━━╇━━━━━━━━━┩
│ shell           │ ?       │
│ fsv path        │ fsv     │
│ network         │ off     │
│ schema changes  │ missing │
│ schema tickets  │ missing │
│ schema problems │ missing │
└─────────────────┴─────────┘
Use TAB after a prefix, e.g. `fsv changes ls --where requester=phu<TAB>`.
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn completion_install_needs_shell() {
    let r = run_fixture("local", &["completion", "install"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: could not detect shell; pass one of: bash, zsh, fish, powershell, pwsh
"#
    );
}

#[test]
fn setup_needs_terminal() {
    let r = run_fixture("local", &["setup"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: setup requires an interactive terminal
"#
    );
}

#[test]
fn search_table() {
    let r = run_fixture("auth", &["search", "printer"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"┏━━━━━━━━┳━━━━━━━┳━━━━━━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━━━━━┓
┃ Type   ┃ ID    ┃ Title         ┃ Status   ┃ Group        ┃
┡━━━━━━━━╇━━━━━━━╇━━━━━━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━━━━━┩
│ ticket │ INC-5 │ Printer jam   │ Open     │ Service Desk │
│        │       │  on 3rd floor │          │              │
│ change │ CHN-9 │ Patch DB      │ Planning │ -            │
│ ci     │ AST-3 │ db-01         │ Server   │ Ops          │
└────────┴───────┴───────────────┴──────────┴──────────────┘
"#
    );
    assert_eq!(
        r.err,
        r#"3 rows
"#
    );
}

#[test]
fn search_csv() {
    let r = run_fixture("auth", &["search", "printer", "-o", "csv"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        "type,id,title,status,group\r\nticket,INC-5,\"Printer jam\n on 3rd floor\",Open,Service Desk\r\nchange,CHN-9,Patch DB,Planning,-\r\nci,AST-3,db-01,Server,Ops\r\n"
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn search_tsv() {
    let r = run_fixture("auth", &["search", "printer", "-o", "tsv"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        "type\tid\ttitle\tstatus\tgroup\r\nticket\tINC-5\t\"Printer jam\n on 3rd floor\"\tOpen\tService Desk\r\nchange\tCHN-9\tPatch DB\tPlanning\t-\r\nci\tAST-3\tdb-01\tServer\tOps\r\n"
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn search_json() {
    let r = run_fixture("auth", &["search", "printer", "--json"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "result_type": "ticket",
    "ticket_display_id": "INC-5",
    "subject": "Printer <b>jam</b>\n on 3rd floor",
    "ticket_status": "Open",
    "ticket_group": "Service Desk"
  },
  {
    "result_type": "change",
    "itil_module_display_id": "CHN-9",
    "subject": "Patch DB",
    "itil_module_status": "Planning"
  },
  {
    "result_type": "ci",
    "ci_display_id": "AST-3",
    "name": "db-01",
    "ci_type": "Server",
    "owner_name": "Ops"
  }
]
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn search_all_pages() {
    let r = run_fixture("auth", &["search", "printer", "--all"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"┏━━━━━━━━┳━━━━━━━┳━━━━━━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━━━━━┓
┃ Type   ┃ ID    ┃ Title         ┃ Status   ┃ Group        ┃
┡━━━━━━━━╇━━━━━━━╇━━━━━━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━━━━━┩
│ ticket │ INC-5 │ Printer jam   │ Open     │ Service Desk │
│        │       │  on 3rd floor │          │              │
│ change │ CHN-9 │ Patch DB      │ Planning │ -            │
│ ci     │ AST-3 │ db-01         │ Server   │ Ops          │
└────────┴───────┴───────────────┴──────────┴──────────────┘
"#
    );
    assert_eq!(
        r.err,
        r#"  fetched 3...
3 rows
"#
    );
}

#[test]
fn search_error() {
    let r = run_fixture("local", &["search", "printer"], None);
    assert_eq!(r.code, 5, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: HTTP 404 listing all: no route
"#
    );
}

#[test]
fn auth_logout_confirmed() {
    let r = run_with(
        "local",
        &[],
        &["auth", "logout"],
        Some("y\n"),
        Opts {
            tty: true,
            env: &[],
        },
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"Wipe session for acme.freshservice.com? [y/N]: session cleared
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn auth_logout_refused() {
    let r = run_with(
        "local",
        &[],
        &["auth", "logout"],
        Some("n\n"),
        Opts {
            tty: true,
            env: &[],
        },
    );
    assert_eq!(r.code, 1, "{}", r.err);
    assert_eq!(r.out, r#"Wipe session for acme.freshservice.com? [y/N]: "#);
    assert_eq!(
        r.err,
        r#"Aborted.
"#
    );
}

#[test]
fn auth_login_prompts_domain_and_store() {
    let r = run_with(
        "auth",
        &[],
        &[
            "auth",
            "login",
            "--header",
            "_x_m=1; _x_d=2; _x_w=3; fw-session-id=4",
            "--force",
        ],
        Some("\n3\n"),
        Opts {
            tty: true,
            env: &[],
        },
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"Domain [acme.freshservice.com]:                           Store session                           
┏━━━┳━━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ # ┃ Backend        ┃ Notes                                     ┃
┡━━━╇━━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┩
│ 1 │ keychain       │ macOS Keychain; recommended               │
│ 2 │ argon          │ encrypted file; asks passphrase when used │
│ 3 │ file (default) │ plain JSON file; chmod 600                │
└───┴────────────────┴───────────────────────────────────────────┘
Choose store [3]: saved 4 cookies to ~/config/session.json
logged in as: Me Agent <me@acme.test>
"#
    );
    assert_eq!(
        r.err,
        r#"warming cache in background...
"#
    );
}

#[test]
fn auth_login_store_by_name() {
    let r = run_with(
        "auth",
        &[],
        &[
            "auth",
            "login",
            "--header",
            "_x_m=1; _x_d=2; _x_w=3; fw-session-id=4",
            "--force",
        ],
        Some("\nfile\n"),
        Opts {
            tty: true,
            env: &[],
        },
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"Domain [acme.freshservice.com]:                           Store session                           
┏━━━┳━━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ # ┃ Backend        ┃ Notes                                     ┃
┡━━━╇━━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┩
│ 1 │ keychain       │ macOS Keychain; recommended               │
│ 2 │ argon          │ encrypted file; asks passphrase when used │
│ 3 │ file (default) │ plain JSON file; chmod 600                │
└───┴────────────────┴───────────────────────────────────────────┘
Choose store [3]: saved 4 cookies to ~/config/session.json
logged in as: Me Agent <me@acme.test>
"#
    );
    assert_eq!(
        r.err,
        r#"warming cache in background...
"#
    );
}

#[test]
fn auth_login_pasted_url() {
    let r = run_with(
        "auth",
        &[],
        &[
            "auth",
            "login",
            "--header",
            "_x_m=1; _x_d=2; _x_w=3; fw-session-id=4",
            "--force",
        ],
        Some("https://globex.freshservice.com/a/tickets/5\ny\n3\n"),
        Opts {
            tty: true,
            env: &[],
        },
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"Domain [acme.freshservice.com]: Extracted globex.freshservice.com — login? [Y/n]:                           Store session                           
┏━━━┳━━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ # ┃ Backend        ┃ Notes                                     ┃
┡━━━╇━━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┩
│ 1 │ keychain       │ macOS Keychain; recommended               │
│ 2 │ argon          │ encrypted file; asks passphrase when used │
│ 3 │ file (default) │ plain JSON file; chmod 600                │
└───┴────────────────┴───────────────────────────────────────────┘
Choose store [3]: saved 4 cookies to ~/config/session.json
logged in as: Me Agent <me@acme.test>
"#
    );
    assert_eq!(
        r.err,
        r#"warming cache in background...
"#
    );
}

#[test]
fn auth_login_pasted_url_refused() {
    let r = run_with(
        "auth",
        &[],
        &[
            "auth",
            "login",
            "--header",
            "_x_m=1; _x_d=2; _x_w=3; fw-session-id=4",
            "--force",
        ],
        Some("https://globex.freshservice.com/a/tickets/5\nn\n"),
        Opts {
            tty: true,
            env: &[],
        },
    );
    assert_eq!(r.code, 1, "{}", r.err);
    assert_eq!(
        r.out,
        r#"Domain [acme.freshservice.com]: Extracted globex.freshservice.com — login? [Y/n]: "#
    );
    assert_eq!(
        r.err,
        r#"Aborted.
"#
    );
}

#[test]
fn setup_domain_step() {
    let r = run_with(
        "local",
        &[],
        &["setup"],
        Some("1\nglobex.freshservice.com\n"),
        Opts {
            tty: true,
            env: &[],
        },
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                Current status                
┌────────────────────┬───────────────────────┐
│ domain             │ acme.freshservice.com │
│ auth               │ ✓ file, 0.0h ago      │
│ completion         │ ✗ not installed       │
│ network completion │ (unset)               │
│ defaults.changes   │ (none)                │
│ defaults.tickets   │ (none)                │
│ defaults.problems  │ (none)                │
└────────────────────┴───────────────────────┘

┌───┬────────────────────┬───────────────────────────────┐
│ 1 │ Domain             │ Freshservice instance URL     │
│ 2 │ Authentication     │ Login with session cookies    │
│ 3 │ Shell completion   │ Install tab completion        │
│ 4 │ Network completion │ Remote requester/agent lookup │
│ 5 │ Default filters    │ Default --where per resource  │
│ 6 │ All                │ Run full setup                │
└───┴────────────────────┴───────────────────────────────┘
Choose [6]: 
── Domain ──
Domain [acme.freshservice.com]: domain set to globex.freshservice.com

✓ setup complete
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn setup_reprompts_choice() {
    let r = run_with(
        "local",
        &[],
        &["setup"],
        Some("9\nx\n1\n\n"),
        Opts {
            tty: true,
            env: &[],
        },
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                Current status                
┌────────────────────┬───────────────────────┐
│ domain             │ acme.freshservice.com │
│ auth               │ ✓ file, 0.0h ago      │
│ completion         │ ✗ not installed       │
│ network completion │ (unset)               │
│ defaults.changes   │ (none)                │
│ defaults.tickets   │ (none)                │
│ defaults.problems  │ (none)                │
└────────────────────┴───────────────────────┘

┌───┬────────────────────┬───────────────────────────────┐
│ 1 │ Domain             │ Freshservice instance URL     │
│ 2 │ Authentication     │ Login with session cookies    │
│ 3 │ Shell completion   │ Install tab completion        │
│ 4 │ Network completion │ Remote requester/agent lookup │
│ 5 │ Default filters    │ Default --where per resource  │
│ 6 │ All                │ Run full setup                │
└───┴────────────────────┴───────────────────────────────┘
Choose [6]: Choose [6]: Choose [6]: 
── Domain ──
Domain [acme.freshservice.com]: domain set to acme.freshservice.com

✓ setup complete
"#
    );
    assert_eq!(
        r.err,
        r#"error: choose 1-6
error: choose 1-6
"#
    );
}

#[test]
fn setup_network_completion_off() {
    let r = run_with(
        "local",
        &[],
        &["setup"],
        Some("4\noff\n"),
        Opts {
            tty: true,
            env: &[],
        },
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                Current status                
┌────────────────────┬───────────────────────┐
│ domain             │ acme.freshservice.com │
│ auth               │ ✓ file, 0.0h ago      │
│ completion         │ ✗ not installed       │
│ network completion │ (unset)               │
│ defaults.changes   │ (none)                │
│ defaults.tickets   │ (none)                │
│ defaults.problems  │ (none)                │
└────────────────────┴───────────────────────┘

┌───┬────────────────────┬───────────────────────────────┐
│ 1 │ Domain             │ Freshservice instance URL     │
│ 2 │ Authentication     │ Login with session cookies    │
│ 3 │ Shell completion   │ Install tab completion        │
│ 4 │ Network completion │ Remote requester/agent lookup │
│ 5 │ Default filters    │ Default --where per resource  │
│ 6 │ All                │ Run full setup                │
└───┴────────────────────┴───────────────────────────────┘
Choose [6]: 
── Network completion ──
current: (unset)
when on, TAB fetches live requester/agent names
Enable? [on/off] [on]: completion.network = off

✓ setup complete
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn setup_network_completion_bad() {
    let r = run_with(
        "local",
        &[],
        &["setup"],
        Some("4\nmaybe\n"),
        Opts {
            tty: true,
            env: &[],
        },
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                Current status                
┌────────────────────┬───────────────────────┐
│ domain             │ acme.freshservice.com │
│ auth               │ ✓ file, 0.0h ago      │
│ completion         │ ✗ not installed       │
│ network completion │ (unset)               │
│ defaults.changes   │ (none)                │
│ defaults.tickets   │ (none)                │
│ defaults.problems  │ (none)                │
└────────────────────┴───────────────────────┘

┌───┬────────────────────┬───────────────────────────────┐
│ 1 │ Domain             │ Freshservice instance URL     │
│ 2 │ Authentication     │ Login with session cookies    │
│ 3 │ Shell completion   │ Install tab completion        │
│ 4 │ Network completion │ Remote requester/agent lookup │
│ 5 │ Default filters    │ Default --where per resource  │
│ 6 │ All                │ Run full setup                │
└───┴────────────────────┴───────────────────────────────┘
Choose [6]: 
── Network completion ──
current: (unset)
when on, TAB fetches live requester/agent names
Enable? [on/off] [on]: 
✓ setup complete
"#
    );
    assert_eq!(
        r.err,
        r#"expected on/off; skipping
"#
    );
}

#[test]
fn setup_defaults_step() {
    let r = run_with(
        "local",
        &[],
        &["setup"],
        Some("5\nstatus=Open, priority=High\n\n agent=me \n"),
        Opts {
            tty: true,
            env: &[],
        },
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                Current status                
┌────────────────────┬───────────────────────┐
│ domain             │ acme.freshservice.com │
│ auth               │ ✓ file, 0.0h ago      │
│ completion         │ ✗ not installed       │
│ network completion │ (unset)               │
│ defaults.changes   │ (none)                │
│ defaults.tickets   │ (none)                │
│ defaults.problems  │ (none)                │
└────────────────────┴───────────────────────┘

┌───┬────────────────────┬───────────────────────────────┐
│ 1 │ Domain             │ Freshservice instance URL     │
│ 2 │ Authentication     │ Login with session cookies    │
│ 3 │ Shell completion   │ Install tab completion        │
│ 4 │ Network completion │ Remote requester/agent lookup │
│ 5 │ Default filters    │ Default --where per resource  │
│ 6 │ All                │ Run full setup                │
└───┴────────────────────┴───────────────────────────────┘
Choose [6]: 
── Default filters ──
set default --where filters for every `ls` query
examples: status=Open, agent=alice@example.com, requester=john@example.com

changes --where (comma-sep, enter to skip):   defaults.changes.where = ['status=Open', 'priority=High']

tickets --where (comma-sep, enter to skip): 
problems --where (comma-sep, enter to skip):   defaults.problems.where = ['agent=me']

✓ setup complete
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn setup_shows_existing_defaults() {
    let r = run_with(
        "local",
        &[
            &["config", "defaults", "set", "tickets", "-w", "status=Open"],
            &["config", "set", "completion.network", "on"],
        ],
        &["setup"],
        Some("5\n\n\n\n"),
        Opts {
            tty: true,
            env: &[],
        },
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                Current status                
┌────────────────────┬───────────────────────┐
│ domain             │ acme.freshservice.com │
│ auth               │ ✓ file, 0.0h ago      │
│ completion         │ ✗ not installed       │
│ network completion │ on                    │
│ defaults.changes   │ (none)                │
│ defaults.tickets   │ status=Open           │
│ defaults.problems  │ (none)                │
└────────────────────┴───────────────────────┘

┌───┬────────────────────┬───────────────────────────────┐
│ 1 │ Domain             │ Freshservice instance URL     │
│ 2 │ Authentication     │ Login with session cookies    │
│ 3 │ Shell completion   │ Install tab completion        │
│ 4 │ Network completion │ Remote requester/agent lookup │
│ 5 │ Default filters    │ Default --where per resource  │
│ 6 │ All                │ Run full setup                │
└───┴────────────────────┴───────────────────────────────┘
Choose [6]: 
── Default filters ──
set default --where filters for every `ls` query
examples: status=Open, agent=alice@example.com, requester=john@example.com

changes --where (comma-sep, enter to skip): 
tickets --where (comma-sep, enter to skip) [status=Open]: 
problems --where (comma-sep, enter to skip): 
✓ setup complete
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn setup_completion_no_shell() {
    let r = run_with(
        "local",
        &[],
        &["setup"],
        Some("3\n"),
        Opts {
            tty: true,
            env: &[],
        },
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                Current status                
┌────────────────────┬───────────────────────┐
│ domain             │ acme.freshservice.com │
│ auth               │ ✓ file, 0.0h ago      │
│ completion         │ ✗ not installed       │
│ network completion │ (unset)               │
│ defaults.changes   │ (none)                │
│ defaults.tickets   │ (none)                │
│ defaults.problems  │ (none)                │
└────────────────────┴───────────────────────┘

┌───┬────────────────────┬───────────────────────────────┐
│ 1 │ Domain             │ Freshservice instance URL     │
│ 2 │ Authentication     │ Login with session cookies    │
│ 3 │ Shell completion   │ Install tab completion        │
│ 4 │ Network completion │ Remote requester/agent lookup │
│ 5 │ Default filters    │ Default --where per resource  │
│ 6 │ All                │ Run full setup                │
└───┴────────────────────┴───────────────────────────────┘
Choose [6]: 
── Shell completion ──

✓ setup complete
"#
    );
    assert_eq!(
        r.err,
        r#"could not detect shell; skipping
"#
    );
}

#[test]
fn setup_completion_declined() {
    let r = run_with(
        "local",
        &[],
        &["setup"],
        Some("3\nn\n"),
        Opts {
            tty: true,
            env: &[("SHELL", "/bin/zsh")],
        },
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                Current status                
┌────────────────────┬───────────────────────┐
│ domain             │ acme.freshservice.com │
│ auth               │ ✓ file, 0.0h ago      │
│ completion         │ ✗ not installed       │
│ network completion │ (unset)               │
│ defaults.changes   │ (none)                │
│ defaults.tickets   │ (none)                │
│ defaults.problems  │ (none)                │
└────────────────────┴───────────────────────┘

┌───┬────────────────────┬───────────────────────────────┐
│ 1 │ Domain             │ Freshservice instance URL     │
│ 2 │ Authentication     │ Login with session cookies    │
│ 3 │ Shell completion   │ Install tab completion        │
│ 4 │ Network completion │ Remote requester/agent lookup │
│ 5 │ Default filters    │ Default --where per resource  │
│ 6 │ All                │ Run full setup                │
└───┴────────────────────┴───────────────────────────────┘
Choose [6]: 
── Shell completion ──
Install zsh completion? [Y/n]: 
✓ setup complete
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn completion_show_unknown_env_shell() {
    let r = run_with(
        "local",
        &[],
        &["completion", "show"],
        None,
        Opts {
            tty: false,
            env: &[("SHELL", "/bin/tcsh")],
        },
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: could not detect shell; pass one of: bash, zsh, fish, powershell, pwsh
"#
    );
}

#[test]
fn completion_doctor_reports_shell() {
    let r = run_with(
        "local",
        &[&["config", "set", "completion.network", "on"]],
        &["completion", "doctor"],
        None,
        Opts {
            tty: false,
            env: &[("SHELL", "/bin/zsh")],
        },
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"      completion doctor       
┏━━━━━━━━━━━━━━━━━┳━━━━━━━━━━┓
┃ check           ┃ value    ┃
┡━━━━━━━━━━━━━━━━━╇━━━━━━━━━━┩
│ shell           │ /bin/zsh │
│ fsv path        │ fsv      │
│ network         │ on       │
│ schema changes  │ missing  │
│ schema tickets  │ missing  │
│ schema problems │ missing  │
└─────────────────┴──────────┘
Use TAB after a prefix, e.g. `fsv changes ls --where requester=phu<TAB>`.
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn completion_show_sh_is_not_detected() {
    let r = run_with(
        "local",
        &[],
        &["completion", "show"],
        None,
        Opts {
            tty: false,
            env: &[("SHELL", "/bin/sh")],
        },
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: could not detect shell; pass one of: bash, zsh, fish, powershell, pwsh
"#
    );
}

#[test]
fn setup_completion_sh_counts_as_bash() {
    let r = run_with(
        "local",
        &[],
        &["setup"],
        Some("3\nn\n"),
        Opts {
            tty: true,
            env: &[("SHELL", "/bin/sh")],
        },
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                Current status                
┌────────────────────┬───────────────────────┐
│ domain             │ acme.freshservice.com │
│ auth               │ ✓ file, 0.0h ago      │
│ completion         │ ✗ not installed       │
│ network completion │ (unset)               │
│ defaults.changes   │ (none)                │
│ defaults.tickets   │ (none)                │
│ defaults.problems  │ (none)                │
└────────────────────┴───────────────────────┘

┌───┬────────────────────┬───────────────────────────────┐
│ 1 │ Domain             │ Freshservice instance URL     │
│ 2 │ Authentication     │ Login with session cookies    │
│ 3 │ Shell completion   │ Install tab completion        │
│ 4 │ Network completion │ Remote requester/agent lookup │
│ 5 │ Default filters    │ Default --where per resource  │
│ 6 │ All                │ Run full setup                │
└───┴────────────────────┴───────────────────────────────┘
Choose [6]: 
── Shell completion ──
Install bash completion? [Y/n]: 
✓ setup complete
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn setup_completion_casefolds_shell() {
    let r = run_with(
        "local",
        &[],
        &["setup"],
        Some("3\nn\n"),
        Opts {
            tty: true,
            env: &[("SHELL", "/opt/FISH")],
        },
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                Current status                
┌────────────────────┬───────────────────────┐
│ domain             │ acme.freshservice.com │
│ auth               │ ✓ file, 0.0h ago      │
│ completion         │ ✗ not installed       │
│ network completion │ (unset)               │
│ defaults.changes   │ (none)                │
│ defaults.tickets   │ (none)                │
│ defaults.problems  │ (none)                │
└────────────────────┴───────────────────────┘

┌───┬────────────────────┬───────────────────────────────┐
│ 1 │ Domain             │ Freshservice instance URL     │
│ 2 │ Authentication     │ Login with session cookies    │
│ 3 │ Shell completion   │ Install tab completion        │
│ 4 │ Network completion │ Remote requester/agent lookup │
│ 5 │ Default filters    │ Default --where per resource  │
│ 6 │ All                │ Run full setup                │
└───┴────────────────────┴───────────────────────────────┘
Choose [6]: 
── Shell completion ──
Install fish completion? [Y/n]: 
✓ setup complete
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn auth_login_store_reprompts() {
    let r = run_with(
        "auth",
        &[],
        &[
            "auth",
            "login",
            "--header",
            "_x_m=1; _x_d=2; _x_w=3; fw-session-id=4",
            "--force",
        ],
        Some("\n9\nfile\n"),
        Opts {
            tty: true,
            env: &[],
        },
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"Domain [acme.freshservice.com]:                           Store session                           
┏━━━┳━━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ # ┃ Backend        ┃ Notes                                     ┃
┡━━━╇━━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┩
│ 1 │ keychain       │ macOS Keychain; recommended               │
│ 2 │ argon          │ encrypted file; asks passphrase when used │
│ 3 │ file (default) │ plain JSON file; chmod 600                │
└───┴────────────────┴───────────────────────────────────────────┘
Choose store [3]: Choose store [3]: saved 4 cookies to ~/config/session.json
logged in as: Me Agent <me@acme.test>
"#
    );
    assert_eq!(
        r.err,
        r#"error: choose 1-3 or one of: keychain, argon, file
warming cache in background...
"#
    );
}

fn env_with(vars: &[(&str, &str)]) -> Env {
    let mut env = Env::new(fixture("local"));
    let work = env.home().join("work");
    std::fs::create_dir_all(&work).unwrap();
    env.cwd = Some(work);
    env.stdin = Some(String::new());
    env.vars = vars
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    env
}

/// The script the binary should print; `tests/completion.rs` checks the
/// generator itself against Python.
fn script(shell: &str) -> String {
    let tree = fsv_cli::completion_tree("fsv");
    fsv_completion::build_script(shell, &tree, "fsv", env!("CARGO_BIN_EXE_fsv")).unwrap()
}

// Python prints the same lines with its own temp HOME.
#[test]
fn completion_install_bash_writes_script_and_rc() {
    let env = env_with(&[]);
    let h = env.home().display().to_string();
    let r = env.run(&["completion", "install", "bash"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        format!(
            "bash completion installed in {h}/.fsv-complete.bash\nsourced from {h}/.bashrc\nrestart shell: exec $SHELL\n"
        )
    );
    assert_eq!(
        std::fs::read_to_string(env.home().join(".fsv-complete.bash")).unwrap(),
        script("bash")
    );
    let rc = format!("\n. {h}/.fsv-complete.bash\n");
    assert_eq!(
        std::fs::read_to_string(env.home().join(".bashrc")).unwrap(),
        rc
    );
    env.run(&["completion", "install", "bash"]);
    assert_eq!(
        std::fs::read_to_string(env.home().join(".bashrc")).unwrap(),
        rc
    );
}

#[test]
fn completion_install_uses_shell_env() {
    let env = env_with(&[("SHELL", "/usr/local/bin/zsh")]);
    let h = env.home().display().to_string();
    let r = env.run(&["completion", "install"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        format!(
            "zsh completion installed in {h}/.fsv-complete.zsh\nsourced from {h}/.zshrc\nrestart shell: exec $SHELL\n"
        )
    );
    assert_eq!(
        std::fs::read_to_string(env.home().join(".fsv-complete.zsh")).unwrap(),
        script("zsh")
    );
}

#[test]
fn completion_install_fish() {
    let env = env_with(&[]);
    let h = env.home().display().to_string();
    let r = env.run(&["completion", "install", "fish"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        format!(
            "fish completion installed in {h}/.config/fish/completions/fsv.fish\nrestart shell: exec $SHELL\n"
        )
    );
    let path = env.home().join(".config/fish/completions/fsv.fish");
    assert_eq!(std::fs::read_to_string(path).unwrap(), script("fish"));
}

#[test]
fn completion_show_uses_shell_env() {
    let env = env_with(&[("SHELL", "/usr/local/bin/fish")]);
    let r = env.run(&["completion", "show"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(r.out, script("fish"));
    let r = env.run(&["completion", "show", "bash"]);
    assert_eq!(r.out, script("bash"));
}

#[test]
fn show_completion_flag_maps_sh_to_bash() {
    let env = env_with(&[("SHELL", "/bin/sh")]);
    let r = env.run(&["--show-completion"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(r.out, script("bash"));
}

// Divergence: Typer prints "Shell  not supported." / "Shell None is not
// supported."; Rust names the problem. Exit code 1 is the same.
#[test]
fn completion_flags_without_a_known_shell() {
    let env = env_with(&[("SHELL", "/bin/tcsh")]);
    let r = env.run(&["--show-completion"]);
    assert_eq!((r.code, r.out.as_str()), (1, ""));
    assert_eq!(r.err, "error: could not detect shell\n");
    let r = env.run(&["--install-completion"]);
    assert_eq!((r.code, r.out.as_str()), (1, ""));
    assert_eq!(r.err, "error: could not detect shell\n");
}

// Python prints the same lines with its own temp HOME.
#[test]
fn setup_completion_installs_for_env_shell() {
    let mut env = env_with(&[("SHELL", "/bin/zsh"), ("FSV_TEST_TTY", "1")]);
    env.stdin = Some("3\ny\n".into());
    let h = env.home().display().to_string();
    let r = env.run(&["setup"]);
    assert_eq!(r.code, 0, "{}", r.err);
    let tail = format!(
        "Choose [6]: \n── Shell completion ──\nInstall zsh completion? [Y/n]: installed: {h}/.fsv-complete.zsh\nsourced from: {h}/.zshrc\nrestart shell: exec $SHELL\n\n✓ setup complete\n"
    );
    assert!(r.out.ends_with(&tail), "{}", r.out);
    assert_eq!(
        std::fs::read_to_string(env.home().join(".fsv-complete.zsh")).unwrap(),
        script("zsh")
    );
    env.stdin = Some("3\nn\n".into());
    let r = env.run(&["setup"]);
    assert!(r.out.contains("│ completion         │ ✓ zsh "), "{}", r.out);
    assert!(
        r.out.contains(&format!(
            "already installed: {h}/.fsv-complete.zsh\nReinstall? [y/N]: \n✓ setup complete"
        )),
        "{}",
        r.out
    );
}

// The journal directory hashes the resolved config dir, so the middle
// segment depends on the temp HOME; the last one is sha256("acme.freshservice.com").
#[test]
fn journal_path_layout() {
    let env = env_with(&[]);
    let r = env.run(&["journal", "path"]);
    assert_eq!(r.code, 0, "{}", r.err);
    let prefix = format!("{}/state/fsv/journals/", env.home().display());
    let rest = r
        .out
        .strip_prefix(&prefix)
        .unwrap_or_else(|| panic!("{}", r.out));
    let (config_hash, tail) = rest.split_once('/').unwrap();
    assert_eq!(config_hash.len(), 64);
    assert!(config_hash.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(
        tail,
        "d0ba64d859ebafab6b3b77efc7d19e8e5784676cd027af2c9819e609dc3c5bd4/journal.jsonl\n"
    );
}

// Rows come in thread-completion order (Python and Rust both), so compare them as a set.
#[test]
fn cache_refresh_rows() {
    for cmd in [["cache", "refresh"], ["completion", "refresh"]] {
        let r = run_fixture("auth", &cmd, None);
        assert_eq!(r.code, 0, "{}", r.err);
        assert_eq!(r.err, "");
        let mut lines: Vec<&str> = r.out.lines().collect();
        assert_eq!(lines[0], "      cache refresh      ");
        let mut rows: Vec<&str> = lines.drain(4..7).collect();
        rows.sort_unstable();
        assert_eq!(
            rows,
            [
                "│ groups        │     1 │",
                "│ tickets       │     1 │",
                "│ tickets views │     1 │"
            ]
        );
    }
}

// Divergence: Python's Textual app waits headless for a terminal forever.
#[test]
fn tui_without_terminal_exits_1() {
    let r = run_fixture("local", &["tui"], None);
    assert_eq!(r.code, 1, "{}", r.err);
    assert!(r.err.starts_with("error: "), "{}", r.err);
}

#[test]
fn search_csv_no_results_prints_header() {
    let r = run_fixture("auth", &["search", "zzz", "-o", "csv"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(r.out, "\r\n");
    assert_eq!(r.err, r#""#);
}

#[test]
fn search_tsv_no_results() {
    let r = run_fixture("auth", &["search", "zzz", "-o", "tsv"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(r.out, "\r\n");
    assert_eq!(r.err, r#""#);
}

#[test]
fn search_table_no_results() {
    let r = run_fixture("auth", &["search", "zzz"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"┏━━━━━━┳━━━━┳━━━━━━━┳━━━━━━━━┳━━━━━━━┓
┃ Type ┃ ID ┃ Title ┃ Status ┃ Group ┃
┡━━━━━━╇━━━━╇━━━━━━━╇━━━━━━━━╇━━━━━━━┩
└──────┴────┴───────┴────────┴───────┘
"#
    );
    assert_eq!(
        r.err,
        r#"0 rows
"#
    );
}

#[test]
fn search_npages() {
    let r = run_fixture(
        "auth",
        &["search", "printer", "-N", "1", "--sort", "created"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"┏━━━━━━━━┳━━━━━━━┳━━━━━━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━━━━━┓
┃ Type   ┃ ID    ┃ Title         ┃ Status   ┃ Group        ┃
┡━━━━━━━━╇━━━━━━━╇━━━━━━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━━━━━┩
│ ticket │ INC-5 │ Printer jam   │ Open     │ Service Desk │
│        │       │  on 3rd floor │          │              │
│ change │ CHN-9 │ Patch DB      │ Planning │ -            │
│ ci     │ AST-3 │ db-01         │ Server   │ Ops          │
└────────┴───────┴───────────────┴──────────┴──────────────┘
"#
    );
    assert_eq!(
        r.err,
        r#"  fetched 3...
3 rows
"#
    );
}

#[test]
fn cache_status_after_refresh() {
    let r = run_steps("auth", &[&["cache", "refresh"]], &["cache", "status"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"               cache status               
┏━━━━━━━━━┳━━━━━━━━┳━━━━━━┳━━━━━━━┳━━━━━━┓
┃ cache   ┃ kind   ┃ age  ┃ state ┃ ttl  ┃
┡━━━━━━━━━╇━━━━━━━━╇━━━━━━╇━━━━━━━╇━━━━━━┩
│ tickets │ schema │ 0.0h │ ok    │ 168h │
│ tickets │ views  │ 0.0h │ ok    │ 15m  │
│ groups  │ groups │ 0.0h │ ok    │ 6h   │
└─────────┴────────┴──────┴───────┴──────┘
"#
    );
    assert_eq!(r.err, r#""#);
}
