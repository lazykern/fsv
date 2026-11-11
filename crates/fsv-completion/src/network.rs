//! `completion._config_load` and `completion._completion_network`, with the
//! config directory and the environment value passed in.

use std::path::Path;

use serde_json::{Map, Value};

/// Port of fsv.completion._config_load: `config.json` as parsed, `{}` when
/// it is missing or unreadable. A non-object document is returned as is.
pub fn config_load(config_dir: &Path) -> Value {
    std::fs::read_to_string(config_dir.join("config.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| Value::Object(Map::new()))
}

/// Port of fsv.completion._completion_network. `env` is
/// `FSV_COMPLETION_NETWORK`. A config whose `completion` is not an object
/// counts as off (Python raises `AttributeError`).
pub fn completion_network(env: Option<&str>, cfg: &Value) -> bool {
    if env == Some("1") {
        return true;
    }
    let Some(Value::Object(completion)) = cfg.get("completion") else {
        return false;
    };
    match completion.get("network") {
        Some(Value::String(s)) => matches!(s.as_str(), "on" | "true" | "1"),
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64() == Some(1.0),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXTRA: &str = include_str!("../tests/fixtures/extra_expected.json");

    fn extra() -> Value {
        serde_json::from_str(EXTRA).expect("extra fixture")
    }

    #[test]
    fn config_load_matches_python() {
        let dir = tempfile::tempdir().expect("tempdir");
        for row in extra()["config_load"].as_array().expect("rows") {
            let path = dir.path().join("config.json");
            match row[0].as_str() {
                Some(text) => std::fs::write(&path, text).expect("write"),
                None => {
                    let _ = std::fs::remove_file(&path);
                }
            }
            assert_eq!(config_load(dir.path()), row[1], "{row}");
        }
    }

    #[test]
    fn completion_network_matches_python() {
        for row in extra()["completion_network"].as_array().expect("rows") {
            let got = completion_network(row[0].as_str(), &row[1]);
            match &row[2] {
                Value::Bool(want) => assert_eq!(got, *want, "{row}"),
                _ => assert!(!got, "Python raised; Rust treats it as off: {row}"),
            }
        }
    }
}
