"""Shell-native completion script generator.

Walks the live typer app at install time and emits per-shell completion scripts
that handle the static tier (subcommand names, flag names, fixed-value lists)
entirely in the shell, and call ``python -m fsv._complete`` only for dynamic
value completions.

Usage:
    from fsv.completion_gen import build_script
    script = build_script("fish", "fsv")
"""
from __future__ import annotations

import inspect
import shlex
from typing import Any

_STATIC_COMPLETERS: dict[str, list[tuple[str, str]]] = {
    "complete_format": [
        ("table", "rich table"),
        ("json", "JSON array"),
        ("csv", "comma-separated values"),
        ("tsv", "tab-separated values"),
    ],
    "complete_store": [
        ("file", "plain JSON, chmod 600"),
        ("argon", "Argon2id + AES-256-GCM encrypted"),
        ("keychain", "macOS Keychain"),
    ],
    "complete_shell": [
        ("bash", "Bash"),
        ("zsh", "Z shell"),
        ("fish", "Fish shell"),
        ("powershell", "Windows PowerShell"),
        ("pwsh", "PowerShell Core"),
    ],
    "complete_sort_order": [("asc", "ascending"), ("desc", "descending")],
    "complete_config_key": [
        ("completion.network", "enable remote requester/agent completion [on|off]")
    ],
    "complete_config_value": [
        ("on", ""),
        ("off", ""),
        ("true", ""),
        ("false", ""),
        ("1", ""),
        ("0", ""),
    ],
    "complete_cache_target": [
        ("schema", "field definitions"),
        ("filters", "saved view names"),
        ("groups", "agent groups"),
        ("all", "everything"),
    ],
    "complete_search_sort": [
        ("relevance", "best text match"),
        ("created", "created time"),
        ("modified", "last modified time"),
    ],
    "complete_duplicate_mode": [
        ("prompt", "ask on duplicate filename"),
        ("skip", "keep existing, skip new file"),
        ("replace", "replace existing file"),
        ("append", "upload alongside existing file"),
    ],
}


def _is_bool_annotation(ann: Any) -> bool:
    if ann is bool or ann == "bool":
        return True
    origin = getattr(ann, "__origin__", None)
    if origin is None:
        return False
    return bool in getattr(ann, "__args__", ())


def _completer_name(comp: Any) -> str | None:
    """Return the canonical completer/factory name from an autocompletion= value."""
    if comp is None:
        return None
    qualname = getattr(comp, "__qualname__", "")
    # Factory closure: e.g. "complete_where.<locals>.complete" → "complete_where"
    if ".<locals>." in qualname:
        return qualname.split(".<locals>.")[0]
    name = getattr(comp, "__name__", None)
    return name or None


def _resource_from_closure(comp: Any) -> str | None:
    """Extract the resource name from a factory closure's free variables."""
    try:
        nonlocals = inspect.getclosurevars(comp).nonlocals
        res = nonlocals.get("res")
        if res is not None:
            return str(res.name)
    except Exception:
        pass
    return None


class _Param:
    __slots__ = ("flags", "kind", "completer", "resource", "static_values")

    def __init__(
        self,
        flags: list[str],
        kind: str,  # boolean | static | dynamic | freeform
        completer: str | None,
        resource: str | None,
        static_values: list[tuple[str, str]] | None,
    ) -> None:
        self.flags = flags
        self.kind = kind
        self.completer = completer
        self.resource = resource
        self.static_values = static_values


class _Command:
    __slots__ = ("path", "help", "params", "subcommands")

    def __init__(
        self,
        path: list[str],
        help: str,
        params: list[_Param],
        subcommands: list["_Command"],
    ) -> None:
        self.path = path
        self.help = help
        self.params = params
        self.subcommands = subcommands


def _walk(typer_app: Any, path: list[str]) -> _Command:
    """Recursively walk a Typer app into a _Command tree."""
    params: list[_Param] = []
    subcommands: list[_Command] = []

    # Root-level commands (direct callbacks on this app)
    for cmd_info in getattr(typer_app, "registered_commands", []):
        cb = getattr(cmd_info, "callback", None)
        if cb is None:
            continue
        name = cmd_info.name or (cb.__name__.replace("_", "-") if cb else None)
        if name is None:
            continue
        sub_params = _extract_params(cb)
        sub_help = (getattr(cb, "__doc__", "") or "").split("\n")[0].strip()
        subcommands.append(
            _Command(path=path + [name], help=sub_help, params=sub_params, subcommands=[])
        )

    # Nested Typer sub-apps
    for grp_info in getattr(typer_app, "registered_groups", []):
        grp_name = grp_info.name
        sub_app = grp_info.typer_instance
        if grp_name is None or sub_app is None:
            continue
        sub = _walk(sub_app, path + [grp_name])
        subcommands.append(sub)

    help_text = ""
    return _Command(path=path, help=help_text, params=params, subcommands=subcommands)


