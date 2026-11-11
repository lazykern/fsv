//! `fsv.forms.fetch.BusinessRulesQuery.path` / `.url`.

use serde_json::json;

use super::{BusinessRulesOptions, BusinessRulesQuery, blank_str, business_rules_query, null_str};
use crate::py_golden::{check, g, obj};

#[test]
fn business_rules_query_path_matches_python() {
    for (ws, want) in obj(g(&["fetch", "queries"])) {
        let id = match ws.parse::<i64>() {
            Ok(n) if ws != "7" => json!(n),
            _ => json!(ws),
        };
        let q: BusinessRulesQuery =
            business_rules_query("change", &BusinessRulesOptions::workspace(id)).expect("query");
        check(&format!("path {ws}"), &json!(q.path()), &want["path"]);
        check(
            &format!("url {ws}"),
            &json!(q.url("https://acme.example.com/")),
            &want["url"],
        );
    }
}

#[test]
fn blank_and_null_match_python() {
    for row in g(&["fetch", "blank"]).as_array().unwrap() {
        check(
            &format!("_blank {}", row[0]),
            &json!(blank_str(&row[0])),
            &row[1],
        );
        check(
            &format!("_null {}", row[0]),
            &json!(null_str(&row[0])),
            &row[2],
        );
    }
}
