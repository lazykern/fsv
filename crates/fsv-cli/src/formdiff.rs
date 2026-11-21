//! Form UI session differential: Python's `FormSession` scenarios
//! (`tests/golden/formui_scenarios.py`, played by `gen_formui.py` into
//! `formui.json`) replayed on the Rust session over the routes the Python run
//! recorded (`tests/fixtures/routes/formui/<name>.json`).

use std::collections::BTreeMap;
use std::sync::LazyLock;

use fsv_app::{ChangeForm, FormDeps, change_session, sr_session, ticket_session};
use fsv_forms::pydt::DateTimeValue;
use fsv_forms::{LookupValue, Value};
use fsv_formui::kit::{Theme, WorkersMode, parse_key};
use fsv_formui::screen::review::review_lines;
use fsv_formui::{FormKind, FormMode, FormScreen, FormSession, Plan};
use serde_json::{Map, Value as Json, json};

use crate::testkit::{isolated, tenant};

static GOLDEN: LazyLock<Json> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../tests/golden/formui.json")).expect("formui.json")
});

fn enc(v: &Value) -> Json {
    match v {
        Value::Null => Json::Null,
        Value::Bool(b) => json!(b),
        Value::Int(i) => json!(i),
        Value::Float(f) => json!(f),
        Value::Str(s) => json!(s),
        Value::List(l) => Json::Array(l.iter().map(enc).collect()),
        Value::Lookup(l) => json!({"lookup": [l.id, l.display, l.email]}),
        Value::Date(d) => json!({"date": d.to_string()}),
        Value::DateTime(d) => json!({"dt": d.isoformat()}),
        Value::Json(j) => j.clone(),
    }
}

fn parse_dt(s: &str) -> DateTimeValue {
    let (local, off) = s.split_at(s.len() - 6);
    let sign = if off.starts_with('-') { -1 } else { 1 };
    let secs =
        sign * (off[1..3].parse::<i32>().unwrap() * 3600 + off[4..6].parse::<i32>().unwrap() * 60);
    DateTimeValue::new(
        local.parse().unwrap(),
        jiff::tz::Offset::from_seconds(secs).unwrap(),
    )
}

fn dec(v: &Json, tmp: &str) -> Value {
    match v {
        Json::Object(m) if m.contains_key("lookup") => {
            let l = m["lookup"].as_array().unwrap();
            Value::Lookup(LookupValue {
                id: l[0].clone(),
                display: l[1].as_str().unwrap().into(),
                email: l[2].as_str().map(str::to_string),
            })
        }
        Json::Object(m) if m.contains_key("dt") => {
            Value::DateTime(parse_dt(m["dt"].as_str().unwrap()))
        }
        Json::Object(m) if m.contains_key("date") => {
            Value::Date(m["date"].as_str().unwrap().parse().unwrap())
        }
        Json::Array(a) => Value::List(a.iter().map(|x| dec(x, tmp)).collect()),
        Json::String(s) => Value::Str(s.replace("@TMP@", tmp)),
        other => Value::from_json(other),
    }
}

fn row_of(s: &FormSession, key: &str) -> Json {
    let v = s.view(key);
    json!([
        v.visible,
        v.required,
        v.disabled,
        v.disabled_reason,
        enc(&v.value),
        v.text,
        v.error,
        v.note
    ])
}

fn snap(s: &FormSession) -> Json {
    let items: Map<String, Json> = s
        .items
        .iter()
        .map(|it| (it.key.clone(), row_of(s, &it.key)))
        .collect();
    json!({"items": items, "problems": s.problems(), "missing": s.missing_keys(),
           "input_errors": s.input_errors(), "visible": s.visible_keys()})
}

fn delta(old: &Json, new: &Json) -> Json {
    let mut out = new.as_object().unwrap().clone();
    let items: Map<String, Json> = new["items"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(k, v)| old["items"].get(k.as_str()) != Some(v))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    out.insert("items".into(), Json::Object(items));
    Json::Object(out)
}

fn draft_json(s: &FormSession) -> Json {
    let d = &s.draft;
    let planning: Map<String, Json> = d
        .planning
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                json!({"files": v.files, "description_html": v.description_html}),
            )
        })
        .collect();
    let mut ack = d.ack.clone();
    ack.sort();
    json!({"answers": d.answers.iter().map(|(k, v)| (k.clone(), enc(v))).collect::<Map<_, _>>(),
           "requester": d.requester, "description_html": d.description_html,
           "attachments": d.attachments, "planning": planning, "ack": ack, "s_ack": s.ack,
           "requester_value": s.requester.as_ref().map_or(Json::Null, |l| enc(&Value::Lookup(l.clone())))})
}

