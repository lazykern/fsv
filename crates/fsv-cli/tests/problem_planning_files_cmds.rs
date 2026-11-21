//! Problem analysis-section files on the captured change wire (Python
//! 6cd384b, `tests/test_problem_planning_files.py`).

mod common;
use common::{Env, Run, get, route};
use serde_json::{Value, json};

const PID: i64 = 47;

fn impact_row() -> Value {
    json!({"id": 3, "name": "problem_impact", "description": "<div>Acme impact</div>",
           "attachments": [{"id": 611, "name": "impact.txt", "size": 5,
                            "attachment_url": "https://acme.freshservice.com/helpdesk/attachments/611"}]})
}

fn planning(rows: Value) -> Value {
    json!({"method": "GET", "path": format!("/api/_/problems/{PID}/planning-fields"),
           "json": {"problem_planning_fields": rows}})
}

fn once(mut r: Value) -> Value {
    r["once"] = json!(true);
    r
}

fn base_routes() -> Vec<Value> {
    vec![
        json!({"method": "GET", "path": "/a/dashboard",
               "text": "<html><head><meta name=\"csrf-token\" content=\"tok\"></head></html>"}),
        get(
            "problem_form_fields",
            json!({"attachment_settings": {"max_attachment_size_bytes": 4096}}),
        ),
        get(
            &format!("problems/{PID}"),
            json!({"problem": {"id": PID, "updated_at": "2026-01-01T00:00:00Z"}}),
        ),
    ]
}

fn env_with(mut routes: Vec<Value>, files: &[(&str, &[u8])]) -> Env {
    let mut all = base_routes();
    all.append(&mut routes);
    let env = Env::new(all);
    let work = env.home().join("work");
    std::fs::create_dir_all(&work).unwrap();
    for (n, b) in files {
        std::fs::write(work.join(n), b).unwrap();
    }
    env
}

fn run(env: &mut Env, args: &[&str]) -> Run {
    env.cwd = Some(env.home().join("work"));
    env.stdin = Some(String::new());
    env.run(args)
}

