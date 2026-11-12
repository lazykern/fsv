//! Draft knowledge articles and reviewed problem solution links
//! (`solutions.py`).
//!
//! Create writes a draft only. Link and unlink are separate reviewed PUTs.
//! Publication and attachments stay unsupported.

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::pystr::{casefold, strip};
use fsv_core::resource::{PROBLEMS, parse_id};
use fsv_py::{py_str, truthy};

use crate::problem_workflows::{inventory, record};
use crate::readback::submitted_value_matches;
use crate::reviewed_write::{WritePlan, execute, refused};
use crate::service::{Api, Journal, MAX_DETAIL_PAGES, collect_pages, obj, snapshot_hash};
use crate::watchers::positive_id;

/// Accepted application enum values. Never tenant folder or category ids.
pub const ARTICLE_TYPES: &[(&str, i64)] = &[("permanent", 1), ("workaround", 2)];

/// Port of solutions.SolutionPlan. The hash is the write's; inputs are not bound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolutionPlan {
    pub write: WritePlan,
    pub inputs_json: String,
}

impl SolutionPlan {
    pub fn hash(&self) -> String {
        self.write.hash()
    }

    pub fn review(&self) -> Map<String, Value> {
        self.write.review()
    }
}

fn dumps(v: &Value) -> String {
    fsv_py::Dumps::default().sort_keys(true).dump(v)
}

fn proven_article_type(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Number(n)) if n.is_i64() || n.is_u64() => matches!(n.as_i64(), Some(1 | 2)),
        _ => false,
    }
}

/// Port of solutions.metadata: one complete, unique, nondeleted page of
/// categories or folders, sorted by id.
pub fn metadata(
    c: &dyn Api,
    path: &str,
    key: &str,
    extra: Option<&Map<String, Value>>,
) -> Result<Vec<Value>> {
    let mut params = obj(json!({"per_page": 300, "page": 1}));
    if let Some(extra) = extra {
        params.extend(extra.clone());
    }
    let pages = collect_pages(
        |p, q| c.int_get(p, q),
        path,
        key,
        Some(&params),
        key,
        MAX_DETAIL_PAGES,
    )?;
    if pages.missing.is_some() || !pages.meta.get("complete").is_some_and(truthy) {
        return Err(refused(format!(
            "complete knowledge {key} metadata unavailable"
        )));
    }
    let ids = pages
        .rows
        .iter()
        .map(|row| positive_id(row.get("id").unwrap_or(&Value::Null)))
        .collect::<Result<Vec<_>>>()?;
    if ids.iter().collect::<HashSet<_>>().len() != ids.len() {
        return Err(refused("duplicate knowledge identities"));
    }
    let mut kept = pages
        .rows
        .into_iter()
        .filter(|row| !row.get("deleted").is_some_and(truthy))
        .map(|row| {
            let id = positive_id(row.get("id").unwrap_or(&Value::Null))?;
            Ok((id, row))
        })
        .collect::<Result<Vec<_>>>()?;
    kept.sort_by_key(|(id, _)| *id);
    Ok(kept.into_iter().map(|(_, row)| row).collect())
}

fn choose<'a>(rows: &'a [Value], value: Option<&str>, default_key: &str) -> Result<&'a Value> {
    let matches: Vec<&Value> = match value {
        None => rows
            .iter()
            .filter(|row| row.get(default_key) == Some(&Value::Bool(true)))
            .collect(),
        Some(value) => {
            let want = casefold(value);
            rows.iter()
                .filter(|row| {
                    row.get("id").is_some_and(|id| py_str(id) == value)
                        || casefold(&row.get("name").map(py_str).unwrap_or_default()) == want
                })
                .collect()
        }
    };
    if matches.len() != 1 {
        return Err(refused(
            "category/folder must resolve uniquely from runtime metadata; specify --category and --folder",
        ));
    }
    Ok(matches[0])
}

/// Port of solutions.folder: the unique category and a folder inside it.
pub fn folder(
    c: &dyn Api,
    category: Option<&str>,
    selected: Option<&str>,
) -> Result<(Value, Value)> {
    let categories = metadata(c, "solutions/categories", "categories", None)?;
    let cat = choose(&categories, category, "default_category")?.clone();
    let cid = positive_id(cat.get("id").unwrap_or(&Value::Null))?;
    let folders = metadata(
        c,
        "solutions/folders",
        "folders",
        Some(&obj(json!({"category_id": cid}))),
    )?;
    let dest = choose(&folders, selected, "default_folder")?.clone();
    if positive_id(dest.get("category_id").unwrap_or(&Value::Null))? != cid {
        return Err(refused("folder category identity conflicts"));
    }
    Ok((cat, dest))
}

