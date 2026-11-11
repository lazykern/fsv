//! Sending a reviewed change: form metadata that changed since the review
//! (`change_cli._send`'s re-confirm) and a write that fails after the change
//! exists (`_report_partial`, exit 5). Expected output was captured from the
//! Python fsv on the same routes (`tests/golden/cmp.py --emit`); the first
//! `change_form_fields` route answers once, so the send-time reload differs.

mod common;
use common::{Opts, run_fixture, run_with};

/// The review's `fetched <now>` stamps.
fn mask_fetched(text: &str) -> String {
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"fetched \d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\+00:00").unwrap()
    });
    RE.replace_all(text, "fetched NOW").into_owned()
}

#[test]
fn change_create_metadata_changed_yes_exits_4() {
    let r = run_fixture(
        "change_resend",
        &[
            "changes",
            "create",
            "--set",
            "Subject=Hello",
            "--set",
            "Priority=Low",
            "--requester",
            "ann@acme.com",
            "--no-validate",
            "--yes",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"form metadata changed since the review; the plan is now different:
  payload priority changed
error: plan changed to 17d0e4495751b6753ecd06c7691ad2c1ba94912caaebf49ac680003ba018c698; review it again with --dry-run
"#
    );
}

#[test]
fn change_create_metadata_changed_declined() {
    let r = run_with(
        "change_resend",
        &[],
        &[
            "changes",
            "create",
            "--set",
            "Subject=Hello",
            "--set",
            "Priority=Low",
            "--requester",
            "ann@acme.com",
            "--no-validate",
        ],
        Some("y\nn\n"),
        Opts {
            tty: true,
            env: &[],
        },
    );
    assert_eq!(r.code, 1, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[y] create  [e] edit  [s] save draft  [n] cancel: Send the updated plan? [y/N]: "#
    );
    assert_eq!(
        mask_fetched(&r.err),
        r#"──────── Review: New Change: Hello ────────
Requester         Ann Lee <ann@acme.com>
Change Type       Normal                
Subject           Hello                 
Impact            Low                   
Priority          Low                   
Risk              Low                   
Status on create  Open (form default)   
Will notify / create work for
  Requester: Ann Lee <ann@acme.com> (requester notifications)
Rule notes
  conflict: cloud_only.required: unmandate by 'Not labs: optional extras' (id 1); mandate by 'Mandatory unless labs (new only)' (id 15) -> mandate wins ('Mandatory unless labs (new only)')
  conflict: stage.required: unmandate by 'Not labs: optional extras' (id 1); mandate by 'Mandatory unless labs (new only)' (id 15) -> mandate wins ('Mandatory unless labs (new only)')
Warnings
  --no-validate: sending without the client-side required check: Team is required; Group is required; Agent is required; Services is required; Cloud Only is required; Stage is required; Description is
required; Planned Start Date is required; Planned End Date is required; Downtime is required; Sign-off Date is required
form metadata as of unknown (fetched NOW, server, fingerprint 9f57a1ae09548281)
plan 10df27a1e78a8382e879aa47ee0592d8e855fe2ac2cfb41591e477cb13263692
form metadata changed since the review; the plan is now different:
  payload priority changed
──────── Review: New Change: Hello ────────
Requester         Ann Lee <ann@acme.com>
Change Type       Normal                
Subject           Hello                 
Impact            Low                   
Priority          Low                   
Risk              Low                   
Status on create  Open (form default)   
Will notify / create work for
  Requester: Ann Lee <ann@acme.com> (requester notifications)
Messages
  [BLOCKING (needs --ack or y)] Urgent changes need the duty manager's approval
Rule notes
  conflict: cloud_only.required: unmandate by 'Not labs: optional extras' (id 1); mandate by 'Mandatory unless labs (new only)' (id 15) -> mandate wins ('Mandatory unless labs (new only)')
  conflict: stage.required: unmandate by 'Not labs: optional extras' (id 1); mandate by 'Mandatory unless labs (new only)' (id 15) -> mandate wins ('Mandatory unless labs (new only)')
Warnings
  --no-validate: sending without the client-side required check: Team is required; Group is required; Agent is required; Services is required; Cloud Only is required; Stage is required; Description is
required; Planned Start Date is required; Planned End Date is required; Downtime is required; Sign-off Date is required; blocked by rule (acknowledge to continue): Urgent changes need the duty 
manager's approval
form metadata as of unknown (fetched NOW, server, fingerprint bde07ae3a7a94782)
plan 17d0e4495751b6753ecd06c7691ad2c1ba94912caaebf49ac680003ba018c698
error: cancelled; nothing was sent
"#
    );
}

#[test]
fn change_create_metadata_changed_confirmed() {
    let r = run_with(
        "change_resend",
        &[],
        &[
            "changes",
            "create",
            "--set",
            "Subject=Hello",
            "--set",
            "Priority=Low",
            "--requester",
            "ann@acme.com",
            "--no-validate",
        ],
        Some("y\ny\n"),
        Opts {
            tty: true,
            env: &[],
        },
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[y] create  [e] edit  [s] save draft  [n] cancel: Send the updated plan? [y/N]: created CHN-900  https://acme.freshservice.com/a/changes/900
"#
    );
    assert_eq!(
        mask_fetched(&r.err),
        r#"──────── Review: New Change: Hello ────────
Requester         Ann Lee <ann@acme.com>
Change Type       Normal                
Subject           Hello                 
Impact            Low                   
Priority          Low                   
Risk              Low                   
Status on create  Open (form default)   
Will notify / create work for
  Requester: Ann Lee <ann@acme.com> (requester notifications)
Rule notes
  conflict: cloud_only.required: unmandate by 'Not labs: optional extras' (id 1); mandate by 'Mandatory unless labs (new only)' (id 15) -> mandate wins ('Mandatory unless labs (new only)')
  conflict: stage.required: unmandate by 'Not labs: optional extras' (id 1); mandate by 'Mandatory unless labs (new only)' (id 15) -> mandate wins ('Mandatory unless labs (new only)')
Warnings
  --no-validate: sending without the client-side required check: Team is required; Group is required; Agent is required; Services is required; Cloud Only is required; Stage is required; Description is
required; Planned Start Date is required; Planned End Date is required; Downtime is required; Sign-off Date is required
form metadata as of unknown (fetched NOW, server, fingerprint 9f57a1ae09548281)
plan 10df27a1e78a8382e879aa47ee0592d8e855fe2ac2cfb41591e477cb13263692
form metadata changed since the review; the plan is now different:
  payload priority changed
──────── Review: New Change: Hello ────────
Requester         Ann Lee <ann@acme.com>
Change Type       Normal                
Subject           Hello                 
Impact            Low                   
Priority          Low                   
Risk              Low                   
Status on create  Open (form default)   
Will notify / create work for
  Requester: Ann Lee <ann@acme.com> (requester notifications)
Messages
  [BLOCKING (needs --ack or y)] Urgent changes need the duty manager's approval
Rule notes
  conflict: cloud_only.required: unmandate by 'Not labs: optional extras' (id 1); mandate by 'Mandatory unless labs (new only)' (id 15) -> mandate wins ('Mandatory unless labs (new only)')
  conflict: stage.required: unmandate by 'Not labs: optional extras' (id 1); mandate by 'Mandatory unless labs (new only)' (id 15) -> mandate wins ('Mandatory unless labs (new only)')
Warnings
  --no-validate: sending without the client-side required check: Team is required; Group is required; Agent is required; Services is required; Cloud Only is required; Stage is required; Description is
required; Planned Start Date is required; Planned End Date is required; Downtime is required; Sign-off Date is required; blocked by rule (acknowledge to continue): Urgent changes need the duty 
manager's approval
form metadata as of unknown (fetched NOW, server, fingerprint bde07ae3a7a94782)
plan 17d0e4495751b6753ecd06c7691ad2c1ba94912caaebf49ac680003ba018c698
"#
    );
    assert_eq!(r.write_paths(), ["POST /api/_/changes"]);
}

#[test]
fn change_create_partial_failure() {
    let r = run_fixture(
        "change_partial",
        &[
            "changes",
            "create",
            "--set",
            "Subject=Hello",
            "--requester",
            "ann@acme.com",
            "--no-validate",
            "--attach",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/files/sr/notes.txt"
            ),
            "--yes",
            "--expect-plan",
            "2ecbfa2b72bbbb86",
        ],
        None,
    );
    assert_eq!(r.code, 5, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"  uploading notes.txt... ok (5 B)
error: change 900 was written, but a later step failed: update failed: {'errors': 'boom'}
uploaded attachment ids: 77 (notes.txt)
  {'step': 'create', 'ok': True, 'id': 900}
  {'step': 'after', 'ok': False, 'error': "update failed: {'errors': 'boom'}"}
error: partial failure; change id 900
"#
    );
    assert_eq!(
        r.write_paths(),
        [
            "POST /api/_/changes",
            "POST /api/_/attachments",
            "PUT /api/_/changes/900"
        ]
    );
}
