//! Reviewed native planning text saves from captured section-query
//! contracts (`planning_text.py`).

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::resource::{CHANGES, PROBLEMS, parse_id};

use crate::create::{
    match_planning_row, planning_definitions, planning_server_key, resolve_planning_definition,
};
use crate::readback::same_text;
use crate::reviewed_write::{WritePlan, execute_as, refused};
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

const TEXT_KEYS: [&str; 5] = [
    "description",
    "content",
    "description_html",
    "description_text",
    "body",
];

/// Port of fsv.planning_text.section_content: native section text in both
/// the edit and detail response shapes.
pub fn section_content(row: Option<&Value>) -> Option<String> {
    let row = row?;
    TEXT_KEYS
        .iter()
        .find_map(|k| row.get(*k).and_then(Value::as_str))
        .map(str::to_string)
}

/// Port of fsv.planning_text.editable_content: a readable text baseline
/// with an explicitly empty file inventory (`""` without a row).
pub fn editable_content(row: Option<&Value>) -> Result<String> {
    let Some(r) = row else {
        return Ok(String::new());
    };
    let inventories: Vec<&Value> = ["attachments", "files"]
        .iter()
        .filter_map(|k| r.get(*k))
        .collect();
    if inventories.is_empty() || inventories.iter().any(|v| **v != json!([])) {
        return Err(refused(
            "planning attachments are present or unavailable; text-only preservation is not proven",
        ));
    }
    let content = section_content(row);
    if content.is_none()
        || TEXT_KEYS
            .iter()
            .any(|k| r.get(*k).is_some_and(|v| !v.is_string()))
    {
        return Err(refused(
            "planning text state is unavailable; no write attempted",
        ));
    }
    Ok(content.unwrap_or_default())
}

/// Port of fsv.planning_text.PlanningTextPlan: a first-write POST whose
/// hash also binds the reviewed (empty) baseline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningTextPlan {
    pub plan: WritePlan,
    pub before: String,
}

impl PlanningTextPlan {
    pub fn payload(&self) -> Value {
        self.plan.payload()
    }

    pub fn hash(&self) -> String {
        plan_hash(&json!({"wire_plan": self.plan.hash(), "before": self.before}))
    }

    pub fn review(&self) -> Map<String, Value> {
        let mut review = self.plan.review();
        review.insert("plan_hash".into(), json!(self.hash()));
        review.insert("operation".into(), json!("create"));
        review.insert("before".into(), json!(self.before));
        review.insert("after".into(), self.payload()["description"].clone());
        review
    }
}

/// Port of fsv.planning_text.build_plan: one native section's first text.
/// Refused when the section has (or may have) attachments or any stored
/// text, since only the create POST is captured.
pub fn build_plan(
    c: &dyn Api,
    module: &str,
    target: &str,
    section: &str,
    description: &str,
    schema: &ChangeSchema,
) -> Result<PlanningTextPlan> {
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
    let existing = match_planning_row(&definition, &rows)?;
    let before = editable_content(existing.as_ref())?;
    let has_text = existing.as_ref().is_some_and(|r| {
        TEXT_KEYS.iter().any(|k| {
            r.get(*k)
                .and_then(Value::as_str)
                .is_some_and(|t| !t.is_empty())
        }) || r.get("empty") == Some(&json!(false))
    });
    if !before.is_empty() || has_text {
        return Err(refused(
            "planning section already has content; native edit wire is not captured; use the web editor (no write attempted)",
        ));
    }
    let plan = WritePlan {
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
    };
    Ok(PlanningTextPlan { plan, before })
}