def _extract_params(callback: Any) -> list[_Param]:
    try:
        sig = inspect.signature(callback)
    except (ValueError, TypeError):
        return []

    result: list[_Param] = []
    for _name, param in sig.parameters.items():
        default = param.default
        ann = param.annotation

        if not hasattr(default, "param_decls"):
            continue

        param_decls: list[str] = list(default.param_decls or [])
        flags = [d for d in param_decls if d.startswith("-")]
        if not flags:
            continue

        is_bool = _is_bool_annotation(ann)
        comp = getattr(default, "autocompletion", None)
        comp_name = _completer_name(comp)
        resource = _resource_from_closure(comp) if comp is not None else None

        if is_bool:
            kind = "boolean"
            result.append(_Param(flags, "boolean", None, None, None))
            continue

        if comp_name in _STATIC_COMPLETERS:
            result.append(_Param(flags, "static", comp_name, None, _STATIC_COMPLETERS[comp_name]))
        elif comp_name is not None:
            # Dynamic — check if it's a closure from a known factory (contains resource)
            # or a plain function
            result.append(_Param(flags, "dynamic", comp_name, resource, None))
        else:
            result.append(_Param(flags, "freeform", None, None, None))

    return result


# ──────────────────────────────────────────────────────────────────
# FISH emitter
# ──────────────────────────────────────────────────────────────────

def _fish_cond(path: list[str]) -> str:
    if not path:
        return ""
    parts = [f"__fish_seen_subcommand_from {shlex.quote(p)}" for p in path]
    return "; and ".join(parts)


def _fish_lines(cmd: _Command, prog: str, py_exec: str) -> list[str]:
    lines: list[str] = []
    path = cmd.path[1:]  # drop prog name

    cond = _fish_cond(path)
    n_cond = f" -n '{cond}'" if cond else ""

    # Direct sub-commands at this level
    direct_subcmds = [
        s.path[-1] for s in cmd.subcommands if len(s.path) == len(cmd.path) + 1
    ]
    if direct_subcmds:
        not_yet = " ".join(shlex.quote(c) for c in direct_subcmds)
        not_cond = f"not __fish_seen_subcommand_from {not_yet}"
        full_cond = f"{cond}; and {not_cond}" if cond else not_cond
        for sub in cmd.subcommands:
            if len(sub.path) != len(cmd.path) + 1:
                continue
            lines.append(
                f"complete -c {prog} -f -n '{full_cond}'"
                f" -a {shlex.quote(sub.path[-1])}"
                + (f" -d {shlex.quote(sub.help)}" if sub.help else "")
            )

    # All value-expecting flags for this command (used to suppress flag names during value entry)
    value_flags = [f for p in cmd.params if p.kind in ("static", "dynamic", "freeform") for f in p.flags]
    if value_flags:
        vf_q = " ".join(shlex.quote(f) for f in value_flags)
        not_value_prev = f"not contains -- (commandline -opc)[-1] {vf_q}"
        flag_guard = f"{cond}; and {not_value_prev}" if cond else not_value_prev
    else:
        flag_guard = cond

    # Params for this command
    for p in cmd.params:
        for flag in p.flags:
            flag_q = shlex.quote(flag)
            if p.kind == "boolean":
                n_flag_cond = f" -n '{flag_guard}'" if flag_guard else ""
                lines.append(f"complete -c {prog} -f{n_flag_cond} -a {flag_q}")
            elif p.kind in ("static", "dynamic", "freeform"):
                n_flag_cond = f" -n '{flag_guard}'" if flag_guard else ""
                lines.append(f"complete -c {prog} -f{n_flag_cond} -a {flag_q}")

        if p.kind == "static" and p.static_values:
            flags_q = " ".join(shlex.quote(f) for f in p.flags)
            prev = f"contains -- (commandline -opc)[-1] {flags_q}"
            vc = f"{cond}; and {prev}" if cond else prev
            for val, desc in p.static_values:
                lines.append(
                    f"complete -c {prog} -f -n '{vc}'"
                    f" -a {shlex.quote(val)}"
                    + (f" -d {shlex.quote(desc)}" if desc else "")
                )

        elif p.kind == "dynamic":
            flags_q = " ".join(shlex.quote(f) for f in p.flags)
            prev = f"contains -- (commandline -opc)[-1] {flags_q}"
            vc = f"{cond}; and {prev}" if cond else prev
            resource = p.resource or "-"
            extra = " -- (commandline -opc)" if p.completer == "complete_lookup_query" else ""
            py_call = (
                f"(FSV_THIN_COMPLETE=1 {py_exec} -m fsv._complete fish"
                f" {p.completer} {resource} (commandline -ct){extra})"
            )
            lines.append(f"complete -c {prog} -n '{vc}' -a \"{py_call}\"")

    for sub in cmd.subcommands:
        lines.extend(_fish_lines(sub, prog, py_exec))

    return lines


