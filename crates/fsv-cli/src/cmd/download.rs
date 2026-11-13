//! `download_generic` (tickets, problems) and `changes download`.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use fsv_api::create::{self, download_attachment};
use fsv_api::service;
use fsv_core::resource::{CHANGES, Resource, TICKETS};
use fsv_core::term::{Column, Justify, Table, err, escape, out};
use fsv_core::{Client, Error, Result};

use crate::generated::{ChangesDownloadArgs, ResDownloadArgs};
use crate::util::{CmdResult, cid, client, fail_exc, uncaught};
use crate::{lookup, schema};

/// `_download_dir`: exactly `--dir`, or by default `./downloads/<ID>`
/// (never a bare `./<ID>`); the choice is announced on stderr.
pub fn download_dir(out: Option<&Path>, label: &str) -> PathBuf {
    let target =
        py_path(&out.map_or_else(|| Path::new("downloads").join(label), Path::to_path_buf));
    let origin = if out.is_some() {
        "--dir"
    } else {
        "default ./downloads/<ID>; choose with --dir"
    };
    err().print(&format!(
        "output directory: {} ({origin})",
        escape(&target.to_string_lossy())
    ));
    target
}

/// `str(Path(p))`: no `.` segments, repeated or trailing slashes.
pub fn py_path(p: &Path) -> PathBuf {
    let text = p.to_string_lossy();
    let absolute = text.starts_with('/');
    let parts: Vec<&str> = text
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();
    let joined = parts.join("/");
    PathBuf::from(match (absolute, joined.is_empty()) {
        (true, _) => format!("/{joined}"),
        (false, true) => ".".into(),
        (false, false) => joined,
    })
}

fn fetch_one(c: &Client, att: &Value, dir: &Path, force: bool, source: &str) -> Result<Value> {
    let mut r = download_attachment(c, att, dir, force)?;
    if let Value::Object(m) = &mut r {
        m.insert("source".into(), json!(source));
        let skipped = m.get("skipped").is_some_and(fsv_py::truthy);
        m.insert(
            "status".into(),
            json!(if skipped { "skipped" } else { "downloaded" }),
        );
    }
    Ok(r)
}

fn description_att(url: &str) -> Value {
    let id = url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .split('?')
        .next()
        .unwrap_or_default()
        .to_string();
    json!({"canonical_url": url, "name": format!("attachment-{id}")})
}

fn report(results: &[Value], label: &str, dir: &Path, json_out: bool) {
    if json_out {
        fsv_core::output::emit_json(&Value::Array(results.to_vec()));
        return;
    }
    let mut t = Table::new().title(format!("Downloaded attachments for {label}"));
    t.add_column(Column::new("Status").style("cyan"));
    t.add_column(Column::new("Source"));
    t.add_column(Column::new("File"));
    t.add_column(Column::new("Size").justify(Justify::Right));
    let get = |r: &Value, k: &str| r.get(k).map(fsv_py::py_str).unwrap_or_default();
    for r in results {
        let file = ["name", "path"]
            .iter()
            .find_map(|k| r.get(*k).filter(|v| fsv_py::truthy(v)))
            .map(fsv_py::py_str)
            .unwrap_or_default();
        t.add_row([get(r, "status"), get(r, "source"), file, get(r, "size")]);
    }
    out().print_table(&t);
    out().print(&format!("[green]output[/] {}", dir.display()));
}

/// `download_generic`: only Session/API errors are reported; the rest are uncaught.
pub fn generic(res: &'static Resource, a: ResDownloadArgs) -> CmdResult {
    let caught = |e: Error| match e {
        Error::Session(_) | Error::Api(_) => fail_exc(e),
        other => uncaught(other),
    };
    let id = cid(&a.id_, res)?;
    let c = client()?;
    let (mut main, mut threads, mut description) =
        (a.main_attachments, a.threads, a.description_attachments);
    if a.all_ {
        (main, threads, description) = (true, true, true);
    }
    if !main && !threads && !description {
        main = true;
    }
    let pfx = res.display_prefixes[0];
    let dir = download_dir(a.out.as_deref(), &format!("{pfx}-{id}"));
    let mut results = Vec::new();
    let params = crate::util::obj(vec![("include", json!("requester,stats"))]);
    let raw = c
        .int_get(&format!("{}/{id}", res.api_path), Some(params))
        .map_err(caught)?;
    let item = raw.get(res.item_key).cloned().unwrap_or(raw);
    if main {
        for att in item
            .get("attachments")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            results.push(fetch_one(&c, att, &dir, a.force, "attachments").map_err(caught)?);
        }
    }
    if threads {
        let key = if res.has_notes {
            "notes"
        } else {
            "conversations"
        };
        let params = crate::util::obj(vec![("page", json!(1)), ("per_page", json!(30))]);
        let got = service::collect_pages(
            |p, q| c.int_get(p, q),
            &format!("{}/{id}/{key}", res.api_path),
            key,
            Some(&params),
            key,
            service::MAX_DETAIL_PAGES,
        )
        .map_err(caught)?;
        if !got.complete() {
            let note = got.meta.get("partial").cloned().unwrap_or(Value::Null);
            results.push(json!({
                "source": key, "status": "incomplete", "complete": false,
                "note": note, "meta": got.meta,
            }));
            err().print(&escape(&fsv_py::py_str(&note)));
        }
        for thread in &got.rows {
            for att in thread
                .get("attachments")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                results.push(fetch_one(&c, att, &dir, a.force, key).map_err(caught)?);
            }
        }
    }
    if description {
        let html = item
            .get("description")
            .filter(|v| fsv_py::truthy(v))
            .map(fsv_py::py_str);
        for url in service::extract_description_attachment_urls(html.as_deref(), &c.tenant().domain)
        {
            results.push(
                fetch_one(&c, &description_att(&url), &dir, a.force, "description")
                    .map_err(caught)?,
            );
        }
    }
    let _ = TICKETS;
    report(&results, &format!("{pfx}-{id}"), &dir, a.json_out);
    Ok(())
}

