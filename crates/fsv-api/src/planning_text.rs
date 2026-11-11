//! Reviewed native planning text saves from captured section-query
//! contracts (`planning_text.py`).

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::resource::{CHANGES, PROBLEMS, parse_id};

use crate::create::{
    match_planning_row, planning_definitions, planning_server_key, resolve_planning_definition,
};
use crate::reviewed_write::{WritePlan, execute, refused};
use crate::service::{Api, Journal, obj, plan_hash};
use crate::watchers::{has_continuation, positive_id, total_differs};

/// Port of fsv.planning_text.PROBLEM_SECTIONS: native semantic section keys,
/// not tenant-specific numeric choice ids, with their `label.title()`.
pub const PROBLEM_SECTIONS: [(&str, &str, &str); 3] = [
    ("root cause", "problem_cause", "Root Cause"),
    ("impact", "problem_impact", "Impact"),
    ("symptoms", "problem_symptom", "Symptoms"),
];

/// `schema.load(CHANGES, client, force=True)`, injectable for tests.
pub type ChangeSchema = dyn Fn(&dyn Api) -> Result<Value>;

/// The live [`ChangeSchema`]: a forced change-schema fetch saved to the
/// process cache.
pub fn live_change_schema(c: &dyn Api) -> Result<Value> {
    crate::schema::load_forced(&CHANGES, c)
}

/// Port of fsv.planning_text.definitions.
pub fn definitions(c: &dyn Api, module: &str, schema: &ChangeSchema) -> Result<Vec<Value>> {
    match module {
        "changes" => {
            let doc = schema(c)?;
            let fields = doc
                .get("fields")
                .and_then(Value::as_array)
                .ok_or_else(|| Error::other("KeyError: 'fields'"))?;
            Ok(planning_definitions(fields))
        }
        "problems" => Ok(PROBLEM_SECTIONS
            .iter()
            .map(|(_, key, title)| json!({"name": key, "label": title}))
            .collect()),
        _ => Err(refused(
            "planning wire is proven only for changes and the three native problem analysis sections",
        )),
    }
}

/// Port of fsv.planning_text.inventory: every planning row of the record.
pub fn inventory(c: &dyn Api, module: &str, target: i64) -> Result<Vec<Value>> {
    let raw = c.int_get(&format!("{module}/{target}/planning-fields"), None)?;
    let key = if module == "changes" {
        "change_planning_fields"
    } else {
        "problem_planning_fields"
    };
    let Some(rows) = raw
        .get(key)
        .and_then(Value::as_array)
        .filter(|_| raw.is_object() && !has_continuation(&raw))
    else {
        return Err(refused("complete planning-field inventory is unavailable"));
    };
    let named = |r: &Value| {
        ["name", "label"]
            .iter()
            .any(|k| r.get(*k).is_some_and(fsv_py::truthy))
    };
    if rows.iter().any(|r| !r.is_object() || !named(r)) {
        return Err(refused("planning rows have unavailable identities"));
    }
    if total_differs(raw.get("total").unwrap_or(&Value::Null), rows.len()) {
        return Err(refused("planning-field inventory is incomplete"));
    }
    Ok(rows.clone())
}

/// `urllib.parse.quote(text, safe="")`.
fn quote(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"_.-~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `parse_qs(urlsplit(path).query)["id"][0]`.
fn query_id(path: &str) -> Result<String> {
    let query = path.split_once('?').map_or("", |(_, q)| q);
    let query = query.split_once('#').map_or(query, |(q, _)| q);
    for pair in query.split(['&', ';']) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if unquote_plus(k) == "id" && !v.is_empty() {
            return Ok(unquote_plus(v));
        }
    }
    Err(Error::other("KeyError: 'id'"))
}

