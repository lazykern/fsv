use clap::{ArgAction, Command};

/// Fixed value lists completed by the shell itself (`_STATIC_COMPLETERS`).
pub static STATIC_COMPLETERS: &[(&str, &[(&str, &str)])] = &[
    (
        "complete_format",
        &[
            ("table", "rich table"),
            ("json", "JSON array"),
            ("csv", "comma-separated values"),
            ("tsv", "tab-separated values"),
        ],
    ),
    (
        "complete_store",
        &[
            ("file", "plain JSON, chmod 600"),
            ("argon", "Argon2id + AES-256-GCM encrypted"),
            ("keychain", "macOS Keychain"),
        ],
    ),
    (
        "complete_shell",
        &[
            ("bash", "Bash"),
            ("zsh", "Z shell"),
            ("fish", "Fish shell"),
            ("powershell", "Windows PowerShell"),
            ("pwsh", "PowerShell Core"),
        ],
    ),
    (
        "complete_sort_order",
        &[("asc", "ascending"), ("desc", "descending")],
    ),
    (
        "complete_config_key",
        &[(
            "completion.network",
            "enable remote requester/agent completion [on|off]",
        )],
    ),
    (
        "complete_config_value",
        &[
            ("on", ""),
            ("off", ""),
            ("true", ""),
            ("false", ""),
            ("1", ""),
            ("0", ""),
        ],
    ),
    (
        "complete_cache_target",
        &[
            ("schema", "field definitions"),
            ("views", "saved view names"),
            ("groups", "agent groups"),
            ("all", "everything"),
        ],
    ),
    (
        "complete_search_sort",
        &[
            ("relevance", "best text match"),
            ("created", "created time"),
            ("modified", "last modified time"),
        ],
    ),
    (
        "complete_duplicate_mode",
        &[
            ("prompt", "ask on duplicate filename"),
            ("skip", "keep existing, skip new file"),
            ("replace", "replace existing file"),
            ("append", "upload alongside existing file"),
        ],
    ),
];

fn static_values(completer: &str) -> Option<&'static [(&'static str, &'static str)]> {
    STATIC_COMPLETERS
        .iter()
        .find(|(n, _)| *n == completer)
        .map(|(_, v)| *v)
}

/// What clap does not record about an option: the flags in declaration
/// order and the Python completer (`autocompletion=`) with its resource.
#[derive(Clone, Copy, Debug)]
pub struct OptionHint {
    /// clap arg id; decides whether the option takes a value.
    pub id: &'static str,
    pub flags: &'static [&'static str],
    pub completer: Option<&'static str>,
    pub resource: Option<&'static str>,
    /// Python reads a string annotation (`Optional[bool]`) as non-boolean, so
    /// the scripts treat this clap flag as taking a value.
    pub value_flag: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Boolean,
    Static(&'static [(&'static str, &'static str)]),
    Dynamic {
        completer: String,
        resource: Option<String>,
    },
    Freeform,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    pub flags: Vec<String>,
    pub kind: Kind,
}

impl Param {
    pub(crate) fn takes_value(&self) -> bool {
        !matches!(self.kind, Kind::Boolean)
    }
}

/// One command: `path` starts with the program name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub path: Vec<String>,
    pub help: String,
    pub params: Vec<Param>,
    pub subcommands: Vec<Node>,
}

type Hints = [(&'static str, &'static str, &'static [OptionHint])];

fn param_for(cmd: &Command, hint: &OptionHint) -> Param {
    let flag = cmd.get_arguments().find(|a| a.get_id() == hint.id);
    let boolean = !hint.value_flag
        && flag.is_none_or(|a| {
            matches!(
                a.get_action(),
                ArgAction::SetTrue | ArgAction::SetFalse | ArgAction::Count
            )
        });
    let kind = if boolean {
        Kind::Boolean
    } else if let Some(values) = hint.completer.and_then(static_values) {
        Kind::Static(values)
    } else if let Some(c) = hint.completer {
        Kind::Dynamic {
            completer: c.to_string(),
            resource: hint.resource.map(str::to_string),
        }
    } else {
        Kind::Freeform
    };
    Param {
        flags: hint.flags.iter().map(|f| f.to_string()).collect(),
        kind,
    }
}

/// `_walk`: the completion tree for `cmd`. `hints` is keyed by the command
/// path under `cmd`'s own name; options in `inherit` (e.g. `-v`) are
/// offered on every descendant.
pub fn tree_from_clap(cmd: &Command, prog: &str, hints: &Hints, inherit: &[&str]) -> Node {
    walk(
        cmd,
        &[cmd.get_name().to_string()],
        vec![prog.to_string()],
        Vec::new(),
        hints,
        inherit,
        true,
    )
}

fn hint_for<'h>(hints: &'h Hints, key: &str) -> (&'h str, &'h [OptionHint]) {
    hints
        .iter()
        .find(|(k, _, _)| *k == key)
        .map_or(("", &[]), |(_, doc, opts)| (*doc, *opts))
}

fn walk(
    cmd: &Command,
    key: &[String],
    path: Vec<String>,
    inherited: Vec<Param>,
    hints: &Hints,
    inherit: &[&str],
    group: bool,
) -> Node {
    let (doc, opts) = hint_for(hints, &key.join(" "));
    let local: Vec<Param> = opts.iter().map(|h| param_for(cmd, h)).collect();
    let mut params = inherited.clone();
    params.extend(local.iter().cloned());
    if !group {
        return Node {
            path,
            help: doc.to_string(),
            params,
            subcommands: Vec::new(),
        };
    }
    let mut next = inherited;
    next.extend(
        local
            .into_iter()
            .filter(|p| p.flags.iter().any(|f| inherit.contains(&f.as_str()))),
    );
    let subcommands = cmd
        .get_subcommands()
        .map(|sub| {
            let mut k = key.to_vec();
            k.push(sub.get_name().to_string());
            let mut p = path.clone();
            p.push(sub.get_name().to_string());
            walk(
                sub,
                &k,
                p,
                next.clone(),
                hints,
                inherit,
                sub.has_subcommands(),
            )
        })
        .collect();
    Node {
        path,
        help: String::new(),
        params,
        subcommands,
    }
}
