//! Shell-native completion (`completion_gen` + `_complete`).
//!
//! Scripts complete subcommands, flags and fixed value lists inside the
//! shell; only dynamic values (fields, saved views, groups, people) call back
//! into `fsv _complete`.

mod dynamic;
mod emit;
#[cfg(test)]
mod gen_tests;
mod model;
mod network;
mod quote;

pub use dynamic::{complete, complete_help_topic, complete_update_choice, run};
pub use emit::{emit_bash, emit_fish, emit_zsh};
pub use model::{Kind, Node, OptionHint, Param, STATIC_COMPLETERS, tree_from_clap};
pub use network::{completion_network, config_load};
pub use quote::shlex_quote;

/// Environment variable the generated scripts set when calling `_complete`.
pub const THIN_COMPLETE_ENV: &str = "FSV_THIN_COMPLETE";

/// `build_script`: the script for `shell`; `exe` is what the script runs for
/// dynamic values.
pub fn build_script(shell: &str, root: &Node, prog: &str, exe: &str) -> Result<String, String> {
    match shell {
        "fish" => Ok(emit_fish(root, prog, exe)),
        "bash" => Ok(emit_bash(root, prog, exe)),
        "zsh" => Ok(emit_zsh(root, prog, exe)),
        _ => Err(format!("unsupported shell: {shell}")),
    }
}
