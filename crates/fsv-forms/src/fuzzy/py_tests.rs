//! `fsv.formui.fuzzy._subsequence`.

use serde_json::json;

use super::subsequence;
use crate::py_golden::{check, g, obj};

#[test]
fn subsequence_matches_python() {
    for (k, want) in obj(g(&["fuzzy"])) {
        let (needle, hay) = k.split_once('|').expect("needle|hay");
        check(k, &json!(subsequence(needle, hay)), want);
    }
}
