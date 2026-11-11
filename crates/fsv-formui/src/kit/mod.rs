//! Reusable terminal-UI pieces shared with `fsv-tui`: key names, timers on
//! a virtual clock, workers, the theme, Rich markup → ratatui, text inputs
//! and small drawing helpers.

pub mod draw;
pub mod input;
pub mod keys;
pub mod markup;
pub mod textarea;
pub mod theme;
pub mod timers;
pub mod workers;

pub use input::{InputEvent, LineInput};
pub use keys::{key_name, parse_key};
pub use textarea::TextArea;
pub use theme::Theme;
pub use timers::Timers;
pub use workers::{Workers, WorkersMode};
