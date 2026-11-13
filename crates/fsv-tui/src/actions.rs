//! Write-action helpers (`tui/actions.py`), no UI.

use std::path::{Component, Path, PathBuf};

use fsv_core::Error;
use fsv_core::safety::{self, RiskContext};
use serde_json::{Map, Value};

use crate::backend::{BResult, Backend, Failure, Item, Res, ReviewPlan, TabData};
use crate::format::resolve_status;
use crate::site;

/// CLI flag wins; otherwise the `tui.read_only` config option.
pub fn resolve_read_only(flag: bool, setting: &Value) -> bool {
    if flag {
        return true;
    }
    match setting {
        Value::String(s) => matches!(
            s.trim().to_lowercase().as_str(),
            "on" | "true" | "1" | "yes"
        ),
        v => fsv_py::truthy(v),
    }
}

pub fn can_change_status(res: Res) -> bool {
    matches!(res.name, "tickets" | "changes")
}

pub fn can_note(res: Res) -> bool {
    res.name == "tickets" || (matches!(res.name, "changes" | "problems") && res.has_notes)
}

pub fn can_reply(res: Res) -> bool {
    res.name == "tickets"
}

/// `(id, label)` pairs for the status field, in schema order.
pub fn status_choices(schema: &Value) -> Vec<(Value, String)> {
    let Some(fields) = schema.get("fields").and_then(Value::as_array) else {
        return Vec::new();
    };
    let Some(f) = fields
        .iter()
        .find(|f| f.get("name").and_then(Value::as_str) == Some("status"))
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for c in f
        .get("choices")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let cid = c
            .get("id")
            .or_else(|| c.get("value"))
            .cloned()
            .unwrap_or(Value::Null);
        if cid.is_null() {
            continue;
        }
        let label = ["value", "name"]
            .iter()
            .find_map(|k| c.get(*k).filter(|v| fsv_py::truthy(v)))
            .map_or_else(|| fsv_py::py_str(&cid), fsv_py::py_str);
        out.push((cid, label));
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConfirmPlan {
    pub title: String,
    pub lines: Vec<String>,
    pub risks: Vec<String>,
}

pub fn status_plan(
    res: Res,
    item: &Item,
    schema: &Value,
    new_id: &Value,
    new_label: &str,
) -> ConfirmPlan {
    let old = resolve_status(&item.data, schema);
    let mut body = Map::new();
    body.insert("status".into(), new_id.clone());
    let label = |v: &Value| {
        let l = site::choice_label("status", v, schema);
        if l.is_empty() { fsv_py::py_str(v) } else { l }
    };
    let ctx = RiskContext {
        current: item.data.as_object(),
        status_label: Some(&label),
        kind: "item",
    };
    let risks = safety::write_risks(res.name, &body, &ctx);
    let mut lines = vec![format!("Status: {old}  ->  {new_label}")];
    if risks.is_empty() {
        lines.push("No side-effect warnings for this change.".into());
    } else {
        lines.extend(risks.iter().cloned());
    }
    if res.name == "changes" {
        lines.push(
            "Changing a change's status can send approval requests and fire workflows.".into(),
        );
    }
    ConfirmPlan {
        title: "Confirm status change".into(),
        lines,
        risks,
    }
}

/// Every write, including private notes, needs explicit review.
pub fn note_plan(public: bool, reply: bool) -> ConfirmPlan {
    let risks = if reply {
        safety::reply_risks()
    } else {
        safety::note_risks(public)
    };
    let title = if reply {
        "Confirm reply"
    } else if public {
        "Confirm public note"
    } else {
        "Confirm private note"
    };
    let lines = if risks.is_empty() {
        vec!["Add a private note to this record.".to_string()]
    } else {
        risks.clone()
    };
    ConfirmPlan {
        title: title.into(),
        lines,
        risks,
    }
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// One-line message suitable for a toast.
pub fn clean_error(f: &Failure) -> String {
    match f.error.as_ref() {
        Error::Session(m) => {
            if m.is_empty() {
                "Session expired. Log in again.".into()
            } else {
                m.clone()
            }
        }
        Error::Api(e) => {
            let body = match &e.body {
                Value::Object(o) => ["description", "message", "errors"]
                    .iter()
                    .find_map(|k| o.get(*k).filter(|v| fsv_py::truthy(v)))
                    .cloned()
                    .unwrap_or_else(|| e.body.clone()),
                other => other.clone(),
            };
            let text = collapse(&fsv_py::py_str(&body));
            format!(
                "Server rejected the request (HTTP {}): {}",
                e.status,
                text.chars().take(200).collect::<String>()
            )
        }
        other => {
            let t: String = collapse(&other.to_string()).chars().take(240).collect();
            if t.is_empty() { "Error".into() } else { t }
        }
    }
}

pub fn failure_detail(f: &Failure) -> String {
    let msg = f.error.to_string();
    let mut lines = vec![if msg.is_empty() {
        "Error".to_string()
    } else {
        msg
    }];
    for (k, v) in &f.outcome {
        lines.push(format!("{k}: {v}"));
    }
    lines.push("No automatic retry. Check the exact record and journal before another send; outcome may be unknown.".into());
    lines.join("\n")
}

/// `(label, created id)` for the toast after a form send.
pub fn form_result(mode: &str, out: &Value) -> (String, String) {
    let (label, prefix) = match mode {
        "incident" => ("Incident", "INC"),
        "sr" => ("Service Request", "SR"),
        _ => ("Change", "CHN"),
    };
    let empty = Value::Object(Map::new());
    let root = if out.is_object() { out } else { &empty };
    let result = root
        .get("result")
        .filter(|v| fsv_py::truthy(v))
        .unwrap_or(&empty);
    let candidates = [
        root.get("created"),
        result.get("ticket"),
        result.get("change"),
        Some(result),
        Some(root),
    ];
    let rows: Vec<&Value> = candidates
        .into_iter()
        .flatten()
        .filter(|r| r.is_object())
        .collect();
    let human = rows
        .iter()
        .find_map(|r| r.get("human_display_id").filter(|v| fsv_py::truthy(v)));
    let display = rows
        .iter()
        .find_map(|r| r.get("display_id").filter(|v| !v.is_null()));
    let internal = rows
        .iter()
        .find_map(|r| r.get("id").filter(|v| !v.is_null()));
    let id = if let Some(h) = human {
        fsv_py::py_str(h)
    } else if let Some(d) = display {
        format!("{prefix}-{}", fsv_py::py_str(d))
    } else if let Some(i) = internal {
        format!("ID {}", fsv_py::py_str(i))
    } else {
        "(ID unavailable; inspect journal)".into()
    };
    (label.into(), id)
}

/// Body for a one-field task update (`{}` when unchanged).
pub fn task_patch(
    current: &Value,
    field: &str,
    value: &str,
    fields: &[Value],
) -> Result<Map<String, Value>, String> {
    if !matches!(field, "title" | "status" | "group_id" | "owner_id") {
        return Err("Unsupported task field".into());
    }
    let v = match field {
        "status" => site::resolve_task_status(value, fields)?,
        "title" => Value::String(value.to_string()),
        // `str.isdecimal()` then `int()`. Unicode decimal digits count; `²` does not.
        _ => Value::from(decimal_id(value)?),
    };
    let mut out = Map::new();
    if current.get(field) != Some(&v) {
        out.insert(field.into(), v);
    }
    Ok(out)
}

/// Python `value.isdecimal() and int(value) >= 1`.
fn decimal_id(value: &str) -> Result<i64, String> {
    // `to_digit` misses some Nd blocks this toolchain's tables omit; `parse_decimal` has them.
    match fsv_core::pystr::parse_decimal(value) {
        Some(n) if !value.is_empty() && n >= 1 => Ok(n),
        _ => Err("Enter a positive numeric ID".into()),
    }
}

/// `(rows, warnings, complete)` for task / initiating-ticket targets.
pub fn target_inventory(
    tab: &TabData,
    tasks: bool,
) -> Result<(Vec<Value>, Vec<String>, bool), String> {
    if let Some(m) = &tab.missing {
        return Err(m.note());
    }
    let (rows, meta) = if tasks {
        (
            tab.data.as_array().cloned().unwrap_or_default(),
            tab.meta.clone(),
        )
    } else {
        let Some(rows) = tab.parts.get("Tickets") else {
            return Err("Initiating-ticket list unavailable; not an empty list".into());
        };
        let meta = tab
            .parts_meta
            .get("Tickets")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        (rows.as_array().cloned().unwrap_or_default(), meta)
    };
    let complete = meta.get("complete") == Some(&Value::Bool(true))
        && !meta.get("has_more").is_some_and(fsv_py::truthy)
        && !meta.get("partial").is_some_and(fsv_py::truthy);
    let mut warnings = tab.notes.clone();
    if !complete {
        warnings.push("Target inventory is partial or completeness is unknown".into());
    }
    Ok((rows, warnings, complete))
}

pub fn apply_status(b: &dyn Backend, res: Res, item_id: &Value, new_id: &Value) -> BResult<Value> {
    let _ = (b, res, item_id, new_id);
    Err(Failure::value(
        "Status requires the shared reviewed property/form sender",
    ))
}

pub fn apply_note(
    b: &dyn Backend,
    res: Res,
    item_id: &Value,
    body: &str,
    public: bool,
) -> BResult<Value> {
    let _ = (b, res, item_id, body, public);
    Err(Failure::value(
        "Notes require a shared reviewed note sender; legacy visibility is not proven",
    ))
}

pub fn apply_reply(b: &dyn Backend, res: Res, item_id: &Value, body: &str) -> BResult<Value> {
    let _ = (b, res, item_id, body);
    Err(Failure::value(
        "Replies require the shared reviewed conversation sender",
    ))
}

/// One helper invocation, with no fallback or write retry here. Link and
/// unlink send only when `reviewed` equals a freshly built plan.
pub fn apply_change_action(
    b: &dyn Backend,
    kind: &str,
    change_id: &Value,
    target: &Value,
    body: &Map<String, Value>,
    reviewed: Option<&ReviewPlan>,
) -> BResult<Option<Value>> {
    let tid = target.get("id").cloned().unwrap_or(Value::Null);
    match kind {
        "task-update" | "task-delete" => {
            let _ = (b, change_id, tid, body);
            Err(Failure::value(
                "Task mutations require the shared reviewed task sender",
            ))
        }
        "link" | "unlink" => {
            let fresh = b.association_plan(change_id, std::slice::from_ref(&tid), kind == "unlink");
            match (reviewed, fresh) {
                (Some(p), Ok(f)) if *p == f => b.send_association(p).map(Some),
                _ => Err(Failure::value(
                    "association requires the exact reviewed plan",
                )),
            }
        }
        _ => Err(Failure::value("Unsupported change action")),
    }
}

/// Fresh attachment inventory; rows without a source are `"request"`.
pub fn download_choices(b: &dyn Backend, item: &Item) -> BResult<(Vec<Value>, Vec<String>)> {
    let (mut rows, warnings) = b.download_choices(item)?;
    for row in &mut rows {
        if let Value::Object(o) = row {
            o.entry("source")
                .or_insert_with(|| Value::String("request".into()));
        }
    }
    Ok((rows, warnings))
}

pub fn download_selected(b: &dyn Backend, attachment: &Value, destination: &str) -> BResult<Value> {
    b.download(attachment, &expanduser(destination).to_string_lossy())
}

fn expanduser(p: &str) -> PathBuf {
    match p.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => match std::env::var_os("HOME") {
            Some(home) => PathBuf::from(home).join(rest.trim_start_matches('/')),
            None => PathBuf::from(p),
        },
        _ => PathBuf::from(p),
    }
}

fn resolve(p: &Path) -> PathBuf {
    if let Ok(c) = p.canonicalize() {
        return c;
    }
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(p)
    };
    let mut out = PathBuf::new();
    for c in abs.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// `catalog.safe_target`: where a server-supplied file name lands inside
/// `directory`, or why it is unsafe.
pub fn safe_target(directory: &str, name: &str) -> Result<PathBuf, String> {
    let n = name;
    let r = fsv_py::py_repr_str(n);
    if n.trim().is_empty() {
        return Err("empty file name".into());
    }
    if n.contains("..") {
        return Err(format!("unsafe file name {r}: contains '..'"));
    }
    if n.contains('/') || n.contains('\\') {
        return Err(format!("unsafe file name {r}: contains a path separator"));
    }
    let b = n.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        return Err(format!("unsafe file name {r}: absolute path"));
    }
    if n.chars().any(|c| (c as u32) < 32 || c as u32 == 127) {
        return Err(format!("unsafe file name {r}: control characters"));
    }
    let root = resolve(&expanduser(directory));
    let target = resolve(&root.join(n));
    if target.parent() != Some(root.as_path()) {
        return Err(format!(
            "unsafe file name {r}: resolves outside {}",
            root.display()
        ));
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use crate::backend::Missing;
    use fsv_core::ApiError;
    use fsv_core::resource::{CHANGES, PROBLEMS, Resource, TICKETS};
    use serde_json::json;

    #[test]
    fn read_only_setting_table() {
        for (v, want) in [
            (json!("on"), true),
            (json!(" YES "), true),
            (json!("off"), false),
            (json!(1), true),
            (json!(0), false),
            (Value::Null, false),
            (json!({}), false),
        ] {
            assert_eq!(resolve_read_only(false, &v), want, "{v}");
        }
        assert!(resolve_read_only(true, &json!("off")));
    }

    #[test]
    fn status_choices_and_plan() {
        let schema = json!({"fields": [{"name": "status", "choices": [{"id": 1, "value": "Open"}, {"id": 4, "value": "Review"}, {"value": "Odd"}]}]});
        assert_eq!(
            status_choices(&schema),
            vec![
                (json!(1), "Open".into()),
                (json!(4), "Review".into()),
                (json!("Odd"), "Odd".into())
            ]
        );
        let item = Item::new(
            &CHANGES,
            json!({"id": 7, "status": 1, "status_name": "Open"}),
        );
        let plan = status_plan(&CHANGES, &item, &schema, &json!(4), "Review");
        assert_eq!(plan.lines[0], "Status: Open  ->  Review");
        assert!(
            plan.lines
                .iter()
                .any(|l| l.contains("change status -> Review"))
        );
        assert!(
            plan.lines
                .last()
                .is_some_and(|l| l.contains("approval requests"))
        );
    }

    /// Python tests/test_tui_actions.py::test_ticket_every_status_transition_warns
    #[test]
    fn ticket_every_status_transition_warns() {
        let schema = json!({"fields": [{"name": "status", "choices": [{"id": 1, "value": "Open"}, {"id": 2, "value": "Review"}, {"id": 3, "value": "Closed"}]}]});
        let item = Item::new(
            &TICKETS,
            json!({"id": 42, "status": 1, "subject": "Demo", "display_id": 42}),
        );
        let plan = status_plan(&TICKETS, &item, &schema, &json!(2), "Review");
        assert!(!plan.risks.is_empty() && plan.risks[0].contains("Review"));
        let closed = status_plan(&TICKETS, &item, &schema, &json!(3), "Closed");
        assert!(!closed.risks.is_empty() && closed.risks[0].contains("Closed"));
    }

    #[test]
    fn clean_error_shapes() {
        assert_eq!(
            clean_error(&Failure::from(Error::session(""))),
            "Session expired. Log in again."
        );
        let api = Failure::from(Error::from(ApiError::new(
            400,
            json!({"description": "bad   thing"}),
        )));
        assert_eq!(
            clean_error(&api),
            "Server rejected the request (HTTP 400): bad thing"
        );
        assert!(can_reply(&TICKETS) && !can_reply(&CHANGES));
    }

    #[test]
    fn form_result_ids() {
        assert_eq!(
            form_result("incident", &json!({"created": {"display_id": 9}})),
            ("Incident".into(), "INC-9".into())
        );
        assert_eq!(
            form_result(
                "sr",
                &json!({"result": {"ticket": {"human_display_id": "SR-3"}}})
            ),
            ("Service Request".into(), "SR-3".into())
        );
        assert_eq!(
            form_result("change", &json!(null)).1,
            "(ID unavailable; inspect journal)"
        );
    }

    #[test]
    fn task_patch_rules() {
        let cur = json!({"title": "a", "group_id": 3});
        assert!(task_patch(&cur, "title", "a", &[]).unwrap().is_empty());
        assert_eq!(
            task_patch(&cur, "group_id", "4", &[]).unwrap()["group_id"],
            json!(4)
        );
        assert_eq!(
            task_patch(&cur, "owner_id", "0", &[]).unwrap_err(),
            "Enter a positive numeric ID"
        );
        assert_eq!(
            task_patch(&cur, "nope", "1", &[]).unwrap_err(),
            "Unsupported task field"
        );
        assert_eq!(
            task_patch(&cur, "owner_id", "1.5", &[]).unwrap_err(),
            "Enter a positive numeric ID"
        );
        assert_eq!(
            task_patch(&cur, "group_id", "१२", &[]).unwrap()["group_id"],
            json!(12)
        );
        assert_eq!(
            task_patch(&cur, "owner_id", "²", &[]).unwrap_err(),
            "Enter a positive numeric ID"
        );
    }

    #[test]
    fn capability_rules() {
        assert!(
            can_change_status(&TICKETS)
                && can_change_status(&CHANGES)
                && !can_change_status(&PROBLEMS)
        );
        assert!(can_note(&TICKETS) && can_note(&CHANGES) && can_note(&PROBLEMS));
        let no_notes = Resource {
            has_notes: false,
            ..PROBLEMS
        };
        let leaked: Res = Box::leak(Box::new(no_notes));
        assert!(!can_note(leaked));
        assert!(!can_reply(&PROBLEMS));
    }

    #[test]
    fn note_plan_titles_and_risks() {
        let private = note_plan(false, false);
        assert_eq!(private.title, "Confirm private note");
        assert_eq!(
            private.lines,
            vec!["Add a private note to this record.".to_string()]
        );
        assert!(private.risks.is_empty());
        let public = note_plan(true, false);
        assert_eq!(public.title, "Confirm public note");
        assert!(public.lines[0].contains("visible to the requester"));
        let reply = note_plan(false, true);
        assert_eq!(reply.title, "Confirm reply");
        assert!(reply.lines[0].contains("sent to the requester by email"));
    }

    #[test]
    fn ticket_status_plan_has_no_change_footer() {
        let schema = json!({"fields": [{"name": "status", "choices": [{"id": 2, "value": "Open"}, {"id": 5, "value": "Closed"}]}]});
        let item = Item::new(&TICKETS, json!({"id": 1, "status": 2}));
        let plan = status_plan(&TICKETS, &item, &schema, &json!(5), "Closed");
        assert_eq!(plan.title, "Confirm status change");
        assert!(!plan.risks.is_empty(), "{plan:?}");
        assert!(!plan.lines.iter().any(|l| l.contains("approval requests")));
    }

    #[test]
    fn failure_detail_lists_outcome_and_no_retry() {
        let mut f = Failure::value("boom");
        f.outcome.push(("created_id".into(), "42".into()));
        let text = failure_detail(&f);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "boom");
        assert_eq!(lines[1], "created_id: 42");
        assert!(lines[2].starts_with("No automatic retry."));
        assert_eq!(clean_error(&Failure::value("a\n  b")), "a b");
    }

    #[test]
    fn target_inventory_rules() {
        let mut tab = TabData {
            data: json!([{"id": 1}]),
            ..Default::default()
        };
        tab.meta.insert("complete".into(), json!(true));
        let (rows, warnings, complete) = target_inventory(&tab, true).unwrap();
        assert_eq!((rows.len(), warnings.len(), complete), (1, 0, true));
        tab.meta.insert("has_more".into(), json!(true));
        let (_, warnings, complete) = target_inventory(&tab, true).unwrap();
        assert!(!complete && warnings[0].contains("partial"));
        assert_eq!(
            target_inventory(&tab, false).unwrap_err(),
            "Initiating-ticket list unavailable; not an empty list"
        );
        let denied = TabData {
            missing: Some(Missing {
                what: "Associations".into(),
                status: Some(403),
            }),
            ..Default::default()
        };
        assert_eq!(
            target_inventory(&denied, false).unwrap_err(),
            "Associations: not permitted for your role (403)"
        );
        let mut assoc = TabData::default();
        assoc.parts.insert("Tickets".into(), json!([{"id": 9}]));
        assoc
            .parts_meta
            .insert("Tickets".into(), json!({"complete": true}));
        assert!(target_inventory(&assoc, false).unwrap().2);
    }

    #[derive(Default)]
    struct Rec {
        calls: Mutex<Vec<String>>,
        plan: Option<ReviewPlan>,
    }

    impl Rec {
        fn log(&self, s: String) {
            self.calls.lock().unwrap().push(s);
        }
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Backend for Rec {
        fn update_item(&self, res: Res, id: &Value, body: &Map<String, Value>) -> BResult<Value> {
            self.log(format!(
                "update {} {id} {}",
                res.name,
                Value::Object(body.clone())
            ));
            Ok(json!({}))
        }
        fn add_note(&self, res: Res, id: &Value, body: &str, public: bool) -> BResult<Value> {
            self.log(format!("note {} {id} {body} {public}", res.name));
            Ok(json!({}))
        }
        fn update_task(&self, c: &Value, t: &Value, body: &Map<String, Value>) -> BResult<()> {
            self.log(format!(
                "task-update {c} {t} {}",
                Value::Object(body.clone())
            ));
            Ok(())
        }
        fn delete_task(&self, c: &Value, t: &Value) -> BResult<()> {
            self.log(format!("task-delete {c} {t}"));
            Ok(())
        }
        fn association_plan(
            &self,
            _c: &Value,
            _ids: &[Value],
            _unlink: bool,
        ) -> BResult<ReviewPlan> {
            self.plan.clone().ok_or_else(|| Failure::value("no plan"))
        }
        fn send_association(&self, plan: &ReviewPlan) -> BResult<Value> {
            self.log(format!("assoc {}", plan.hash));
            Ok(json!({"verified": true}))
        }
        fn download_choices(&self, _item: &Item) -> BResult<(Vec<Value>, Vec<String>)> {
            Ok((
                vec![json!({"id": 1}), json!({"id": 2, "source": "cloud file"})],
                vec!["partial".into()],
            ))
        }
        fn download(&self, attachment: &Value, destination: &str) -> BResult<Value> {
            self.log(format!("download {} {destination}", attachment["id"]));
            Ok(json!({"status": "downloaded"}))
        }
        crate::backend_stubs!(
            load_schema,
            current_user,
            load_views,
            ticket_headers,
            view_settings,
            build_query_hash,
            list_page,
            search_items,
            load_detail,
            load_tab,
            remote_user_values,
            lookup_choices,
            ticket_note_plan,
            send_ticket_note,
            change_task_plan,
            send_change_task,
            search_change_tickets,
            task_for_edit,
            approval_request_plan,
            approval_targets,
            approval_action_plan,
            send_approval,
            ticket_property_fields,
            ticket_property_plan,
            send_ticket_property,
            known_error_plan,
            send_known_error,
            planning_definitions,
            planning_row,
            planning_text_plan,
            send_planning_text,
            ticket_conversation_plan,
            send_ticket_conversation,
            asset_candidates,
            open_form,
            catalog
        );
    }

    fn plan(hash: &str) -> ReviewPlan {
        ReviewPlan {
            review: json!({}),
            hash: hash.into(),
            public: false,
            target_id: json!(1),
            raw: Value::Null,
        }
    }

    #[test]
    fn apply_status_note_send_once() {
        let b = Rec::default();
        let status = apply_status(&b, &CHANGES, &json!(7), &json!(4)).unwrap_err();
        assert!(
            status
                .to_string()
                .contains("shared reviewed property/form sender")
        );
        let note = apply_note(&b, &CHANGES, &json!(7), "hi", true).unwrap_err();
        assert!(note.to_string().contains("shared reviewed note sender"));
        let reply = apply_reply(&b, &TICKETS, &json!(3), "x").unwrap_err();
        assert!(
            reply
                .to_string()
                .contains("shared reviewed conversation sender")
        );
        assert!(b.calls().is_empty());
    }

    #[test]
    fn apply_change_action_kinds() {
        let b = Rec {
            plan: Some(plan("h1")),
            ..Default::default()
        };
        let body: Map<String, Value> = [("title".to_string(), json!("t"))].into_iter().collect();
        let target = json!({"id": 5});
        let update =
            apply_change_action(&b, "task-update", &json!(1), &target, &body, None).unwrap_err();
        assert!(update.to_string().contains("shared reviewed task sender"));
        let delete = apply_change_action(&b, "task-delete", &json!(1), &target, &Map::new(), None)
            .unwrap_err();
        assert!(delete.to_string().contains("shared reviewed task sender"));
        let sent = apply_change_action(
            &b,
            "link",
            &json!(1),
            &target,
            &Map::new(),
            Some(&plan("h1")),
        )
        .unwrap();
        assert_eq!(sent, Some(json!({"verified": true})));
        let stale = apply_change_action(
            &b,
            "unlink",
            &json!(1),
            &target,
            &Map::new(),
            Some(&plan("old")),
        )
        .unwrap_err();
        assert!(stale.to_string().contains("exact reviewed plan"));
        assert!(apply_change_action(&b, "link", &json!(1), &target, &Map::new(), None).is_err());
        let bad =
            apply_change_action(&b, "nope", &json!(1), &target, &Map::new(), None).unwrap_err();
        assert_eq!(bad.to_string(), "Unsupported change action");
        assert_eq!(b.calls(), vec!["assoc h1"]);
    }

    #[test]
    fn download_choices_default_source_and_selected() {
        let b = Rec::default();
        let item = Item::new(&TICKETS, json!({"id": 1}));
        let (rows, warnings) = download_choices(&b, &item).unwrap();
        assert_eq!(rows[0]["source"], json!("request"));
        assert_eq!(rows[1]["source"], json!("cloud file"));
        assert_eq!(warnings, vec!["partial".to_string()]);
        download_selected(&b, &json!({"id": 1}), "/tmp/x").unwrap();
        assert_eq!(b.calls(), vec!["download 1 /tmp/x"]);
    }

    #[test]
    fn safe_target_rejects_unsafe_names() {
        let dir = std::env::temp_dir().join(format!("fsv-tui-safe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let d = dir.to_string_lossy().to_string();
        let ok = safe_target(&d, "report.pdf").unwrap();
        assert_eq!(ok, dir.canonicalize().unwrap().join("report.pdf"));
        assert_eq!(safe_target(&d, "  ").unwrap_err(), "empty file name");
        assert!(
            safe_target(&d, "a..b")
                .unwrap_err()
                .ends_with("contains '..'")
        );
        assert!(
            safe_target(&d, "a/b")
                .unwrap_err()
                .ends_with("contains a path separator")
        );
        assert!(
            safe_target(&d, "a\\b")
                .unwrap_err()
                .ends_with("contains a path separator")
        );
        assert!(
            safe_target(&d, "C:evil")
                .unwrap_err()
                .ends_with("absolute path")
        );
        assert!(
            safe_target(&d, "a\u{7}b")
                .unwrap_err()
                .ends_with("control characters")
        );
        assert!(
            safe_target(&d, ".")
                .unwrap_err()
                .contains("resolves outside")
        );
        assert_eq!(
            safe_target(&d, "bad/x").unwrap_err(),
            "unsafe file name 'bad/x': contains a path separator"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
