use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use serde_json::{Value as Json, json};

use fsv_api::catalog::{self, ItemMeta, Recipients, SRPlan};
use fsv_forms::LookupValue;

use super::*;
use crate::drafts::DRAFT_KEYS;

static GOLDEN: LazyLock<Json> = LazyLock::new(|| {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/sr.json");
    serde_json::from_str(&std::fs::read_to_string(p).expect("golden file")).expect("golden json")
});

const TENANT: &str = "acme.freshservice.com";

fn item_meta(key: &str) -> ItemMeta {
    let g = &GOLDEN["items"][key];
    ItemMeta::new(
        g["item"].clone(),
        g["rules"].clone(),
        GOLDEN["workspace_id"].clone(),
        GOLDEN["fetched_at"].as_f64().unwrap(),
        "server",
    )
    .expect("item meta")
}

fn base() -> &'static Path {
    Path::new(GOLDEN["base"].as_str().unwrap())
}

fn lookup(v: &Json) -> Option<LookupValue> {
    (!v.is_null()).then(|| LookupValue {
        id: v["id"].clone(),
        display: v["display"].as_str().unwrap().to_string(),
        email: v["email"].as_str().map(str::to_string),
    })
}

fn draft_json(d: &SrDraft) -> Json {
    let x = &d.draft;
    let planning: serde_json::Map<String, Json> = x
        .planning
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                json!({"files": v.files, "description_html": v.description_html}),
            )
        })
        .collect();
    json!({"answers": x.answers, "requester": x.requester, "requested_for": d.requested_for,
           "description_html": x.description_html, "attachments": x.attachments, "planning": planning,
           "ack": x.ack, "preset": x.preset,
           "base_dir": x.base_dir.as_ref().map(|b| b.to_string_lossy().into_owned())})
}

/// Exact, key order included.
fn same(got: &Json, want: &Json, ctx: &str) {
    assert_eq!(
        serde_json::to_string_pretty(got).unwrap(),
        serde_json::to_string_pretty(want).unwrap(),
        "{ctx}"
    );
}

fn plan_for(case: &Json) -> (SRPlan, SrDraft) {
    let meta = item_meta(case["item"].as_str().unwrap());
    let mut d = parse_sr_template(case["draft"].as_str().unwrap(), Some(base())).unwrap();
    let spec = meta.spec();
    d.draft.answers = file_answers(&spec, &d.draft.answers, d.draft.base_dir.as_deref());
    let req = lookup(&case["requester"]);
    let plan = catalog::build_sr_plan(
        &meta,
        &d.catalog_draft(),
        None,
        req.as_ref(),
        case["ack"].as_bool().unwrap(),
        &Recipients::default(),
        TENANT,
    )
    .unwrap();
    (plan, d)
}

#[test]
fn golden_templates_byte_for_byte() {
    let cases = GOLDEN["templates"].as_object().unwrap();
    assert!(cases.len() >= 10);
    for (name, case) in cases {
        let (plan, d) = plan_for(case);
        same(&json!(plan.errors), &case["plan_errors"], name);
        let errors = if case["with_errors"].as_bool().unwrap() {
            plan.errors.clone()
        } else {
            Vec::new()
        };
        let text = render_sr_template(
            &plan,
            &SrTemplateOptions {
                errors: &errors,
                requester: case["flag"].as_str(),
                draft: Some(&d),
                base_dir: case["base_dir"].as_str(),
            },
        )
        .unwrap();
        assert_eq!(text, case["template"].as_str().unwrap(), "{name}");
        let back = parse_sr_template(&text, Some(base())).unwrap();
        same(&draft_json(&back), &case["reparsed"], name);
    }
}

#[test]
fn golden_parse_cases() {
    for case in GOLDEN["parse"].as_array().unwrap() {
        let text = case["text"].as_str().unwrap();
        match parse_sr_template(text, Some(base())) {
            Ok(d) => same(&draft_json(&d), &case["draft"], text),
            Err(e) => assert_eq!(
                e.to_string(),
                case["error"].as_str().unwrap_or("<ok>"),
                "{text}"
            ),
        }
    }
}

