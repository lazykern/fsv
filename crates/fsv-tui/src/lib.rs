//! The main fsv TUI (`fsv tui`): list / detail panes, filter and search,
//! saved views, write modals. Port of `fsv.tui`.
//!
//! I/O goes through [`Backend`] and page rendering through [`Pages`]; see
//! `backend.rs` for why these are local traits.

pub mod actions;
pub mod app;
pub mod backend;
pub mod filter;
pub mod format;
pub mod input;
pub mod keys;
pub mod markup;
pub mod modals;
mod runtime;
pub mod site;
pub mod view;

pub use app::App;
pub use backend::{Backend, BasicPages, Pages};
pub use runtime::{launch, run};
