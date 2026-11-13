//! Static function-level map from every Python def at the pinned upstream
//! commit to its Rust counterpart (`port/function_map.json`).
//!
//! The map is data, not a proof: `matched` entries are name matches that a
//! reviewer still has to check line by line. This test keeps the map honest:
//! it must be stamped to `port/UPSTREAM_BASE`, every Rust reference must still
//! exist, every Python module must be covered, and the open and unreviewed
//! queues may only shrink. `reviewed` entries were checked line by line and
//! carry a note on what was compared.

use serde_json::Value;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// Ratchet: the number of `open` entries may only go down.
const OPEN_CEILING: usize = 0;
/// Ratchet: `matched` (name match, not yet reviewed line by line) entries
/// may only go down as they become `reviewed`.
const UNREVIEWED_CEILING: usize = 265;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load() -> Value {
    let raw = std::fs::read_to_string(repo_root().join("port/function_map.json")).unwrap();
    serde_json::from_str(&raw).unwrap()
}

/// A Rust item declared in `text`: `fn name`, or a `struct`/`enum`/`trait`/`type`.
fn declares(text: &str, item: &str) -> bool {
    let needles = [
        format!("fn {item}("),
        format!("fn {item}<"),
        format!("struct {item}"),
        format!("enum {item}"),
        format!("trait {item}"),
        format!("type {item}"),
    ];
    needles.iter().any(|n| text.contains(n.as_str()))
}

#[test]
fn map_is_stamped_to_the_pinned_upstream() {
    let map = load();
    let pinned = std::fs::read_to_string(repo_root().join("port/UPSTREAM_BASE")).unwrap();
    assert_eq!(map["upstream"].as_str().unwrap(), pinned.trim());
}

#[test]
fn every_entry_has_a_known_status_and_its_reason() {
    let map = load();
    for e in map["entries"].as_array().unwrap() {
        let status = e["status"].as_str().unwrap();
        match status {
            "matched" => assert!(
                e["rust"].is_string() && e["rust_item"].is_string(),
                "matched entry needs rust and rust_item: {e}"
            ),
            "reviewed" => assert!(
                e["rust"].is_string()
                    && e["rust_item"].is_string()
                    && e["note"].as_str().is_some_and(|n| !n.is_empty()),
                "reviewed entry needs rust, rust_item and a note: {e}"
            ),
            "idiom" | "cli-tree" => assert!(
                e["note"].as_str().is_some_and(|n| !n.is_empty()),
                "{status} entry needs a note: {e}"
            ),
            "open" => {}
            other => panic!("unknown status {other}: {e}"),
        }
    }
}

#[test]
fn every_matched_reference_still_declares_its_item() {
    let map = load();
    let mut missing = Vec::new();
    for e in map["entries"].as_array().unwrap() {
        if e["status"] != "matched" && e["status"] != "reviewed" {
            continue;
        }
        let file = e["rust"].as_str().unwrap();
        let item = e["rust_item"].as_str().unwrap();
        match std::fs::read_to_string(repo_root().join(file)) {
            Ok(text) if declares(&text, item) => {}
            Ok(_) => missing.push(format!("{file}: no `{item}`")),
            Err(_) => missing.push(format!("{file}: file missing")),
        }
    }
    assert!(
        missing.is_empty(),
        "stale references:\n{}",
        missing.join("\n")
    );
}

#[test]
fn every_python_module_is_covered() {
    let map = load();
    let covered: BTreeSet<String> = map["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["module"].as_str().unwrap().to_string())
        .collect();
    let no_defs: BTreeSet<String> = map["no_defs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let modules = std::fs::read_to_string(repo_root().join("port/module_map.json")).unwrap();
    let modules: Value = serde_json::from_str(&modules).unwrap();
    let mut all = BTreeSet::new();
    for m in modules["modules"].as_array().unwrap() {
        let py = m["py"].as_str().unwrap().to_string();
        assert!(
            covered.contains(&py) != no_defs.contains(&py),
            "{py} must have entries or be listed in no_defs, not both or neither"
        );
        all.insert(py);
    }
    assert!(covered.is_subset(&all), "entries for unknown modules");
}

#[test]
fn open_queue_only_shrinks() {
    let map = load();
    let open = map["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["status"] == "open")
        .count();
    assert!(
        open <= OPEN_CEILING,
        "open parity items grew to {open} (ceiling {OPEN_CEILING})"
    );
}

#[test]
fn unreviewed_queue_only_shrinks() {
    let map = load();
    let unreviewed = map["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["status"] == "matched")
        .count();
    assert!(
        unreviewed <= UNREVIEWED_CEILING,
        "unreviewed matches grew to {unreviewed} (ceiling {UNREVIEWED_CEILING})"
    );
}
