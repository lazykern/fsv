//! Site pages (`pages.py`), the classic list/detail output (`render.py`),
//! and the page assembly half of `views.py` (`load_tab`, `load_detail`).
//!
//! Output is Rich markup in [`render::Block`]s. Reads go through a
//! caller-supplied [`source::Source`]; this crate never opens a connection.

pub mod detail;
pub mod pages;
pub mod render;
pub mod source;

#[cfg(test)]
mod coverage;

pub use detail::{Detail, DetailInput, Properties, TabData, load_detail, load_lifecycle, load_planning, load_tab};
pub use render::{Block, blocks_lines, blocks_text, print_blocks};
pub use source::{FetchError, SessionError, Source};
