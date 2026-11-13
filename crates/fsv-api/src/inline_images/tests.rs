//! `tests/test_inline_images.py` (Python 520bb68): uploads embedded in ticket
//! replies and notes over a real `fsv_core::Client` and a mock transport.

use std::sync::{Arc, Mutex};

use fsv_core::client::mock::MockTransport;
use fsv_core::client::{Request, Response, TransportError};
use fsv_core::config::Paths;
use fsv_core::{Client, Ctx};

use super::*;
use crate::formmeta::MemStore;
use crate::service::testing::MemJournal;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nsynthetic-image-bytes";
const CSRF_PAGE: &str = r#"<html><head><meta name="csrf-token" content="synthetic"></head></html>"#;
const TENANTS: [(&str, i64); 2] = [("acme.example.com", 7001), ("globex.example.com", 8001)];

#[derive(Default)]
struct State {
    writes: Vec<(String, String, String, Vec<u8>)>,
    conversations: Vec<Value>,
    fail: Option<&'static str>,
    strip: bool,
    next: i64,
}

fn path_of(url: &str) -> String {
    let rest = url.splitn(4, '/').nth(3).unwrap_or("");
    format!("/{}", rest.split('?').next().unwrap_or(""))
}

fn handle(s: &mut State, req: &Request) -> std::result::Result<Response, TransportError> {
    let full = path_of(&req.url);
    if req.method == "GET" && full.starts_with("/a/") {
        return Ok(Response::html(200, CSRF_PAGE));
    }
    let path = full.strip_prefix("/api/_/").unwrap_or(&full).to_string();
    if req.method == "GET" {
        return Ok(match path.as_str() {
            "ticket_form_fields" => Response::json(
                200,
                &json!({"attachment_settings": {"max_attachment_size_bytes": 4096}}),
            ),
            "tickets/123" => {
                Response::json(200, &json!({"ticket": {"id": 123, "attachments": []}}))
            }
            "tickets/123/conversations" => Response::json(
                200,
                &json!({"conversations": s.conversations, "has_more": false}),
            ),
            other => panic!("unexpected GET {other}"),
        });
    }
    let ctype = req.header("content-type").unwrap_or_default().to_string();
    let content = req.body.clone().unwrap_or_default();
    s.writes.push((
        req.method.clone(),
        path.clone(),
        ctype.clone(),
        content.clone(),
    ));
    if path == UPLOAD_PATH {
        assert!(ctype.starts_with("multipart/form-data; boundary="));
        assert_eq!(req.header("accept"), Some("application/json"));
        assert_eq!(req.header("x-csrf-token"), Some("synthetic"));
        let text = String::from_utf8_lossy(&content);
        assert!(text.contains(r#"name="file"; filename=""#));
        assert!(content.windows(PNG.len()).any(|w| w == PNG));
        if s.fail == Some("upload") {
            return Ok(Response::json(500, &json!({"error": "failed"})));
        }
        s.next += 1;
        return Ok(Response::json(
            201,
            &json!({"link": format!("https://files.example.com/inline/attachment?token=t{}&v=1", s.next),
                    "fileid": s.next}),
        ));
    }
    let body: Value = serde_json::from_slice(&content).unwrap();
    assert!(
        path == "tickets/123/notes" || path == "tickets/123/reply",
        "{path}"
    );
    if s.fail == Some("owner") {
        return Err(TransportError::new("ReadError", "disconnect"));
    }
    let re = regex::Regex::new(r#" fileid="[0-9]+""#).unwrap();
    let mut stored = re
        .replace_all(body["body"].as_str().unwrap(), "")
        .into_owned();
    if s.strip {
        stored = "<div>Acme evidence</div>".into();
    }
    let mut row = obj(json!({"id": 901}));
    row.extend(body.as_object().unwrap().clone());
    row.insert("body".into(), json!(stored));
    row.insert("attachments".into(), json!([]));
    if let Some(v) = row.shift_remove("notify_emails") {
        row.insert("to_emails".into(), v);
    }
    s.conversations.push(Value::Object(row));
    Ok(Response::json(201, &json!({"conversation": {"id": 901}})))
}

struct Fx {
    c: Client,
    s: Arc<Mutex<State>>,
    paths: Vec<String>,
    base: i64,
    store: MemStore,
    journal: MemJournal,
    dir: tempfile::TempDir,
}

fn fixture((domain, base): (&str, i64)) -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let s = Arc::new(Mutex::new(State {
        next: base,
        ..State::default()
    }));
    let mut paths = Vec::new();
    for name in ["acme.png", "globex.gif"] {
        let p = dir.path().join(name);
        std::fs::write(&p, PNG).unwrap();
        paths.push(p.to_string_lossy().into_owned());
    }
    let ctx = Arc::new(Ctx::new(Paths::under(dir.path())).with_tenant(domain));
    let state = s.clone();
    let c = Client::builder(ctx)
        .transport(MockTransport::new(move |r| {
            handle(&mut state.lock().unwrap(), r)
        }))
        .cookies(Default::default())
        .sleeper(|_| {})
        .journal(|_, _, _, _| {})
        .build()
        .unwrap();
    Fx {
        c,
        s,
        paths,
        base,
        store: MemStore::default(),
        journal: MemJournal::default(),
        dir,
    }
}

fn build(fx: &Fx, paths: &[String], kind: &str, max_bytes: Option<i64>) -> Result<InlinePlan> {
    let args = ConversationAttach {
        kind,
        sender: (kind == "reply").then_some("support@example.com"),
        max_bytes,
        ..Default::default()
    };
    plan(&fx.c, &fx.store, "INC-123", "Acme evidence", paths, &args)
}

fn writes(fx: &Fx) -> Vec<(String, String, String, Vec<u8>)> {
    fx.s.lock().unwrap().writes.clone()
}

fn message(e: Error) -> String {
    e.to_string()
}

#[test]
fn upload_embed_and_verify_link_in_body() {
    for tenant in TENANTS {
        for kind in ["note", "reply"] {
            let fx = fixture(tenant);
            let b = || build(&fx, &fx.paths, kind, None);
            let p = b().unwrap();
            let review = p.review();
            let names: Vec<&Value> = review["inline_images"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| &i["name"])
                .collect();
            assert_eq!(names, [&json!("acme.png"), &json!("globex.gif")]);
            assert_eq!(review["inline_images"][0]["size"], json!(PNG.len()));
            assert!(
                review["warning"]
                    .as_str()
                    .unwrap()
                    .contains("no delete route")
            );
            assert!(writes(&fx).is_empty());
            let r = send(&p, &fx.c, &fx.journal, Some(&p.hash()), &b).unwrap();
            assert_eq!(r["verified"], json!(true), "{r}");
            assert_eq!(r["inline_image_ids"], json!([fx.base + 1, fx.base + 2]));
            let paths: Vec<String> = writes(&fx).into_iter().map(|w| w.1).collect();
            let owner = if kind == "note" {
                "tickets/123/notes"
            } else {
                "tickets/123/reply"
            };
            assert_eq!(paths, [UPLOAD_PATH, UPLOAD_PATH, owner]);
            let last = writes(&fx).pop().unwrap();
            assert_eq!(last.2, "application/json");
            let sent: Value = serde_json::from_slice(&last.3).unwrap();
            assert!(sent.get("attachments").is_none());
            let body = sent["body"].as_str().unwrap();
            assert!(body.contains(&format!("data-id=\"{}\"", fx.base + 1)));
            assert!(body.contains(r#"class="fr-fil fr-dib""#));
            assert!(body.starts_with(p.attached.base.payload()["body"].as_str().unwrap()));
            if kind == "reply" {
                assert_eq!(sent["full_text"], sent["body"]);
            }
            let again = send(&p, &fx.c, &fx.journal, Some(&p.hash()), &b).unwrap_err();
            assert!(message(again).contains("do not replay"));
            assert_eq!(writes(&fx).len(), 3);
        }
    }
}

#[test]
fn readback_without_image_is_not_verified() {
    for tenant in TENANTS {
        let fx = fixture(tenant);
        fx.s.lock().unwrap().strip = true;
        let b = || build(&fx, &fx.paths, "note", None);
        let p = b().unwrap();
        let r = send(&p, &fx.c, &fx.journal, Some(&p.hash()), &b).unwrap();
        assert_eq!(r["verified"], json!(false));
        let states = r["inline_images"].as_object().unwrap();
        assert_eq!(states.len(), 2);
        assert!(states.values().all(|v| v == "mismatched"));
        assert!(r["warning"].as_str().unwrap().contains("do not replay"));
    }
}

#[test]
fn failure_is_single_attempt_and_reports_orphans() {
    for tenant in TENANTS {
        for step in ["upload", "owner"] {
            let fx = fixture(tenant);
            fx.s.lock().unwrap().fail = Some(step);
            let b = || build(&fx, &fx.paths, "note", None);
            let p = b().unwrap();
            let e = send(&p, &fx.c, &fx.journal, Some(&p.hash()), &b).unwrap_err();
            let text = message(e);
            assert!(text.contains("no delete route exists"), "{text}");
            let count = writes(&fx).len();
            assert_eq!(count, if step == "upload" { 1 } else { 3 });
            if step == "owner" {
                assert!(
                    text.contains(&format!("[{}, {}]", fx.base + 1, fx.base + 2)),
                    "{text}"
                );
            }
            let again = send(&p, &fx.c, &fx.journal, Some(&p.hash()), &b).unwrap_err();
            assert!(message(again).contains("do not replay"));
            assert_eq!(writes(&fx).len(), count);
        }
    }
}

#[test]
fn non_image_and_oversize_are_refused_before_upload() {
    for tenant in TENANTS {
        let fx = fixture(tenant);
        let doc = fx.dir.path().join("acme.txt");
        std::fs::write(&doc, b"Acme").unwrap();
        let e = build(&fx, &[doc.to_string_lossy().into_owned()], "note", None).unwrap_err();
        assert!(message(e).contains("JPEG, PNG, GIF or WebP"));
        let e = build(&fx, &fx.paths, "note", Some(PNG.len() as i64)).unwrap_err();
        assert!(message(e).contains("size limit"));
        assert!(writes(&fx).is_empty());
    }
}

#[test]
fn drifted_bytes_block() {
    for tenant in TENANTS {
        let fx = fixture(tenant);
        let b = || build(&fx, &fx.paths, "note", None);
        let p = b().unwrap();
        let mut raw = PNG.to_vec();
        raw.push(b'x');
        std::fs::write(&fx.paths[0], raw).unwrap();
        assert!(send(&p, &fx.c, &fx.journal, Some(&p.hash()), &b).is_err());
        assert!(writes(&fx).is_empty());
    }
}

#[test]
fn multipart_and_image_block_shapes() {
    let (body, ctype) = multipart_with("----fsvb", "a\"c\r\n.png", "image/png", b"PNG");
    assert_eq!(ctype, "multipart/form-data; boundary=----fsvb");
    assert_eq!(
        String::from_utf8(body).unwrap(),
        "------fsvb\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a%22c.png\"\r\nContent-Type: image/png\r\n\r\nPNG\r\n------fsvb--\r\n"
    );
    assert_eq!(
        image_html("https://files.example.com/i?a=1&b=\"2\"", 7),
        r#"<div><img src="https://files.example.com/i?a=1&amp;b=&quot;2&quot;" style="width: auto;" class="fr-fil fr-dib" fileid="7" data-id="7"></div>"#
    );
    let (_, random) = multipart("a.png", "image/png", b"x");
    assert!(random.len() == "multipart/form-data; boundary=----fsv".len() + 32);
}

/// Reviews emitted by Python 520bb68 for the fixture files on both tenants.
#[test]
fn review_matches_python() {
    let oracle: Value =
        serde_json::from_str(include_str!("../testdata/inline_images_oracle.json")).unwrap();
    for tenant in TENANTS {
        for kind in ["note", "reply"] {
            let fx = fixture(tenant);
            let p = build(&fx, &fx.paths, kind, None).unwrap();
            let want = &oracle[format!("{} {kind}", tenant.0)];
            assert_eq!(json!(p.hash()), want["hash"]);
            assert_eq!(Value::Object(p.review()), want["review"]);
            assert_eq!(
                serde_json::to_string(&p.review()).unwrap(),
                serde_json::to_string(&want["review"]).unwrap()
            );
        }
    }
}
