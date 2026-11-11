//! The invented "Acme" change form (port of `tests/acme_change_form.py`)
//! over recording fakes: no network, no terminal.

#![allow(dead_code)]

use std::path::Path;
use std::sync::{Arc, Mutex};

use fsv_core::review::Review;
use fsv_forms::tz::TzEnv;
use fsv_forms::{EngineOptions, Field, FormSpec, LookupValue, RulesEngine, Values};
use fsv_formui::lookups::{ApiFailure, FormApi, Resolver};
use fsv_formui::session::{FormSession, SessionInit};
use fsv_formui::{FormBackend, FormKind, FormMode, Plan, PlanRequest};
use serde_json::{Map, Value as Json, json};

pub const ACCOUNT: &str = "900000777";

fn field(id: u64, name: &str, label: &str, ft: &str, pos: Option<i64>, extra: Json) -> Json {
    let default = ft.starts_with("default_");
    let mut out = json!({
        "id": id, "name": name, "label": label, "description": "", "field_type": ft,
        "required": false, "required_for_closure": false, "position": pos,
        "default_field": default, "choices": [], "nested_fields": [],
        "placeholder": "", "helptext": "", "hint": false,
        "content": null, "displayed_to_agents": true, "data_source": 0,
        "lookup_config": null, "workspace_id": if default { 1 } else { 2 },
        "updated_at": "2030-01-02T03:04:05Z",
    });
    if let (Some(o), Some(e)) = (out.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            o.insert(k.clone(), v.clone());
        }
    }
    out
}

fn cc(pairs: &[(u64, u64, &str)]) -> Json {
    Json::Array(
        pairs
            .iter()
            .map(|(i, d, v)| json!({"id": i, "display_id": d, "value": v, "label": v}))
            .collect(),
    )
}

pub const SERVICES: [&str; 14] = [
    "Billing", "Search", "Reports", "Ledger", "Mail", "Chat", "Docs", "Wiki", "Pager", "Auth",
    "Queue", "Cache", "Storage", "Metrics",
];

pub fn fields() -> Json {
    let mut id = 9000006000u64;
    let mut next = || {
        id += 1;
        id
    };
    let services: Vec<(u64, u64, &str)> = SERVICES
        .iter()
        .enumerate()
        .map(|(i, s)| (9000002200 + i as u64, 9000003200 + i as u64, *s))
        .collect();
    let lohi = json!([{"id": 1, "value": "Low"}, {"id": 2, "value": "Medium"}, {"id": 3, "value": "High"}]);
    json!({"change_fields": [
        field(next(), "requester", "Requester", "default_requester", Some(1), json!({"required": true})),
        field(next(), "subject", "Subject", "default_subject", Some(2),
              json!({"required": true, "placeholder": "Service: objective"})),
        field(next(), "change_type", "Change Type", "default_change_type", Some(3), json!({"required": true,
              "choices": [{"id": 1, "value": "Normal"}, {"id": 2, "value": "Standard"}, {"id": 4, "value": "Emergency"}]})),
        field(next(), "cloud_only", "Cloud Only", "custom_dropdown", Some(4), json!({"required": true,
              "helptext": "Only the cloud team performs this change",
              "choices": cc(&[(9000002101, 9000003101, "Yes"), (9000002102, 9000003102, "No")])})),
        field(next(), "stage", "Stage", "custom_dropdown", Some(5),
              json!({"choices": cc(&[(9000002111, 9000003111, "Live"), (9000002112, 9000003112, "Staging")])})),
        field(next(), "staging_start", "Staging Start", "custom_date", Some(6), json!({})),
        field(next(), "staging_end", "Staging End", "custom_date", Some(7), json!({})),
        field(next(), "services", "Services", "custom_multi_select_dropdown", Some(8),
              json!({"choices": cc(&services)})),
        field(next(), "notify_ops", "Notify Ops Team", "custom_dropdown", Some(9),
              json!({"choices": cc(&[(9000002301, 9000003301, "Yes"), (9000002302, 9000003302, "No")])})),
        field(next(), "reviewer", "Reviewer", "custom_lookup", Some(10), json!({"data_source": 5,
              "lookup_config": {"data_source": 5,
                                "link": format!("/lookup_choices?field_name=reviewer_{ACCOUNT}&module=change")}})),
        field(next(), "downtime_window", "Downtime Window", "custom_text", Some(11),
              json!({"placeholder": "22:00 - 22:30"})),
        field(next(), "legacy_flag", "Legacy Flag", "custom_checkbox", Some(12), json!({})),
        field(next(), "description", "Description", "default_description", Some(13),
              json!({"required": true, "helptext": "What and why"})),
        field(next(), "priority", "Priority", "default_priority", Some(14), json!({"required": true,
              "helptext": "Low - Can wait || Medium - Planned work || High - Revenue at risk || Urgent - Outage",
              "choices": [{"id": 1, "value": "Low"}, {"id": 2, "value": "Medium"}, {"id": 3, "value": "High"},
                          {"id": 4, "value": "Urgent"}]})),
        field(next(), "impact", "Impact", "default_impact", Some(15), json!({"required": true, "choices": lohi})),
        field(next(), "risk", "Risk", "default_risk", Some(16), json!({"required": true, "choices": lohi})),
        field(next(), "group", "Group", "default_group", Some(17), json!({})),
        field(next(), "agent", "Agent", "default_agent", Some(18), json!({})),
        field(next(), "planned_start_date", "Planned Start Date", "default_planned_start_date", Some(19),
              json!({"required": true})),
        field(next(), "planned_end_date", "Planned End Date", "default_planned_end_date", Some(20),
              json!({"required": true})),
        field(next(), "status", "Status", "default_status", Some(21), json!({"required": true,
              "choices": [{"id": 1, "value": "Open"}, {"id": 2, "value": "Planning"}, {"id": 3, "value": "Closed"}]})),
        field(next(), "go_live_checklist", "Go-Live Checklist", "planning_field", None, json!({})),
    ]})
}

