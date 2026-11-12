//! Completer output against `fsv._complete` run on the same cache files.
//!
//! `tests/fixtures/dynamic_input.json` holds the cache documents, the fake
//! remote rows and the cases; `dynamic_expected.json` is what Python printed
//! for them (regenerate with `/tmp/fsv_port/completion_oracle.py`).

use std::cell::RefCell;
use std::path::Path;
use std::sync::OnceLock;

use serde_json::{Value, json};

use super::*;

const INPUT: &str = include_str!("../../tests/fixtures/dynamic_input.json");
const EXPECTED: &str = include_str!("../../tests/fixtures/dynamic_expected.json");

fn input() -> Value {
    serde_json::from_str(INPUT).expect("input fixture")
}

fn put(path: &Path, doc: &Value) {
    let mut doc = doc.as_object().cloned().expect("cache doc");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs_f64();
    doc.entry("saved_at").or_insert(json!(now));
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, Value::Object(doc).to_string()).expect("write cache");
}

/// Point the process config at a temp dir holding the fixture caches; done
/// once so parallel tests all see the same files.
pub(super) fn fixture_cache() {
    static DIR: OnceLock<tempfile::TempDir> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = tempfile::tempdir().expect("tempdir");
        let inp = input();
        let cache = dir.path().join("cache");
        for (name, doc) in inp["schema"].as_object().expect("schema") {
            put(
                &cache
                    .join("schema")
                    .join(format!("default--wsauto--{name}.json")),
                doc,
            );
        }
        for (name, doc) in inp["filters"].as_object().expect("filters") {
            put(
                &cache.join("filters").join(format!("default--{name}.json")),
                doc,
            );
        }
        put(&cache.join("groups--default.json"), &inp["groups"]);
        std::fs::create_dir_all(dir.path().join("cfg")).expect("mkdir cfg");
        // SAFETY: set once, before any test reads these variables.
        unsafe {
            std::env::set_var("FSV_CONFIG_DIR", dir.path().join("cfg"));
            std::env::set_var("FSV_CACHE_DIR", &cache);
        }
        config::reload();
        dir
    });
}

/// Records calls and answers from the fixture's `remote` rows.
pub(super) struct FakeRemote {
    rows: Value,
    pub(super) calls: RefCell<Vec<Value>>,
}

impl FakeRemote {
    pub(super) fn new() -> Self {
        FakeRemote {
            rows: input()["remote"].clone(),
            calls: RefCell::new(Vec::new()),
        }
    }

    fn rows(&self, key: &str) -> Option<Vec<Value>> {
        self.rows[key].as_array().cloned()
    }
}

impl Remote for FakeRemote {
    fn autocomplete(&self, kind: &str, query: &str, params: Option<Obj>) -> Option<Vec<Value>> {
        self.calls.borrow_mut().push(json!([
            "autocomplete",
            kind,
            query,
            params.map_or(Value::Null, Value::Object)
        ]));
        self.rows(kind)
    }

    fn lookup_choices(&self, link: &str, query: &str) -> Option<Vec<Value>> {
        self.calls
            .borrow_mut()
            .push(json!(["lookup_choices", link, query, null]));
        self.rows("lookup")
    }
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .expect("array")
        .iter()
        .map(|s| s.as_str().expect("string").to_string())
        .collect()
}

#[test]
fn completers_match_python() {
    fixture_cache();
    let expected: Value = serde_json::from_str(EXPECTED).expect("expected fixture");
    let mut failures = Vec::new();
    for (mode, network) in [("offline", false), ("network", true)] {
        for case in expected[mode]["cases"].as_array().expect("cases") {
            let c = &case["case"];
            let remote = FakeRemote::new();
            let env = Env {
                network,
                remote: &remote,
            };
            let got: Vec<Value> = complete_in(
                &env,
                c[0].as_str().expect("completer"),
                c[1].as_str().expect("resource"),
                c[2].as_str().expect("token"),
                &strings(&c[3]),
            )
            .into_iter()
            .map(|(v, h)| json!([v, h]))
            .collect();
            let got = Value::Array(got);
            if got != case["pairs"] {
                failures.push(format!(
                    "{mode} {c}\n  python: {}\n  rust:   {got}",
                    case["pairs"]
                ));
            }
            let calls = Value::Array(remote.calls.take());
            if calls != case["calls"] {
                failures.push(format!(
                    "{mode} {c} calls\n  python: {}\n  rust:   {calls}",
                    case["calls"]
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn run_matches_python_main() {
    fixture_cache();
    let expected: Value = serde_json::from_str(EXPECTED).expect("expected fixture");
    let mut failures = Vec::new();
    for (mode, network) in [("offline", false), ("network", true)] {
        for run in expected[mode]["runs"].as_array().expect("runs") {
            let remote = FakeRemote::new();
            let env = Env {
                network,
                remote: &remote,
            };
            let got = run_in(&env, &strings(&run["argv"]));
            if got != run["stdout"].as_str().expect("stdout") {
                failures.push(format!(
                    "{mode} {}\n  python: {:?}\n  rust:   {got:?}",
                    run["argv"], run["stdout"]
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
