//! `fsv.forms.tz` against the Python reference.

use jiff::civil::DateTime;
use jiff::tz::Offset;
use serde_json::{Map, Value as Json, json};

use super::{
    TzEnv, Zone, localize, resolve_timezone, resolve_timezone_in, system_timezone, tz_name, zone,
};
use crate::py_golden::{check, g, obj, run, run_str};
use crate::pydt::{DateTimeValue, offset_text};

#[test]
fn system_timezone_matches_python() {
    for (tz, want) in obj(g(&["tz", "system_timezone"])) {
        let env = TzEnv {
            tz: Some(tz.clone()),
            // the generator monkeypatches os.path.realpath("/etc/localtime")
            localtime: Some("/usr/share/zoneinfo/Europe/Berlin".into()),
            ..TzEnv::default()
        };
        check(tz, &json!(tz_name(&system_timezone(&env))), want);
    }
}

#[test]
fn resolve_timezone_matches_python() {
    for (name, case) in obj(g(&["tz", "resolve_timezone"])) {
        let explicit = case["explicit"].as_str();
        let user: Option<Map<String, Json>> = case["user"].as_object().cloned();
        let env = TzEnv {
            fsv_tz: case["fsv_tz"].as_str().map(str::to_string),
            tz: Some("UTC".into()),
            localtime: None,
        };
        let project = |(z, src): (Zone, String)| json!([tz_name(&z), src]);
        check(
            name,
            &run(resolve_timezone_in(explicit, user.as_ref(), &env), project),
            &case["out"],
        );
        // an explicit zone wins over everything the real process has
        if explicit.is_some_and(|e| !e.is_empty()) {
            check(
                name,
                &run(resolve_timezone(explicit, user.as_ref()), project),
                &case["out"],
            );
        }
    }
}

#[test]
fn tz_name_matches_python() {
    for (name, want) in obj(g(&["tz", "tz_name"])) {
        let z = match name.as_str() {
            "fixed+7" => Zone::fixed(Offset::from_seconds(7 * 3600).unwrap()),
            "fixed-5:30" => Zone::fixed(Offset::from_seconds(-(5 * 3600 + 30 * 60)).unwrap()),
            "fixed0" => Zone::fixed(Offset::UTC),
            other => zone(other).expect("zone"),
        };
        check(name, &json!(tz_name(&z)), want);
    }
}

#[test]
fn localize_matches_python() {
    for (key, want) in obj(g(&["tz", "localize"])) {
        let (zname, wall) = key.split_once(' ').expect("zone wall");
        let naive: DateTime = wall.replace(' ', "T").parse().expect("wall time");
        let z = zone(zname).expect("zone");
        check(
            key,
            &run_str(localize(naive, &z), |v| json!(v.isoformat())),
            want,
        );
    }
}

/// Python's `tz._off` and `state._offset` are `pydt::offset_text`.
#[test]
fn off_and_offset_match_python() {
    for (secs, want) in obj(g(&["tz", "offsets"])) {
        let off = Offset::from_seconds(secs.parse().expect("secs")).expect("offset");
        check(&format!("_off {secs}"), &json!(offset_text(off)), &want[0]);
        let v = DateTimeValue::new(jiff::civil::date(2026, 1, 1).at(0, 0, 0, 0), off);
        check(
            &format!("_offset {secs}"),
            &json!(v.offset_text()),
            &want[1],
        );
    }
}
