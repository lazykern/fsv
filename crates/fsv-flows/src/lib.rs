//! Plan → review → send for changes and incidents (`drafts`,
//! `change_flow`, `ticket_flow`).
//!
//! Faces (YAML `--from`/`--edit`, `--set`, presets, legacy flags) end in a
//! [`change::Plan`]: engine state, the exact request body, files, the plan
//! hash and the review. Sending is behind traits ([`change::ChangeWriter`],
//! [`fsv_api::service::Api`], [`fsv_api::service::Journal`]) so every step runs
//! offline in tests.

pub mod change;
pub mod drafts;
pub mod meta;
pub mod sr;
pub mod ticket;

#[cfg(test)]
pub(crate) mod testing;

pub use change::{Plan, PlanningEntry};
pub use drafts::Draft;
pub use meta::{ClientResolver, FormMeta, Resolver};