def _emit_fish(root: _Command, prog: str) -> str:
    import sys as _sys
    py_exec = _sys.executable
    lines = [
        f"# Generated by {prog} completion install -- do not edit",
        f"# Regenerate: {prog} completion install fish",
        "",
        f"complete -c {prog} -f",
        "",
    ]
    lines.extend(_fish_lines(root, prog, py_exec))
    return "\n".join(lines) + "\n"


# ──────────────────────────────────────────────────────────────────
# BASH emitter
# ──────────────────────────────────────────────────────────────────

def _bash_escape(s: str) -> str:
    return s.replace("\\", "\\\\").replace('"', '\\"').replace("'", "'\\''").replace("\n", " ")


def _collect_bash(cmd: _Command, cases: dict[str, Any], parent_path: list[str]) -> None:
    path_key = " ".join(cmd.path[1:])

    # Subcommand names at this node
    direct_subs = [s.path[-1] for s in cmd.subcommands if len(s.path) == len(cmd.path) + 1]

    # Flags at this node
    flags = [f for p in cmd.params for f in p.flags]

    cases[path_key] = {
        "subcommands": direct_subs,
        "flags": flags,
        "params": cmd.params,
    }

    for sub in cmd.subcommands:
        _collect_bash(sub, cases, cmd.path[1:])


def _emit_bash(root: _Command, prog: str) -> str:
    import sys as _sys
    py_exec = _sys.executable

    cases: dict[str, Any] = {}
    _collect_bash(root, cases, [])

    func = f"_{prog.replace('-', '_')}_complete"
    lines = [
        f"# Generated by {prog} completion install -- do not edit",
        "",
        f"{func}() {{",
        "    local cur prev words cword",
        "    _init_completion 2>/dev/null || {",
        "        COMPREPLY=(); cur=${COMP_WORDS[COMP_CWORD]}; prev=${COMP_WORDS[COMP_CWORD-1]}",
        "        words=(${COMP_WORDS[@]}); cword=$COMP_CWORD",
        "    }",
        "",
        "    # Determine current subcommand path",
        "    local cmd_path=''",
        "    local i",
        "    for ((i=1; i<cword; i++)); do",
        "        case ${words[$i]} in",
        "            -*) ;;",
        "            *) cmd_path=\"${cmd_path:+$cmd_path }${words[$i]}\" ;;",
        "        esac",
        "    done",
        "",
        "    case \"$cmd_path\" in",
    ]

    for key, info in sorted(cases.items(), key=lambda x: -len(x[0].split())):
        lines.append(f'        "{key}")')
        dyn_cases = []
        stat_cases = []
        for p in info["params"]:
            for f in p.flags:
                if p.kind == "static" and p.static_values:
                    vals = " ".join(v for v, _ in p.static_values)
                    stat_cases.append((f, vals))
                elif p.kind == "dynamic":
                    resource = p.resource or "-"
                    dyn_cases.append((f, p.completer, resource))

        if dyn_cases or stat_cases:
            lines.append("            case \"$prev\" in")
            for f, vals in stat_cases:
                lines.append(f'                {f})')
                lines.append(f'                    COMPREPLY=( $(compgen -W "{vals}" -- "$cur") ); return 0 ;;')
            for f, completer, resource in dyn_cases:
                extra = ""
                if completer == "complete_lookup_query":
                    extra = ' -- "${COMP_WORDS[@]}"'
                lines.append(f'                {f})')
                lines.append(
                    f'                    COMPREPLY=( $(FSV_THIN_COMPLETE=1 {py_exec} -m fsv._complete bash {completer} {resource} "$cur"{extra}) ); return 0 ;;'
                )
            lines.append("            esac")

        completions = info["subcommands"] + info["flags"]
        if completions:
            comp_str = " ".join(completions)
            lines.append(f'            COMPREPLY=( $(compgen -W "{comp_str}" -- "$cur") )')
        lines.append("            return 0 ;;")

    lines += [
        "    esac",
        "}",
        "",
        f"complete -F {func} {prog}",
    ]
    return "\n".join(lines) + "\n"


