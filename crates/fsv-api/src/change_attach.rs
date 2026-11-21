//! Reviewed attach-only change update (`changes update ID --attach FILE`,
//! `change_attach.py`).
//!
//! One transaction of two kinds of step: one upload (`POST
//! /api/_/attachments`) per new file, then `PUT /api/_/changes/<id>` with the
//! full attachment ID list (kept + uploaded), the same wire the legacy path
//! sent. The plan hash binds the tenant, change, file names + sizes +
//! sha256, the duplicate decisions, the size limit (`--max-bytes` or the
//! tenant's published limit) and a record-state token. `--yes` needs the
//! full `--expect-plan`; the journal gets one entry per upload and one for
//! the save; an unknown step is never replayed. Readback re-reads the
//! change's attachment list and compares IDs and names (`verified`).

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::{Map, Value, json};

use fsv_core::client::Body;
use fsv_core::error::{Error, Result};
use fsv_py::Dumps;

use crate::attachments::{LocalFile, NOT_PUBLISHED, local_files, owner_row, resolve_limit};
use crate::create::Prompter;
use crate::planning_files::{
    DUPLICATE_MODES, Failure, ask_duplicate, att_name, att_size, upload_files,
};
use crate::readback::submitted_value_matches;
use crate::reviewed_write::refused;
use crate::service::{Api, Journal, last_event, obj, plan_hash, replay_refusal, snapshot_hash};
use crate::watchers::positive_id;

pub const ACTION: &str = "change_attachments_update";

/// `_attachments`.
fn attachments(row: &Map<String, Value>) -> Result<Vec<Value>> {
    let value = match row.get("attachments") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) if a.iter().all(Value::is_object) => a.clone(),
        _ => {
            return Err(refused(
                "change attachment inventory unavailable; no write attempted",
            ));
        }
    };
    let mut ids = Vec::new();
    for a in &value {
        ids.push(positive_id(a.get("id").unwrap_or(&Value::Null))?);
    }
    if ids.iter().collect::<BTreeSet<_>>().len() != ids.len() {
        return Err(refused(
            "change attachment inventory has duplicate IDs; no write attempted",
        ));
    }
    Ok(value)
}

/// Port of `ChangeAttachPlan`.
#[derive(Debug, Clone, PartialEq)]
pub struct ChangeAttachPlan {
    pub tenant: String,
    pub target: i64,
    pub kept_ids: Vec<i64>,
    pub replaced: Vec<(i64, String)>,
    pub skipped: Vec<String>,
    pub files: Vec<LocalFile>,
    pub duplicate: String,
    pub limit: Option<i64>,
    pub limit_source: String,
    pub published_limit: Option<i64>,
    pub context_hash: String,
    pub state: Map<String, Value>,
}

impl ChangeAttachPlan {
    pub fn path(&self) -> String {
        format!("changes/{}", self.target)
    }

    pub fn noop(&self) -> bool {
        self.files.is_empty()
    }

    pub fn hash(&self) -> String {
        plan_hash(&json!({
            "action": ACTION, "tenant": self.tenant, "target": self.target, "method": "PUT",
            "path": self.path(), "kept_attachment_ids": self.kept_ids,
            "replaced": self.replaced.iter().map(|(i, n)| json!([i, n])).collect::<Vec<_>>(),
            "skipped": self.skipped,
            "files": self.files.iter().map(LocalFile::review).collect::<Vec<_>>(),
            "duplicate": self.duplicate, "max_bytes": self.limit,
            "limit_source": self.limit_source, "published_bytes": self.published_limit,
            "context": self.context_hash, "record_state": self.state,
        }))
    }

    /// `payload(uploaded)`.
    pub fn payload(&self, uploaded: &[i64]) -> Value {
        let ids: Vec<i64> = self.kept_ids.iter().chain(uploaded).copied().collect();
        json!({"attachments": ids})
    }

