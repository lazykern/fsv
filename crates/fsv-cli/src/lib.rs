//! The `fsv` command line: clap tree, argv rewriting, and handlers.

// `fsv_core::Error` is large and owned by fsv-core.
#![allow(clippy::result_large_err)]

mod app;
mod cmd;
pub mod generated;
mod pending;
pub mod types;
mod util;

pub use app::{ROOT_FLAGS_ANYWHERE, VERSION, command, rewrite, run};
pub use cmd::completion::{build_script, tree as completion_tree};
