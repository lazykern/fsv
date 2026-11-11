//! fsv infrastructure: config, session store, HTTP client, journal, cache,
//! YAML, terminal output.

#![allow(clippy::result_large_err)]

pub mod cache;
pub mod client;
pub mod config;
pub mod ctx;
pub mod editor;
pub mod error;
pub mod journal;
pub mod mdlite;
pub mod output;
pub mod pystr;
pub mod resource;
pub mod review;
pub mod runtime;
pub mod safety;
pub mod session;
pub mod term;
pub mod value;
pub mod yaml;

#[cfg(test)]
mod testenv;

pub use client::{Client, get_client};
pub use config::Paths;
pub use ctx::{Ctx, Flags, Tenant};
pub use error::{ApiError, Error, Result};