/// `urllib.parse.unquote_plus`.
fn unquote_plus(text: &str) -> String {
    let bytes = text.replace('+', " ").into_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Port of fsv.planning_text.build_plan: one native section's text, refused
/// when an existing row has (or may have) attachments.
pub fn build_plan(
    c: &dyn Api,
    module: &str,
    target: &str,
    section: &str,
    description: &str,
    schema: &ChangeSchema,
) -> Result<WritePlan> {
    if fsv_core::pystr::strip(description).is_empty() {
        return Err(Error::value(
            "planning description must be nonempty text/HTML",
        ));
    }
    let defs = definitions(c, module, schema)?;
    let res = if module == "changes" {
        &CHANGES
    } else {
        &PROBLEMS
    };
    let cid = positive_id(&json!(parse_id(target, res)?))?;
    let definition = if module == "changes" {
        resolve_planning_definition(section, &defs)?
    } else {
        let want = fsv_core::pystr::casefold(section);
        let hits: Vec<&Value> = defs
            .iter()
            .filter(|d| {
                let label = d["label"].as_str().unwrap_or_default();
                want == fsv_core::pystr::casefold(label)
                    || Some(want.as_str()) == d["name"].as_str()
            })
            .collect();
        match hits.as_slice() {
            [one] => (*one).clone(),
            _ => {
                return Err(refused(
                    "choose Root Cause, Impact or Symptoms; other analysis wires are not proven",
                ));
            }
        }
    };
    let key = planning_server_key(&definition);
    let rows = inventory(c, module, cid)?;
    if let Some(existing) = match_planning_row(&definition, &rows)?
        && existing.get("attachments").is_none_or(fsv_py::truthy)
    {
        return Err(refused(
            "planning attachments are present or unavailable; text-only preservation is not proven",
        ));
    }
    Ok(WritePlan {
        action: format!("{module}_planning_text"),
        target: cid,
        tenant: c.domain(),
        method: "POST".into(),
        path: format!("{module}/{cid}/planning-fields?id={}", quote(&key)),
        payload_json: fsv_py::Dumps::default()
            .sort_keys(true)
            .ensure_ascii(false)
            .dump(&json!({"description": description})),
        context_hash: plan_hash(&json!({"definitions": defs, "rows": rows})),
    })
}

/// Port of fsv.planning_text.send: one POST, then the section's description
/// read back from a fresh inventory.
pub fn send(
    plan: &WritePlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
    schema: &ChangeSchema,
) -> Result<Value> {
    let module = plan.path.split('/').next().unwrap_or_default().to_string();
    let section = query_id(&plan.path)?;
    let payload = plan.payload();
    let description = payload
        .get("description")
        .ok_or_else(|| Error::other("KeyError: 'description'"))?
        .clone();
    let verify = |_: &Value| -> Result<Map<String, Value>> {
        let defs = definitions(c, &module, schema)?;
        let definition = defs
            .iter()
            .find(|d| planning_server_key(d) == section)
            .ok_or_else(|| Error::other("StopIteration"))?;
        let row = match_planning_row(definition, &inventory(c, &module, plan.target)?)?;
        let field = match row.as_ref().and_then(|r| r.get("description")) {
            None => "unavailable",
            Some(d) if *d == description => "matched",
            Some(_) => "mismatched",
        };
        Ok(obj(
            json!({"section": section, "description": field, "verified": field == "matched"}),
        ))
    };
    let target = plan.target.to_string();
    let text = description.as_str().unwrap_or_default().to_string();
    let rebuild = || build_plan(c, &module, &target, &section, &text, schema);
    execute(plan, c, journal, reviewed_hash, &rebuild, &verify)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::service::testing::*;
    use fsv_core::ApiError;

    struct World {
        module: &'static str,
        rows: Value,
        failure: Option<u16>,
    }

    fn schema(_: &dyn Api) -> Result<Value> {
        Ok(
            json!({"fields": [{"name": "acme_plan", "label": "Rollout Plan", "field_type": "planning_field", "id": 901}]}),
        )
    }

    fn client(w: &Arc<Mutex<World>>) -> FnApi {
        let w = w.clone();
        FnApi::new(move |call| {
            let mut w = w.lock().unwrap();
            let key = if w.module == "changes" {
                "change_planning_fields"
            } else {
                "problem_planning_fields"
            };
            if call.method == "GET" {
                return Ok(json!({key: w.rows}));
            }
            if let Some(s) = w.failure {
                return Err(ApiError::new(s, json!("uncertain")).into());
            }
            let name = query_id(&call.path).unwrap();
            let description = call.body.as_ref().unwrap()["description"].clone();
            w.rows = json!([{"name": name, "description": description, "attachments": []}]);
            Ok(json!({}))
        })
    }

    fn world(module: &'static str) -> Arc<Mutex<World>> {
        Arc::new(Mutex::new(World {
            module,
            rows: json!([]),
            failure: None,
        }))
    }

    fn err<T: std::fmt::Debug>(r: Result<T>) -> String {
        r.unwrap_err().to_string()
    }

    /// `test_planning_text.py::test_section_query_exact_wire_and_readback`.
    #[test]
    fn section_query_exact_wire_and_readback() {
        let oracle: Value =
            serde_json::from_str(include_str!("testdata/reviewed_oracle.json")).unwrap();
        for (module, section, key) in [
            ("changes", "Rollout Plan", "acme_plan"),
            ("problems", "Root Cause", "problem_cause"),
            ("problems", "Impact", "problem_impact"),
            ("problems", "Symptoms", "problem_symptom"),
        ] {
            let w = world(module);
            let c = client(&w);
            let j = MemJournal::default();
            let p = build_plan(
                &c,
                module,
                "123",
                section,
                "<div>Acme evidence</div>",
                &schema,
            )
            .unwrap();
            assert_eq!(p.method, "POST");
            assert!(p.path.ends_with(&format!("?id={key}")));
            assert_eq!(
                p.payload(),
                json!({"description": "<div>Acme evidence</div>"})
            );
            assert_eq!(p.hash(), oracle["planning"][key]["hash"], "{key}");
            let out = send(&p, &c, &j, Some(&p.hash()), &schema).unwrap();
            assert_eq!(out["verified"], true, "{out}");
            assert_eq!(out["section"], key);
            assert_eq!(c.writes().len(), 1);
            assert!(err(send(&p, &c, &j, Some(&p.hash()), &schema)).contains("was sent"));
        }
    }

    /// `test_attachments_unknown_identity_and_ambiguous_send_refused`.
    #[test]
    fn attachments_identity_and_ambiguous_send() {
        let w = world("changes");
        let c = client(&w);
        let j = MemJournal::default();
        let p = build_plan(&c, "changes", "123", "Rollout Plan", "Acme", &schema).unwrap();
        w.lock().unwrap().failure = Some(403);
        for _ in 0..2 {
            assert!(err(send(&p, &c, &j, Some(&p.hash()), &schema)).contains("unknown"));
        }
        assert_eq!(c.writes().len(), 1);
        w.lock().unwrap().rows =
            json!([{"name": "acme_plan", "description": "old", "attachments": [{"id": 999}]}]);
        assert!(
            err(build_plan(
                &c,
                "changes",
                "123",
                "Rollout Plan",
                "new",
                &schema
            ))
            .contains("attachments")
        );
        w.lock().unwrap().rows = json!([{"name": "acme_plan", "description": "old"}]);
        assert!(
            err(build_plan(
                &c,
                "changes",
                "123",
                "Rollout Plan",
                "new",
                &schema
            ))
            .contains("attachments")
        );
        let pc = client(&world("problems"));
        assert_eq!(
            err(build_plan(
                &pc,
                "problems",
                "123",
                "Workaround",
                "Acme",
                &schema
            )),
            "choose Root Cause, Impact or Symptoms; other analysis wires are not proven"
        );
    }

    #[test]
    fn inputs_inventory_and_query() {
        let w = world("changes");
        let c = client(&w);
        assert_eq!(
            err(build_plan(&c, "changes", "1", "x", "  ", &schema)),
            "planning description must be nonempty text/HTML"
        );
        assert_eq!(
            err(build_plan(&c, "releases", "1", "x", "y", &schema)),
            "planning wire is proven only for changes and the three native problem analysis sections"
        );
        w.lock().unwrap().rows = json!([{"description": "no identity"}]);
        assert_eq!(
            err(inventory(&c, "changes", 1)),
            "planning rows have unavailable identities"
        );
        let paged = FnApi::new(|_| Ok(json!({"change_planning_fields": [], "next_page": 2})));
        assert_eq!(
            err(inventory(&paged, "changes", 1)),
            "complete planning-field inventory is unavailable"
        );
        let short = FnApi::new(|_| Ok(json!({"change_planning_fields": [], "total": 1})));
        assert_eq!(
            err(inventory(&short, "changes", 1)),
            "planning-field inventory is incomplete"
        );
        assert_eq!(quote("a b/ü_.-~"), "a%20b%2F%C3%BC_.-~");
        assert_eq!(
            query_id("changes/1/planning-fields?id=a%20b+c").unwrap(),
            "a b c"
        );
        assert!(query_id("changes/1/planning-fields?id=").is_err());
    }
}
