//! Freshservice operations above the HTTP client.
//!
//! `query` and `task_status` are pure: they take already-loaded JSON and,
//! where a person lookup is unavoidable, a [`query::Directory`]. The client
//! and the schema cache land here once `fsv-core` settles.

pub mod approvals;
pub mod associations;
pub mod catalog;
pub mod conversations;
pub mod create;
pub mod formmeta;
pub mod notes;
pub mod properties;
mod query;
mod schema;
pub mod service;
pub mod site;
pub mod state_flow;
mod task_status;
pub mod tasks;

pub use schema::{CACHE_TTL_SECS, IMPACT, PRIORITY, RISK, choice_label, doc_fields, field, merge_v2_choices, unwrap_schema};

pub use query::{Directory, Explain, build_query_hash, build_query_hash_from_schema, load_query_hash};
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
