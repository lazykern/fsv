//! Python compatibility helpers.
//!
//! fsv stores hashes of `json.dumps(...)` output (plan hashes, journal
//! entries, metadata fingerprints) that must stay identical between the
//! Python and Rust builds, and many user-facing messages embed Python's
//! `str()`/`repr()` of JSON values. Everything here reproduces CPython's
//! output byte for byte for values that come from JSON (`serde_json::Value`).

mod float;
mod json;
mod repr;

pub use float::float_repr;
pub use json::{Dumps, canonical, compact, dumps, pretty};
pub use repr::{py_repr, py_repr_str, py_str};

/// `serde_json` object type (insertion ordered, like a Python dict).
pub type Map = serde_json::Map<String, serde_json::Value>;
