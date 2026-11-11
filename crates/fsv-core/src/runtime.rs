//! Process-wide runtime flags set by the CLI (`--verbose`, `--no-input`,
//! and the current command path recorded in journal entries).

use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

static VERBOSE: AtomicBool = AtomicBool::new(false);
static NO_INPUT: AtomicBool = AtomicBool::new(false);
static COMMAND_PATH: RwLock<Option<String>> = RwLock::new(None);

/// `--verbose`: log every HTTP request to stderr.
pub fn set_verbose(value: bool) {
    VERBOSE.store(value, Ordering::Relaxed);
}

pub fn verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

/// `--no-input`: prompts fail instead of reading stdin.
pub fn set_no_input(value: bool) {
    NO_INPUT.store(value, Ordering::Relaxed);
}

pub fn no_input() -> bool {
    NO_INPUT.load(Ordering::Relaxed)
}

/// Click's `ctx.command_path` of the running command, e.g. `"fsv changes create"`.
pub fn set_command_path(path: Option<&str>) {
    if let Ok(mut g) = COMMAND_PATH.write() {
        *g = path.map(str::to_string);
    }
}

pub fn command_path() -> Option<String> {
    COMMAND_PATH.read().ok().and_then(|g| g.clone())
}