fn rule(id: u64, name: &str, cond: Json, actions: Json, form: &str, validation: bool) -> Json {
    json!({"id": id, "name": name, "workspace_id": 2, "module_type": "change", "item_id": null,
           "deleted": false, "user_type": "both", "form_type": form, "state": "active",
           "rule_type": "client_side", "description": "", "condition_data": cond,
           "action_data": {"action": actions}, "is_reversible": false, "is_validation": validation,
           "updated_at": "2030-01-03T00:00:00Z"})
}

fn c(name: &str, op: &str, value: &str) -> Json {
    json!({"name": name, "operator": op, "evaluate_on": "change", "value": value})
}

pub fn rules(user_tz: Option<&str>) -> Json {
    let cf = "customFields.";
    let mut out = json!({"business_rules": [
        rule(9000004101, "Cloud only: no", json!({"all": [c(&format!("{cf}cloud_only"), "is", "No")]}), json!([
            {"name": [format!("{cf}stage")], "property": "populate", "values": [9000003112u64]},
            {"name": [format!("{cf}staging_start"), format!("{cf}staging_end")], "property": "show"},
            {"name": [format!("{cf}staging_start"), format!("{cf}staging_end")], "property": "mandate"},
        ]), "both", false),
        rule(9000004102, "Cloud only: yes", json!({"all": [c(&format!("{cf}cloud_only"), "is", "Yes")]}), json!([
            {"name": [format!("{cf}stage")], "property": "populate", "values": [9000003111u64]},
            {"name": [format!("{cf}staging_start"), format!("{cf}staging_end")], "property": "hide"},
        ]), "both", false),
        rule(9000004103, "Hide legacy flag", json!({"all": null}),
             json!([{"name": [format!("{cf}legacy_flag")], "property": "hide"}]), "both", false),
        rule(9000004104, "Urgent priority", json!({"all": [c("priority", "is", "4")]}),
             json!([{"name": ["Urgent changes need the duty lead's approval"], "property": "message"}]),
             "both", true),
        rule(9000004105, "Edit: live needs a window", json!({"all": [c(&format!("{cf}stage"), "is", "Live")]}),
             json!([{"name": [format!("{cf}downtime_window")], "property": "mandate"}]), "edit_form", false),
    ]});
    if let Some(tz) = user_tz {
        out["user"] = json!({"first_name": "Robin", "last_name": "Agent",
                             "email": "robin.agent@example.com", "time_zone": tz});
    }
    out
}

pub fn people() -> Vec<Json> {
    vec![
        json!({"id": 9000007001u64, "value": "Jane Doe", "email": "jane.doe@example.com", "details": "jane.doe@example.com"}),
        json!({"id": 9000007002u64, "value": "Jane Doerr", "email": "jane.doerr@example.com", "details": "jane.doerr@example.com"}),
    ]
}

pub fn jane() -> LookupValue {
    LookupValue::new(9000007001u64, "Jane Doe", Some("jane.doe@example.com"))
}

/// Records every call; serves the Acme people and groups.
#[derive(Default)]
pub struct FakeApi {
    pub calls: Mutex<Vec<String>>,
    pub deny_people: bool,
    pub deny_boot: bool,
    pub profile_tz: Option<String>,
}

impl FakeApi {
    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("lock").clone()
    }
}

