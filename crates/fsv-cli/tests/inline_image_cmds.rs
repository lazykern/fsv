//! `tickets add-note/reply --inline-image` (Python 520bb68
//! `test_cli_note_and_reply_review_then_send`).

mod common;
use common::{Env, get, route};
use serde_json::json;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nsynthetic-image-bytes";
const LINK: &str = "https://files.example.com/inline/attachment?token=t7002&v=1";

fn env() -> Env {
    let stored = format!(
        r#"<div>Acme evidence</div><div><img src="{}" style="width: auto;" class="fr-fil fr-dib" data-id="7002"></div>"#,
        LINK.replace('&', "&amp;")
    );
    let mut env = Env::new(vec![
        json!({"method": "GET", "path": "/a/tickets", "status": 200,
               "text": "<html><head><meta name=\"csrf-token\" content=\"tok\"></head></html>"}),
        get(
            "ticket_form_fields",
            json!({"attachment_settings": {"max_attachment_size_bytes": 4096}}),
        ),
        get(
            "tickets/123",
            json!({"ticket": {"id": 123, "attachments": []}}),
        ),
        get(
            "tickets/123/conversations",
            json!({"conversations": [{"id": 901, "body": stored, "private": true,
                                      "conversation_from_summary": false, "to_emails": [],
                                      "attachments": []}], "has_more": false}),
        ),
        route(
            "POST",
            "/api/_/solutions/inline_images",
            201,
            json!({"link": LINK, "fileid": 7002}),
        ),
        route(
            "POST",
            "/api/_/tickets/123/notes",
            201,
            json!({"conversation": {"id": 901}}),
        ),
    ]);
    let work = env.home().join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(work.join("acme.png"), PNG).unwrap();
    std::fs::write(work.join("globex.gif"), PNG).unwrap();
    env.cwd = Some(work);
    env
}

#[test]
fn note_and_reply_review_then_send() {
    let env = env();
    let note = [
        "tickets",
        "add-note",
        "INC-123",
        "--body",
        "Acme evidence",
        "--inline-image",
        "acme.png",
    ];
    let r = env.run(&[&note[..], &["--dry-run"]].concat());
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.write_paths().is_empty());
    let review = r.json();
    assert_eq!(review["action"], json!("ticket_note_inline_images"));
    let hash = review["plan_hash"].as_str().unwrap().to_string();
    let r = env.run(&[&note[..], &["--yes", "--expect-plan", &hash]].concat());
    assert_eq!(r.code, 0, "{}\n{}", r.out, r.err);
    let out = r.json();
    assert_eq!(out["verified"], json!(true), "{out}");
    assert_eq!(out["inline_image_ids"], json!([7002]));
    assert_eq!(
        r.write_paths(),
        [
            "POST /api/_/solutions/inline_images",
            "POST /api/_/tickets/123/notes"
        ]
    );
    let sent = &r.writes()[1].2;
    assert!(
        sent["body"]
            .as_str()
            .unwrap()
            .ends_with(r#"fileid="7002" data-id="7002"></div>"#)
    );
    let reply = [
        "tickets",
        "reply",
        "INC-123",
        "Acme evidence",
        "--inline-image",
        "acme.png",
    ];
    let r = env.run(
        &[
            &reply[..],
            &[
                "--attach",
                "globex.gif",
                "--sender",
                "support@example.com",
                "--dry-run",
            ],
        ]
        .concat(),
    );
    assert_eq!(r.code, 2);
    assert_eq!(
        r.err,
        "--inline-image with --attach/--cc/--bcc/--set-status is not a captured combination; nothing was sent\n"
    );
    let r = env.run(
        &[
            &reply[..],
            &["--sender", "support@example.com", "--dry-run"],
        ]
        .concat(),
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(r.json()["inline_images"][0]["name"], json!("acme.png"));
    assert!(r.write_paths().is_empty());
}

#[test]
fn note_refusals() {
    let env = env();
    let base = [
        "tickets",
        "add-note",
        "INC-123",
        "--body",
        "Acme evidence",
        "--inline-image",
        "acme.png",
    ];
    let r = env.run(&[&base[..], &["--set-status", "Pending", "--dry-run"]].concat());
    assert_eq!(r.code, 2);
    assert_eq!(
        r.err,
        "--inline-image with --attach/--set-status is not a captured combination; nothing was sent\n"
    );
    let r = env.run(&[&base[..], &["--read-only", "--yes"]].concat());
    assert_eq!(
        (r.code, r.err.as_str()),
        (2, "read-only mode: nothing was sent\n")
    );
    let r = env.run(&[&base[..], &["--public", "--yes"]].concat());
    assert_eq!(
        (r.code, r.err.as_str()),
        (2, "public note requires --confirm-public\n")
    );
    let r = env.run(&[
        "tickets",
        "add-note",
        "INC-123",
        "--body",
        "x",
        "--inline-image",
        "globex.gif",
        "--yes",
        "--expect-plan",
        "abc",
    ]);
    assert_eq!(
        (r.code, r.err.as_str()),
        (
            2,
            "review with --dry-run, then --yes --expect-plan FULL_HASH\n"
        )
    );
    assert!(r.write_paths().is_empty());
}
