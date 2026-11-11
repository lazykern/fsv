//! [`ChangeWriter`] over `fsv_api::create` (the `fsv.create` calls
//! `change_flow.execute` makes).

use std::sync::Arc;

use serde_json::{Map, Value as Json};

use fsv_api::create::{self, DuplicateOpts, Prompter, StdinPrompter};
use fsv_api::service::Api;
use fsv_core::{Error, Result};
use fsv_flows::change::{ChangeWriter, PlanningUpdate};

/// Change writes on the live tenant. Same-name prompts (`--duplicate
/// prompt`) go to `prompter`.
#[derive(Clone)]
pub struct LiveWriter {
    pub api: Arc<dyn Api + Send + Sync>,
    pub prompter: Arc<dyn Prompter + Send + Sync>,
}

impl LiveWriter {
    pub fn new(api: Arc<dyn Api + Send + Sync>) -> LiveWriter {
        LiveWriter {
            api,
            prompter: Arc::new(StdinPrompter),
        }
    }

    pub fn with_prompter(mut self, prompter: Arc<dyn Prompter + Send + Sync>) -> LiveWriter {
        self.prompter = prompter;
        self
    }
}

/// The change id a path is built from (`f"changes/{change_id}"`).
pub fn change_id(v: &Json) -> Result<i64> {
    match v {
        Json::Number(n) if n.is_i64() => Ok(n.as_i64().unwrap_or_default()),
        Json::String(s) if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) => s
            .parse()
            .map_err(|_| Error::value(format!("invalid change id {}", fsv_py::py_repr(v)))),
        other => Err(Error::value(format!(
            "invalid change id {}",
            fsv_py::py_repr(other)
        ))),
    }
}

fn dup(mode: &str) -> DuplicateOpts {
    DuplicateOpts {
        duplicate: Some(mode.to_string()),
        ..DuplicateOpts::default()
    }
}

impl ChangeWriter for LiveWriter {
    fn upload_file(&self, path: &str) -> Result<Json> {
        create::upload_file(self.api.as_ref(), path).map(Json::from)
    }

    fn post_change_body(&self, body: &Map<String, Json>) -> Result<Json> {
        create::post_change_body(self.api.as_ref(), &Json::Object(body.clone()))
    }

    fn put_change_body(&self, change_id_v: &Json, body: &Map<String, Json>) -> Result<Json> {
        create::put_change_body(
            self.api.as_ref(),
            change_id(change_id_v)?,
            &Json::Object(body.clone()),
        )
    }

    fn attach_files_to_change(
        &self,
        change_id_v: &Json,
        paths: &[String],
        duplicate: &str,
    ) -> Result<()> {
        create::attach_files_to_change(
            self.api.as_ref(),
            change_id(change_id_v)?,
            paths,
            &dup(duplicate),
            self.prompter.as_ref(),
        )
        .map(|_| ())
    }

