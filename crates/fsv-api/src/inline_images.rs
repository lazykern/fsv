//! Reviewed inline images in ticket replies and notes (`inline_images.py`).
//!
//! Captured wire (agent rich-text editor):
//!
//! 1. `POST /api/_/solutions/inline_images` as `multipart/form-data` with one
//!    field `file` (filename + image content type) -> 201 `{"link", "fileid"}`.
//! 2. The editor embeds `<img src="<link>" ... fileid="<id>" data-id="<id>">`
//!    in the body HTML; the reply/note POST carries that HTML in `body` (and
//!    the reply's `full_text`). There is no attachment-id array for inline
//!    images.
//! 3. Readback: the exact conversation's `body` keeps the `<img>` with the
//!    link and `data-id` (the server drops `fileid`).
//!
//! No delete path exists, so an uploaded inline image whose reply/note fails
//! stays orphaned. Every step is single-attempt and journaled; never replay.

use rand::RngCore;
use serde_json::{Map, Value, json};

use fsv_core::client::Body;
use fsv_core::error::{Error, Result};
use fsv_core::journal::sha256_hex;
use fsv_core::pystr::{html_escape, html_unescape};

use crate::attachments::{self, AttachedPlan, ConversationAttach, LocalFile};
use crate::formmeta::MetaStore;
use crate::notes::returned_note;
use crate::reviewed_write::refused;
use crate::service::{
    Api, Journal, last_event, load_conversations, obj, plan_hash, replay_refusal,
};
use crate::watchers::positive_id;

pub const UPLOAD_PATH: &str = "solutions/inline_images";
/// The editor's file input accepts exactly these image types.
pub const IMAGE_TYPES: [&str; 4] = ["image/jpeg", "image/png", "image/gif", "image/webp"];

/// Port of inline_images.InlinePlan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlinePlan {
    pub attached: AttachedPlan,
}

impl InlinePlan {
    pub fn files(&self) -> &[LocalFile] {
        &self.attached.files
    }

    /// Port of inline_images.InlinePlan.hash.
    pub fn hash(&self) -> String {
        plan_hash(&json!({"inline_images": self.attached.hash()}))
    }

    /// Port of inline_images.InlinePlan.review.
    pub fn review(&self) -> Map<String, Value> {
        let mut review = self.attached.review();
        review.insert("plan_hash".into(), json!(self.hash()));
        review.insert(
            "inline_images".into(),
            Value::Array(
                self.files()
                    .iter()
                    .map(|f| json!({"name": f.name, "size": f.size, "content_type": f.mime}))
                    .collect(),
            ),
        );
        review.insert(
            "upload".into(),
            json!(
                "POST /api/_/solutions/inline_images, multipart field `file`, once per image; each returned \
                 link is embedded as <img src=link data-id=fileid> after the body (no attachment IDs)"
            ),
        );
        review.insert(
            "warning".into(),
            json!(
                "Uploads and the reply/note are not atomic. A failed or unknown step leaves orphan inline \
                 images that cannot be deleted (no delete route exists). Inspect the journal and ticket; \
                 never replay."
            ),
        );
        review
    }
}

fn check_images(files: &[LocalFile]) -> Result<()> {
    for f in files {
        let guessed = crate::create::guess_mime(&f.name);
        let ok = |m: &str| IMAGE_TYPES.contains(&m);
        if !guessed.as_deref().is_some_and(ok) || !ok(&f.mime) {
            return Err(refused(format!(
                "inline image must be JPEG, PNG, GIF or WebP: {}",
                f.name
            )));
        }
    }
    Ok(())
}

/// Port of inline_images.plan: the attachment conversation plan with the
/// `_inline_images` action; every file must be an editor image type.
pub fn plan(
    c: &dyn Api,
    store: &dyn MetaStore,
    target: &str,
    body: &str,
    paths: &[String],
    args: &ConversationAttach<'_>,
) -> Result<InlinePlan> {
    if args.kind != "reply" && args.kind != "note" {
        return Err(refused(
            "inline images are captured only for ticket replies and notes",
        ));
    }
    let mut attached = attachments::conversation_plan(c, store, target, body, paths, args)?;
    attached.base.action = attached
        .base
        .action
        .replace("_attachments", "_inline_images");
    check_images(&attached.files)?;
    Ok(InlinePlan { attached })
}

