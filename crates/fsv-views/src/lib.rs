//! Site pages (`pages.py`), the classic list/detail output (`render.py`),
//! and the page assembly half of `views.py` (`load_tab`, `load_detail`).
//!
//! Output is Rich markup in [`render::Block`]s. Reads go through a
//! caller-supplied [`source::Source`] (the Properties pane through an
//! `fsv_api::service::Api`); this crate never opens a connection.

pub mod detail;
pub mod pages;
pub mod properties;
pub mod render;
pub mod source;

#[cfg(test)]
mod coverage;
#[cfg(test)]
mod provenance_tests;

pub use detail::{
    Detail, DetailInput, Properties, TabData, load_detail, load_lifecycle, load_planning, load_tab,
};
pub use render::{Block, blocks_lines, blocks_text, print_blocks};
pub use source::{FetchError, SessionError, Source};