fn widget(w: fsv_formui::WidgetKind) -> String {
    format!("{w:?}").to_lowercase()
}

fn head(s: &FormSession) -> Json {
    json!({"title": s.title, "tz_source": s.tz_source, "load_notes": s.load_notes,
           "items": s.items.iter().map(|it| json!([it.key, it.label, widget(it.widget), it.entity, it.help,
               it.placeholder, it.allowed_types, it.max_files, it.field.is_none()])).collect::<Vec<_>>()})
}

fn plan_json(p: &Plan, tmp: &str) -> Json {
    let review: Vec<String> = review_lines(&p.review, 100)
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect();
    let blocking: Vec<String> = p
        .review
        .blocking_unacked()
        .iter()
        .map(|b| b.to_string())
        .collect();
    let v = json!({"errors": p.errors, "hash": p.hash, "review_hash": p.review.plan_hash,
                   "ok": p.ok(), "blocking": blocking, "review": review,
                   "payload": p.payload.clone().unwrap_or(Json::Null)});
    sub(v, tmp)
}

/// The notes line is cleared by an 8 s timer: Python's runs on the wall
/// clock, Rust's on the replay clock, so only a note both show is compared.
fn align_notes(want: &mut Json, got: &mut Json) {
    let walk = |w: &mut Json, g: &mut Json| {
        let (Some(a), Some(b)) = (w.pointer_mut("/obs/notes"), g.pointer_mut("/obs/notes")) else {
            return;
        };
        if a.as_str() == Some("") || b.as_str() == Some("") {
            *a = json!("");
            *b = json!("");
        }
    };
    walk(&mut want["initial"], &mut got["initial"]);
    if let (Some(ws), Some(gs)) = (want["steps"].as_array_mut(), got["steps"].as_array_mut()) {
        for (w, g) in ws.iter_mut().zip(gs.iter_mut()) {
            walk(w, g);
        }
    }
}

fn norm_ui(v: &mut Json) {
    match v {
        Json::Object(m) => {
            if m.contains_key("cursor") {
                m.remove("preview");
            }
            if m.get("open") == Some(&json!(false)) {
                m.insert("options".into(), json!([]));
                m.insert("highlighted".into(), Json::Null);
            }
            for x in m.values_mut() {
                norm_ui(x);
            }
        }
        Json::Array(a) => a.iter_mut().for_each(norm_ui),
        _ => {}
    }
}

fn sub(v: Json, tmp: &str) -> Json {
    static FETCHED: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"fetched [0-9TZ:+.-]+").unwrap());
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let text = v
        .to_string()
        .replace(tmp, "@TMP@")
        .replace(tmp.trim_start_matches('/'), "@TMP@")
        .replace(&cwd, "@TMP@");
    serde_json::from_str(&FETCHED.replace_all(&text, "fetched @T@")).unwrap()
}

fn suggestions(r: Result<(Vec<fsv_formui::Suggestion>, String), String>) -> Json {
    match r {
        Ok((sug, note)) => {
            json!({"suggestions": sug.iter().map(|x| json!([x.label, x.detail, enc(&x.value), x.selected])).collect::<Vec<_>>(), "note": note})
        }
        Err(e) => json!({"raised": e}),
    }
}

