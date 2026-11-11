//! Live implementations of the seams the flows, views, form and TUI crates
//! are written against, plus the entry points that wire them to a client.
#![allow(clippy::result_large_err)]

pub mod form;
pub mod meta;
pub mod source;
pub mod tui;
pub mod writer;

#[cfg(test)]
pub(crate) mod testkit;

pub use form::{
    ChangeForm, Changed, FormDeps, FormResolver, LiveForm, SendError, SendOptions, SendOutcome,
    Submitted, change_session, partial_lines, report_partial, run_form, sr_session, ticket_session,
};
pub use meta::LiveMeta;
pub use source::ClientSource;
pub use tui::{LiveBackend, LivePages, SchemaLoader, run_tui, schema_loader};
pub use writer::LiveWriter;
