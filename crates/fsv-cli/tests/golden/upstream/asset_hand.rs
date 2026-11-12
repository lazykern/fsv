
#[test]
fn asset_picker_api_error_is_uncaught() {
    // Python leaves APIError uncaught (traceback, exit 1).
    let r = run_fixture(
        "asset_picker",
        &["changes", "asset-picker", "12", "--search", "boom"],
        None,
    );
    assert_eq!(r.code, 1, "{}", r.err);
    assert_eq!(r.out, "");
    assert!(r.err.contains("HTTP 500 listing device: boom"), "{}", r.err);
}

#[test]
fn asset_picker_page_must_be_positive() {
    // Click's `min=1` range error; clap's text differs.
    let r = run_fixture("asset_picker", &["changes", "asset-picker", "12", "--page", "0"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert!(r.requests.is_empty());
}
