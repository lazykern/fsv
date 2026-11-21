//! Form UI differential: the pure logic of `fsv-forms` and `fsv-formui`
//! against `tests/golden/formui.json` (from `gen_formui.py`, Python's
//! `fuzzy`, `when`, file checks and path completion).

use std::path::PathBuf;
use std::sync::LazyLock;

use fsv_forms::pydt::DateTimeValue;
use fsv_forms::when::{self, WhenOptions};
use fsv_forms::{Value, fuzzy, zone};
use fsv_formui::files::{FileRules, Paths, check_files, complete_path, human_size};
use serde_json::{Value as Json, json};

static GOLDEN: LazyLock<Json> = LazyLock::new(|| {
    serde_json::from_str(include_str!("golden/formui.json")).expect("formui.json")
});

fn strs(v: &Json) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn fuzzy_matches_python() {
    let g = &GOLDEN["fuzzy"];
    let items: Vec<(String, String)> = g["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p[0].as_str().unwrap().to_string(),
                p[1].as_str().unwrap().to_string(),
            )
        })
        .collect();
    for q in strs(&g["queries"]) {
        let want: Vec<usize> = g["filter"][&q]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_u64().unwrap() as usize)
            .collect();
        assert_eq!(fuzzy::fuzzy_filter(&q, &items), want, "filter {q:?}");
        let scores: Vec<Json> = items
            .iter()
            .map(|(l, d)| fuzzy::score(&q, l, d).map_or(Json::Null, |s| json!(s)))
            .collect();
        assert_eq!(Json::Array(scores), g["score"][&q], "score {q:?}");
    }
    for (text, want) in g["fold"].as_object().unwrap() {
        assert_eq!(fuzzy::fold(text), want.as_str().unwrap(), "fold {text:?}");
    }
}

