//! Site list and detail data (`views.py`), with no rendering and no HTTP.
//! Callers pass JSON the client already fetched.

mod detail;
mod format;
mod list;
mod pages;
mod record;

pub use detail::*;
pub use format::{
    fmt_when, html_to_text, html_to_text_str, is_date_only, one_line, parse_dt, parse_dt_str,
    py_splitlines, relative, span,
};
pub use list::*;
pub use pages::*;
pub use record::*;