    pub fn review(&self) -> Map<String, Value> {
        let published = self
            .published_limit
            .map_or_else(|| json!(NOT_PUBLISHED), |n| json!(n));
        let mut steps: Vec<Value> = self
            .files
            .iter()
            .map(|f| json!({"step": "upload", "method": "POST", "path": "/api/_/attachments", "file": f.name}))
            .collect();
        steps.push(
            json!({"step": "attach", "method": "PUT", "path": format!("/api/_/{}", self.path())}),
        );
        let mut limit = obj(json!({
            "max_bytes": self.limit, "source": self.limit_source, "published_bytes": published,
            "total_bytes": self.files.iter().map(|f| f.size).sum::<i64>(),
            "applies_to": "total of all files in this submit",
        }));
        if self.limit.is_none() {
            limit.insert(
                "note".into(),
                json!("the tenant publishes no attachment size limit; pass --max-bytes N to set one before sending"),
            );
        }
        obj(json!({
            "action": ACTION, "target": self.target, "steps": steps,
            "files": self.files.iter().map(LocalFile::review).collect::<Vec<_>>(),
            "duplicate": self.duplicate, "kept_attachment_ids": self.kept_ids,
            "replaced_attachments": self.replaced.iter().map(|(i, n)| json!({"id": i, "name": n})).collect::<Vec<_>>(),
            "skipped_files": self.skipped, "record_state": self.state,
            "plan_hash": self.hash(), "attachment_size_limit": limit,
            "warning": "Uploads and the attachment save are not atomic. A failed or unknown step may leave orphan uploads; inspect the change and journal, never replay the transaction.",
        }))
    }
}

/// The `build_plan` keyword arguments.
pub struct AttachOpts<'a> {
    pub duplicate: &'a str,
    pub backup_replaced: bool,
    pub max_bytes: Option<i64>,
    pub no_input: bool,
}

fn expanduser_size(p: &str) -> Option<i64> {
    let path = if p == "~" || p.starts_with("~/") {
        let home = fsv_core::config::home_dir();
        format!(
            "{}{}",
            home.to_string_lossy().trim_end_matches('/'),
            &p[1..]
        )
    } else {
        p.to_string()
    };
    let path = Path::new(&path);
    path.is_file()
        .then(|| std::fs::metadata(path).map(|m| m.len() as i64).ok())
        .flatten()
}

/// Port of `build_plan`.
pub fn build_plan(
    c: &dyn Api,
    journal: &dyn Journal,
    prompter: &dyn Prompter,
    target: i64,
    paths: &[String],
    opts: &AttachOpts,
) -> Result<ChangeAttachPlan> {
    let mode = fsv_core::pystr::casefold(opts.duplicate);
    if !DUPLICATE_MODES.contains(&mode.as_str()) {
        return Err(Error::value(
            "--duplicate must be one of: prompt, skip, replace, append",
        ));
    }
    if opts.backup_replaced {
        return Err(Error::value(
            "--backup-replaced is not on the reviewed attachment path; download the attachment first (fsv changes download ID), then --duplicate replace",
        ));
    }
    if paths.is_empty() {
        return Err(Error::value("--attach needs at least one file"));
    }
    let cid = positive_id(&json!(target))?;
    let row = owner_row(c, "changes", cid)?;
    let existing = attachments(&row)?;
    let mut kept: Vec<i64> = Vec::new();
    for a in &existing {
        kept.push(positive_id(a.get("id").unwrap_or(&Value::Null))?);
    }
    let mut replaced: Vec<(i64, String)> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut upload: Vec<String> = Vec::new();
    for value in paths {
        let name = Path::new(value)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let matches: Vec<&Value> = existing.iter().filter(|a| att_name(a) == name).collect();
        let mut action = mode.clone();
        if !matches.is_empty() && action == "prompt" {
            if opts.no_input {
                return Err(refused(format!(
                    "attachment already exists: {name}; --no-input cannot prompt, use --duplicate skip|replace|append"
                )));
            }
            action = ask_duplicate(prompter, &name, matches.len())?;
        }
        if !matches.is_empty() && action == "skip" {
            skipped.push(name);
            continue;
        }
        if !matches.is_empty() && action == "replace" {
            for a in &matches {
                let ident = positive_id(a.get("id").unwrap_or(&Value::Null))?;
                if let Some(pos) = kept.iter().position(|k| *k == ident) {
                    kept.remove(pos);
                    replaced.push((ident, name.clone()));
                }
            }
        }
        upload.push(value.clone());
    }
    let (mut limit_value, mut source, mut published) = (None, NOT_PUBLISHED.to_string(), None);
    let mut files: Vec<LocalFile> = Vec::new();
    if !upload.is_empty() {
        let limit = resolve_limit(c, &Value::Null, opts.max_bytes)?;
        (limit_value, source, published) = (limit.max_bytes, limit.source, limit.published);
        let total: i64 = upload.iter().filter_map(|p| expanduser_size(p)).sum();
        if let Some(max) = limit_value
            && total > max
        {
            return Err(refused(format!(
                "--attach files total {total} bytes and exceed the {max}-byte limit ({source}); nothing was uploaded"
            )));
        }
        files = local_files(&upload, limit_value)?;
    } else if opts.max_bytes.is_some_and(|n| n <= 0) {
        return Err(refused(
            "--max-bytes must be a positive whole number of bytes",
        ));
    }
    let state = journal.repeat_token(&[ACTION], &json!(cid), Some(&Value::Object(row)));
    Ok(ChangeAttachPlan {
        tenant: c.domain(),
        target: cid,
        kept_ids: kept,
        replaced,
        skipped,
        files,
        duplicate: mode,
        limit: limit_value,
        limit_source: source,
        published_limit: published,
        context_hash: snapshot_hash(&json!({"attachments": existing}))?,
        state,
    })
}