/// Port of fsv.planning_text.send: one POST, then the section's description
/// read back from a fresh inventory.
pub fn send(
    plan: &PlanningTextPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
    schema: &ChangeSchema,
) -> Result<Value> {
    let wire = &plan.plan;
    let module = wire.path.split('/').next().unwrap_or_default().to_string();
    let section = query_id(&wire.path)?;
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
        let row = match_planning_row(definition, &inventory(c, &module, wire.target)?)?;
        let field = match section_content(row.as_ref()) {
            None => "unavailable",
            Some(d) if same_text(&json!(d), &description) => "matched",
            Some(_) => "mismatched",
        };
        Ok(obj(
            json!({"section": section, "description": field, "verified": field == "matched"}),
        ))
    };
    let target = wire.target.to_string();
    let text = description.as_str().unwrap_or_default().to_string();
    let rebuild = || build_plan(c, &module, &target, &section, &text, schema).map(|p| p.hash());
    execute_as(
        wire,
        &plan.hash(),
        c,
        journal,
        reviewed_hash,
        &rebuild,
        &verify,
    )
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
        /// The rows a POST leaves behind (native detail shape) instead of the echo.
        after: Option<Value>,
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
            w.rows = w
                .after
                .clone()
                .unwrap_or(json!([{"name": name, "description": description, "attachments": []}]));
            Ok(json!({}))
        })
    }

    fn world(module: &'static str) -> Arc<Mutex<World>> {
        Arc::new(Mutex::new(World {
            module,
            rows: json!([]),
            failure: None,
            after: None,
        }))
    }

    fn err<T: std::fmt::Debug>(r: Result<T>) -> String {
        r.unwrap_err().to_string()
    }

    static ORACLE: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| {
        serde_json::from_str::<Value>(include_str!("testdata/workflows_oracle.json")).unwrap()
            ["planning_text"]
            .clone()
    });

    fn module_of(case: &Value) -> &'static str {
        if case["module"] == "problems" {
            "problems"
        } else {
            "changes"
        }
    }

    fn plan_json(p: &PlanningTextPlan) -> Value {
        let w = &p.plan;
        json!({"hash": p.hash(), "review": p.review(), "action": w.action, "target": w.target,
               "method": w.method, "path": w.path, "payload_json": w.payload_json,
               "context_hash": w.context_hash, "before": p.before})
    }

    fn outcome(r: Result<Value>) -> Value {
        match r {
            Ok(v) => json!({"ok": v}),
            Err(e) => json!({"error": e.to_string()}),
        }
    }

    fn writes(c: &FnApi) -> Value {
        json!(
            c.writes()
                .iter()
                .map(|w| json!([w.method, w.path, w.body]))
                .collect::<Vec<_>>()
        )
    }

    fn same(got: &Value, want: &Value, ctx: &str) {
        for key in ["ok", "error"] {
            assert_eq!(got.get(key), want.get(key), "{ctx}: {key}");
        }
    }

    /// `test_planning_text.py::test_section_query_exact_wire_and_readback`,
    /// with plans and results from `gen_fsv_api_workflows.py`.
    #[test]
    fn section_query_exact_wire_and_readback() {
        for case in ORACLE["sections"].as_array().unwrap() {
            let module = module_of(case);
            let section = case["section"].as_str().unwrap();
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
            assert_eq!(
                plan_json(&p),
                json!(
                    case.as_object()
                        .unwrap()
                        .iter()
                        .filter(|(k, _)| !["module", "section", "send", "resend", "writes"]
                            .contains(&k.as_str()))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect::<Map<_, _>>()
                ),
                "{section}"
            );
            same(
                &outcome(send(&p, &c, &j, Some(&p.hash()), &schema)),
                &case["send"],
                section,
            );
            same(
                &outcome(send(&p, &c, &j, Some(&p.hash()), &schema)),
                &case["resend"],
                section,
            );
            assert_eq!(writes(&c), case["writes"], "{section}");
        }
    }

    /// `test_planning_state.py`: known-empty creates, refused existing or
    /// unknown text and files, the GET race, and normalized readback.
    #[test]
    fn create_versus_edit_boundaries() {
        let build = |case: &Value, rows: &Value, text: &str| {
            let module = module_of(case);
            let w = world(module);
            w.lock().unwrap().rows = rows.clone();
            let c = client(&w);
            let section = case["section"].as_str().unwrap_or("Rollout Plan");
            (
                w.clone(),
                build_plan(&c, module, "123", section, text, &schema).map(|p| plan_json(&p)),
                c,
            )
        };
        for case in ORACLE["create"].as_array().unwrap() {
            let (_, got, c) = build(case, &case["rows"], "Acme replacement");
            let got = got.unwrap();
            for key in ["hash", "review", "payload_json", "context_hash", "before"] {
                assert_eq!(got[key], case[key], "{} {key}", case["rows"]);
            }
            let module = module_of(case);
            let p = build_plan(
                &c,
                module,
                "123",
                case["section"].as_str().unwrap(),
                "Acme replacement",
                &schema,
            )
            .unwrap();
            let mut other = p.clone();
            other.before = "other baseline".into();
            assert_ne!(other.hash(), p.hash());
            same(
                &outcome(send(
                    &p,
                    &c,
                    &MemJournal::default(),
                    Some(&p.hash()),
                    &schema,
                )),
                &case["send"],
                "create",
            );
            assert_eq!(writes(&c), case["writes"]);
        }
        for key in ["existing", "unknown", "files"] {
            for case in ORACLE[key].as_array().unwrap() {
                let text = if key == "files" {
                    "Acme"
                } else {
                    "Acme replacement"
                };
                let (_, got, c) = build(case, &case["rows"], text);
                let ctx = format!("{key} {}", case["rows"]);
                assert_eq!(
                    got.err().map(|e| e.to_string()),
                    case["error"].as_str().map(str::to_string),
                    "{ctx}"
                );
                assert!(c.writes().is_empty(), "{ctx}");
            }
        }
        for case in ORACLE["race"].as_array().unwrap() {
            let (w, got, c) = build(case, &json!([]), "Acme");
            assert_eq!(got.unwrap()["hash"], case["hash"]);
            let p = build_plan(
                &c,
                module_of(case),
                "123",
                case["section"].as_str().unwrap(),
                "Acme",
                &schema,
            )
            .unwrap();
            w.lock().unwrap().rows = case["rows"].clone();
            let j = MemJournal::default();
            same(
                &outcome(send(&p, &c, &j, Some(&p.hash()), &schema)),
                case,
                "race",
            );
            assert!(c.writes().is_empty() && j.events().is_empty());
        }
        for case in ORACLE["readback"].as_array().unwrap() {
            let section = case["section"].as_str().unwrap();
            let key = if module_of(case) == "changes" {
                "acme_plan"
            } else {
                "problem_cause"
            };
            let w = world(module_of(case));
            w.lock().unwrap().rows =
                json!([{"name": key, "label": section, "content": "", "files": [], "empty": true}]);
            w.lock().unwrap().after = Some(
                json!([{"name": key, "label": section, "content": case["stored"], "files": [], "empty": false}]),
            );
            let c = client(&w);
            let text = "<div>Acme &amp; Globex\n details</div>";
            let p = build_plan(&c, module_of(case), "123", section, text, &schema).unwrap();
            assert_eq!(p.hash(), case["hash"]);
            same(
                &outcome(send(
                    &p,
                    &c,
                    &MemJournal::default(),
                    Some(&p.hash()),
                    &schema,
                )),
                case,
                &format!("readback {}", case["stored"]),
            );
        }
        for case in ORACLE["same_text"].as_array().unwrap() {
            assert_eq!(
                same_text(&case["actual"], &case["expected"]),
                case["same"] == true,
                "{case}"
            );
        }
    }

    /// `planning_text.section_content` and `editable_content` on the rows
    /// `gen_fsv_api_workflows.py` ran them on.
    #[test]
    fn editable_and_section_content_match_python() {
        for case in ORACLE["section_content"].as_array().unwrap() {
            let row = Some(&case["row"]).filter(|r| !r.is_null());
            assert_eq!(json!(section_content(row)), case["content"], "{case}");
        }
        for case in ORACLE["editable_content"].as_array().unwrap() {
            let row = Some(&case["row"]).filter(|r| !r.is_null());
            let got = match editable_content(row) {
                Ok(text) => json!({"ok": text}),
                Err(e) => json!({"error": e.to_string()}),
            };
            let want = match case.get("ok") {
                Some(ok) => json!({"ok": ok}),
                None => json!({"error": case["error"]}),
            };
            assert_eq!(got, want, "{case}");
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
