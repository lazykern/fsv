//! Python compatibility helpers.
//!
//! fsv stores hashes of `json.dumps(...)` output (plan hashes, journal
//! entries, metadata fingerprints) that must stay identical between the
//! Python and Rust builds, and many user-facing messages embed Python's
//! `str()`/`repr()` of JSON values. Everything here reproduces CPython's
//! output byte for byte for values that come from JSON (`serde_json::Value`).

mod decimal;
mod float;
pub mod html_entities;
mod json;
#[cfg(test)]
mod provenance_tests;
mod repr;
mod value;

pub use decimal::decimal_cmp;
pub use float::float_repr;
pub use json::{Dumps, canonical, compact, dumps, pretty};
pub use repr::{py_repr, py_repr_str, py_str};
pub use value::{ValueExt, big_int, truthy};

/// `serde_json` object type (insertion ordered, like a Python dict).
pub type Map = serde_json::Map<String, serde_json::Value>;