fn run_op(
    s: &mut FormSession,
    op: &Json,
    tmp: &str,
    dir: &std::path::Path,
    plan: &mut Option<Plan>,
) -> Json {
    let kind = op
        .as_object()
        .unwrap()
        .keys()
        .find(|k| *k != "note")
        .unwrap()
        .as_str();
    match kind {
        "set" => {
            let out = s.set(op["set"].as_str().unwrap(), dec(&op["value"], tmp));
            json!({"notes": out.notes, "error": out.error})
        }
        "suggest" => suggestions(s.suggestions(
            op["suggest"].as_str().unwrap(),
            op["query"].as_str().unwrap(),
        )),
        "help" => {
            let h = s.help_for(op["help"].as_str().unwrap());
            json!({"help": {"label": h.label, "required": h.required, "help": h.help,
                "placeholder": h.placeholder,
                "choices": h.choices.iter().map(|(a, b)| json!([a, b])).collect::<Vec<_>>(),
                "effects": h.effects, "why": h.why, "kind": h.kind.map(widget),
                "disabled": h.disabled, "note": h.note}})
        }
        "effects" => json!({"effects": s.effects(op["effects"].as_str().unwrap())}),
        "mark_invalid" => {
            s.mark_invalid(
                op["mark_invalid"].as_str().unwrap(),
                op["error"].as_str().unwrap(),
            );
            json!({})
        }
        "ack" => {
            s.ack = op["ack"].as_bool().unwrap();
            json!({})
        }
        "plan" => {
            let p = s.build_plan(op["plan"].as_bool()).expect("plan");
            let j = plan_json(&p, tmp);
            *plan = Some(p);
            json!({"plan": j})
        }
        "save" => {
            let name = op["save"].as_str().unwrap();
            let path = dir.join(name);
            let p = path.to_str().unwrap();
            let out = s.save_draft(p, plan.as_ref()).expect("save");
            let text = std::fs::read_to_string(&path).unwrap();
            let command = plan.as_ref().map(|pl| s.command_for(p, pl));
            sub(
                json!({"path": out.to_str().unwrap(), "text": text, "command": command}),
                tmp,
            )
        }
        "send" => {
            let dup = op
                .get("allow_duplicate")
                .and_then(Json::as_bool)
                .unwrap_or(false);
            match s.send(plan.as_ref().expect("plan"), dup) {
                Ok(v) => json!({"sent": v}),
                Err(fsv_core::Error::Exit { code: 5, message }) => json!({"partial": message}),
                Err(fsv_core::Error::Exit { code, message }) => {
                    json!({"exit": code, "message": message})
                }
                Err(e) => json!({"error": e.to_string()}),
            }
        }
        other => panic!("unknown op {other}"),
    }
}

fn requests(t: &crate::testkit::Tenant) -> Json {
    Json::Array(
        t.mock
            .requests()
            .iter()
            .filter(|r| url::Url::parse(&r.url).is_ok_and(|u| !u.path().starts_with("/a/")))
            .map(|r| {
                let url = url::Url::parse(&r.url).unwrap();
                let mut query: Vec<(String, String)> = url
                    .query_pairs()
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect();
                query.sort();
                let mut row = json!({"method": r.method, "path": url.path(),
                    "query": query.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>()});
                if r.method != "GET" {
                    let body = r.body.as_deref().unwrap_or(b"null");
                    row["body"] = serde_json::from_slice(body).unwrap_or(Json::Null);
                }
                row
            })
            .collect(),
    )
}

fn play(name: &str, sc: &Json) -> (Json, Json) {
    let env = isolated();
    env.config(sc.get("config").cloned().unwrap_or_else(|| json!({})));
    let setenv: Vec<(String, String)> = sc
        .get("env")
        .and_then(Json::as_object)
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
                .collect()
        })
        .unwrap_or_default();
    env.set_env("TZ", Some("UTC"));
    for (k, v) in &setenv {
        env.set_env(k, Some(v));
    }
    let files = tempfile::tempdir().unwrap();
    let dir = files.path().canonicalize().unwrap();
    for (n, size) in sc
        .get("files")
        .and_then(Json::as_object)
        .into_iter()
        .flatten()
    {
        std::fs::write(dir.join(n), vec![b'x'; size.as_u64().unwrap() as usize]).unwrap();
    }
    let tmp = dir.to_str().unwrap().to_string();
    let t = tenant(&format!("formui/{name}"));
    let mut s = build_session(&t, sc);
    s.paths.cwd = dir.clone();
    let _ = (FormKind::Change,);
    let mut prev = snap(&s);
    let mut out = json!({"head": head(&s), "initial": prev.clone(), "draft": draft_json(&s)});
    let mut steps = vec![];
    let mut plan = None;
    for op in sc["ops"].as_array().unwrap() {
        let mut res = run_op(&mut s, op, &tmp, &dir, &mut plan);
        let cur = snap(&s);
        res["snap"] = delta(&prev, &cur);
        res["draft"] = draft_json(&s);
        prev = cur;
        steps.push(sub(res, &tmp));
    }
    out["steps"] = Json::Array(steps);
    out["requests"] = requests(&t);
    for (k, _) in &setenv {
        env.set_env(k, None);
    }
    (out, Json::Null)
}

