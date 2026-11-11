//! Plan hashes and metadata fingerprints for the write path
//! (`fsv.forms.plan`).
//!
//! [`plan_hash`] identifies exactly what a create would send: the request
//! body plus the content of every file that will be uploaded. `--dry-run`
//! prints it; `--yes --expect-plan <hash>` refuses to send anything else (a
//! metadata refresh that changed a populate, an edited file, a different
//! answer).
//!
//! `br_validation_excludes` is *included* in the hash. It is not volatile:
//! it is computed deterministically from the engine state (no timestamp, no
//! signature), and it changes exactly when the rules' effect on the form
//! changes, which is a change an approver should re-review.
//!
//! [`metadata_fingerprint`] summarises the form metadata (fields, rules,
//! item) so the caller can re-fetch just before submit and re-run the
//! engine when anything moved; `updated_at` gives "form metadata as of ..."
//! for reviews.

use std::io::Read;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use jiff::tz::Offset;
use serde_json::{Map, Value as Json, json};
use sha2::{Digest, Sha256};

use crate::pydt::{DateTimeValue, datetime_fromisoformat};
use crate::pytext::{basename, strip};

pub const PLAN_VERSION: i64 = 1;
pub const MIN_PREFIX: usize = 12;

/// Stable JSON bytes: sorted keys, no whitespace, UTF-8.
pub fn canonical_json(data: &Json) -> String {
    fsv_py::canonical(data)
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Name, size and sha256 of one file to upload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDigest {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

/// Read `path` and digest it.
pub fn file_digest(path: impl AsRef<Path>) -> std::io::Result<FileDigest> {
    let path = path.as_ref();
    let mut fh = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut size = 0u64;
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = fh.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        size += n as u64;
    }
    let name = basename(&path.to_string_lossy()).to_string();
    Ok(FileDigest {
        name,
        size,
        sha256: hex::encode(h.finalize()),
    })
}

/// The files a plan uploads: `{field: [paths]}` or a list of general
/// attachment paths (Python's `files` argument).
#[derive(Clone, Debug)]
pub enum PlanFiles {
    Fields(IndexMap<String, Vec<PathBuf>>),
    Attachments(Vec<PathBuf>),
}

/// One uploaded file as hashed: its field, position in that field, and digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub field: String,
    pub index: usize,
    pub digest: FileDigest,
}

impl FileEntry {
    fn to_json(&self) -> Json {
        json!({
            "field": self.field,
            "index": self.index,
            "name": self.digest.name,
            "size": self.digest.size,
            "sha256": self.digest.sha256,
        })
    }
}

/// Digest every file, sorted by (field, index).
pub fn file_entries(files: Option<&PlanFiles>) -> std::io::Result<Vec<FileEntry>> {
    let mut out = Vec::new();
    let mut add = |field: &str, paths: &[PathBuf]| -> std::io::Result<()> {
        for (index, p) in paths.iter().enumerate() {
            out.push(FileEntry {
                field: field.to_string(),
                index,
                digest: file_digest(p)?,
            });
        }
        Ok(())
    };
    match files {
        None => {}
        Some(PlanFiles::Attachments(paths)) => add("attachments", paths)?,
        Some(PlanFiles::Fields(groups)) => {
            for (key, paths) in groups {
                add(key, paths)?;
            }
        }
    }
    out.sort_by(|a, b| (&a.field, a.index).cmp(&(&b.field, b.index)));
    Ok(out)
}

/// sha256 hex over the canonical payload and each file's name, size and
/// sha256 (reads the files).
pub fn plan_hash(payload: &Json, files: Option<&PlanFiles>) -> std::io::Result<String> {
    Ok(plan_hash_entries(payload, &file_entries(files)?))
}

/// [`plan_hash`] with the file digests already computed (`entries` must be
/// sorted by (field, index), as [`file_entries`] returns them).
pub fn plan_hash_entries(payload: &Json, entries: &[FileEntry]) -> String {
    let files: Vec<Json> = entries.iter().map(FileEntry::to_json).collect();
    let doc = json!({"v": PLAN_VERSION, "payload": payload, "files": files});
    sha256_hex(canonical_json(&doc).as_bytes())
}

/// `expected` may be the full hash or a prefix of at least 12 hex chars
/// (optionally written `sha256:<hex>`).
pub fn plan_matches(expected: &str, actual: &str) -> bool {
    let e = strip(expected).to_lowercase();
    let e = e.strip_prefix("sha256:").unwrap_or(&e);
    e.chars().count() >= MIN_PREFIX && actual.to_lowercase().starts_with(e)
}

/// Summary of the metadata a plan was computed from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetadataFingerprint {
    /// sha256 over the canonical metadata.
    pub hash: String,
    /// Newest `updated_at` found (ISO), for "metadata as of".
    pub updated_at: Option<String>,
    pub counts: Vec<(String, usize)>,
}

impl MetadataFingerprint {
    pub fn changed(&self, other: &MetadataFingerprint) -> bool {
        self.hash != other.hash
    }
}

fn parse_ts(v: &str) -> Option<DateTimeValue> {
    let s = strip(v).replace('Z', "+00:00");
    let (local, off) = datetime_fromisoformat(&s)?;
    Some(DateTimeValue::new(local, off.unwrap_or(Offset::UTC)))
}

fn walk_updated(data: &Json, out: &mut Vec<DateTimeValue>) {
    match data {
        Json::Object(m) => {
            for (k, v) in m {
                match v {
                    Json::String(s) if k == "updated_at" => out.extend(parse_ts(s)),
                    _ => walk_updated(v, out),
                }
            }
        }
        Json::Array(a) => a.iter().for_each(|v| walk_updated(v, out)),
        _ => {}
    }
}