/// Port of `readback`.
pub fn readback(
    c: &dyn Api,
    plan: &ChangeAttachPlan,
    uploaded: &[i64],
) -> Result<Map<String, Value>> {
    let mut observed: Vec<(i64, Value)> = Vec::new();
    for a in attachments(&owner_row(c, "changes", plan.target)?)? {
        observed.push((positive_id(a.get("id").unwrap_or(&Value::Null))?, a));
    }
    let expected: BTreeSet<i64> = plan.kept_ids.iter().chain(uploaded).copied().collect();
    let seen: BTreeSet<i64> = observed.iter().map(|(i, _)| *i).collect();
    let find = |id: i64| observed.iter().find(|(i, _)| *i == id).map(|(_, a)| a);
    let ok = seen == expected
        && uploaded.iter().zip(&plan.files).all(|(i, f)| {
            find(*i).is_some_and(|a| {
                att_name(a) == f.name
                    && (att_size(a).is_null()
                        || submitted_value_matches(&att_size(a), &json!(f.size)))
            })
        });
    observed.sort_by_key(|(i, _)| *i);
    Ok(obj(json!({
        "attachments": observed.iter().map(|(i, a)| json!({"id": i, "name": att_name(a)})).collect::<Vec<_>>(),
        "attachments_verified": ok, "verified": ok,
    })))
}