/// Port of solutions.article: one nondeleted article with this id.
pub fn article(c: &dyn Api, ident: i64) -> Result<Map<String, Value>> {
    let raw = c.int_get(&format!("solutions/articles/{ident}"), None)?;
    let Some(row) = raw.get("article").and_then(Value::as_object) else {
        return Err(refused("exact active article unavailable"));
    };
    if positive_id(row.get("id").unwrap_or(&Value::Null))? != ident
        || row.get("deleted").is_some_and(truthy)
    {
        return Err(refused("exact active article unavailable"));
    }
    Ok(row.clone())
}

/// Port of solutions.create_plan. The plan target is the folder id.
pub fn create_plan(
    c: &dyn Api,
    title: &str,
    description: &str,
    kind: &str,
    category: Option<&str>,
    selected_folder: Option<&str>,
) -> Result<SolutionPlan> {
    let article_type = ARTICLE_TYPES
        .iter()
        .find(|(name, _)| *name == kind)
        .map(|(_, id)| *id);
    if article_type.is_none()
        || strip(title).is_empty()
        || strip(description).is_empty()
        || title.chars().any(|ch| (ch as u32) < 32)
    {
        return Err(Error::value(
            "nonempty title/body and workaround or permanent type required",
        ));
    }
    let (cat, dest) = folder(c, category, selected_folder)?;
    let folder_id = positive_id(dest.get("id").unwrap_or(&Value::Null))?;
    let html = fsv_core::pystr::html_escape(description, true).replace('\n', "<br>");
    let payload = json!({
        "title": title,
        "description": format!("<div>{html}</div>"),
        "status": 1,
        "article_type": article_type,
        "folder_id": folder_id,
        "source": "problem",
    });
    let inputs = json!({
        "title": title,
        "description": description,
        "kind": kind,
        "category": category,
        "selected_folder": selected_folder,
    });
    Ok(SolutionPlan {
        write: WritePlan {
            action: "solution_create".into(),
            target: folder_id,
            tenant: c.domain(),
            method: "POST".into(),
            path: "solutions/articles".into(),
            payload_json: dumps(&payload),
            context_hash: snapshot_hash(&json!({"category": cat, "folder": dest}))?,
        },
        inputs_json: dumps(&inputs),
    })
}

fn opt_str(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Null) | None => None,
        Some(other) => Some(py_str(other)),
    }
}