fn when_json(v: &Value) -> (Json, &'static str) {
    match v {
        Value::Null => (Json::Null, "NoneType"),
        Value::Date(d) => (json!(d.to_string()), "date"),
        Value::DateTime(d) => (json!(d.isoformat()), "datetime"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn when_matches_python() {
    let g = &GOLDEN["when"];
    let today: jiff::civil::Date = g["today"].as_str().unwrap().parse().unwrap();
    let mut bad = Vec::new();
    for case in g["cases"].as_array().unwrap() {
        let tz = zone(case["tz"].as_str().unwrap()).unwrap();
        let text = case["text"].as_str().unwrap();
        let p = when::parse_when(
            text,
            &tz,
            WhenOptions {
                tz_source: "Freshservice user time zone (X)",
                date_only: case["date_only"].as_bool().unwrap(),
                today: Some(today),
            },
        );
        let (value, kind) = when_json(&p.value);
        let mut got = json!({"value": value, "kind": kind, "preview": p.preview, "error": p.error});
        if let Value::DateTime(_) = &p.value {
            got["to_text"] = json!(when::to_text(&p.value, Some(&tz)));
            got["to_text_no_tz"] = json!(when::to_text(&p.value, None));
        }
        if got != case["want"] {
            bad.push(format!("{case}\n   got {got}"));
        }
    }
    assert!(
        bad.is_empty(),
        "{} differ:\n{}",
        bad.len(),
        bad[..bad.len().min(12)].join("\n")
    );
}

#[test]
fn when_helpers_match_python() {
    let g = &GOLDEN["when"];
    for c in g["previews"].as_array().unwrap() {
        let v = match c["value"].as_str() {
            None => Value::Null,
            Some(s) if s.contains('T') => {
                let (local, off) = s.split_at(s.len() - 6);
                let sign = if off.starts_with('-') { -1 } else { 1 };
                let secs = sign
                    * (off[1..3].parse::<i32>().unwrap() * 3600
                        + off[4..6].parse::<i32>().unwrap() * 60);
                Value::DateTime(DateTimeValue::new(
                    local.parse().unwrap(),
                    jiff::tz::Offset::from_seconds(secs).unwrap(),
                ))
            }
            Some(s) => Value::Date(s.parse().unwrap()),
        };
        assert_eq!(
            when::preview(
                &v,
                c["source"].as_str().unwrap(),
                c["typed"].as_bool().unwrap()
            ),
            c["want"].as_str().unwrap(),
            "{c}"
        );
    }
    for (mins, want) in g["offsets"].as_object().unwrap() {
        let off = jiff::tz::Offset::from_seconds(mins.parse::<i32>().unwrap() * 60).unwrap();
        let v = DateTimeValue::new(jiff::civil::date(2026, 1, 1).at(0, 0, 0, 0), off);
        assert_eq!(when::offset_text(&v), want.as_str().unwrap(), "{mins}");
    }
    for (key, want) in g["weeks"].as_object().unwrap() {
        let p: Vec<i64> = key.split('-').map(|n| n.parse().unwrap()).collect();
        let got: Vec<Vec<String>> = when::month_weeks(p[0], p[1], p[2])
            .unwrap()
            .iter()
            .map(|w| w.iter().map(|d| d.to_string()).collect())
            .collect();
        assert_eq!(json!(got), *want, "{key}");
    }
    for (key, want) in g["months"].as_object().unwrap() {
        let (d, n) = key.split_once('+').unwrap();
        let got = when::add_months(d.parse().unwrap(), n.parse().unwrap()).unwrap();
        assert_eq!(got.to_string(), want.as_str().unwrap(), "{key}");
    }
}

fn file_tree() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap().join("files");
    std::fs::create_dir_all(root.join("docs/sub")).unwrap();
    std::fs::create_dir_all(root.join("docs/.hidden")).unwrap();
    for (rel, data) in [
        ("a.pdf", b"pdf".to_vec()),
        ("b.PNG", b"png".to_vec()),
        ("empty.pdf", vec![]),
        ("doc one.txt", b"x".to_vec()),
        ("docs/readme.md", b"r".to_vec()),
        ("docs/readme.txt", b"r".to_vec()),
        ("docs/.env", b"e".to_vec()),
        ("docs/sub/z.bin", b"zz".to_vec()),
        ("big.dat", vec![b'x'; 2_500_000]),
        ("ünï.pdf", b"u".to_vec()),
    ] {
        std::fs::write(root.join(rel), data).unwrap();
    }
    (dir, root)
}

fn raw_value(j: &Json) -> Value {
    match j {
        Json::Array(items) => Value::Json(Json::Array(items.clone())),
        Json::Null => Value::Null,
        Json::String(s) => Value::str(s.clone()),
        other => Value::from_json(other),
    }
}

#[test]
fn files_match_python() {
    let (_keep, root) = file_tree();
    let paths = Paths::new(Some(root.join("docs")), root.clone());
    let at = |s: &str| s.replace(root.to_str().unwrap(), "@ROOT@");
    let g = &GOLDEN["files"];
    let mut bad = Vec::new();
    for c in g["check"].as_array().unwrap() {
        let r = &c["rules"];
        let allowed: Vec<String> = r.get("allowed").map(strs).unwrap_or_default();
        let rules = FileRules {
            allowed: &allowed,
            max_files: r.get("max_files").and_then(Json::as_i64),
            max_bytes: r.get("max_bytes").and_then(Json::as_i64),
        };
        let (ps, err) = check_files(&raw_value(&c["raw"]), &rules, &paths);
        let got = json!([ps.iter().map(|p| at(p)).collect::<Vec<_>>(), at(&err)]);
        if got != c["want"] {
            bad.push(format!("{c}\n   got {got}"));
        }
    }
    assert!(
        bad.is_empty(),
        "{} differ:\n{}",
        bad.len(),
        bad[..bad.len().min(12)].join("\n")
    );
    for (v, want) in g["complete"].as_object().unwrap() {
        let got = complete_path(v, &paths).map(|s| at(&s));
        assert_eq!(got.as_deref(), want.as_str(), "complete {v:?}");
    }
    for (n, want) in g["sizes"].as_object().unwrap() {
        assert_eq!(
            human_size(n.parse().unwrap()),
            want.as_str().unwrap(),
            "size {n}"
        );
    }
    for (s, want) in g["quote"].as_object().unwrap() {
        assert_eq!(
            fsv_formui::session::shell_quote(s),
            want.as_str().unwrap(),
            "quote {s:?}"
        );
    }
}