fn count(data: &Json) -> usize {
    match data {
        Json::Array(a) => a.len(),
        Json::Object(m) => m.values().find_map(Json::as_array).map_or(1, Vec::len),
        _ => 0,
    }
}

/// Fingerprint of the metadata a plan was computed from. The rules
/// response's `user` object is included: rules can condition on it.
pub fn metadata_fingerprint(
    form_fields: Option<&Json>,
    rules: Option<&Json>,
    item: Option<&Json>,
) -> MetadataFingerprint {
    let mut doc = Map::new();
    for (k, v) in [
        ("form_fields", form_fields),
        ("rules", rules),
        ("item", item),
    ] {
        doc.insert(k.to_string(), v.cloned().unwrap_or(Json::Null));
    }
    let doc = Json::Object(doc);
    let mut stamps = Vec::new();
    walk_updated(&doc, &mut stamps);
    let mut newest: Option<DateTimeValue> = None;
    for t in stamps {
        if newest.is_none_or(|n| t > n) {
            newest = Some(t);
        }
    }
    let counts = doc
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, v)| !v.is_null())
        .map(|(k, v)| (k.clone(), count(v)))
        .collect();
    MetadataFingerprint {
        hash: sha256_hex(canonical_json(&doc).as_bytes()),
        updated_at: newest.map(|n| n.isoformat()),
        counts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_matches_python() {
        for (data, n) in [
            (json!([1, 2, 3]), 3),
            (json!([]), 0),
            (json!({"a": 1, "b": [1, 2]}), 2),
            (json!({"a": 1}), 1),
            (json!({}), 1),
            (json!({"a": [], "b": [1]}), 0),
            (Json::Null, 0),
            (json!("x"), 0),
            (json!(5), 0),
        ] {
            assert_eq!(count(&data), n, "{data}");
        }
    }

    #[test]
    fn plan_hash_matches_python() {
        let payload = json!({"b": [1, 2.5, null, true], "a": "é", "c": {"z": 1, "y": "ü\n"}});
        assert_eq!(
            plan_hash(&payload, None).expect("hash"),
            "ec190c09ebe8fc9dd364c963b9e9796dc7464710468c7b5e0a8518d64f46f98e"
        );
        assert_eq!(
            plan_hash(&json!({}), None).expect("hash"),
            "e5d6f62de050b2466561081118d47d4d7a40f1411587758dd1943edaf4c225dd"
        );
    }

    #[test]
    fn plan_hash_with_files_matches_python() {
        let dir = std::env::temp_dir().join(format!("fsv-forms-plan-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let x = dir.join("x.txt");
        let y = dir.join("y.bin");
        std::fs::write(&x, b"hello").expect("write");
        std::fs::write(&y, b"\x00\x01".repeat(70000)).expect("write");

        let mut groups = IndexMap::new();
        groups.insert("zfield".to_string(), vec![x.clone()]);
        groups.insert("afield".to_string(), vec![y.clone(), x.clone()]);
        let h2 = plan_hash(&json!({"a": 1}), Some(&PlanFiles::Fields(groups))).expect("hash");
        assert_eq!(
            h2,
            "6ea804e5523c75edf8c7e25c7b32f1ed79a9c438aecd683bb54e5c083e89e46f"
        );
        let h3 = plan_hash(
            &json!({"a": 1}),
            Some(&PlanFiles::Attachments(vec![x.clone()])),
        )
        .expect("hash");
        assert_eq!(
            h3,
            "b63bb5ce981e598f8925bfd833dd53e211af53a6ca642eb6b8629f1a7652a813"
        );
        assert!(
            plan_hash(
                &json!({}),
                Some(&PlanFiles::Attachments(vec![dir.join("missing")]))
            )
            .is_err()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plan_matches_prefix_and_sha256_label() {
        let h = "ec190c09ebe8fc9dd364c963b9e9796dc7464710468c7b5e0a8518d64f46f98e";
        assert!(plan_matches(h, h));
        assert!(plan_matches("ec190c09ebe8", h));
        assert!(plan_matches("  EC190C09EBE8fc ", h));
        assert!(plan_matches("sha256:ec190c09ebe8", h));
        assert!(plan_matches("SHA256:EC190C09EBE8", h));
        assert!(!plan_matches("ec190c09ebe", h));
        assert!(!plan_matches("sha256:ec190c09ebe", h));
        assert!(!plan_matches("ec190c09ebe9", h));
        assert!(!plan_matches("", h));
    }

    #[test]
    fn metadata_fingerprint_matches_python() {
        let fields = json!({"fields": [
            {"updated_at": "2026-01-02T03:04:05Z"},
            {"updated_at": "2026-01-02T10:04:05+07:00"}
        ]});
        let rules =
            json!({"business_rules": [{"updated_at": "2026-03-01 10:00:00"}], "user": {"x": 1}});
        let fp = metadata_fingerprint(Some(&fields), Some(&rules), None);
        assert_eq!(
            fp.hash,
            "89a19c2a883c9644a96c3b99e2e290bca5a48b36e9c05c839076dc1f79b8f117"
        );
        assert_eq!(fp.updated_at.as_deref(), Some("2026-03-01T10:00:00+00:00"));
        assert_eq!(
            fp.counts,
            vec![("form_fields".to_string(), 2), ("rules".to_string(), 1)]
        );
        assert!(!fp.changed(&fp.clone()));
        assert_eq!(metadata_fingerprint(None, None, None).updated_at, None);
    }
}

#[cfg(test)]
mod py_tests;
