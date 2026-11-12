//! Freshservice operations above the HTTP client.
//!
//! `query` and `task_status` are pure: they take already-loaded JSON and,
//! where a person lookup is unavoidable, a [`query::Directory`]. `schema`
//! owns the form-schema fetch and cache; `views` the site's read-side
//! loaders (saved views, edit-form rules, properties).

pub mod approvals;
pub mod asset_picker;
pub mod associations;
pub mod attachments;
pub mod bulk_priority;
pub mod canned;
pub mod catalog;
pub mod change_task_update;
pub mod conversations;
pub mod create;
pub mod downloads;
pub mod email_addr;
pub mod formmeta;
pub mod integrated_assets;
pub mod list_preferences;
pub mod notes;
pub mod planning_text;
pub mod private_views;
pub mod problem_incidents;
pub mod problem_known_error;
pub mod problem_status;
pub mod problem_workflows;
pub mod properties;
mod query;
pub mod readback;
pub mod recipients;
pub mod reviewed_write;
pub mod scenarios;
pub mod schema;
pub mod service;
pub mod site;
pub mod solutions;
pub mod sr_approvals;
pub mod state_flow;
mod task_status;
pub mod tasks;
pub mod ticket_assignment;
pub mod ticket_lifecycle;
pub mod ticket_merge;
pub mod ticket_note_mutation;
pub mod ticket_tasks;
pub mod ticket_time;
pub mod views;
pub mod watchers;

pub use schema::{
    CACHE_TTL_SECS, IMPACT, PRIORITY, RISK, choice_label, doc_fields, field, merge_v2_choices,
    unwrap_schema,
};

pub use query::{
    Directory, Explain, build_query_hash, build_query_hash_from_schema, load_query_hash,
    merge_same_conditions, resolve_group,
};
pub use task_status::{active_choices, display, fields_from_payload, resolve};

/// User-facing validation failure. Maps to exit code 2 (`ValueError`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryError(pub String);

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for QueryError {}

/// A [`QueryError`] or an expired session, kept apart so the caller can still
/// stop on (and exit with the code of) a session failure. `QueryError` itself
/// stays a plain message because other crates build it as `QueryError(msg)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryFailure {
    /// `SessionError`: no session, expired session, wrong tenant.
    Session(String),
    Query(QueryError),
}

impl QueryFailure {
    pub fn is_session(&self) -> bool {
        matches!(self, QueryFailure::Session(_))
    }

    pub fn message(&self) -> &str {
        match self {
            QueryFailure::Session(msg) | QueryFailure::Query(QueryError(msg)) => msg,
        }
    }

    /// The message alone, dropping the session flag.
    pub fn into_query_error(self) -> QueryError {
        match self {
            QueryFailure::Session(msg) => QueryError(msg),
            QueryFailure::Query(e) => e,
        }
    }
}

impl std::fmt::Display for QueryFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for QueryFailure {}

impl From<QueryError> for QueryFailure {
    fn from(e: QueryError) -> Self {
        QueryFailure::Query(e)
    }
}

impl From<fsv_core::Error> for QueryFailure {
    fn from(e: fsv_core::Error) -> Self {
        match e {
            fsv_core::Error::Session(msg) => QueryFailure::Session(msg),
            other => QueryFailure::Query(QueryError(other.to_string())),
        }
    }
}

impl From<QueryFailure> for QueryError {
    fn from(e: QueryFailure) -> Self {
        e.into_query_error()
    }
}

/// `Session` stays a session error; a query failure is a `ValueError` (exit 2).
impl From<QueryFailure> for fsv_core::Error {
    fn from(e: QueryFailure) -> Self {
        match e {
            QueryFailure::Session(msg) => fsv_core::Error::Session(msg),
            QueryFailure::Query(QueryError(msg)) => fsv_core::Error::Value(msg),
        }
    }
}

/// `isinstance(e, SessionError)`.
pub fn is_session_error(e: &fsv_core::Error) -> bool {
    matches!(e, fsv_core::Error::Session(_))
}

#[cfg(test)]
mod provenance_tests;
#[cfg(test)]
mod workflows_tests;
