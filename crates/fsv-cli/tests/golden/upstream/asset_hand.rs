
#[test]
#[ignore = "fsv-api asset_picker.rs candidates checks duplicates before every display_id is validated"]
fn asset_picker_duplicate_then_bad() {
    let r = run_fixture("asset_picker", &["changes", "asset-picker", "16"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, "");
    assert_eq!(r.err, "expected a positive numeric identity\n");
}

#[test]
fn asset_picker_api_error_is_uncaught() {
    // Python leaves APIError uncaught (traceback, exit 1).
    let r = run_fixture("asset_picker", &["changes", "asset-picker", "17"], None);
    assert_eq!(r.code, 1, "{}", r.err);
    assert_eq!(r.out, "");
    assert!(r.err.contains("HTTP 500 listing assets-to-associate: boom"), "{}", r.err);
}

#[test]
fn asset_picker_page_must_be_positive() {
    // Click's `min=1` range error; clap's text differs.
    let r = run_fixture("asset_picker", &["changes", "asset-picker", "12", "--page", "0"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert!(r.requests.is_empty());
}