/// Port of inline_images.image_html: the editor's exact inline-image block.
pub fn image_html(link: &str, fileid: i64) -> String {
    format!(
        r#"<div><img src="{}" style="width: auto;" class="fr-fil fr-dib" fileid="{fileid}" data-id="{fileid}"></div>"#,
        html_escape(link, true)
    )
}

/// Port of inline_images._multipart: one `file` part and its content type.
pub fn multipart(name: &str, mime: &str, raw: &[u8]) -> (Vec<u8>, String) {
    let mut token = [0u8; 16];
    rand::rng().fill_bytes(&mut token);
    let hex: String = token.iter().map(|b| format!("{b:02x}")).collect();
    multipart_with(&format!("----fsv{hex}"), name, mime, raw)
}

fn multipart_with(boundary: &str, name: &str, mime: &str, raw: &[u8]) -> (Vec<u8>, String) {
    let safe = name.replace('"', "%22").replace(['\r', '\n'], "");
    let mut out = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{safe}\"\r\nContent-Type: {mime}\r\n\r\n"
    )
    .into_bytes();
    out.extend_from_slice(raw);
    out.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (out, format!("multipart/form-data; boundary={boundary}"))
}

fn upload_result(response: &Value) -> Result<(String, i64)> {
    let Value::Object(m) = response else {
        return Err(refused("inline image upload returned no JSON object"));
    };
    let link = m.get("link");
    let fileid = positive_id(m.get("fileid").unwrap_or(&Value::Null))?;
    match link.and_then(Value::as_str) {
        Some(l) if l.starts_with("https://") && !l.chars().any(|c| "\"'<> \r\n\t".contains(c)) => {
            Ok((l.to_string(), fileid))
        }
        _ => Err(refused("inline image upload returned no usable https link")),
    }
}

/// Port of inline_images.final_payload: the image blocks appended to
/// `body` and `full_text`.
pub fn final_payload(payload: &Value, images: &[(String, i64)]) -> Value {
    let blocks: String = images.iter().map(|(l, f)| image_html(l, *f)).collect();
    let mut out = payload.clone();
    if let Value::Object(m) = &mut out {
        for key in ["body", "full_text"] {
            if let Some(Value::String(s)) = m.get_mut(key) {
                s.push_str(&blocks);
            }
        }
    }
    out
}

/// Port of inline_images.images_in_body.
pub fn images_in_body(row: Option<&Value>, images: &[(String, i64)]) -> Map<String, Value> {
    let body = row.and_then(|r| r.get("body")).and_then(Value::as_str);
    let Some(body) = body else {
        return images
            .iter()
            .map(|(_, f)| (f.to_string(), json!("unavailable")))
            .collect();
    };
    let text = html_unescape(body);
    images
        .iter()
        .map(|(link, fid)| {
            let ok = text.contains(link.as_str()) && text.contains(&format!("data-id=\"{fid}\""));
            (
                fid.to_string(),
                json!(if ok { "matched" } else { "mismatched" }),
            )
        })
        .collect()
}

fn dumps(v: &Value) -> Vec<u8> {
    fsv_py::Dumps::default().dump(v).into_bytes()
}

fn transact(
    p: &InlinePlan,
    c: &dyn Api,
    journal: &dyn Journal,
    hash: &str,
    csrf: &str,
    images: &mut Vec<(String, i64)>,
) -> Result<(Value, Value)> {
    let base = &p.attached.base;
    let target = json!(base.target);
    for f in p.files() {
        let raw = std::fs::read(&f.path).map_err(|e| Error::other(e.to_string()))?;
        if raw.len() as i64 != f.size || sha256_hex(&raw) != f.sha256 {
            return Err(refused("image changed after review"));
        }
        let (content, ctype) = multipart(&f.name, &f.mime, &raw);
        let response = c.send_once_typed("POST", UPLOAD_PATH, content, &ctype, csrf)?;
        let (link, fid) = upload_result(&response)?;
        if images.iter().any(|(_, known)| *known == fid) {
            return Err(refused("upload returned a duplicate inline image identity"));
        }
        images.push((link, fid));
        journal.record_plan(
            "uploaded",
            hash,
            &format!("{}_upload", base.action),
            Some(&target),
            Some(&json!(fid)),
        );
    }
    let payload = final_payload(&base.payload(), images);
    let response = c.send_once(
        &base.method,
        &base.path,
        Body::Raw(dumps(&payload)),
        None,
        csrf,
    )?;
    Ok((payload, response))
}

