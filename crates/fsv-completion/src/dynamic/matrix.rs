//! Upstream `tests/test_completion_matrix.py` and
//! `tests/test_completion_coverage_subprocess.py`, replayed from
//! `tests/fixtures/matrix_expected.json` (`tests/golden/gen_matrix.py`).
//!
//! Each scenario needs its own config and cache dirs, and `fsv_core::config`
//! is process-global, so the parent test writes a scenario's files and runs
//! this test binary again filtered to `child`, which loads them and compares
//! every call.

use std::cell::RefCell;
use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};

use super::*;

const EXPECTED: &str = include_str!("../../tests/fixtures/matrix_expected.json");
const SCENARIO_VAR: &str = "FSV_MATRIX_SCENARIO";

fn expected() -> Value {
    serde_json::from_str(EXPECTED).expect("matrix fixture")
}

/// `client.get_client()` stand-in: a failing client answers `None`, as
/// `LiveRemote` does for any client or request error.
struct MatrixRemote {
    remote: Value,
    calls: RefCell<Vec<Value>>,
}

impl MatrixRemote {
    fn rows(&self, key: &str) -> Option<Vec<Value>> {
        if self.remote["fail"].as_bool() == Some(true) {
            return None;
        }
        Some(self.remote[key].as_array().cloned().unwrap_or_default())
    }

    fn record(&self, call: Value) {
        if self.remote["fail"].as_bool() != Some(true) {
            self.calls.borrow_mut().push(call);
        }
    }
}

impl Remote for MatrixRemote {
    fn autocomplete(&self, kind: &str, query: &str, params: Option<Obj>) -> Option<Vec<Value>> {
        self.record(json!([
            "autocomplete",
            kind,
            query,
            params.map_or(Value::Null, Value::Object)
        ]));
        self.rows("autocomplete")
    }

    fn lookup_choices(&self, link: &str, query: &str) -> Option<Vec<Value>> {
        self.record(json!(["lookup_choices", link, query]));
        self.rows("lookup")
    }
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .expect("args")
        .iter()
        .map(|s| s.as_str().expect("string arg").to_string())
        .collect()
}

fn pairs(p: Pairs) -> Value {
    p.into_iter().map(|(a, b)| json!([a, b])).collect()
}

fn objects(v: Vec<Obj>) -> Value {
    v.into_iter().map(Value::Object).collect()
}

fn res(name: &str) -> &'static Resource {
    resource::lookup(name).unwrap_or_else(|| panic!("no resource {name}"))
}

fn run_case(case: &Value) -> (Value, Vec<Value>) {
    let args = strings(&case["args"]);
    let remote = MatrixRemote {
        remote: case["remote"].clone(),
        calls: RefCell::new(Vec::new()),
    };
    let env = Env {
        network: case["network"].as_bool().unwrap_or(false),
        remote: &remote,
    };
    let got = match case["fn"].as_str().expect("fn") {
        "thin" => run_in(&env, &args).lines().map(|l| json!(l)).collect(),
        "complete" => pairs(complete_in(&env, &args[0], &args[1], &args[2], &args[3..])),
        "cached_fields" => objects(cached_fields(res(&args[0]))),
        "cached_filters" => pairs(cached_filters(res(&args[0]))),
        "cached_groups" => objects(cached_groups()),
        "find_field" => find_field(res(&args[0]), &args[1]).map_or(Value::Null, Value::Object),
        "complete_update_choice" => {
            pairs(complete_update_choice(res(&args[0]), &args[1], &args[2]))
        }
        "remote_user_values" => pairs(env.remote_user_values(&args[0], &args[1])),
        "network" => json!(crate::network::completion_network(
            case["env"].as_str(),
            &crate::network::config_load(&config::config_dir()),
        )),
        "config_load" => crate::network::config_load(&config::config_dir()),
        other => panic!("unknown fn {other}"),
    };
    (got, remote.calls.into_inner())
}

/// Runs only when the parent test sets [`SCENARIO_VAR`].
#[test]
fn child() {
    let Ok(name) = std::env::var(SCENARIO_VAR) else {
        return;
    };
    config::reload();
    let doc = expected();
    let scenario = doc["scenarios"]
        .as_array()
        .expect("scenarios")
        .iter()
        .find(|s| s["name"] == name.as_str())
        .unwrap_or_else(|| panic!("no scenario {name}"));
    let mut bad = Vec::new();
    for case in scenario["cases"].as_array().expect("cases") {
        let (got, calls) = run_case(case);
        if got != case["want"] || Value::Array(calls.clone()) != case["calls"] {
            bad.push(format!(
                "{} {}\n  python: {} calls {}\n  rust:   {} calls {}",
                case["fn"],
                case["args"],
                case["want"],
                case["calls"],
                got,
                Value::Array(calls)
            ));
        }
    }
    assert!(bad.is_empty(), "{name}:\n{}", bad.join("\n"));
}

fn write_files(root: &Path, files: &Value) {
    std::fs::create_dir_all(root.join("cfg")).expect("mkdir cfg");
    for (rel, text) in files.as_object().expect("files") {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, text.as_str().expect("file text")).expect("write");
    }
}

#[test]
fn scenarios_match_python() {
    let doc = expected();
    let exe = std::env::current_exe().expect("test binary");
    let mut failed = Vec::new();
    let scenarios = doc["scenarios"].as_array().expect("scenarios");
    for scenario in scenarios {
        let name = scenario["name"].as_str().expect("name");
        let dir = tempfile::tempdir().expect("tempdir");
        write_files(dir.path(), &scenario["files"]);
        let out = Command::new(&exe)
            .args(["--exact", "dynamic::matrix::child", "--nocapture"])
            .env(SCENARIO_VAR, name)
            .env("FSV_CONFIG_DIR", dir.path().join("cfg"))
            .env("FSV_CACHE_DIR", dir.path().join("cache"))
            .env_remove("FSV_COMPLETION_NETWORK")
            .output()
            .expect("run child");
        let stdout = String::from_utf8_lossy(&out.stdout);
        if !out.status.success() || !stdout.contains("1 passed") {
            failed.push(format!("{stdout}{}", String::from_utf8_lossy(&out.stderr)));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
    assert_eq!(scenarios.len(), 20);
}
