//! `fsv.formui.when` helpers against the Python reference (today =
//! 2026-11-12).

use jiff::civil::date;
use serde_json::{Value as Json, json};

use super::{
    Parsed, WhenOptions, month, offset_text, parse_date, parse_when, split_time, tz_phrase,
};
use crate::py_golden::{check, g, obj, run_str};
use crate::pydt::{DateTimeValue, date_iso};
use crate::tz::zone;

#[test]
fn month_matches_python() {
    for (word, want) in obj(g(&["when", "month"])) {
        check(word, &json!(month(word)), want);
    }
}

#[test]
fn parse_date_matches_python() {
    let today = date(2026, 11, 12);
    for (text, want) in obj(g(&["when", "parse_date"])) {
        let got = run_str(parse_date(text, today), |d| {
            d.map(|d| json!(date_iso(d))).unwrap_or(Json::Null)
        });
        check(text, &got, want);
    }
}

#[test]
fn split_time_matches_python() {
    for (text, want) in obj(g(&["when", "split_time"])) {
        let got = run_str(split_time(text), |(rest, time, off)| {
            json!([rest, time.map(|t| t.to_string()), off.map(|o| o.seconds())])
        });
        check(text, &got, want);
    }
}

#[test]
fn tz_phrase_matches_python() {
    for (src, want) in obj(g(&["when", "tz_phrase"])) {
        check(src, &json!(tz_phrase(src)), want);
    }
}

#[test]
fn parsed_ok_matches_python() {
    let bkk = zone("Asia/Bangkok").expect("zone");
    for (text, want) in obj(g(&["when", "parsed_ok"])) {
        let p: Parsed = parse_when(
            text,
            &bkk,
            WhenOptions {
                tz_source: "FSV_TZ",
                date_only: false,
                today: Some(date(2026, 11, 12)),
            },
        );
        check(text, &json!([p.ok(), p.error]), want);
    }
}

#[test]
fn offset_text_matches_python() {
    for (secs, want) in obj(g(&["tz", "offsets"])) {
        let off = jiff::tz::Offset::from_seconds(secs.parse().expect("secs")).expect("offset");
        let v = DateTimeValue::new(date(2026, 1, 1).at(0, 0, 0, 0), off);
        check(secs, &json!(offset_text(&v)), &want[2]);
    }
}
