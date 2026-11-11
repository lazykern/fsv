//! In-terminal interactive forms (ratatui), laid out like the Freshservice
//! web create/edit forms: fields in web order, type-to-filter dropdowns,
//! people/group/agent search as you type, date pickers, a Markdown
//! description, file pickers, live business rules, and the review screen.
//!
//! ```text
//! let session = FormSession::new(init)?;        // fsv-flows builds `init`
//! let result = run_form(session, &config)?;     // standalone (CLI)
//! if result.action == Action::Submit {
//!     result.session.send(result.plan.as_ref().unwrap(), false)?;
//! }
//! ```
//!
//! Embedders (`fsv-tui`) drive a [`FormScreen`] with keys, ticks and draws
//! and take its [`FormResult`] when it finishes. Nothing is sent by the
//! screen itself.

pub mod app;
pub mod backend;
pub mod files;
pub mod kit;
pub mod lookups;
pub mod screen;
pub mod session;
pub mod widgets;

#[cfg(test)]
extern crate self as fsv_formui;
/// The Acme fixture of the integration tests, for unit tests of private fns.
#[cfg(test)]
#[path = "../tests/common/mod.rs"]
pub(crate) mod acme;

pub use app::{FormResult, form_available, form_available_in, run_form};
pub use backend::{Draft, FormBackend, FormKind, FormMode, Plan, PlanRequest, PlanningFiles};
pub use lookups::{ApiFailure, FormApi, LookupUnavailable, Resolver};
pub use screen::{Action, FormScreen};
pub use session::{FormItem, FormSession, ItemView, Outcome, SessionInit, Suggestion, WidgetKind};