fn verify_create(c: &dyn Api, response: &Value, payload: &Value) -> Result<Map<String, Value>> {
    let raw = response
        .get("article")
        .filter(|_| response.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let ident = positive_id(raw.get("id").unwrap_or(&Value::Null))?;
    let exact = article(c, ident)?;
    let mut results = Map::new();
    for key in [
        "title",
        "description",
        "status",
        "article_type",
        "folder_id",
    ] {
        let expected = &payload[key];
        let Some(actual) = exact.get(key) else {
            results.insert(key.into(), json!("unavailable"));
            continue;
        };
        let matched = if key == "description" {
            match (actual.as_str(), expected.as_str()) {
                (Some(got), Some(want)) => {
                    let got = fsv_core::output::strip_html(Some(got));
                    let want = fsv_core::output::strip_html(Some(want));
                    strip(&got) == strip(&want)
                }
                _ => false,
            }
        } else {
            submitted_value_matches(actual, expected)
        };
        results.insert(
            key.into(),
            json!(if matched { "matched" } else { "mismatched" }),
        );
    }
    let verified = results.values().all(|v| v == "matched");
    Ok(obj(json!({
        "article_id": ident,
        "verified": verified,
        "fields": results,
        "readback": exact,
    })))
}

/// Port of solutions.send_create.
pub fn send_create(
    plan: &SolutionPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let inputs: Value =
        serde_json::from_str(&plan.inputs_json).map_err(|e| Error::value(e.to_string()))?;
    let title = inputs["title"].as_str().unwrap_or("").to_string();
    let description = inputs["description"].as_str().unwrap_or("").to_string();
    let kind = inputs["kind"].as_str().unwrap_or("").to_string();
    let category = opt_str(inputs.get("category"));
    let selected = opt_str(inputs.get("selected_folder"));
    let payload = plan.write.payload();
    let rebuild = || {
        create_plan(
            c,
            &title,
            &description,
            &kind,
            category.as_deref(),
            selected.as_deref(),
        )
        .map(|p| p.write)
    };
    let verify = |response: &Value| verify_create(c, response, &payload);
    execute(&plan.write, c, journal, reviewed_hash, &rebuild, &verify)
}

/// Port of solutions.link_plan.
pub fn link_plan(c: &dyn Api, target: &Value, ident: &Value, unlink: bool) -> Result<SolutionPlan> {
    let target = positive_id(&json!(parse_id(&py_str(target), &PROBLEMS)?))?;
    let ident = positive_id(ident)?;
    let problem = record(c, target)?;
    let before = inventory(c, target, "solutions")?;
    let exact = article(c, ident)?;
    if !proven_article_type(exact.get("article_type")) {
        return Err(refused(
            "only proven workaround/permanent article types can be linked",
        ));
    }
    let mut exists = false;
    for row in &before {
        if positive_id(row.get("id").unwrap_or(&Value::Null))? == ident {
            exists = true;
            break;
        }
    }
    if exists != unlink {
        return Err(refused(if exists {
            "article already linked"
        } else {
            "article is not linked to this problem"
        }));
    }
    let inputs = json!({"ident": ident, "unlink": unlink});
    Ok(SolutionPlan {
        write: WritePlan {
            action: if unlink {
                "problem_solution_unlink"
            } else {
                "problem_solution_link"
            }
            .into(),
            target,
            tenant: c.domain(),
            method: "PUT".into(),
            path: format!("problems/{target}/solutions/{ident}"),
            payload_json: dumps(&json!({"type": if unlink { "unlink" } else { "link" }})),
            context_hash: snapshot_hash(&json!({
                "problem": problem,
                "solutions": before,
                "article": exact,
            }))?,
        },
        inputs_json: dumps(&inputs),
    })
}

/// Port of solutions.send_link.
pub fn send_link(
    plan: &SolutionPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let inputs: Value =
        serde_json::from_str(&plan.inputs_json).map_err(|e| Error::value(e.to_string()))?;
    let ident = inputs.get("ident").cloned().unwrap_or(Value::Null);
    let unlink = inputs.get("unlink") == Some(&Value::Bool(true));
    let target = plan.write.target;
    let rebuild = || link_plan(c, &json!(target), &ident, unlink).map(|p| p.write);
    let verify = |_: &Value| {
        let rows = inventory(c, target, "solutions")?;
        let exact = rows.iter().find(|row| {
            positive_id(row.get("id").unwrap_or(&Value::Null)).ok() == positive_id(&ident).ok()
        });
        let matched = if unlink {
            exact.is_none()
        } else {
            let source = article(c, positive_id(&ident)?)?;
            exact.is_some_and(|row| row.get("article_type") == source.get("article_type"))
        };
        Ok(obj(json!({"verified": matched, "readback": rows})))
    };
    execute(&plan.write, c, journal, reviewed_hash, &rebuild, &verify)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::service::testing::*;

    struct World {
        articles: HashMap<i64, Value>,
        linked: Vec<Value>,
        persist: bool,
    }

    fn api(state: Arc<Mutex<World>>) -> FnApi {
        let mut api = FnApi::new(move |call: &Call| {
            let mut s = state.lock().unwrap();
            if call.method == "GET" {
                if call.path == "solutions/categories" {
                    return Ok(json!({
                        "categories": [{"id": 71, "name": "Acme", "default_category": true, "workspace_id": 1}],
                        "meta": {"count": 1, "page": null, "per_page": "300"}
                    }));
                }
                if call.path == "solutions/folders" {
                    let cid = call
                        .params
                        .as_ref()
                        .and_then(|p| p.get("category_id"))
                        .and_then(Value::as_i64);
                    if cid != Some(71) {
                        return Err(Error::other("category_id"));
                    }
                    return Ok(json!({
                        "folders": [{"id": 81, "name": "Drafts", "category_id": 71, "default_folder": true, "workspace_id": 1}],
                        "meta": {"count": 1, "page": null, "per_page": "300"}
                    }));
                }
                if let Some(id) = call.path.strip_prefix("solutions/articles/") {
                    let id: i64 = id.parse().map_err(|_| Error::other("article id"))?;
                    let Some(row) = s.articles.get(&id).cloned() else {
                        return Err(Error::other("missing article"));
                    };
                    return Ok(json!({"article": row}));
                }
                if call.path.ends_with("/solutions") {
                    return Ok(json!({"articles": s.linked.clone(), "has_more": false}));
                }
                return Ok(
                    json!({"problem": {"id": 23, "subject": "Acme problem", "workspace_id": 2}}),
                );
            }
            let body = call.body.clone().unwrap_or(Value::Null);
            if call.method == "POST" {
                let mut row = body.clone();
                if let Some(obj) = row.as_object_mut() {
                    obj.insert("id".into(), json!(91));
                }
                if s.persist {
                    let mut stored = row.clone();
                    if let (Some(obj), Some(desc)) = (
                        stored.as_object_mut(),
                        body.get("description").and_then(Value::as_str),
                    ) {
                        obj.insert(
                            "description".into(),
                            json!(format!("<div style='font:inherit'>{desc}</div>")),
                        );
                    }
                    s.articles.insert(91, stored);
                }
                return Ok(json!({"article": row}));
            }
            if s.persist {
                if body.get("type").and_then(Value::as_str) == Some("link") {
                    if let Some(row) = s.articles.get(&91).cloned() {
                        s.linked.push(row);
                    }
                } else {
                    s.linked.clear();
                }
            }
            Ok(json!({}))
        });
        api.domain = "acme.example.com".into();
        api
    }

    #[test]
    fn cases_match_python() {
        let oracle: Value =
            serde_json::from_str(include_str!("testdata/solutions_oracle.json")).unwrap();
        let cases = oracle["cases"].as_object().unwrap();
        assert_eq!(cases.len(), 4);
        for (name, case) in cases {
            let state = Arc::new(Mutex::new(World {
                articles: HashMap::new(),
                linked: Vec::new(),
                persist: true,
            }));
            let c = api(state.clone());
            let journal = MemJournal::default();
            let mut plan: Option<SolutionPlan> = None;
            for (i, (op, want)) in case["ops"]
                .as_array()
                .unwrap()
                .iter()
                .zip(case["outputs"].as_array().unwrap())
                .enumerate()
            {
                let ctx = format!("{name} op {i}");
                let got = match op["op"].as_str().unwrap() {
                    "set" => {
                        if let Some(v) = op.get("persist").and_then(Value::as_bool) {
                            state.lock().unwrap().persist = v;
                        }
                        Ok(json!({}))
                    }
                    "create" => create_plan(
                        &c,
                        op["title"].as_str().unwrap(),
                        op["description"].as_str().unwrap(),
                        op["kind"].as_str().unwrap(),
                        op.get("category").and_then(Value::as_str),
                        op.get("selected_folder").and_then(Value::as_str),
                    )
                    .map(|p| {
                        let out = json!({"plan": {
                            "action": p.write.action,
                            "target": p.write.target,
                            "tenant": p.write.tenant,
                            "method": p.write.method,
                            "path": p.write.path,
                            "payload": p.write.payload(),
                            "context_hash": p.write.context_hash,
                            "hash": p.hash(),
                            "review": p.review(),
                            "inputs": serde_json::from_str::<Value>(&p.inputs_json).unwrap(),
                        }});
                        plan = Some(p);
                        out
                    }),
                    "send_create" => {
                        let p = plan.as_ref().unwrap();
                        send_create(p, &c, &journal, Some(&p.hash())).map(|r| json!({"result": r}))
                    }
                    "link" => link_plan(
                        &c,
                        op.get("target").unwrap_or(&json!(23)),
                        op.get("ident").unwrap_or(&json!(91)),
                        op.get("unlink").and_then(Value::as_bool).unwrap_or(false),
                    )
                    .map(|p| {
                        let out = json!({"plan": {
                            "action": p.write.action,
                            "target": p.write.target,
                            "tenant": p.write.tenant,
                            "method": p.write.method,
                            "path": p.write.path,
                            "payload": p.write.payload(),
                            "context_hash": p.write.context_hash,
                            "hash": p.hash(),
                            "review": p.review(),
                            "inputs": serde_json::from_str::<Value>(&p.inputs_json).unwrap(),
                        }});
                        plan = Some(p);
                        out
                    }),
                    "send_link" => {
                        let p = plan.as_ref().unwrap();
                        send_link(p, &c, &journal, Some(&p.hash())).map(|r| json!({"result": r}))
                    }
                    other => panic!("{ctx} {other}"),
                };
                match got {
                    Ok(mut v) => {
                        v["writes"] = json!(c.writes().len());
                        assert_eq!(v, *want, "{ctx}");
                    }
                    Err(e) => {
                        assert_eq!(
                            e.to_string(),
                            want["error"]["msg"].as_str().unwrap(),
                            "{ctx}"
                        );
                        match want["error"]["type"].as_str().unwrap() {
                            "ValueError" => assert!(matches!(e, Error::Value(_)), "{ctx}"),
                            "ReviewedWriteError" => {
                                assert!(matches!(e, Error::Exit { code: 2, .. }), "{ctx} {e:?}")
                            }
                            other => panic!("{ctx} {other}"),
                        }
                        assert_eq!(
                            c.writes().len() as u64,
                            want["writes"].as_u64().unwrap(),
                            "{ctx}"
                        );
                    }
                }
            }
        }
    }
}
