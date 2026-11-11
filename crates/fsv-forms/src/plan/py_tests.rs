//! `fsv.forms.plan` helpers against the Python reference.

use std::path::PathBuf;

use indexmap::IndexMap;
use serde_json::{Value as Json, json};

use super::{PlanFiles, canonical_json, file_digest, file_entries, parse_ts, walk_updated};
use crate::py_golden::{check, g, obj};

fn files_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fsv-forms-py-plan-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(dir.join("a.txt"), b"hello\n".repeat(1000)).expect("write");
    std::fs::write(dir.join("empty.bin"), b"").expect("write");
    let big: Vec<u8> = (0..=255u8).collect::<Vec<_>>().repeat(300);
    std::fs::write(dir.join("big.bin"), big).expect("write");
    dir
}

fn entries_json(entries: &[super::FileEntry]) -> Json {
    Json::Array(
        entries
            .iter()
            .map(|e| {
                json!({
                    "field": e.field, "index": e.index, "name": e.digest.name,
                    "size": e.digest.size, "sha256": e.digest.sha256,
                })
            })
            .collect(),
    )
}

#[test]
fn canonical_json_matches_python() {
    let docs = g(&["plan", "canonical", "docs"]);
    for (k, want) in obj(g(&["plan", "canonical", "out"])) {
        check(k, &json!(canonical_json(&docs[k])), want);
    }
}

#[test]
fn file_digest_and_entries_match_python() {
    let dir = files_dir();
    for (name, want) in obj(g(&["plan", "digests"])) {
        let d = file_digest(dir.join(name)).expect("digest");
        check(
            name,
            &json!({"name": d.name, "size": d.size, "sha256": d.sha256}),
            want,
        );
    }
    assert!(file_digest(dir.join("missing")).is_err());

    let mut groups = IndexMap::new();
    groups.insert("zeta".to_string(), vec![dir.join("a.txt")]);
    groups.insert(
        "alpha".to_string(),
        vec![dir.join("big.bin"), dir.join("empty.bin")],
    );
    let got = file_entries(Some(&PlanFiles::Fields(groups))).expect("entries");
    check(
        "fields",
        &entries_json(&got),
        g(&["plan", "entries_fields"]),
    );

    let list = PlanFiles::Attachments(vec![dir.join("empty.bin"), dir.join("a.txt")]);
    let got = file_entries(Some(&list)).expect("entries");
    check(
        "attachments",
        &entries_json(&got),
        g(&["plan", "entries_list"]),
    );
    assert!(file_entries(None).expect("none").is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn parse_ts_matches_python() {
    for (s, want) in obj(g(&["plan", "parse_ts"])) {
        let got = parse_ts(s)
            .map(|t| json!(t.isoformat()))
            .unwrap_or(Json::Null);
        check(s, &got, want);
    }
}

#[test]
fn walk_updated_matches_python() {
    let mut out = Vec::new();
    walk_updated(g(&["plan", "walk_doc"]), &mut out);
    let got: Vec<String> = out.iter().map(|t| t.isoformat()).collect();
    check("walk", &json!(got), g(&["plan", "walk"]));
}