#[test]
fn golden_resolve_precedence_and_errors() {
    let dir = tempfile::tempdir().unwrap();
    let tmp = dir.path().to_string_lossy().into_owned();
    for (rel, text) in GOLDEN["files"].as_object().unwrap() {
        let p = dir.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text.as_str().unwrap()).unwrap();
    }
    let cwd = dir.path().join("cwd");
    std::fs::create_dir_all(&cwd).unwrap();
    let presets = dir.path().join("config/presets");
    let spec = item_meta("feature").spec();
    let strs = |v: &Json| -> Vec<String> {
        v.as_array()
            .into_iter()
            .flatten()
            .map(|x| x.as_str().unwrap().to_string())
            .collect()
    };
    let cases = GOLDEN["resolve"].as_array().unwrap();
    assert!(cases.len() >= 20);
    for case in cases {
        let file = case["file"].as_str().map(|f| {
            if f.is_empty() {
                String::new()
            } else {
                format!("{tmp}/{f}")
            }
        });
        let preset = case["preset"].as_str().map(|p| p.replace("<TMP>", &tmp));
        let got = resolve_sr_draft_in(
            &spec,
            file.as_deref(),
            preset.as_deref(),
            &strs(&case["sets"]),
            &strs(&case["attach"]),
            &presets,
            &cwd,
        );
        let ctx = case.to_string();
        match got {
            Ok(d) => {
                let j: Json =
                    serde_json::from_str(&draft_json(&d).to_string().replace(&tmp, "<TMP>"))
                        .unwrap();
                same(&j, &case["draft"], &ctx);
            }
            Err(e) => assert_eq!(
                e.to_string().replace(&tmp, "<TMP>"),
                case["error"].as_str().unwrap_or("<ok>"),
                "{ctx}"
            ),
        }
    }
}

#[test]
fn sr_keys_extend_draft_keys() {
    assert_eq!(&SR_DRAFT_KEYS[..8], &DRAFT_KEYS[..]);
    assert_eq!(SR_DRAFT_KEYS[8], "requested_for");
}

#[test]
fn merged_keeps_requested_for_unless_overridden() {
    let a = parse_sr_template("requested_for: target@example.com\n", Some(base())).unwrap();
    let b = parse_sr_template("answers: {}\n", Some(base())).unwrap();
    assert_eq!(a.merged(&b).requested_for, a.requested_for);
    let c = parse_sr_template("requested_for: other@example.com\n", Some(base())).unwrap();
    assert_eq!(
        a.merged(&c).requested_for.as_deref(),
        Some("other@example.com")
    );
}

#[test]
fn catalog_draft_carries_answers_files_planning_ack() {
    let d = parse_sr_template(
        "answers: {Title: x}\nattachments: [a.pdf]\nack: [all]\nplanning:\n  P: [f.txt]\n",
        Some(base()),
    )
    .unwrap();
    let c = d.catalog_draft();
    assert_eq!(Json::Object(c.answers), json!({"Title": "x"}));
    assert_eq!(c.attachments, ["/work/drafts/a.pdf"]);
    assert_eq!(c.ack, ["all"]);
    assert_eq!(
        Json::Object(c.planning),
        json!({"P": {"files": ["/work/drafts/f.txt"], "description_html": null}})
    );
}

#[test]
fn reveals_maps_choices_to_labels() {
    let meta = item_meta("feature");
    let spec = meta.spec();
    let rv = reveals(&spec, spec.get("goal").unwrap());
    assert_eq!(
        rv.get(&norm("More revenue  (Total USD)")).unwrap(),
        &["1.1 Revenue target"]
    );
    assert_eq!(rv.get(&norm("Lower cost")).unwrap(), &["1.2 Cost target"]);
    assert!(reveals(&spec, spec.get("priority").unwrap()).is_empty());
}

#[test]
fn save_sr_draft_writes_paths_relative_to_the_file() {
    let case = &GOLDEN["templates"]["feature_full"];
    let (plan, d) = plan_for(case);
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().canonicalize().unwrap();
    let path = dir.join("sr-draft.yaml");
    let written = save_sr_draft(&plan, &d, &path.to_string_lossy()).unwrap();
    assert_eq!(written, path.to_string_lossy());
    let text = std::fs::read_to_string(&path).unwrap();
    let back = parse_sr_template(&text, Some(&dir)).unwrap();
    let files: Vec<String> = back
        .draft
        .attachments
        .iter()
        .map(|p| crate::drafts::normpath(p))
        .collect();
    assert_eq!(files, ["/work/drafts/brief.pdf", "/abs/spec.pdf"]);
    assert_eq!(back.requested_for.as_deref(), Some("sam.leeds@example.com"));
    assert_eq!(
        back.draft.requester.as_deref(),
        Some("jane.doe@example.com")
    );
}

#[test]
fn resolve_sr_draft_uses_the_current_directory() {
    let spec = item_meta("feature").spec();
    let d = resolve_sr_draft(&spec, None, None, &["Title=x".into()], &["a.pdf".into()]).unwrap();
    let cwd = std::env::current_dir().unwrap();
    assert_eq!(d.draft.attachments, [cwd.join("a.pdf").to_string_lossy()]);
    assert!(load_sr_preset("definitely-not-a-preset-name").is_err());
}