/// `changes download`: every error goes through `_err_exc`.
pub fn changes(a: ChangesDownloadArgs) -> CmdResult {
    let id = cid(&a.id_, &CHANGES)?;
    let c = client()?;
    changes_inner(&a, id, &c).map_err(fail_exc)
}

fn changes_inner(a: &ChangesDownloadArgs, id: i64, c: &std::sync::Arc<Client>) -> Result<()> {
    let (mut all_planning, mut main, mut description) = (
        a.all_planning,
        a.main_attachments,
        a.description_attachments,
    );
    if a.all_ {
        (all_planning, main, description) = (true, true, true);
    }
    if a.planning.is_empty() && !all_planning && !main && !description {
        all_planning = true;
    }
    let dir = download_dir(a.out.as_deref(), &format!("CHN-{id}"));
    let mut results = Vec::new();
    let mut evidence: Option<Value> = None;
    if all_planning || !a.planning.is_empty() {
        let ev = service::get_change_evidence(c.as_ref(), id)?;
        let fields: Vec<Value> = ev
            .get("planning_fields")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let by_name = ev
            .get("planning_fields_by_name")
            .cloned()
            .unwrap_or(json!({}));
        let mut selected: Vec<String> = Vec::new();
        if all_planning {
            for f in &fields {
                if f.get("name").is_some_and(fsv_py::truthy)
                    && f.get("attachments").is_some_and(fsv_py::truthy)
                {
                    selected.push(lookup::s(f, "name"));
                }
            }
        }
        for value in &a.planning {
            let sch = schema::load(&CHANGES, c, false).map_err(fail_exc)?;
            let defn = create::resolve_planning_definition(value, &schema::fields(&sch))?;
            let row = create::match_planning_row(&defn, &fields)?;
            let name = row
                .as_ref()
                .and_then(|r| r.get("name"))
                .filter(|v| fsv_py::truthy(v))
                .map_or_else(|| create::planning_server_key(&defn), fsv_py::py_str);
            selected.push(name);
        }
        let mut seen: Vec<String> = Vec::new();
        for field_id in selected {
            if seen.contains(&field_id) {
                continue;
            }
            seen.push(field_id.clone());
            let atts: Vec<Value> = by_name
                .get(&field_id)
                .and_then(|f| f.get("attachments"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let source = format!("planning:{field_id}");
            if atts.is_empty() {
                results.push(json!({"source": source, "status": "no attachments"}));
                continue;
            }
            for att in &atts {
                results.push(fetch_one(c, att, &dir, a.force, &source)?);
            }
        }
        evidence = Some(ev);
    }
    if (main || description) && evidence.is_none() {
        evidence = Some(service::get_change_evidence(c.as_ref(), id)?);
    }
    let ev = evidence.unwrap_or(json!({}));
    if main {
        for att in ev
            .get("main_attachments")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            results.push(fetch_one(c, att, &dir, a.force, "attachments")?);
        }
    }
    if description {
        for url in ev
            .get("description_attachment_urls")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let url = fsv_py::py_str(url);
            results.push(fetch_one(
                c,
                &description_att(&url),
                &dir,
                a.force,
                "description",
            )?);
        }
    }
    report(&results, &format!("CHN-{id}"), &dir, a.json_out);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `test_change_download_default_matches`.
    #[test]
    fn change_download_default_matches() {
        assert_eq!(
            download_dir(None, "CHN-4"),
            Path::new("downloads").join("CHN-4")
        );
        assert_eq!(
            download_dir(Some(Path::new("x")), "CHN-4"),
            PathBuf::from("x")
        );
    }
}
