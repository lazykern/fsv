
#[test]
fn canned_show_api_error_is_uncaught() {
    // Python leaves APIError uncaught (traceback, exit 1).
    let r = run_fixture("canned", &["tickets", "canned", "show", "706"], None);
    assert_eq!(r.code, 1, "{}", r.err);
    assert_eq!(r.out, "");
    assert!(r.err.contains("HTTP 404 fetching canned response: not found"), "{}", r.err);
}

#[test]
fn private_view_show_refusal_is_uncaught() {
    // `show` has no `try`: ReviewedWriteError is a traceback (exit 1), not exit 2.
    let r = run_fixture("private_views", &["tickets", "private-views", "show", "711"], None);
    assert_eq!(r.code, 1, "{}", r.err);
    assert_eq!(r.out, "");
    assert!(r.err.contains("exact saved-view identity is unavailable"), "{}", r.err);
}

#[test]
fn columns_list_api_error_is_uncaught() {
    let r = run_fixture("private_views", &["problems", "columns", "list"], None);
    assert_eq!(r.code, 1, "{}", r.err);
    assert_eq!(r.out, "");
    assert!(r.err.contains("HTTP 404 listing headers"), "{}", r.err);
}

#[test]
fn form_dump_problems() {
    // `taken_at` is the clock, so compare the rest of Python's snapshot.
    let r = run_fixture("problem_flows", &["form", "dump", "problems"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    let v: serde_json::Value = serde_json::from_str(&r.out).unwrap();
    let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        ["fsv_form_snapshot", "module", "form_type", "workspace_id", "taken_at", "fields", "business_rules"]
    );
    assert_eq!(v["module"], "problem");
    assert_eq!(v["form_type"], "new_form");
    assert_eq!(v["workspace_id"], serde_json::Value::Null);
    assert_eq!(v["fields"].as_array().unwrap().len(), 7);
}

#[test]
fn update_reviewed_multi_field_dry_run() {
    let r = run_fixture(
        "ticket_reviewed",
        &["tickets", "update", "5", "--reviewed", "--priority", "High", "--set", "Subject=Globex", "--set", "Cost Code=new", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(
        r.out.contains(
            r#"  "after": {
    "custom_fields": {
      "cost_code": "new"
    },
    "priority": 73,
    "subject": "Globex"
  },"#
        ),
        "{}",
        r.out
    );
}

#[test]
fn json_only_output_rejects_other_formats() {
    // Click's rich usage panel is not reproduced for clap value errors.
    let r = run_fixture("watchers", &["tickets", "watchers", "list", "5", "--output", "csv"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert!(r.requests.is_empty());
}