fn build_session(t: &crate::testkit::Tenant, sc: &Json) -> FormSession {
    let deps = FormDeps::live(t.client.clone()).unwrap();
    let who = sc.get("requester").filter(|r| !r.is_null()).map(|r| {
        match dec(&json!({"lookup": r}), "") {
            Value::Lookup(l) => l,
            _ => unreachable!(),
        }
    });
    match sc["kind"].as_str().unwrap() {
        "change" => {
            let mode = if sc.get("mode").and_then(Json::as_str) == Some("edit") {
                FormMode::Edit
            } else {
                FormMode::New
            };
            let mut o = ChangeForm::new(mode);
            o.target = sc.get("target").cloned();
            o.requester = who;
            o.dry_run = sc.get("dry_run").and_then(Json::as_bool).unwrap_or(false);
            change_session(&deps, o).expect("session")
        }
        "sr" => {
            let item = fsv_py::py_str(&sc["item"]);
            sr_session(
                &deps,
                &item,
                None,
                who,
                sc.get("dry_run").and_then(Json::as_bool).unwrap_or(false),
            )
            .expect("sr")
        }
        _ => ticket_session(
            &deps,
            None,
            who,
            sc.get("dry_run").and_then(Json::as_bool).unwrap_or(false),
        )
        .expect("ticket"),
    }
}

fn cut(v: &Json) -> String {
    let s = v.to_string();
    if s.chars().count() > 700 {
        format!("{}...", s.chars().take(700).collect::<String>())
    } else {
        s
    }
}

fn first_diff(a: &Json, b: &Json, path: &str, out: &mut Vec<String>) {
    if out.len() >= 60 || a == b {
        return;
    }
    match (a, b) {
        (Json::Object(x), Json::Object(y)) => {
            let keys: BTreeMap<_, _> = x.keys().chain(y.keys()).map(|k| (k.clone(), ())).collect();
            for k in keys.keys() {
                first_diff(
                    x.get(k).unwrap_or(&Json::Null),
                    y.get(k).unwrap_or(&Json::Null),
                    &format!("{path}.{k}"),
                    out,
                );
            }
        }
        (Json::Array(x), Json::Array(y)) => {
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                first_diff(p, q, &format!("{path}[{i}]"), out);
            }
            if x.len() != y.len() {
                out.push(format!("{path} length python {} rust {}", x.len(), y.len()));
            }
        }
        (Json::String(x), Json::String(y)) => {
            let at = x.chars().zip(y.chars()).take_while(|(p, q)| p == q).count();
            let around = |s: &str| {
                s.chars()
                    .skip(at.saturating_sub(60))
                    .take(160)
                    .collect::<String>()
            };
            out.push(format!(
                "{path} first differs at char {at}\n   python: {:?}\n   rust:   {:?}",
                around(x),
                around(y)
            ));
        }
        _ => out.push(format!(
            "{path}\n   python: {}\n   rust:   {}",
            cut(a),
            cut(b)
        )),
    }
}

