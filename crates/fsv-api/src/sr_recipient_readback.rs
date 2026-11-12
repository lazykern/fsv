//! Project SR recipient readbacks without confusing user IDs with emails.
//!
//! Port of `sr_recipient_readback.recipient_fields`.

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::pystr::casefold;

use crate::watchers::positive_id;

/// Every available representation must agree. An email echo cannot overrule
/// a conflicting id or nested recipient.
pub fn recipient_fields(
    row: &Map<String, Value>,
    body: &Map<String, Value>,
    identity: &Map<String, Value>,
) -> Result<Map<String, Value>> {
    let mut fields = Map::new();
    for (key, party, id_key) in [
        ("email", "requester", "requester_id"),
        ("requested_for", "requested_for", "requested_for_id"),
    ] {
        let Some(want) = body.get(key) else {
            continue;
        };
        let expected_id = identity.get(id_key).filter(|v| !v.is_null());
        let names: &[&str] = if key == party { &[key] } else { &[key, party] };
        let mut values = Vec::new();
        for name in names {
            if let Some(value) = row.get(*name) {
                values.push(value.clone());
            }
        }
        if row.contains_key(id_key) && expected_id.is_some() {
            values.push(json!({"id": row[id_key].clone()}));
        }
        let mut matches = Vec::new();
        for value in &values {
            if let Some(text) = value.as_str() {
                let want = want
                    .as_str()
                    .ok_or_else(|| Error::value("recipient readback is not text"))?;
                matches.push(casefold(text) == casefold(want));
            } else if let Some(obj) = value.as_object() {
                if obj.contains_key("email") {
                    let Some(want) = want.as_str() else {
                        return Err(Error::value("recipient readback is not text"));
                    };
                    matches.push(
                        obj.get("email")
                            .and_then(Value::as_str)
                            .is_some_and(|email| casefold(email) == casefold(want)),
                    );
                }
                if obj.contains_key("id")
                    && let Some(expected_id) = expected_id
                {
                    matches.push(positive_id(&obj["id"])? == positive_id(expected_id)?);
                }
            } else if !value.is_null() {
                matches.push(false);
            }
        }
        let state = if matches.is_empty() {
            "unavailable"
        } else if matches.iter().all(|ok| *ok) {
            "matched"
        } else {
            "mismatched"
        };
        fields.insert(key.into(), json!(state));
    }
    if body.contains_key("cc_emails") {
        let state = if !row.contains_key("cc_emails") {
            "unavailable"
        } else {
            let actual = &row["cc_emails"];
            let actual_ok = actual
                .as_array()
                .is_some_and(|xs| xs.iter().all(Value::is_string));
            if !actual_ok {
                "mismatched"
            } else {
                let Some(want) = body["cc_emails"].as_array() else {
                    return Err(Error::value("recipient readback is not text"));
                };
                if !want.iter().all(Value::is_string) {
                    return Err(Error::value("recipient readback is not text"));
                }
                let mut got: Vec<String> = actual
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| casefold(v.as_str().unwrap()))
                    .collect();
                let mut exp: Vec<String> =
                    want.iter().map(|v| casefold(v.as_str().unwrap())).collect();
                got.sort();
                exp.sort();
                if got == exp { "matched" } else { "mismatched" }
            }
        };
        fields.insert("cc_emails".into(), json!(state));
    }
    Ok(fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body() -> Map<String, Value> {
        obj(json!({"email": "actor@example.com", "requested_for": "actor@example.com"}))
    }

    fn identity() -> Map<String, Value> {
        obj(json!({"requester_id": 11, "requested_for_id": 11, "tenant": "acme.example.com"}))
    }

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    fn fields(row: Value) -> Map<String, Value> {
        recipient_fields(row.as_object().unwrap(), &body(), &identity()).unwrap()
    }

    #[test]
    fn projections_match_python_modes() {
        let ids = fields(json!({
            "id": 123, "requester_id": 11, "requested_for_id": 11,
            "requested_for": {"id": 11, "name": "Actor", "email": "actor@example.com"}
        }));
        assert!(ids.values().all(|v| v == "matched"));

        let nested = fields(json!({
            "id": 123, "requested_for_id": 11,
            "requester": {"id": "11", "email": "ACTOR@example.com"},
            "requested_for": {"id": 11, "name": "Actor", "email": "actor@example.com"}
        }));
        assert!(nested.values().all(|v| v == "matched"));

        let wrong = fields(json!({
            "id": 123, "requester_id": 99, "requested_for_id": 11,
            "requested_for": {"id": 11, "email": "actor@example.com"}
        }));
        assert_eq!(wrong["email"], "mismatched");

        let target = fields(json!({
            "id": 123, "requester_id": 11, "requested_for_id": 99,
            "requested_for": {"id": 11, "email": "actor@example.com"}
        }));
        assert_eq!(target["requested_for"], "mismatched");

        let conflict = fields(json!({
            "id": 123, "requester_id": 11, "requested_for_id": 11, "email": "other@example.com",
            "requested_for": {"id": 11, "email": "actor@example.com"}
        }));
        assert_eq!(conflict["email"], "mismatched");

        let missing = fields(json!({"id": 123}));
        assert_eq!(missing["email"], "unavailable");
        assert_eq!(missing["requested_for"], "unavailable");

        let mut cc_body = body();
        cc_body.insert(
            "cc_emails".into(),
            json!(["B@example.com", "a@example.com"]),
        );
        let row = obj(json!({"cc_emails": ["a@example.com", "b@example.com"]}));
        let got = recipient_fields(&row, &cc_body, &identity()).unwrap();
        assert_eq!(got["cc_emails"], "matched");
    }
}
