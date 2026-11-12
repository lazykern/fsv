//! What the form hands to the plan builders and the send path.
//!
//! The form never builds payloads, hashes or journal entries itself: the
//! draft goes to the same builders the YAML and `--set` faces use
//! (`change_flow.build_create_plan` / `build_update_plan`,
//! `ticket_flow.build_create_plan`, `catalog.build_sr_plan`), so the review,
//! payload, plan hash, duplicate check, freshness check and
//! upload-after-confirm are theirs. Those live in `fsv-flows`; this crate
//! only defines the seam.

use std::any::Any;
use std::path::Path;
use std::sync::Arc;

use fsv_core::review::Review;
use fsv_forms::{LookupValue, Value};
use indexmap::IndexMap;
use serde_json::Value as Json;

/// `fsv.drafts.Draft` as the form fills it in. Answers hold parsed values
/// (choice values, aware date-times, lookup values) so building the plan
/// never re-resolves a lookup over the network.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Draft {
    /// API name -> value; `Value::Null` on an edit form clears the field.
    pub answers: IndexMap<String, Value>,
    /// The requester's email (or name when there is none).
    pub requester: Option<String>,
    /// `None`: unchanged (edit) / not given (new); `Some("")`: cleared.
    pub description_html: Option<String>,
    pub attachments: Vec<String>,
    /// Planning field label -> files.
    pub planning: IndexMap<String, PlanningFiles>,
    pub ack: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlanningFiles {
    pub files: Vec<String>,
    pub description_html: Option<String>,
}

/// A reviewed plan. `detail` is the builder's own plan (fsv-flows type),
/// handed back to [`FormBackend::send`] / `save_draft` unchanged.
#[derive(Clone, Default)]
pub struct Plan {
    pub errors: Vec<String>,
    /// Empty when the plan may not be sent (errors, or no changes on edit).
    pub hash: String,
    pub review: Review,
    pub payload: Option<Json>,
    pub detail: Option<Arc<dyn Any + Send + Sync>>,
}

impl Plan {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }

    /// What the person typed last was not taken: never submit the older
    /// value silently. Adds `pending` input errors to the plan and its
    /// review and clears the hash.
    pub fn block_on(&mut self, pending: &[String]) {
        let new: Vec<String> = pending
            .iter()
            .filter(|e| !self.errors.contains(e))
            .cloned()
            .collect();
        if new.is_empty() {
            return;
        }
        self.errors.extend(new.iter().cloned());
        for e in new {
            if !self.review.errors.contains(&e) {
                self.review.errors.push(e);
            }
        }
        self.hash.clear();
        self.review.plan_hash.clear();
    }
}

impl std::fmt::Debug for Plan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Plan")
            .field("errors", &self.errors)
            .field("hash", &self.hash)
            .field("payload", &self.payload)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FormKind {
    Change,
    Sr,
    Ticket,
}

impl FormKind {
    /// `open_session` aliases.
    pub fn parse(kind: &str) -> Result<FormKind, String> {
        match kind {
            "change" | "changes" => Ok(FormKind::Change),
            "sr" | "service_request" => Ok(FormKind::Sr),
            "ticket" | "tickets" | "incident" => Ok(FormKind::Ticket),
            other => Err(format!(
                "unknown form kind {}; expected change, ticket or sr",
                fsv_py::py_repr_str(other)
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FormMode {
    New,
    Edit,
}

impl FormMode {
    pub fn as_str(self) -> &'static str {
        match self {
            FormMode::New => "new",
            FormMode::Edit => "edit",
        }
    }
}

/// Everything a plan builder needs from the form.
pub struct PlanRequest<'a> {
    pub kind: FormKind,
    pub mode: FormMode,
    pub draft: &'a Draft,
    pub record: &'a Json,
    pub target: Option<&'a Json>,
    pub requester: Option<&'a LookupValue>,
    pub ack_all: bool,
    /// Input errors the builder must keep when it rebuilds the plan (the
    /// freshness re-check): apply them with [`Plan::block_on`].
    pub pending_errors: &'a [String],
}

/// The plan builders and the send path (`fsv-flows`), injected.
pub trait FormBackend {
    fn build_plan(&self, req: &PlanRequest<'_>) -> Result<Plan, String>;
    /// Write the draft YAML (SR: `render_sr_template` with the file's folder
    /// as base dir; else `change_flow.save_draft`).
    fn save_draft(&self, req: &PlanRequest<'_>, plan: &Plan, path: &Path) -> std::io::Result<()>;
    /// Duplicate check, metadata freshness re-check (refuse when the plan
    /// changed: `FlowExit(4, ...)`), then the writes (uploads only now).
    fn send(
        &self,
        req: &PlanRequest<'_>,
        plan: &Plan,
        allow_duplicate: bool,
    ) -> Result<Json, fsv_core::Error>;
}
