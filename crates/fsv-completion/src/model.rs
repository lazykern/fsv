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
        /// The field `complete_update_choice` completes (the option id);
        /// `None` for every other completer.
        field: Option<String>,
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

/// Port of fsv.completion_gen._Command. One command: `path` starts with the
/// program name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub path: Vec<String>,
    pub help: String,
    pub params: Vec<Param>,
    pub subcommands: Vec<Node>,
}

type Hints = [(&'static str, &'static str, &'static [OptionHint])];

/// Port of fsv.completion_gen._is_bool_annotation. clap's action stands in
/// for the Python annotation; `value_flag` marks the `Optional[bool]`
/// string annotations Python reads as non-boolean.
fn is_boolean(cmd: &Command, hint: &OptionHint) -> bool {
    !hint.value_flag
        && cmd
            .get_arguments()
            .find(|a| a.get_id() == hint.id)
            .is_none_or(|a| {
                matches!(
                    a.get_action(),
                    ArgAction::SetTrue | ArgAction::SetFalse | ArgAction::Count
                )
            })
}

fn param_for(cmd: &Command, hint: &OptionHint) -> Param {
    let kind = if is_boolean(cmd, hint) {
        Kind::Boolean
    } else if let Some(values) = hint.completer.and_then(static_values) {
        Kind::Static(values)
    } else if let Some(c) = hint.completer {
        Kind::Dynamic {
            completer: c.to_string(),
            resource: hint.resource.map(str::to_string),
            field: (c == "complete_update_choice").then(|| hint.id.to_string()),
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
/// offered on every descendant, and an empty `inherit` passes every group
/// option down.
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

/// Port of fsv.completion_gen._extract_params: the hints record each
/// option's flags and completer, which Python reads from the signature.
fn extract_params(cmd: &Command, opts: &[OptionHint]) -> Vec<Param> {
    opts.iter().map(|h| param_for(cmd, h)).collect()
}

/// Port of fsv.completion_gen._filter_params_by_flags: an empty `allowed`
/// keeps every param.
fn filter_params_by_flags(params: Vec<Param>, allowed: &[&str]) -> Vec<Param> {
    if allowed.is_empty() {
        return params;
    }
    params
        .into_iter()
        .filter(|p| p.flags.iter().any(|f| allowed.contains(&f.as_str())))
        .collect()
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
    let local = extract_params(cmd, opts);
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
    next.extend(filter_params_by_flags(local, inherit));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen_tests::{HINTS, PROG, app, dump, expected_tree};

    /// `_walk(app, [prog], inherited_flags=...)`.
    #[test]
    fn walk_matches_python() {
        for (inherit, key) in [
            (&["-v", "--verbose"][..], "inherit_verbose"),
            (&[][..], "inherit_none"),
        ] {
            let node = walk(
                &app(),
                &["fsv".to_string()],
                vec![PROG.to_string()],
                Vec::new(),
                HINTS,
                inherit,
                true,
            );
            assert_eq!(dump(&node), expected_tree(key), "{key}");
        }
    }

    #[test]
    fn param_for_reads_kind() {
        let cmd = app();
        let list = cmd
            .find_subcommand("changes")
            .and_then(|c| c.find_subcommand("list"))
            .expect("list");
        let opts = HINTS
            .iter()
            .find(|(k, _, _)| *k == "fsv changes list")
            .map(|(_, _, o)| *o)
            .expect("hints");
        let kinds: Vec<Kind> = opts.iter().map(|h| param_for(list, h).kind).collect();
        assert_eq!(
            kinds,
            [
                Kind::Dynamic {
                    completer: "complete_where".into(),
                    resource: Some("changes".into()),
                    field: None,
                },
                Kind::Dynamic {
                    completer: "complete_lookup_query".into(),
                    resource: Some("changes".into()),
                    field: None,
                },
                Kind::Freeform,
                Kind::Freeform,
            ]
        );
        assert_eq!(
            extract_params(list, opts)
                .into_iter()
                .map(|p| p.kind)
                .collect::<Vec<_>>(),
            kinds
        );
        let root_opts = HINTS[0].2;
        assert_eq!(param_for(&cmd, &root_opts[0]).kind, Kind::Boolean);
        assert_eq!(
            param_for(&cmd, &root_opts[1]).kind,
            Kind::Static(static_values("complete_format").expect("format"))
        );
    }

    /// `_is_bool_annotation`: `bool` and `Optional[bool]` are boolean, the
    /// string `"Optional[bool]"` is not (`value_flag`), `str`/`int` are not.
    #[test]
    fn is_boolean_follows_action_and_value_flag() {
        let cmd = app();
        let list = cmd
            .find_subcommand("changes")
            .and_then(|c| c.find_subcommand("list"))
            .expect("list");
        let opts = HINTS
            .iter()
            .find(|(k, _, _)| *k == "fsv changes list")
            .map(|(_, _, o)| *o)
            .expect("hints");
        let yes = opts.iter().find(|h| h.id == "yes").expect("yes");
        assert!(!is_boolean(list, yes));
        assert!(is_boolean(
            list,
            &OptionHint {
                value_flag: false,
                ..*yes
            }
        ));
        let limit = opts.iter().find(|h| h.id == "limit").expect("limit");
        assert!(!is_boolean(list, limit));
        assert!(is_boolean(&cmd, &HINTS[0].2[0]));
    }

    /// Expected lists from `_filter_params_by_flags` on the same params.
    #[test]
    fn filter_params_by_flags_matches_python() {
        let p = |flags: &[&str]| Param {
            flags: flags.iter().map(|f| f.to_string()).collect(),
            kind: Kind::Boolean,
        };
        let params = vec![
            p(&["--verbose", "-v"]),
            p(&["--format", "-f"]),
            p(&["--debug"]),
        ];
        let flags = |allowed: &[&str]| -> Vec<Vec<String>> {
            filter_params_by_flags(params.clone(), allowed)
                .into_iter()
                .map(|p| p.flags)
                .collect()
        };
        assert_eq!(flags(&[]).len(), 3);
        assert_eq!(flags(&["-v"]), [["--verbose", "-v"]]);
        assert_eq!(
            flags(&["--verbose", "--debug"]),
            vec![vec!["--verbose", "-v"], vec!["--debug"]]
        );
        assert!(flags(&["-x"]).is_empty());
    }
}