    fn update_planning_field(&self, change_id_v: &Json, req: &PlanningUpdate<'_>) -> Result<()> {
        let label = Some(req.label).filter(|l| !l.is_empty());
        create::update_planning_field(
            self.api.as_ref(),
            change_id(change_id_v)?,
            req.field_id,
            req.description,
            req.file_paths,
            &dup(req.duplicate),
            label,
            self.prompter.as_ref(),
        )
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{Tenant, ok, tenant};
    use serde_json::json;

    const UPLOAD: &str = r#"{"content":"data:text/plain;base64,aGVsbG8=","content_file_name":"a.txt","content_file_size":5,"content_content_type":"text/plain"}"#;

    fn server() -> Tenant {
        tenant(|c| match (c.method.as_str(), c.path.as_str()) {
            ("POST", "/api/_/attachments") => {
                let b = c.json().unwrap();
                ok(
                    json!({"id": 501, "content_file_name": b["content_file_name"], "content_file_size": b["content_file_size"]}),
                )
            }
            ("POST", "/api/_/changes") => ok(json!({"change": {"id": 77, "display_id": 77}})),
            ("PUT", "/api/_/changes/77") => ok(json!({"change": {"id": 77}})),
            ("GET", "/api/_/changes/77") => {
                ok(json!({"change": {"attachments": [{"id": 9, "name": "old.txt"}]}}))
            }
            ("GET", "/api/_/changes/77/planning-fields") => ok(json!({"change_planning_fields": [
                {"name": "cf_rollout_plan", "label": "Rollout plan", "description": "<p>old</p>", "attachments": [{"id": 3, "name": "z.txt"}]}
            ]})),
            ("GET", "/api/_/changes/78/planning-fields") => {
                ok(json!({"change_planning_fields": []}))
            }
            ("PUT", "/api/_/changes/77/planning-fields/cf_rollout_plan") => {
                ok(json!({"change": {"id": 77}}))
            }
            ("POST", "/api/_/changes/78/planning-fields") => ok(json!({"ok": true})),
            _ => None,
        })
    }

    fn file(t: &Tenant) -> String {
        let p = t.dir.path().join("a.txt");
        std::fs::write(&p, "hello").unwrap();
        p.to_string_lossy().into_owned()
    }

    fn writes(t: &Tenant) -> Vec<(String, String)> {
        t.calls()
            .into_iter()
            .map(|c| (c.line(), c.body.unwrap_or_default()))
            .collect()
    }

    /// Bodies below are what `create.py` sends (captured with a recording client).
    #[test]
    fn upload_file_posts_a_data_uri() {
        let t = server();
        let w = LiveWriter::new(t.client.clone());
        assert_eq!(w.upload_file(&file(&t)).unwrap(), json!(501));
        assert_eq!(
            writes(&t),
            [("POST /api/_/attachments".to_string(), UPLOAD.to_string())]
        );
        let req = &t
            .mock
            .requests()
            .into_iter()
            .find(|r| r.method == "POST")
            .unwrap();
        assert_eq!(req.header("X-CSRF-Token"), Some("tok"));
    }

    #[test]
    fn post_change_body_is_compact_utf8_and_unwrapped() {
        let t = server();
        let w = LiveWriter::new(t.client.clone());
        let body = crate::testkit::obj(json!({"subject": "S é", "custom_fields": {"x": 1}}));
        assert_eq!(
            w.post_change_body(&body).unwrap(),
            json!({"id": 77, "display_id": 77})
        );
        assert_eq!(
            writes(&t),
            [(
                "POST /api/_/changes".to_string(),
                r#"{"subject":"S é","custom_fields":{"x":1}}"#.to_string()
            )]
        );
    }

    #[test]
    fn put_change_body_targets_the_change() {
        let t = server();
        let w = LiveWriter::new(t.client.clone());
        let body = crate::testkit::obj(json!({"attachments": [501]}));
        assert_eq!(
            w.put_change_body(&json!(77), &body).unwrap(),
            json!({"id": 77})
        );
        assert_eq!(
            writes(&t),
            [(
                "PUT /api/_/changes/77".to_string(),
                r#"{"attachments":[501]}"#.to_string()
            )]
        );
        assert!(w.put_change_body(&json!("x"), &body).is_err());
    }

    #[test]
    fn attach_appends_to_the_existing_attachments() {
        let t = server();
        let w = LiveWriter::new(t.client.clone());
        w.attach_files_to_change(&json!(77), &[file(&t)], "append")
            .unwrap();
        assert_eq!(
            writes(&t),
            [
                ("GET /api/_/changes/77".to_string(), String::new()),
                ("POST /api/_/attachments".to_string(), UPLOAD.to_string()),
                (
                    "PUT /api/_/changes/77".to_string(),
                    r#"{"attachments":[9,501]}"#.to_string()
                ),
            ]
        );
    }

    #[test]
    fn planning_field_posts_a_new_row_and_puts_an_existing_one() {
        let t = server();
        let w = LiveWriter::new(t.client.clone());
        let files = [file(&t)];
        let req = PlanningUpdate {
            field_id: "cf_rollout_plan",
            description: Some("<p>x</p>"),
            file_paths: &files,
            duplicate: "append",
            label: "Rollout plan",
        };
        w.update_planning_field(&json!(78), &req).unwrap();
        let req = PlanningUpdate {
            description: None,
            ..req
        };
        w.update_planning_field(&json!(77), &req).unwrap();
        assert_eq!(
            writes(&t),
            [
                (
                    "GET /api/_/changes/78/planning-fields".to_string(),
                    String::new()
                ),
                ("POST /api/_/attachments".to_string(), UPLOAD.to_string()),
                (
                    "POST /api/_/changes/78/planning-fields?id=cf_rollout_plan".to_string(),
                    r#"{"description":"<p>x</p>","attachments":[501]}"#.to_string()
                ),
                (
                    "GET /api/_/changes/77/planning-fields".to_string(),
                    String::new()
                ),
                ("POST /api/_/attachments".to_string(), UPLOAD.to_string()),
                (
                    "PUT /api/_/changes/77/planning-fields/cf_rollout_plan".to_string(),
                    r#"{"attachments":[3,501]}"#.to_string()
                ),
            ]
        );
    }

    #[test]
    fn change_id_accepts_ints_and_digit_strings() {
        assert_eq!(change_id(&json!(7)).unwrap(), 7);
        assert_eq!(change_id(&json!("12")).unwrap(), 12);
        assert!(change_id(&json!(null)).is_err());
    }

    #[test]
    fn with_prompter_replaces_stdin() {
        struct No;
        impl Prompter for No {
            fn interactive(&self) -> bool {
                false
            }
            fn ask(&self, _p: &str) -> String {
                String::new()
            }
        }
        let t = server();
        let w = LiveWriter::new(t.client.clone()).with_prompter(Arc::new(No));
        assert!(!w.prompter.interactive());
    }
}