fn update(extra: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = ["problems", "update", "PRB-47"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    v.extend(extra.iter().map(|s| s.to_string()));
    v
}

fn go(env: &mut Env, extra: &[&str]) -> Run {
    let args = update(extra);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run(env, &refs)
}

fn first_write_env(readback: Value) -> Env {
    env_with(
        vec![
            once(planning(json!([impact_row()]))),
            once(planning(json!([impact_row()]))),
            planning(readback),
            route(
                "POST",
                "/api/_/attachments",
                200,
                json!({"id": 801, "name": "cause.txt", "size": 11}),
            ),
            route(
                "POST",
                &format!("/api/_/problems/{PID}/planning-fields"),
                200,
                json!({}),
            ),
        ],
        &[("cause.txt", b"Acme cause\n")],
    )
}

fn cause_args<'a>() -> Vec<&'a str> {
    vec![
        "--planning",
        "Root Cause",
        "--description",
        "<div>Acme cause</div>",
        "--file",
        "cause.txt",
    ]
}

#[test]
fn first_write_posts_the_section_with_the_uploaded_file_and_reads_back() {
    let readback = json!([impact_row(),
        {"id": 9, "name": "problem_cause", "description": "<div>Acme cause</div>",
         "attachments": [{"id": 801, "name": "cause.txt", "size": 11}]}]);
    let mut e = first_write_env(readback.clone());
    let mut args = cause_args();
    args.push("--dry-run");
    let review = go(&mut e, &args);
    assert_eq!(review.code, 0, "{}", review.err);
    let out = review.json();
    assert_eq!(out["action"], "problem_planning_update");
    assert_eq!(out["section"], "problem_cause");
    assert_eq!(out["steps"][0]["path"], "/api/_/attachments");
    assert_eq!(
        out["steps"][1]["path"],
        "/api/_/problems/47/planning-fields?id=problem_cause"
    );
    let hash = out["plan_hash"].as_str().unwrap().to_string();

    let mut e = first_write_env(readback);
    let mut args = cause_args();
    args.extend(["--yes", "--expect-plan", &hash]);
    let sent = go(&mut e, &args);
    assert_eq!(sent.code, 0, "{}", sent.err);
    let result = sent.json();
    assert_eq!(result["verified"], true, "{result}");
    assert_eq!(result["uploaded_ids"], json!([801]));
    let w = sent.writes();
    assert_eq!(w[0].2["content_file_name"], "cause.txt");
    assert_eq!(w[0].2["content_file_size"], 11);
    assert_eq!(
        (w[1].0.as_str(), w[1].1.as_str(), w[1].2.clone()),
        (
            "POST",
            "/api/_/problems/47/planning-fields?id=problem_cause",
            json!({"description": "<div>Acme cause</div>", "attachments": [801]})
        )
    );
    // the journal in this home refuses the same plan
    let again = go(&mut e, &args);
    assert_eq!(again.code, 2, "{}", again.err);
    assert!(again.write_paths().is_empty());
}

#[test]
fn size_limit_unknown_section_and_other_fields_refuse_before_any_upload() {
    let mut e = env_with(
        vec![planning(json!([impact_row()]))],
        &[("big.bin", &[b'x'; 5000]), ("cause.txt", b"x")],
    );
    for extra in [
        vec![
            "--planning",
            "Root Cause",
            "--description",
            "x",
            "--file",
            "big.bin",
            "--dry-run",
        ],
        vec![
            "--planning",
            "Workaround",
            "--description",
            "x",
            "--dry-run",
        ],
        vec![
            "--planning",
            "Root Cause",
            "--description",
            "x",
            "--priority",
            "High",
            "--dry-run",
        ],
        vec!["--description", "x", "--dry-run"],
    ] {
        let r = go(&mut e, &extra);
        assert_eq!(r.code, 2, "{extra:?}: {}", r.err);
        assert!(r.write_paths().is_empty());
    }
    let r = go(&mut e, &["--max-bytes", "5", "--dry-run"]);
    assert_eq!(r.code, 2);
    assert!(
        r.err
            .contains("--max-bytes applies to problems --planning --file"),
        "{}",
        r.err
    );
}

#[test]
fn planning_options_are_refused_on_tickets() {
    let mut e = env_with(vec![], &[]);
    let r = run(
        &mut e,
        &[
            "tickets",
            "update",
            "INC-5",
            "--planning",
            "Root Cause",
            "--description",
            "x",
        ],
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert!(r.err.contains("supported only for problems"), "{}", r.err);
    let r = run(&mut e, &["tickets", "download", "INC-5", "--all-planning"]);
    assert_eq!(r.code, 2, "{}", r.err);
    assert!(
        r.err
            .contains("for problems; use `fsv changes download --planning`"),
        "{}",
        r.err
    );
}

#[test]
fn download_all_planning_lists_every_analysis_section() {
    let mut e = env_with(
        vec![
            planning(json!([impact_row()])),
            json!({"method": "GET", "path": "/helpdesk/attachments/611", "text": "12345"}),
        ],
        &[],
    );
    let r = run(
        &mut e,
        &[
            "problems",
            "download",
            "PRB-47",
            "--all-planning",
            "--json",
            "--max-bytes",
            "4096",
            "--dir",
            "out",
        ],
    );
    assert_eq!(r.code, 0, "{}", r.err);
    let rows = r.json();
    let sources: Vec<&str> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["source"].as_str().unwrap())
        .collect();
    assert!(sources.contains(&"planning:problem_cause"), "{sources:?}");
    assert!(sources.contains(&"planning:problem_impact"));
    assert!(sources.contains(&"planning:problem_symptom"));
}

fn delete_env(keep_text: bool) -> Env {
    let after = if keep_text {
        json!([{"id": 3, "name": "problem_impact", "description": "<div>Acme impact</div>", "attachments": []}])
    } else {
        json!([{"id": 3, "name": "problem_impact", "description": "", "attachments": []}])
    };
    env_with(
        vec![
            once(planning(json!([impact_row()]))),
            once(planning(json!([impact_row()]))),
            once(planning(json!([impact_row()]))),
            planning(after),
            route("DELETE", "/api/_/attachments/611", 204, json!(null)),
        ],
        &[],
    )
}

fn delete_args<'a>() -> Vec<&'a str> {
    vec![
        "attachments",
        "delete",
        "611",
        "--owner",
        "problems/47",
        "--section",
        "problem_impact",
    ]
}

fn delete_hash() -> String {
    let mut e = delete_env(true);
    let mut args = delete_args();
    args.push("--dry-run");
    let r = run(&mut e, &args);
    assert_eq!(r.code, 0, "{}", r.err);
    r.json()["plan_hash"].as_str().unwrap().to_string()
}

#[test]
fn attachment_delete_on_a_problem_section_keeps_the_text() {
    let hash = delete_hash();
    let mut e = delete_env(true);
    let mut args = delete_args();
    args.extend(["--yes", "--expect-plan", &hash]);
    let r = run(&mut e, &args);
    assert_eq!(r.code, 2, "{}", r.err);
    assert!(r.write_paths().is_empty());
    args.push("--confirm-destructive");
    let mut e = delete_env(true);
    let done = run(&mut e, &args);
    assert_eq!(done.code, 0, "{}", done.err);
    let out = done.json();
    assert_eq!(out["verified"], true, "{out}");
    assert_eq!(out["section_text"], "kept");
    assert_eq!(done.write_paths(), ["DELETE /api/_/attachments/611"]);
}

#[test]
fn attachment_delete_is_unverified_when_the_section_text_is_lost() {
    let hash = delete_hash();
    let mut e = delete_env(false);
    let mut args = delete_args();
    args.extend(["--yes", "--expect-plan", &hash, "--confirm-destructive"]);
    let done = run(&mut e, &args);
    assert_eq!(done.code, 4, "{}{}", done.out, done.err);
    assert_eq!(done.json()["section_text"], "changed");
}