impl FormApi for FakeApi {
    fn autocomplete(
        &self,
        kind: &str,
        query: &str,
        _p: Map<String, Json>,
    ) -> Result<Vec<Json>, ApiFailure> {
        self.calls
            .lock()
            .expect("lock")
            .push(format!("AUTO {kind} {query}"));
        if self.deny_people {
            return Err(ApiFailure::new(Some(403), "forbidden"));
        }
        let q = query.to_lowercase();
        Ok(people()
            .into_iter()
            .filter(|p| {
                p["value"]
                    .as_str()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&q)
                    || p["email"].as_str().unwrap_or("").contains(&q)
            })
            .collect())
    }

    fn int_get(&self, path: &str) -> Result<Json, ApiFailure> {
        self.calls.lock().expect("lock").push(format!("GET {path}"));
        if path == "bootstrap/agents_groups" {
            if self.deny_boot {
                return Err(ApiFailure::new(Some(403), "forbidden"));
            }
            return Ok(json!({"groups": [{"id": 9000009001u64, "name": "Platform Ops"}]}));
        }
        Ok(json!({}))
    }

    fn check_session(&self) -> Result<Json, ApiFailure> {
        self.calls.lock().expect("lock").push("ME".into());
        Ok(match &self.profile_tz {
            Some(tz) => json!({"time_zone": tz}),
            None => json!({}),
        })
    }
}

/// `lookup_choices` over the Acme people.
pub struct FakeResolver;

impl Resolver for FakeResolver {
    fn lookup(
        &self,
        _entity: &str,
        query: &str,
        _f: &Field,
    ) -> Result<Vec<LookupValue>, ApiFailure> {
        let q = query.to_lowercase();
        Ok(people()
            .into_iter()
            .filter(|p| {
                p["value"]
                    .as_str()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&q)
            })
            .map(|p| LookupValue::new(p["id"].clone(), p["value"].as_str().unwrap_or(""), None))
            .collect())
    }
}

/// A plan builder that only reports what it was given.
#[derive(Default)]
pub struct FakeBackend {
    pub errors: Mutex<Vec<String>>,
    pub builds: Mutex<usize>,
    pub saved: Mutex<Vec<String>>,
    pub sent: Mutex<Vec<String>>,
    pub refuse_save: bool,
}

impl FormBackend for FakeBackend {
    fn build_plan(&self, req: &PlanRequest<'_>) -> Result<Plan, String> {
        *self.builds.lock().expect("lock") += 1;
        let errors = self.errors.lock().expect("lock").clone();
        let mut review = Review::new("New Change");
        for (k, v) in &req.draft.answers {
            review.rows.push((k.clone(), v.py_str()));
        }
        review.errors = errors.clone();
        let hash = if errors.is_empty() {
            "0123456789abcdef0123456789".to_string()
        } else {
            String::new()
        };
        review.plan_hash = hash.clone();
        Ok(Plan {
            errors,
            hash,
            review,
            payload: Some(json!({"subject": req.draft.answers.get("subject").map(|v| v.py_str())})),
            detail: None,
        })
    }

    fn save_draft(&self, req: &PlanRequest<'_>, _plan: &Plan, path: &Path) -> std::io::Result<()> {
        if self.refuse_save {
            return Err(std::io::Error::from_raw_os_error(13));
        }
        std::fs::write(path, format!("answers: {}\n", req.draft.answers.len()))?;
        self.saved
            .lock()
            .expect("lock")
            .push(path.to_string_lossy().into_owned());
        Ok(())
    }

    fn send(
        &self,
        _req: &PlanRequest<'_>,
        plan: &Plan,
        _allow: bool,
    ) -> Result<Json, fsv_core::Error> {
        self.sent.lock().expect("lock").push(plan.hash.clone());
        Ok(json!({"id": 900000502}))
    }
}

pub struct Acme {
    pub api: Arc<FakeApi>,
    pub backend: Arc<FakeBackend>,
}

pub fn spec() -> Arc<FormSpec> {
    Arc::new(FormSpec::from_form_fields(&fields(), "change").expect("spec"))
}

pub fn engine(spec: &Arc<FormSpec>, rules_json: &Json) -> RulesEngine {
    let opts = EngineOptions {
        module: Some("change".into()),
        workspace_id: Some(json!(2)),
        ..EngineOptions::default()
    };
    RulesEngine::new(spec.clone(), Some(rules_json), opts).expect("engine")
}

pub fn init(
    kind: FormKind,
    mode: FormMode,
    api: Arc<FakeApi>,
    backend: Arc<FakeBackend>,
) -> SessionInit {
    let spec = spec();
    let rules = engine(&spec, &rules(Some("Tokyo")));
    SessionInit {
        kind,
        mode,
        title: "New Change".into(),
        spec,
        rules: Some(rules),
        rules_warnings: vec![],
        record: json!({}),
        target: None,
        existing: Values::new(),
        requester: None,
        requester_note: String::new(),
        dry_run: false,
        max_file_bytes: None,
        tz_env: Some(TzEnv::empty()),
        paths: fsv_formui::files::Paths::new(None, std::env::temp_dir()),
        api,
        resolver: Arc::new(FakeResolver),
        backend,
    }
}

pub fn acme() -> (Acme, FormSession) {
    let api = Arc::new(FakeApi::default());
    let backend = Arc::new(FakeBackend::default());
    let s = FormSession::new(init(
        FormKind::Change,
        FormMode::New,
        api.clone(),
        backend.clone(),
    ))
    .expect("session");
    (Acme { api, backend }, s)
}