# ──────────────────────────────────────────────────────────────────
# ZSH emitter
# ──────────────────────────────────────────────────────────────────

def _collect_zsh(cmd: _Command, cases: dict[str, Any]) -> None:
    path_key = " ".join(cmd.path[1:])
    direct_subs = [s.path[-1] for s in cmd.subcommands if len(s.path) == len(cmd.path) + 1]
    flags = [f for p in cmd.params for f in p.flags]
    cases[path_key] = {
        "subcommands": direct_subs,
        "flags": flags,
        "params": cmd.params,
    }
    for sub in cmd.subcommands:
        _collect_zsh(sub, cases)


def _emit_zsh(root: _Command, prog: str) -> str:
    import sys as _sys
    py_exec = _sys.executable

    cases: dict[str, Any] = {}
    _collect_zsh(root, cases)

    func = f"_{prog.replace('-', '_')}"
    lines = [
        f"# Generated by {prog} completion install -- do not edit",
        f"#compdef {prog}",
        "",
        f"{func}() {{",
        "    local cur prev",
        "    cur=${words[$CURRENT]}",
        "    prev=${words[$CURRENT-1]}",
        "",
        "    local cmd_path=''",
        "    local i",
        "    for ((i=2; i<$CURRENT; i++)); do",
        "        case ${words[$i]} in",
        "            -*) ;;",
        "            *) cmd_path=\"${cmd_path:+$cmd_path }${words[$i]}\" ;;",
        "        esac",
        "    done",
        "",
        "    case \"$cmd_path\" in",
    ]

    for key, info in sorted(cases.items(), key=lambda x: -len(x[0].split())):
        lines.append(f'        "{key}")')
        stat_cases = []
        dyn_cases = []
        for p in info["params"]:
            for f in p.flags:
                if p.kind == "static" and p.static_values:
                    vals = " ".join(v for v, _ in p.static_values)
                    stat_cases.append((f, vals))
                elif p.kind == "dynamic":
                    resource = p.resource or "-"
                    dyn_cases.append((f, p.completer, resource))

        if stat_cases or dyn_cases:
            lines.append("            case \"$prev\" in")
            for f, vals in stat_cases:
                lines.append(f'                {f})')
                lines.append(f'                    compadd {vals}; return 0 ;;')
            for f, completer, resource in dyn_cases:
                extra = ""
                if completer == "complete_lookup_query":
                    extra = ' -- "${words[@]}"'
                lines.append(f'                {f})')
                lines.append(
                    f'                    local _vals; _vals=($( FSV_THIN_COMPLETE=1 {py_exec} -m fsv._complete zsh {completer} {resource} "$cur"{extra} ))'
                )
                lines.append("                    compadd -a _vals; return 0 ;;")
            lines.append("            esac")

        completions = info["subcommands"] + info["flags"]
        if completions:
            comp_str = " ".join(shlex.quote(c) for c in completions)
            lines.append(f'            compadd -- {comp_str}; return 0 ;;')
        else:
            lines.append("            return 0 ;;")

    lines += [
        "    esac",
        "}",
        "",
        f"{func}",
    ]
    return "\n".join(lines) + "\n"


# ──────────────────────────────────────────────────────────────────
# Public API
# ──────────────────────────────────────────────────────────────────

def build_script(shell: str, prog_name: str) -> str:
    """Build a shell-native completion script for *prog_name* targeting *shell*."""
    from fsv.cli import app

    root = _walk(app, [prog_name])

    if shell == "fish":
        return _emit_fish(root, prog_name)
    if shell == "bash":
        return _emit_bash(root, prog_name)
    if shell in ("zsh",):
        return _emit_zsh(root, prog_name)

    raise ValueError(f"unsupported shell: {shell}")