/// Port of `send`.
pub fn send(
    plan: &ChangeAttachPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
    rebuild: &dyn Fn() -> Result<ChangeAttachPlan>,
) -> std::result::Result<Value, Failure> {
    let Some(reviewed) = reviewed_hash else {
        return Ok(json!({"status": "cancelled", "verified": false}));
    };
    let hash = plan.hash();
    if plan.tenant.is_empty() || plan.tenant != c.domain() || reviewed != hash {
        return Err(refused("tenant or full attachment review hash changed; review again").into());
    }
    if !plan.files.is_empty() && plan.limit.is_none() {
        return Err(refused(
            "the tenant publishes no attachment size limit; review again with --max-bytes N; no upload attempted",
        )
        .into());
    }
    if let Some(refusal) = replay_refusal(
        journal,
        ACTION,
        &hash,
        "attachment transaction was sent or outcome unknown; do not replay",
    ) {
        return Err(refused(refusal).into());
    }
    let fresh = rebuild()?;
    let paths: Vec<String> = plan.files.iter().map(|f| f.path.clone()).collect();
    if fresh.hash() != hash
        || (!plan.files.is_empty() && local_files(&paths, plan.limit)? != plan.files)
    {
        return Err(
            refused("change attachments or file bytes changed since review; review again").into(),
        );
    }
    let Ok(csrf) = c.get_csrf() else {
        return Err(refused("could not prepare session; no write attempted").into());
    };
    journal.record_plan("started", &hash, ACTION, Some(&json!(plan.target)), None);
    if last_event(journal, ACTION, &hash).as_deref() != Some("started") {
        return Err(refused("could not persist attachment journal; no write attempted").into());
    }
    let mut uploaded: Vec<i64> = Vec::new();
    let mut step = "upload";
    let transact = |uploaded: &mut Vec<i64>, step: &mut &str| -> Result<()> {
        upload_files(
            c,
            journal,
            &csrf,
            &plan.files,
            uploaded,
            &hash,
            ACTION,
            plan.target,
        )?;
        *step = "attach";
        let body = Dumps::default().dump(&plan.payload(uploaded)).into_bytes();
        c.send_once("PUT", &plan.path(), Body::Raw(body), None, &csrf)?;
        Ok(())
    };
    if let Err(exc) = transact(&mut uploaded, &mut step) {
        let status = exc.status().filter(|s| *s != 0);
        let detail = status.map_or_else(String::new, |s| format!(" (HTTP {s})"));
        return Err(Failure {
            error: refused(format!(
                "attachment transaction failed at the {step} step{detail} or outcome unknown; uploaded IDs: {}; inspect the change and journal, do not replay",
                Dumps::default().dump(&json!(uploaded))
            )),
            http_status: status,
        });
    }
    journal.record_plan("done", &hash, ACTION, Some(&json!(plan.target)), None);
    let mut result = obj(json!({
        "status": "submitted", "plan_hash": hash, "uploaded_ids": uploaded,
        "skipped": plan.skipped, "verified": false,
    }));
    match readback(c, plan, &uploaded) {
        Ok(extra) => result.extend(extra),
        Err(_) => {
            result.insert(
                "warning".into(),
                json!("Independent attachment readback failed; do not resend."),
            );
        }
    }
    if result.get("verified") == Some(&Value::Bool(true)) {
        journal.record_verified(&hash, ACTION, Some(&json!(plan.target)));
    } else if !result.contains_key("warning") {
        result.insert(
            "warning".into(),
            json!("Attachment readback was not fully verified; do not resend."),
        );
    }
    Ok(Value::Object(result))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use base64::Engine as _;
    use fsv_core::ApiError;

    use super::*;
    use crate::service::testing::{Call, FnApi, MemJournal};

    const UPLOAD_BASE: i64 = 701;

    struct NoTerminal;
    impl Prompter for NoTerminal {
        fn interactive(&self) -> bool {
            false
        }
        fn ask(&self, _: &str) -> String {
            String::new()
        }
    }

    #[derive(Default)]
    struct Tenant {
        attachments: Vec<Value>,
        uploads: Vec<Value>,
        writes: Vec<(String, String, Value)>,
        fail: Option<(&'static str, Error)>,
        updated_at: String,
    }

    type Shared = Arc<Mutex<Tenant>>;

    /// `tests/test_change_attach_reviewed.py` wire: one change with one
    /// attachment, uploads, `PUT changes/123`.
    fn tenant() -> (Shared, FnApi) {
        let state: Shared = Arc::new(Mutex::new(Tenant {
            attachments: vec![json!({"id": 501, "name": "old.txt", "size": 9})],
            updated_at: "2026-01-01T00:00:00Z".into(),
            ..Tenant::default()
        }));
        let shared = state.clone();
        let api = FnApi::new(move |call: &Call| {
            let mut s = shared.lock().unwrap();
            if call.method == "GET" {
                assert_eq!(call.path, "changes/123");
                return Ok(json!({"change": {"id": 123, "updated_at": s.updated_at,
                                            "attachments": s.attachments}}));
            }
            let body = call.body.clone().unwrap();
            s.writes.push((call.method.clone(), call.path.clone(), body.clone()));
            let step = if call.path == "attachments" { "upload" } else { "attach" };
            if s.fail.as_ref().is_some_and(|(st, _)| *st == step) {
                return Err(s.fail.take().unwrap().1);
            }
            if step == "upload" {
                let ident = UPLOAD_BASE + s.uploads.len() as i64;
                let raw = body["content"].as_str().unwrap().split_once(',').unwrap().1.to_string();
                let data = base64::engine::general_purpose::STANDARD.decode(raw).unwrap();
                let row = json!({"id": ident, "name": body["content_file_name"], "size": data.len().to_string()});
                s.uploads.push(row.clone());
                return Ok(row);
            }
            let mut known = s.attachments.clone();
            known.extend(s.uploads.clone());
            let want: Vec<Value> = body["attachments"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| known.iter().find(|k| k["id"] == *i).cloned().unwrap())
                .collect();
            s.attachments = want;
            s.updated_at = "2026-01-01T00:05:00Z".into();
            Ok(json!({"change": {"id": 123}}))
        })
        .with_fetch(|_| {
            Ok(fsv_core::client::Response::new(200)
                .with_body(b"<html><script>window.MAX_ATTACHMENT_LIMIT = 1;</script></html>".to_vec()))
        });
        (state, api)
    }

    fn file(name: &str, bytes: &[u8]) -> String {
        let test = std::thread::current()
            .name()
            .unwrap_or("t")
            .replace("::", "_");
        let dir: PathBuf =
            std::env::temp_dir().join(format!("fsv_change_attach_{}_{test}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        p.to_string_lossy().into_owned()
    }

    fn plan(
        c: &FnApi,
        j: &MemJournal,
        paths: &[String],
        duplicate: &str,
        max_bytes: Option<i64>,
        no_input: bool,
    ) -> Result<ChangeAttachPlan> {
        build_plan(
            c,
            j,
            &NoTerminal,
            123,
            paths,
            &AttachOpts {
                duplicate,
                backup_replaced: false,
                max_bytes,
                no_input,
            },
        )
    }

    #[test]
    fn dry_run_hash_binds_file_bytes_limit_and_record_state() {
        let (s, c) = tenant();
        let j = MemJournal::default();
        let f = file("plan_hash.txt", b"Acme plan\n");
        let p = plan(&c, &j, std::slice::from_ref(&f), "prompt", None, false).unwrap();
        let review = p.review();
        let steps: Vec<&str> = review["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["step"].as_str().unwrap())
            .collect();
        assert_eq!(steps, ["upload", "attach"]);
        assert_eq!(
            review["attachment_size_limit"]["max_bytes"],
            json!(1024 * 1024)
        );
        assert_eq!(review["record_state"]["sends"], json!(0));
        assert_eq!(review["kept_attachment_ids"], json!([501]));
        assert!(s.lock().unwrap().writes.is_empty());
        std::fs::write(&f, b"Acme plan v2\n").unwrap();
        let again = plan(&c, &j, &[f], "prompt", None, false).unwrap();
        assert_ne!(again.hash(), p.hash());
    }

    #[test]
    fn reviewed_send_uploads_then_attaches_with_journal_and_readback() {
        let (s, c) = tenant();
        let j = MemJournal::default();
        let paths = vec![file("plan_send.txt", b"Acme plan\n")];
        let p = plan(&c, &j, &paths, "prompt", None, false).unwrap();
        let build = || plan(&c, &j, &paths, "prompt", None, false);
        let out = send(&p, &c, &j, Some(&p.hash()), &build).unwrap();
        assert_eq!(out["verified"], json!(true), "{out}");
        assert_eq!(out["uploaded_ids"], json!([UPLOAD_BASE]));
        assert_eq!(
            out["attachments"],
            json!([{"id": 501, "name": "old.txt"}, {"id": UPLOAD_BASE, "name": "plan_send.txt"}])
        );
        let w = s.lock().unwrap().writes.clone();
        assert_eq!(
            w.iter()
                .map(|x| (x.0.as_str(), x.1.as_str()))
                .collect::<Vec<_>>(),
            [("POST", "attachments"), ("PUT", "changes/123")]
        );
        assert_eq!(w[1].2, json!({"attachments": [501, UPLOAD_BASE]}));
        let rows = j.read();
        assert!(rows.iter().any(
            |e| e["kind"] == json!("plan:change_attachments_update_upload")
                && e["event"] == json!("uploaded")
        ));
        assert_eq!(rows.last().unwrap()["event"], json!("done"));
        let again = send(&p, &c, &j, Some(&p.hash()), &build).unwrap_err();
        assert_eq!(again.error.exit_code(), 2);
        assert_eq!(s.lock().unwrap().writes.len(), 2);
    }

    #[test]
    fn duplicates_skip_replace_and_prompt_under_no_input() {
        let (_, c) = tenant();
        let j = MemJournal::default();
        let old = file("old.txt", b"Acme old\n");
        let skipped = plan(&c, &j, std::slice::from_ref(&old), "skip", None, false).unwrap();
        assert!(skipped.noop());
        assert_eq!(skipped.skipped, ["old.txt"]);
        let replaced = plan(&c, &j, std::slice::from_ref(&old), "replace", None, false).unwrap();
        assert_eq!(replaced.replaced, [(501, "old.txt".to_string())]);
        assert!(replaced.kept_ids.is_empty());
        let err = plan(&c, &j, std::slice::from_ref(&old), "prompt", None, true).unwrap_err();
        assert!(
            err.to_string().contains("--no-input cannot prompt"),
            "{err}"
        );
        // a new name never needs a decision, even under --no-input
        let fresh = file("new.txt", b"Acme new\n");
        assert!(plan(&c, &j, &[fresh], "prompt", None, true).is_ok());
        let err = plan(&c, &j, &[old], "prompt", None, false).unwrap_err();
        assert!(err.to_string().contains("skip|replace|append"), "{err}");
    }

    #[test]
    fn backup_replaced_and_oversize_are_refused_before_any_upload() {
        let (s, c) = tenant();
        let j = MemJournal::default();
        let f = file("big.txt", b"Acme plan v2\n");
        let err = build_plan(
            &c,
            &j,
            &NoTerminal,
            123,
            std::slice::from_ref(&f),
            &AttachOpts {
                duplicate: "replace",
                backup_replaced: true,
                max_bytes: None,
                no_input: false,
            },
        )
        .unwrap_err();
        assert_eq!(err.exit_code(), 2);
        let err = plan(&c, &j, std::slice::from_ref(&f), "prompt", Some(5), false).unwrap_err();
        assert!(
            err.to_string().contains("--attach files total 13 bytes and exceed the 5-byte limit (--max-bytes override); nothing was uploaded"),
            "{err}"
        );
        assert!(s.lock().unwrap().writes.is_empty());
    }

    #[test]
    fn unknown_step_is_never_replayed() {
        for (step, exc) in [
            (
                "upload",
                Error::Http {
                    kind: "ReadError".into(),
                    message: "disconnect".into(),
                },
            ),
            ("attach", ApiError::new(503, json!({})).into()),
        ] {
            let (s, c) = tenant();
            let j = MemJournal::default();
            let paths = vec![file("unknown.txt", b"Acme plan\n")];
            let p = plan(&c, &j, &paths, "prompt", None, false).unwrap();
            s.lock().unwrap().fail = Some((step, exc));
            let build = || plan(&c, &j, &paths, "prompt", None, false);
            let err = send(&p, &c, &j, Some(&p.hash()), &build).unwrap_err();
            let text = err.error.to_string();
            assert!(
                text.contains("do not replay") && text.contains(&format!("{step} step")),
                "{text}"
            );
            let n = s.lock().unwrap().writes.len();
            let again = plan(&c, &j, &paths, "prompt", None, false).unwrap();
            assert_eq!(again.hash(), p.hash());
            let refused = send(&again, &c, &j, Some(&again.hash()), &build).unwrap_err();
            assert_eq!(refused.error.exit_code(), 2);
            assert_eq!(s.lock().unwrap().writes.len(), n);
        }
    }
}