#[test]
fn sessions_match_python() {
    let mut bad = vec![];
    for (name, case) in GOLDEN["sessions"].as_object().unwrap() {
        let (got, _) = play(name, &case["spec"]);
        let mut diffs = vec![];
        first_diff(&case["want"], &got, name, &mut diffs);
        bad.extend(diffs);
    }
    assert!(
        bad.is_empty(),
        "{} differences:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

fn ui_observe(s: &mut FormScreen) -> Json {
    if let Some(a) = s.done() {
        return json!({"done": format!("{a:?}").to_lowercase().replace("savedraft", "save_draft")});
    }
    let keys: Vec<String> = s.session.items.iter().map(|it| it.key.clone()).collect();
    let mut rows = Map::new();
    for k in keys {
        if !s.session.view(&k).visible {
            continue;
        }
        if let Some(ed) = s.editor_mut(&k) {
            let o = ed.observe();
            if !o.is_null() {
                rows.insert(k, o);
            }
        }
    }
    let mut layer_state = s.observe_layer();
    if let Some(l) = layer_state.as_object_mut()
        && s.layer() == Some("calendar")
        && !s.date_picker().is_some_and(|d| !d.date_only)
    {
        l.insert("time".into(), Json::Null);
    }
    json!({"layer": s.layer(), "focus": s.focus_label(), "notes": s.notes_text(), "status": s.status_text(),
           "help": s.help_text(), "rows": rows, "layer_state": layer_state,
           "done": s.done().map(|a| format!("{a:?}").to_lowercase().replace("savedraft", "save_draft"))})
}

fn draw(s: &mut FormScreen) {
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    term.draw(|f| {
        let area = f.area();
        s.render(area, f.buffer_mut());
    })
    .unwrap();
}

fn play_ui(name: &str, sc: &Json) -> Json {
    let env = isolated();
    env.config(sc.get("config").cloned().unwrap_or_else(|| json!({})));
    env.set_env("TZ", Some("UTC"));
    let setenv: Vec<(String, String)> = sc
        .get("env")
        .and_then(Json::as_object)
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
                .collect()
        })
        .unwrap_or_default();
    for (k, v) in &setenv {
        env.set_env(k, Some(v));
    }
    let files = tempfile::tempdir().unwrap();
    let dir = files.path().canonicalize().unwrap();
    for (n, size) in sc
        .get("files")
        .and_then(Json::as_object)
        .into_iter()
        .flatten()
    {
        std::fs::write(dir.join(n), vec![b'x'; size.as_u64().unwrap() as usize]).unwrap();
    }
    let tmp = dir.to_str().unwrap().to_string();
    let t = tenant(&format!("formui/ui_{name}"));
    let session = build_session(&t, sc);
    let mut session = session;
    session.paths.cwd = dir.clone();
    let mut s = FormScreen::with_workers(session, Theme::textual_dark(), WorkersMode::Inline);
    let t0 = s.now() + std::time::Duration::from_secs_f64(0.35);
    s.tick(t0);
    draw(&mut s);
    let mut prev = snap(&s.session);
    let obs = ui_observe(&mut s);
    let mut out = json!({"initial": sub(json!({"obs": obs, "snap": prev.clone(), "draft": draft_json(&s.session)}), &tmp)});
    let mut steps = vec![];
    for op in sc["ops"].as_array().unwrap() {
        if s.done().is_some() {
            steps.push(json!({"op": op, "skipped": true}));
            continue;
        }
        let kind = op.as_object().unwrap().keys().next().unwrap().as_str();
        match kind {
            "keys" => {
                for k in op["keys"].as_array().unwrap() {
                    s.handle_key(parse_key(k.as_str().unwrap()));
                }
            }
            "type" => {
                for ch in op["type"].as_str().unwrap().replace("@TMP@", &tmp).chars() {
                    s.handle_key(crossterm::event::KeyEvent::from(
                        crossterm::event::KeyCode::Char(ch),
                    ));
                }
            }
            "wait" => {
                let t = s.now() + std::time::Duration::from_secs_f64(op["wait"].as_f64().unwrap());
                s.tick(t);
            }
            "focus" => s.focus_key(op["focus"].as_str().unwrap()),
            "pre" => {
                for p in op["pre"].as_array().unwrap() {
                    s.session.set(p[0].as_str().unwrap(), dec(&p[1], &tmp));
                }
                s.refresh_rows();
            }
            other => panic!("unknown ui op {other}"),
        }
        let t = s.now() + std::time::Duration::from_secs_f64(0.35);
        s.tick(t);
        draw(&mut s);
        let cur = snap(&s.session);
        let step = json!({"obs": ui_observe(&mut s), "snap": delta(&prev, &cur), "draft": draft_json(&s.session)});
        prev = cur;
        steps.push(sub(step, &tmp));
    }
    out["steps"] = Json::Array(steps);
    out["requests"] = requests(&t);
    for (k, _) in &setenv {
        env.set_env(k, None);
    }
    out
}

#[test]
fn key_replays_match_python() {
    let mut bad = vec![];
    for (name, case) in GOLDEN["ui"].as_object().unwrap() {
        let mut got = play_ui(name, &case["spec"]);
        let mut want = case["want"].clone();
        if std::env::var_os("FORMDIFF_DUMP").is_some() {
            std::fs::write(format!("/tmp/ui_got_{name}.json"), got.to_string()).unwrap();
        }
        norm_ui(&mut got);
        norm_ui(&mut want);
        align_notes(&mut want, &mut got);
        let mut diffs = vec![];
        first_diff(&want, &got, name, &mut diffs);
        bad.extend(diffs.into_iter().take(6));
    }
    assert!(
        bad.is_empty(),
        "{} differences:\n{}",
        bad.len(),
        bad.join("\n")
    );
}
