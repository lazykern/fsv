//! Every Python module at the pinned upstream commit has a Rust counterpart
//! listed in `port/module_map.json`, and every listed file is still in the tree.
//! When the sibling `fsv` checkout is present, its `src/fsv` tree must match
//! the map exactly.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn python_modules(root: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("py") {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(rel);
            }
        }
    }
    out
}

#[test]
fn every_python_module_has_a_rust_counterpart() {
    let root = repo_root();
    let raw = std::fs::read_to_string(root.join("port/module_map.json")).unwrap();
    let map: Value = serde_json::from_str(&raw).unwrap();
    let pinned = std::fs::read_to_string(root.join("port/UPSTREAM_BASE")).unwrap();
    assert_eq!(map["upstream"].as_str().unwrap(), pinned.trim());

    let modules = map["modules"].as_array().expect("modules");
    let mut listed = BTreeSet::new();
    for module in modules {
        let py = module["py"].as_str().expect("py");
        assert!(listed.insert(py.to_string()), "duplicate {py}");
        let rust = module["rust"].as_array().expect("rust");
        assert!(!rust.is_empty(), "{py} has no Rust file");
        for path in rust {
            let rel = path.as_str().expect("rust path");
            let full = root.join(rel);
            assert!(full.is_file(), "{py} -> {rel} is not a file");
        }
    }

    let live = root.join("../fsv/src/fsv");
    if live.is_dir() {
        let found = python_modules(&live);
        let extra: Vec<_> = found.difference(&listed).cloned().collect();
        let gone: Vec<_> = listed.difference(&found).cloned().collect();
        assert!(
            extra.is_empty() && gone.is_empty(),
            "module map drifted from {live:?}\nmissing from map: {extra:?}\ngone from Python: {gone:?}"
        );
    }
}