fn readback(
    p: &InlinePlan,
    c: &dyn Api,
    payload: &Value,
    response: &Value,
    owner: &Value,
    images: &[(String, i64)],
) -> Result<Map<String, Value>> {
    let mut fin = p.attached.clone();
    fin.base.payload_json = fsv_py::Dumps::default().dump(payload);
    let mut checked = attachments::readback(c, &fin, response, &[])?;
    let pages = load_conversations(c, p.attached.base.target)?;
    let owner = fsv_py::py_str(owner);
    let matched: Vec<&Value> = pages
        .rows
        .iter()
        .filter(|r| fsv_py::py_str(r.get("id").unwrap_or(&Value::Null)) == owner)
        .collect();
    let inline = images_in_body(
        if matched.len() == 1 {
            Some(matched[0])
        } else {
            None
        },
        images,
    );
    if let Some(Value::Object(fields)) = checked.get_mut("fields") {
        fields.shift_remove("full_text");
    }
    let verified =
        checked.get("verified") == Some(&json!(true)) && inline.values().all(|v| v == "matched");
    Ok(obj(json!({
        "owner_id": checked.get("owner_id").cloned().unwrap_or(Value::Null),
        "fields": checked.get("fields").cloned().unwrap_or(Value::Null),
        "inline_images": inline,
        "verified": verified,
    })))
}

/// Port of inline_images.send: guards, one multipart upload per image, one
/// owning write carrying the embedded images, then the body readback.
pub fn send(
    p: &InlinePlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
    rebuild: &dyn Fn() -> Result<InlinePlan>,
) -> Result<Value> {
    let Some(reviewed) = reviewed_hash else {
        return Ok(json!({"status": "cancelled", "verified": false}));
    };
    let base = &p.attached.base;
    let hash = p.hash();
    if base.tenant.is_empty() || base.tenant != c.domain() || reviewed != hash {
        return Err(refused("tenant or full inline-image review hash changed"));
    }
    if p.attached.limit.is_none() {
        return Err(refused(
            "the tenant publishes no attachment size limit; review again with --max-bytes N; nothing was sent",
        ));
    }
    let action = &base.action;
    if let Some(refusal) = replay_refusal(
        journal,
        action,
        &hash,
        "inline-image transaction was sent or outcome unknown; do not replay",
    ) {
        return Err(refused(refusal));
    }
    let paths: Vec<String> = p.files().iter().map(|f| f.path.clone()).collect();
    if rebuild()?.hash() != hash || attachments::local_files(&paths, p.attached.limit)? != p.files()
    {
        return Err(refused(
            "ticket, metadata or image bytes changed; review again (nothing was sent)",
        ));
    }
    let csrf = c.get_csrf()?;
    let target = json!(base.target);
    journal.record_plan("started", &hash, action, Some(&target), None);
    if last_event(journal, action, &hash).as_deref() != Some("started") {
        return Err(refused(
            "could not persist inline-image journal; nothing was sent",
        ));
    }
    let mut images = Vec::new();
    let (payload, response) = match transact(p, c, journal, &hash, &csrf, &mut images) {
        Ok(r) => r,
        Err(_) => {
            let ids: Vec<i64> = images.iter().map(|(_, f)| *f).collect();
            return Err(refused(format!(
                "inline-image transaction failed or outcome unknown; uploaded image IDs: {} (no delete route exists); inspect the ticket and journal, do not replay",
                fsv_py::Dumps::default().dump(&json!(ids))
            )));
        }
    };
    let owner = returned_note(&response)
        .filter(|r| r.is_object())
        .and_then(|r| positive_id(r.get("id").unwrap_or(&Value::Null)).ok())
        .map_or(Value::Null, |id| json!(id));
    journal.record_plan(
        "done",
        &hash,
        action,
        Some(&target),
        (!owner.is_null()).then_some(&owner),
    );
    let ids: Vec<i64> = images.iter().map(|(_, f)| *f).collect();
    let mut result = obj(
        json!({"status": "submitted", "plan_hash": hash, "inline_image_ids": ids,
                                "owner_id": owner, "verified": false}),
    );
    if let Ok(extra) = readback(p, c, &payload, &response, &owner, &images) {
        result.extend(extra);
    }
    if result.get("verified") != Some(&json!(true)) {
        result.insert(
            "warning".into(),
            json!(
                "Independent conversation readback did not confirm every image link; do not replay."
            ),
        );
    }
    Ok(Value::Object(result))
}

#[cfg(test)]
mod tests;
