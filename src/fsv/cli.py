from __future__ import annotations

import copy
import os
import re
import sys
import time
from enum import Enum
from pathlib import Path
from typing import Any, Callable, Iterable, List, Optional, TypeVar
from urllib.parse import unquote

import typer
import typer.main as _typer_main
from typer.core import TyperCommand, TyperGroup, TyperOption

from fsv import completion, config, render
from fsv.reviewed_cli import ReviewOutput
from fsv.errors import APIError, SessionError
from fsv.render import console, emit_json, err, strip_html
from fsv.resources import CHANGES, PROBLEMS, TICKETS, REGISTRY, Resource, format_id, parse_id

_COMPLETING = bool(os.environ.get('_FSV_COMPLETE'))

if not _COMPLETING:
    from rich.markup import escape
    from rich.table import Table
else:
    escape = str  # type: ignore[assignment]
    Table = None  # type: ignore[assignment,misc]

_ROOT_FLAGS_ANYWHERE = {"-v", "--verbose"}
_GLOBAL_HELP_PANEL = "Global options"


def _option_arity(command: TyperGroup | TyperCommand | None, token: str) -> int:
    if command is None or "=" in token:
        return 0
    name = token.split("=", 1)[0]
    for param in getattr(command, "params", []):
        if not isinstance(param, TyperOption):
            continue
        if name not in [*param.opts, *param.secondary_opts]:
            continue
        if param.is_flag:
            return 0
        return max(int(getattr(param, "nargs", 1) or 1), 1)
    return 0


def _reorder_root_flags(root: TyperGroup, args: list[str]) -> list[str]:
    moved: list[str] = []
    kept: list[str] = []
    current: TyperGroup | TyperCommand | None = root
    command_started = False
    skip = 0
    passthrough = False
    for token in args:
        if passthrough:
            kept.append(token)
            continue
        if skip:
            kept.append(token)
            skip -= 1
            continue
        if token == "--":
            passthrough = True
            kept.append(token)
            continue
        if command_started and token in _ROOT_FLAGS_ANYWHERE:
            moved.append(token)
            continue
        if token.startswith("-"):
            skip = _option_arity(current, token)
            kept.append(token)
            continue
        if isinstance(current, TyperGroup) and token in current.commands:
            command_started = True
            current = current.commands[token]
        kept.append(token)
    return moved + kept if moved else args


def _global_help_params(current: TyperGroup | TyperCommand, ctx: typer.Context) -> list[TyperOption]:
    if not getattr(ctx, "_fsv_include_global_help", False):
        return []
    root = ctx.find_root()
    root_command = getattr(root, "command", None)
    if root_command is None or root_command is current:
        return []
    params: list[TyperOption] = []
    for param in getattr(root_command, "params", []):
        if not isinstance(param, TyperOption):
            continue
        if not _ROOT_FLAGS_ANYWHERE.intersection([*param.opts, *param.secondary_opts]):
            continue
        clone = copy.copy(param)
        clone.rich_help_panel = _GLOBAL_HELP_PANEL
        params.append(clone)
    return params


class _FSVHelpMixin:
    def get_params(self, ctx: typer.Context):
        params = list(super().get_params(ctx))
        params.extend(_global_help_params(self, ctx))
        return params

    def format_help(self, ctx: typer.Context, formatter) -> None:
        previous = getattr(ctx, "_fsv_include_global_help", False)
        ctx._fsv_include_global_help = True
        try:
            super().format_help(ctx, formatter)
        finally:
            ctx._fsv_include_global_help = previous


class FSVCommand(_FSVHelpMixin, TyperCommand):
    pass


class FSVGroup(_FSVHelpMixin, TyperGroup):
    def parse_args(self, ctx: typer.Context, args: list[str]) -> list[str]:
        return super().parse_args(ctx, _reorder_root_flags(self, args))


class ChangeLegacyGroup(FSVGroup):
    def parse_args(self, ctx: typer.Context, args: list[str]) -> list[str]:
        routed = list(args)
        if routed and not routed[0].startswith("-") and routed[0] not in self.commands:
            try:
                parse_id(routed[0], CHANGES)
            except ValueError:
                pass
            else:
                ctx.meta["fsv_legacy_nested"] = True
                routed = ["legacy", *routed]
        return super().parse_args(ctx, routed)


_typer_main.TyperCommand = FSVCommand
_typer_main.TyperGroup = FSVGroup


app = typer.Typer(
    cls=FSVGroup,
    add_completion=True,
    no_args_is_help=True,
    help="Freshservice CLI",
    rich_markup_mode=None if _COMPLETING else "rich",
    context_settings={"help_option_names": ["-h", "--help"]},
    epilog=(
        "[bold]Examples:[/bold]  "
        "fsv auth login  |  "
        "fsv changes ls --where status=Open  |  "
        "fsv changes get CHN-1234 --internal  |  "
        "fsv changes update CHN-1234 --set 'environment=Production'  |  "
        "fsv changes download CHN-1234 --all"
    ),
)
T = TypeVar("T")
NO_INPUT = False
VERBOSE = False


def _no_input(local: bool = False) -> bool:
    return NO_INPUT or local
_SUPPORTED_SHELLS = {"bash", "zsh", "fish", "powershell", "pwsh"}
_TYPER_SHELL_PATCHED = False


def _shell_from_env() -> str | None:
    shell = Path(os.environ.get("SHELL") or "").name.casefold()
    aliases = {"sh": "bash"}
    shell = aliases.get(shell, shell)
    return shell if shell in _SUPPORTED_SHELLS else None


def _patch_typer_shell_detection(force: bool = False) -> None:
    global _TYPER_SHELL_PATCHED
    if _TYPER_SHELL_PATCHED and not force:
        return
    import typer.completion as _typer_completion
    import typer._completion_shared as _typer_completion_shared

    original = getattr(_typer_completion_shared, "_fsv_original_get_shell_name", None)
    if original is None:
        original = _typer_completion_shared._get_shell_name
        _typer_completion_shared._fsv_original_get_shell_name = original

    def _wrapped() -> str | None:
        try:
            name = original()
        except Exception:
            name = None
        return name or _shell_from_env()

    _typer_completion_shared._get_shell_name = _wrapped
    _typer_completion._get_shell_name = _wrapped

    # Route --show-completion through the fast generator too
    def _fast_get_completion_script(
        prog_name: str, complete_var: str, shell: str
    ) -> str:
        try:
            from fsv.completion_gen import build_script
            return build_script(shell, prog_name)
        except Exception:
            return _typer_completion_shared._fsv_original_get_completion_script(
                prog_name=prog_name, complete_var=complete_var, shell=shell
            )

    if not getattr(_typer_completion_shared, "_fsv_original_get_completion_script", None):
        _typer_completion_shared._fsv_original_get_completion_script = (
            _typer_completion_shared.get_completion_script
        )
        _typer_completion_shared.get_completion_script = _fast_get_completion_script

    _TYPER_SHELL_PATCHED = True


_patch_typer_shell_detection()


@app.callback(invoke_without_command=True)
def main(
    ctx: typer.Context = typer.Option(None, hidden=True),
    no_input: bool = typer.Option(False, "--no-input", help="fail instead of prompting"),
    verbose: bool = typer.Option(False, "--verbose", "-v", help="print debug info to stderr"),
    version: bool = typer.Option(False, "--version", "-V", help="print version and exit", is_eager=True),
) -> None:
    """Freshservice CLI."""
    global NO_INPUT, VERBOSE
    if version:
        try:
            from importlib.metadata import version as _pkg_version
            v = _pkg_version("fsv")
        except Exception:
            v = "0.1.0"
        console.print(f"fsv {v}")
        raise typer.Exit(0)
    NO_INPUT = no_input
    VERBOSE = verbose
    from fsv.client import set_verbose
    set_verbose(verbose)
    if ctx.invoked_subcommand is None:
        console.print(ctx.get_help())
        raise typer.Exit(0)


class OutputFormat(str, Enum):
    table = "table"
    json = "json"
    csv = "csv"
    tsv = "tsv"


class SearchSort(str, Enum):
    relevance = "relevance"
    created = "created"
    modified = "modified"


class SortOrder(str, Enum):
    asc = "asc"
    desc = "desc"


def _err(msg: str, code: int = 2) -> None:
    """Print an error and exit. Default code 2 = validation/usage."""
    err.print(f"[red]error[/red]: {msg}")
    raise typer.Exit(code) from None


def _err_exc(exc: BaseException) -> None:
    """Exit with the stable code for *exc* (3 session, 4 conflict, 5 server, 2 value, 1 other)."""
    from fsv.errors import exit_code_for
    if isinstance(exc, (typer.Abort, KeyboardInterrupt)):
        err.print("Aborted.")
        raise typer.Exit(1) from None
    msg = " ".join(str(exc).split()) or type(exc).__name__
    if VERBOSE and isinstance(exc, APIError):
        msg += "\n" + exc.detail()
    from rich.markup import escape
    _err(escape(msg), exit_code_for(exc))


def _flow_exit(e: Any) -> None:
    """Print a change_flow.FlowExit and exit with its code."""
    from rich.markup import escape
    if e.message:
        err.print(f"[red]error[/red]: {escape(e.message)}" if e.code else escape(e.message), highlight=False)
    raise typer.Exit(e.code)


def _api(fn: Callable[[], T]) -> T:
    try:
        return fn()
    except (SessionError, APIError, ValueError) as e:
        _err_exc(e)


def _client():
    from fsv.client import get_client
    from fsv.session import session_age_hours
    if session_age_hours() is None:
        err.print("[red]error[/red]: no session; run `fsv auth login`")
        raise SystemExit(3)
    try:
        return get_client()
    except (SessionError, RuntimeError) as e:
        err.print(f"[red]error[/red]: {e}")
        raise SystemExit(3 if isinstance(e, SessionError) else 1)


def _edit_body(data: dict[str, Any], hint: str, no_input: bool = False) -> dict[str, Any]:
    """Open $EDITOR with JSON data, return edited body."""
    from fsv.editor import edit_json, EditorAbort
    if _no_input(no_input):
        _err("editor disabled by --no-input; use --dry-run/--dry or provide non-interactive flags")
    from fsv.editor import edit_json, EditorAbort
    try:
        return edit_json(data, hint)
    except EditorAbort as e:
        _err_exc(e)


def _choose_store() -> str:
    from fsv.session import current_backend
    options = []
    if sys.platform == "darwin":
        options.append(("keychain", "macOS Keychain; recommended"))
    options.extend([
        ("argon", "encrypted file; asks passphrase when used"),
        ("file", "plain JSON file; chmod 600"),
    ])
    choices = [name for name, _ in options]
    default = current_backend()
    if default not in choices:
        default = choices[0]
    default_idx = choices.index(default) + 1

    table = Table(title="Store session")
    table.add_column("#", justify="right")
    table.add_column("Backend")
    table.add_column("Notes")
    for i, (name, note) in enumerate(options, 1):
        marker = " [green](default)[/]" if i == default_idx else ""
        table.add_row(str(i), f"{name}{marker}", note)
    console.print(table)

    while True:
        answer = typer.prompt("Choose store", default=str(default_idx)).strip().lower()
        if answer.isdigit():
            idx = int(answer)
            if 1 <= idx <= len(options):
                return options[idx - 1][0]
        if answer in choices:
            return answer
        err.print(f"[red]error[/red]: choose 1-{len(options)} or one of: {', '.join(choices)}")


def _cid(id_: str, res: Resource) -> int:
    try:
        return parse_id(id_, res)
    except ValueError as e:
        _err_exc(e)


def _page_text(text: str) -> bool:
    import subprocess
    pager = os.environ.get("PAGER") or "less -R"
    try:
        proc = subprocess.run(pager, input=text, text=True, shell=True, check=False)
        return proc.returncode == 0
    except Exception:
        return False


def _should_page(row_count: int, enabled: bool) -> bool:
    import shutil
    if not enabled or not sys.stdout.isatty():
        return False
    return row_count + 4 > shutil.get_terminal_size((80, 24)).lines


def _emit_items(items: list[dict], res: Resource, sch: dict[str, Any], fmt: OutputFormat | str, json_out: bool) -> None:
    fmt = fmt.value if isinstance(fmt, OutputFormat) else fmt
    if json_out:
        fmt = "json"
    if fmt == "json":
        emit_json(items)
    elif fmt == "csv":
        render.emit_delimited(render.list_rows(items, res, sch), ",")
    elif fmt == "tsv":
        render.emit_delimited(render.list_rows(items, res, sch), "\t")
    elif fmt == "table":
        console.print(render.list_table(items, res, sch))
    else:
        _err("--output must be table, json, csv, or tsv")


def _emit_fmt(raw: Any, flat_rows: list[dict], fmt: OutputFormat | str, json_out: bool) -> bool:
    """Emit non-table formats. Returns True if emitted, False for table."""
    f = fmt.value if isinstance(fmt, OutputFormat) else fmt
    if json_out or f == "json":
        emit_json(raw)
        return True
    if f == "csv":
        render.emit_delimited(flat_rows, ",")
        return True
    if f == "tsv":
        render.emit_delimited(flat_rows, "\t")
        return True
    return False


def _field_scope(f: dict[str, Any]) -> str:
    return "default" if f.get("default_field") else "custom"


def _field_api_type(f: dict[str, Any]) -> str:
    return "default" if f.get("default_field") else "custom_field"


def _field_matches(f: dict[str, Any], text: str) -> bool:
    needle = text.casefold()
    return needle in str(f.get("name") or "").casefold() or needle in str(f.get("label") or "").casefold()


def _format_field_ref(f: dict[str, Any]) -> str:
    return f"{f.get('name')}  {f.get('label')}  {f.get('field_type')}  {_field_scope(f)}"


def _schema_field_matches(sch: dict[str, Any], text: str, scope: str | None = None) -> list[dict[str, Any]]:
    fields = sch["fields"]
    if scope == "default":
        fields = [f for f in fields if f.get("default_field")]
    elif scope == "custom":
        fields = [f for f in fields if not f.get("default_field")]
    return [f for f in fields if _field_matches(f, text)]


def _find_schema_field(sch: dict[str, Any], text: str) -> dict[str, Any] | None:
    scope = None
    raw = text
    if ":" in text:
        maybe_scope, rest = text.split(":", 1)
        if maybe_scope in ("default", "custom"):
            scope, raw = maybe_scope, rest
    folded = raw.casefold()
    matches = _schema_field_matches(sch, raw, scope)
    exact = [
        f for f in matches
        if str(f.get("name") or "").casefold() == folded or str(f.get("label") or "").casefold() == folded
    ]
    if len(exact) == 1:
        return exact[0]
    if len(exact) > 1:
        by_name = [f for f in exact if str(f.get("name") or "").casefold() == folded]
        if len(by_name) == 1:
            return by_name[0]
        _err("ambiguous field " + repr(text) + ":\n" + "\n".join(f"  {_format_field_ref(f)}" for f in exact[:10]))
    if len(matches) == 1:
        return matches[0]
    if len(matches) > 1:
        _err("ambiguous field " + repr(text) + ":\n" + "\n".join(f"  {_format_field_ref(f)}" for f in matches[:10]))
    return None


def _show_fields_table(fields: list[dict[str, Any]]) -> None:
    t = Table()
    t.add_column("name")
    t.add_column("label")
    t.add_column("type")
    t.add_column("scope")
    t.add_column("req", justify="center")
    t.add_column("choices", style="dim")
    for f in fields:
        ch = f.get("choices") or []
        t.add_row(
            f.get("name", "?"),
            f.get("label", "?"),
            f.get("field_type", "?"),
            _field_scope(f),
            "[red]*[/]" if f.get("required") else "",
            str(len(ch)) if ch else "-",
        )
    console.print(t)


def fields_resource(
    res: Resource,
    search: Optional[str],
    default: bool,
    custom: bool,
    choices: Optional[str],
    refresh: bool,
    json_out: bool,
    required: bool = False,
    format_: OutputFormat | str = "table",
) -> None:
    from fsv import schema as schema_mod
    if default and custom:
        _err("use --default or --custom, not both")
    c = _client()
    sch = _api(lambda: schema_mod.load(res, c, force=refresh))
    if choices:
        f = _find_schema_field(sch, choices)
        if not f:
            _err(f"field not found: {choices}; run `fsv {res.name} fields`")
        items = f.get("choices") or []
        flat = [{"id": str(x.get("id") or "-"), "value": str(x.get("value") or x.get("name") or x.get("label") or "-"), "detail": str(x.get("requester_display_value") or x.get("display_id") or "")} for x in items]
        if _emit_fmt(items, flat, format_, json_out):
            return
        t = Table()
        t.add_column("id")
        t.add_column("value")
        t.add_column("detail", style="dim")
        for x in items:
            t.add_row(str(x.get("id") or "-"), str(x.get("value") or x.get("name") or x.get("label") or "-"), str(x.get("requester_display_value") or x.get("display_id") or ""))
        console.print(t)
        return
    fields = sch["fields"]
    if search:
        fields = [f for f in fields if _field_matches(f, search)]
    if default:
        fields = [f for f in fields if f.get("default_field")]
    if custom:
        fields = [f for f in fields if not f.get("default_field")]
    if required:
        fields = [f for f in fields if f.get("required")]
    if _emit_fmt(fields, fields, format_, json_out):
        return
    _show_fields_table(fields)


def lookup_resource(res: Resource, kind: str, query: str, json_out: bool, format_: OutputFormat | str = "table", *, refresh: bool = False) -> None:
    from fsv import schema as schema_mod
    c = _client()
    key = kind.casefold()
    if key in ("requester", "requesters"):
        items = _api(lambda: c.autocomplete("requesters", query, {"all_users": "true"}))
    elif key in ("agent", "agents"):
        items = _api(lambda: c.autocomplete("agents", query))
    elif key in ("group", "groups"):
        data = _api(lambda: c.int_get("bootstrap/agents_groups"))
        items = [g for g in data.get("groups", []) if query.casefold() in str(g.get("name") or "").casefold()]
    else:
        sch = _api(lambda: schema_mod.load(res, c, force=True) if refresh else schema_mod.load(res, c))
        f = _find_schema_field(sch, kind)
        if not f:
            _err(f"unknown lookup kind or field: {kind}")
        items = [x for x in f.get("choices") or [] if not query or query.casefold() in str(x.get("value") or x.get("label") or x.get("name") or x.get("id")).casefold()]
    flat = [{"id": str(x.get("user_id") or x.get("id") or "-"), "value": str(x.get("value") or x.get("name") or x.get("label") or "-"), "detail": str(x.get("email") or x.get("details") or "")} for x in items]
    if _emit_fmt(items, flat, format_, json_out):
        return
    t = Table()
    t.add_column("id")
    t.add_column("value")
    t.add_column("detail", style="dim")
    for x in items:
        t.add_row(str(x.get("user_id") or x.get("id") or "-"), str(x.get("value") or x.get("name") or x.get("label") or "-"), str(x.get("email") or x.get("details") or ""))
    console.print(t)


def _load_query_hash(raw: str) -> list[dict[str, Any]]:
    import json
    try:
        loaded = json.loads(unquote(raw))
    except json.JSONDecodeError as e:
        _err(f"invalid --query-hash JSON: {e}")
    if not isinstance(loaded, list):
        _err("--query-hash must be a JSON array")
    from fsv.query import validate_raw_query
    try:
        validate_raw_query(loaded, "--query-hash")
    except ValueError as exc:
        _err(str(exc))
    return loaded


def _interactive() -> bool:
    return not _no_input() and sys.stdin.isatty()


def _candidate_label(item: dict[str, Any]) -> str:
    name = item.get("value") or item.get("name") or item.get("id")
    extra = item.get("details") or item.get("email")
    return f"{name} <{extra}>" if extra and extra != name else str(name)


def _pick_lookup(kind: str, value: str, results: list[dict[str, Any]], exact_keys: tuple[str, ...]) -> dict[str, Any]:
    """Exact (NFC, case-insensitive) match wins. Anything else is an error that
    lists candidates (interactive: pick one). Never a silent substring pick."""
    from fsv.safety import norm
    if not results:
        _err(f"no {kind} match: {value}")
    folded = norm(value)
    exact = [x for x in results if any(norm(x.get(k)) == folded for k in exact_keys)]
    if len(exact) == 1:
        return exact[0]
    candidates = exact or results
    problem = f"multiple exact {kind} matches for {value!r}" if exact else f"no exact {kind} match for {value!r}"
    if _interactive():
        shown = candidates[:20]
        err.print(f"{problem}; candidates:", highlight=False)
        for i, item in enumerate(shown, 1):
            err.print(f"  {i}. {_candidate_label(item)}", highlight=False)
        choice = typer.prompt("Pick a number (0 to cancel)", type=int, default=0)
        if 1 <= choice <= len(shown):
            return shown[choice - 1]
        _err(f"no {kind} chosen")
    listed = "; ".join(_candidate_label(x) for x in candidates[:8])
    more = f" (+{len(candidates) - 8} more)" if len(candidates) > 8 else ""
    _err(f"{problem}: {listed}{more}. Use the exact name/email or a numeric ID.")


def _resolve_requester(c: Any, value: str) -> dict[str, Any]:
    if value.isdigit():
        return {"id": int(value), "name": value}
    item = _pick_lookup(
        "requester",
        value,
        c.autocomplete("requesters", value, {"all_users": "true"}),
        ("value", "email", "details"),
    )
    return {"id": item["id"], "name": item.get("value") or str(item["id"])}


def _resolve_agent(c: Any, value: str) -> str:
    if value.isdigit():
        return value
    from fsv.agent_lookup import exact_agent_id, is_email
    if is_email(value):
        try:
            return str(exact_agent_id(c, value))
        except ValueError as exc:
            _err(str(exc))
    item = _pick_lookup("agent", value, c.autocomplete("agents", value), ("value", "id", "email", "details"))
    user_id = item.get("user_id")
    if user_id is None:
        _err(f"agent match has no user_id: {value}")
    return str(user_id)


def _resolve_lookup_user(c: Any, value: str, lookup_link: str | None = None) -> int:
    if value.isdigit():
        return int(value)
    if lookup_link:
        rows = c.lookup_choices(lookup_link, value)
        if len(rows) == 1:
            return int(rows[0]["id"])
        if len(rows) > 1:
            names = ", ".join(str(r.get("name") or r.get("value") or r["id"]) for r in rows[:5])
            _err(f"ambiguous user {value!r}: {names}")
        _err(f"user not found: {value!r}")
    active_agents = [
        r for r in c.autocomplete("agents", value)
        if "(Deactivated)" not in str(r.get("value") or "") and r.get("user_id")
    ]
    if len(active_agents) == 1:
        return int(active_agents[0]["user_id"])
    if len(active_agents) > 1:
        names = ", ".join(str(r.get("value") or r["user_id"]) for r in active_agents[:5])
        _err(f"ambiguous user {value!r}: {names}")
    rows = c.autocomplete("requesters", value, {"all_users": "true"})
    active_req = [r for r in rows if not r.get("deleted")]
    if len(active_req) == 1:
        return int(active_req[0]["id"])
    if len(active_req) > 1:
        names = ", ".join(str(r.get("value") or r["id"]) for r in active_req[:5])
        _err(f"ambiguous user {value!r}: {names}")
    _err(f"user not found: {value!r}")


def _resolve_group(c: Any, value: str) -> str:
    if value.isdigit():
        return value
    data = c.int_get("bootstrap/agents_groups")
    from fsv.safety import norm
    folded = norm(value)
    groups = [
        g for g in data.get("groups", [])
        if folded in norm(g.get("name")) or str(g.get("id") or "") == value
    ]
    item = _pick_lookup("group", value, groups, ("name", "id"))
    group_id = item.get("id")
    if group_id is None:
        _err(f"group match has no id: {value}")
    return str(group_id)


def _resolve_update_choice(sch: dict[str, Any], field_name: str, raw: str) -> int | str:
    f = _find_schema_field(sch, field_name)
    choices = [c for c in (f or {}).get("choices", [])
               if isinstance(c, dict) and not c.get("deleted") and not c.get("is_deleted")]
    if raw.isdigit():
        matches = [c for c in choices if str(c.get("id")) == raw]
    else:
        folded = raw.casefold()
        matches = [c for c in choices if any(str(c.get(k) or "").casefold() == folded
                                              for k in ("value", "name", "label"))]
    if len(matches) > 1:
        _err(f"ambiguous {field_name}: {raw!r}; select a unique active ID")
    if len(matches) == 1 and matches[0].get("id") is not None:
        return matches[0]["id"]
    labels = [str(c.get("value") or c.get("name") or c.get("label") or c.get("id")) for c in choices]
    suffix = f"; choices: {', '.join(labels[:12])}" if labels else "; active choice metadata unavailable"
    _err(f"unknown {field_name}: {raw!r}{suffix}")


def _status_label(sch: dict[str, Any], status: int | str) -> str:
    from fsv import schema as schema_mod
    label = schema_mod.choice_label("status", status, sch)
    return label if label != "-" else str(status)


def _confirm_writes(risks: list[str], subject: str, yes: bool, no_input: bool = False) -> None:
    """Single gate for every notifying/state-changing write.

    No risks or --yes: proceed. Interactive: show what will happen, ask y/N.
    Non-interactive (--no-input or no TTY): refuse unless --yes.
    """
    if not risks or yes:
        return
    if _no_input(no_input) or not sys.stdin.isatty():
        _err(f"{subject}: " + "; ".join(risks) + ". Pass --yes to confirm.")
    err.print(f"[bold]{subject}[/bold] will:", highlight=False)
    for r in risks:
        err.print(f"  - {r}", highlight=False)
    typer.confirm("Proceed?", default=False, abort=True)


def _current_flat(res: Resource, cid: int, c: Any) -> dict[str, Any] | None:
    """Best-effort current item (status/assignee/group) to detect no-op writes."""
    try:
        data = c.int_get(f"{res.api_path}/{cid}")
        item = data.get(res.item_key, data) if isinstance(data, dict) else None
        return item if isinstance(item, dict) else None
    except Exception:
        return None


def _risks_for_item_write(res: Resource, cid: int, body: dict[str, Any], sch: dict[str, Any], c: Any,
                          current: dict[str, Any] | None = None) -> list[str]:
    from fsv.safety import write_risks
    if current is None and any(k in body for k in ("status", "agent_id", "responder_id", "group_id")):
        current = _current_flat(res, cid, c)
    return write_risks(res.name, body, current=current, status_label=lambda v: _status_label(sch, v))


def _confirm_terminal_status(res: Resource, cid: int, sch: dict[str, Any], status: int | str, yes: bool, no_input: bool = False) -> None:
    """Back-compat wrapper: gate a lone status change."""
    from fsv.safety import write_risks
    risks = write_risks(res.name, {"status": status}, status_label=lambda v: _status_label(sch, v))
    _confirm_writes(risks, f"{format_id({'id': cid}, res)}", yes, no_input)


def _emit_dry_run(res: Resource, cid: int, body: dict[str, Any], action: str = "update",
                  risks: list[str] | None = None, display_id: str | None = None,
                  warnings: list[str] | None = None, plan_hash: str | None = None) -> None:
    out: dict[str, Any] = {"dry_run": True, "action": action, "id": display_id or format_id({"id": cid}, res), "body": body}
    if plan_hash:
        out["plan_hash"] = plan_hash
    if risks:
        out["requires_confirmation"] = risks
    if warnings:
        out["warnings"] = warnings
    emit_json(out)


def _strip_quotes(value: str) -> str:
    value = value.strip()
    if len(value) >= 2 and value[0] == value[-1] and value[0] in ('"', "'"):
        return value[1:-1]
    return value


def _parse_where(expr: str) -> tuple[str, str, str]:
    from fsv.query import _parse_where as parse_where
    try:
        return parse_where(expr)
    except ValueError as exc:
        _err(str(exc))


def _where_field(expr: str) -> str:
    m = re.match(r"^(.+?)(>=|<=|!=|~=|=|>|<)", expr.strip())
    return m.group(1).strip() if m else expr.strip()


def _choice_value(field: dict[str, Any], raw: str, type_: str) -> Any:
    choices = field.get("choices") or []
    if not choices:
        return int(raw) if raw.isdigit() else raw
    folded = raw.casefold()
    for c in choices:
        if str(c.get("id") or "") == raw:
            return c.get("value", c.get("id")) if type_ == "custom_field" else c.get("id")
        labels = (c.get("value"), c.get("name"), c.get("label"), c.get("requester_display_value"))
        if any(str(x or "").casefold() == folded for x in labels):
            return c.get("value", c.get("id")) if type_ == "custom_field" else c.get("id", c.get("value", raw))
    shown = ", ".join(str(c.get("value") or c.get("name") or c.get("id")) for c in choices[:12])
    _err(f"no choice {raw!r} for {field.get('label') or field.get('name')}; choices: {shown}")


def _resolve_set_field(c: Any, sch: dict[str, Any], field_text: str, raw_value: str) -> tuple[str, Any]:
    """Resolve --set FIELD=VALUE → (api_field_name, resolved_value).

    FK fields (agent/group/department/requester) are remapped to *_id keys.
    Custom dropdowns return value strings; default dropdowns return IDs.
    """
    f = _find_schema_field(sch, field_text)
    if not f:
        _err(f"field not found: {field_text!r}; run `fsv changes fields`")

    name = str(f.get("name") or field_text)
    ftype = str(f.get("field_type") or "")
    type_ = _field_api_type(f)

    # FK remap: bare schema names → API *_id keys
    if ftype == "default_agent" or name == "agent":
        agent_id = int(_resolve_agent(c, raw_value)) if raw_value.isdigit() else int(_resolve_agent(c, raw_value))
        return "agent_id", agent_id

    if ftype == "default_group" or name == "group":
        group_id = int(_resolve_group(c, raw_value))
        return "group_id", group_id

    if ftype in ("default_requester",) or name == "requester":
        req = _resolve_requester(c, raw_value)
        return "requester_id", req["id"]

    if ftype in ("default_department",) or name == "department":
        choices = f.get("choices") or []
        if choices:
            dept_id = _choice_value(f, raw_value, "default")
            return "department_id", dept_id
        return "department_id", int(raw_value) if raw_value.isdigit() else raw_value

    # custom lookup (single or multi person)
    if "lookup" in ftype:
        link = (f.get("lookup_config") or {}).get("link")
        if "multi" in ftype:
            parts = [p.strip() for p in raw_value.split(",") if p.strip()]
            return name, [_resolve_lookup_user(c, p, link) for p in parts]
        return name, _resolve_lookup_user(c, raw_value, link)

    # checkbox
    if "checkbox" in ftype:
        value = raw_value.strip().casefold()
        if value not in ("true", "1", "yes", "on", "false", "0", "no", "off"):
            _err(f"invalid boolean for {field_text!r}: {raw_value!r}")
        return name, value in ("true", "1", "yes", "on")

    # multi-select → list
    if "multi_select" in ftype:
        parts = [p.strip() for p in raw_value.split(",") if p.strip()]
        return name, [_choice_value(f, p, type_) for p in parts]

    # Status and priority must use a unique active schema choice even via --set.
    if name in ("status", "priority") and (ftype in ("default_status", "default_priority") or type_ != "custom_field"):
        return name, _resolve_update_choice(sch, name, raw_value)

    # dropdown with choices
    choices = f.get("choices") or []
    if choices:
        return name, _choice_value(f, raw_value, type_)

    # text / date / number — pass through
    return name, raw_value


# Required schema field name → acceptable create-body keys (FK fields map to *_id).
_REQUIRED_FK_KEYS = {
    "requester": ("requester_id", "requester", "email"),
    "agent": ("agent_id", "agent"),
    "group": ("group_id", "group"),
    "department": ("department_id", "department"),
}


def _missing_required_change_fields(sch: dict[str, Any], body: dict[str, Any]) -> list[str]:
    """Labels of schema-required change fields absent from a create body.

    Maps embedded FK fields (requester/agent/group/department) to their *_id
    body keys. Skips default_status — the server auto-defaults it at create.
    Presence-only: a required checkbox left False is the server's job to reject.
    """
    present = set(body.keys())
    missing: list[str] = []
    for f in sch.get("fields", []) or []:
        if not f.get("required"):
            continue
        ftype = str(f.get("field_type") or "")
        if ftype == "default_status":
            continue
        name = str(f.get("name") or "")
        keys = _REQUIRED_FK_KEYS.get(name, (name,))
        if not any(k in present for k in keys):
            missing.append(str(f.get("label") or name))
    return missing


def _check_required(sch: dict[str, Any], body: dict[str, Any]) -> None:
    missing = _missing_required_change_fields(sch, body)
    if missing:
        _err("missing required field(s): " + ", ".join(missing) +
             "\n  supply via flags/--set (or in the editor), or pass --no-validate to skip")


def _where_operator(op: str) -> str:
    if op == "=":
        return "is_in"
    if op == "!=":
        return "is_not_in"
    if op in (">", ">=", "<", "<="):
        _err(f"operator {op!r} not supported; inequalities only work for date fields (created_at, updated_at, etc.). Use = or !=")
    if op == "~=":
        _err(f"operator ~= (contains) not supported; use = or !=")
    _err(f"unknown filter operator {op!r}; no equality fallback")


def _query_item(condition: str, operator: str, value: Any, type_: str, explain: list[dict[str, Any]], field: str, raw: str) -> dict[str, Any]:
    explain.append({"field": field, "condition": condition, "operator": operator, "type": type_, "value": raw})
    return {"value": value, "condition": condition, "operator": operator, "type": type_}


# Filter-panel wording accepted by --where (besides field names and labels).
_WHERE_DATE_ALIASES = {
    "created": "created_at", "created date": "created_at", "created on": "created_at", "created at": "created_at",
    "updated": "updated_at", "last modified": "updated_at", "modified": "updated_at", "updated at": "updated_at",
    "due": "due_by", "due by": "due_by", "due date": "due_by",
    "first response due": "fr_due_by", "first response due by": "fr_due_by", "fr due by": "fr_due_by",
    "closed": "closed_at", "closed at": "closed_at", "closed date": "closed_at", "closed on": "closed_at",
    "resolved": "resolved_at", "resolved at": "resolved_at", "resolved date": "resolved_at",
    "resolved on": "resolved_at",
}
_WHERE_FIELD_ALIASES = {"assigned to": "agent", "agents": "agent", "groups": "group", "requesters": "requester",
                        "departments": "department", "responder": "agent"}
# Filter-panel time periods ("Created: Last 6 months") -> the web's tokens.
_WHERE_PERIODS = {
    "today": "today", "yesterday": "yesterday", "this week": "week", "last week": "last_week",
    "this month": "month", "last month": "last_month", "last 2 months": "two_months",
    "last 6 months": "six_months", "last 24 hours": "twenty_four_hours", "last 7 days": "last_week",
}
_PERIOD_TOKENS = frozenset(_WHERE_PERIODS.values()) | {"five_minutes", "fifteen_minutes", "thirty_minutes",
                                                     "one_hour", "four_hours", "twelve_hours"}
# Filter-panel special values. The web sends "0" for "Me" and "-1" for
# "Unassigned"/"(empty)" on the people/group fields of a saved view.
_WHERE_ME = {"me", "(me)", "myself"}
_WHERE_EMPTY = {"(empty)", "empty", "unassigned", "(none)", "none", "-"}


def _date_condition(field_name: str, op: str, raw: str, explain: list[dict[str, Any]], shown: str,
                    res_name: str = "") -> dict[str, Any]:
    from fsv import query as Q
    token = _WHERE_PERIODS.get(raw.strip().casefold(), raw.strip() if raw.strip() in _PERIOD_TOKENS else None)
    if token and field_name in Q.TICKET_ONLY_DATES:
        _err(f"no filter-panel period exists for {field_name}; use on:/last:/next: PERIOD or a YYYY-MM-DD range "
             "(nothing was sent)")
    if token:
        if op not in ("=", ">="):
            _err("relative date periods support only = or >=; no inverse period wire is proven")
        if res_name == "problems" and (token != "today" or field_name != "created_at"):
            _err("only Created=today is a proven relative date filter on problems (other shapes returned 0 rows "
                 "silently); use a created_at>=YYYY-MM-DD and created_at<=YYYY-MM-DD range. Nothing was sent")
        return _query_item(field_name, "is_greater_than", token, "default", explain, shown, raw)
    try:
        return Q.date_condition(res_name, field_name, op, raw, explain, shown)
    except ValueError as exc:
        _err(str(exc))


def _where_schema_field(res: Resource, sch: dict[str, Any], field_text: str) -> dict[str, Any] | None:
    f = _find_schema_field(sch, field_text)
    if f:
        return f
    folded = " ".join(field_text.split()).casefold()
    alias = _WHERE_FIELD_ALIASES.get(folded)
    if alias:
        f = _find_schema_field(sch, alias)
        if f:
            return f
    for cut in ("s", "es"):
        if folded.endswith(cut) and len(folded) > len(cut) + 1:
            f = _find_schema_field(sch, field_text[: -len(cut)])
            if f:
                return f
    return None


def _resolve_where(c: Any, res: Resource, sch: dict[str, Any], expr: str, explain: list[dict[str, Any]]) -> dict[str, Any]:
    """One --where FIELD<op>VALUE: FIELD is an internal name or a label as the
    web's filter panel shows it ("Agents", "Created", "Planned Start Date",
    a custom dropdown's label); VALUE a choice label, a person/group name,
    "Me", "Unassigned"/"(empty)", a date, or a period ("Last 6 months")."""
    field_text, op, raw = _parse_where(expr)
    if field_text.strip().casefold() in ("approval_status", "approval status"):
        # Not a form field: the change list filters it by raw number (query.py
        # holds the proven wire); the ticket list ignores it, so refuse there.
        from fsv import query as Q
        try:
            return Q._resolve_where(c, res, sch, "approval_status" + op + raw, explain)
        except ValueError as exc:
            _err(str(exc))
    from fsv.query import DATE_FIELDS as pseudo_dates
    date_name = field_text if field_text in pseudo_dates else _WHERE_DATE_ALIASES.get(" ".join(field_text.split()).casefold())
    if date_name:
        return _date_condition(date_name, op, raw, explain, field_text, res.name)
    if field_text.endswith("_id"):
        if op in (">", "<", ">=", "<=", "~="):
            _err(f"operator {op!r} not supported on ID fields; use = or !=")
        operator = "is_in" if op == "=" else "is_not_in"
        val: Any = int(raw) if raw.isdigit() else raw
        return _query_item(field_text, operator, [val], "default", explain, field_text, raw)

    f = _where_schema_field(res, sch, field_text)
    if not f:
        labels = ", ".join(sorted({str(x.get("label")) for x in sch.get("fields") or [] if x.get("label")})[:25])
        _err(f"field not found: {field_text}; filter by a label shown on the form ({labels}) "
             f"or see `fsv {res.name} fields`")
    name = f.get("name") or field_text
    if name in pseudo_dates:
        return _date_condition(str(name), op, raw, explain, field_text, res.name)
    from fsv import query as Q
    try:
        typed = Q.typed_condition(res.name, f, op, raw, explain, field_text)
    except ValueError as exc:
        _err(str(exc))
    if typed is not None:
        return typed
    type_ = _field_api_type(f)
    ftype = str(f.get("field_type") or "")
    from fsv.filter_proof import require_filterable_field
    require_filterable_field(f)
    operator = _where_operator(op)
    folded = raw.strip().casefold()

    if name == "requester":
        if folded in _WHERE_ME:
            from fsv import views as _views
            me = _views.current_user(c)
            if not me.get("id"):
                _err("cannot tell who you are (agents/me is not readable); use your email instead of 'me'")
            return _query_item("requester_id", operator, [{"id": me["id"], "name": me.get("name") or "Me"}],
                               "default", explain, name, f"Me ({me['id']})")
        req = _resolve_requester(c, raw)
        return _query_item("requester_id", operator, [{"id": req["id"], "name": req["name"]}], "default", explain, name, f"{req['name']} ({req['id']})")
    if name == "agent":
        condition = "responder_id" if res.name == "tickets" else "agent_id"
        if folded in _WHERE_ME:
            return _query_item(condition, operator, ["0"], "default", explain, name, "Me")
        if folded in _WHERE_EMPTY:
            return _query_item(condition, operator, ["-1"], "default", explain, name, "Unassigned")
        agent_id = _resolve_agent(c, raw)
        return _query_item(condition, operator, [agent_id], "default", explain, name, agent_id)
    if name == "group":
        if folded in _WHERE_EMPTY:
            return _query_item("group_id", operator, ["-1"], "default", explain, name, "(empty)")
        val = _choice_value(f, raw, "default") if f.get("choices") else _resolve_group(c, raw)
        return _query_item("group_id", operator, [str(val)], "default", explain, name, str(val))
    if folded in _WHERE_EMPTY and (f.get("choices") or "dropdown" in ftype or name == "department"):
        has_choice = any(str(ch.get("value") or ch.get("name") or "").strip().casefold() == folded
                         for ch in f.get("choices") or [])
        if not has_choice:
            cond = "department_id" if name == "department" else name
            return _query_item(cond, operator, ["-1"], type_, explain, name, "(empty)")

    from fsv.filter_proof import name_valued_choice
    try:
        named = name_valued_choice(f, raw)
    except ValueError as exc:
        _err(str(exc))
    if named is not None:
        return _query_item(name, operator, [named], type_, explain, name, named)
    if f.get("choices") or "dropdown" in ftype or ftype in ("default_status", "default_priority"):
        val = _choice_value(f, raw, type_)
        cond = "department_id" if name == "department" and type_ == "default" else name
        return _query_item(cond, operator, [val], type_, explain, name, str(val))
    val = int(raw) if raw.isdigit() and ftype.endswith("number") else raw
    return _query_item(name, operator, [val], type_, explain, name, raw)


def _merge_date_ranges(query: list[dict[str, Any]]) -> list[dict[str, Any]]:
    from fsv.query import merge_date_ranges
    return merge_date_ranges(query)


def _merge_same_conditions(query: list[dict[str, Any]]) -> list[dict[str, Any]]:
    seen: dict[tuple[str, str], dict[str, Any]] = {}
    out: list[dict[str, Any]] = []
    for item in query:
        op = item.get("operator")
        if op not in ("is_in", "is_not_in") or not isinstance(item.get("value"), list):
            out.append(item)
            continue
        key = (item.get("condition"), op, item.get("type"))
        if key not in seen:
            merged = dict(item)
            merged["value"] = list(item.get("value") or [])
            seen[key] = merged
            out.append(merged)
        else:
            seen[key]["value"].extend(item.get("value") or [])
    return out


def _show_query_explain(explain: list[dict[str, Any]], query: list[dict[str, Any]], or_grouping: bool = False) -> None:
    t = Table(title="query")
    t.add_column("field")
    t.add_column("condition")
    t.add_column("operator")
    t.add_column("type")
    t.add_column("value", style="dim")
    for x in explain:
        t.add_row(str(x["field"]), str(x["condition"]), str(x["operator"]), str(x["type"]), str(x["value"]))
    console.print(t)
    grouping = "OR" if or_grouping else "AND"
    from fsv.query import _adv_needed
    param = "advanced_query_hash" if (or_grouping or _adv_needed(query)) else "query_hash"
    console.print(f"api: internal /api/_/ with {param} ({len(query)} conditions, {grouping})")


def _build_query_hash(
    c: Any,
    res: Resource,
    raw: Optional[str],
    where: list[str],
    or_grouping: bool = False,
) -> tuple[Optional[str], list[dict[str, Any]], list[dict[str, Any]]]:
    import json
    from fsv import schema as schema_mod
    query: list[dict[str, Any]] = []
    explain: list[dict[str, Any]] = []
    if raw:
        loaded = _load_query_hash(raw)
        from fsv.query import _check_raw_query
        try:
            _check_raw_query(res, loaded)
        except ValueError as exc:
            _err(str(exc))
        query.extend(loaded)
        explain.append({"field": "query_hash", "condition": "raw", "operator": "raw", "type": "raw", "value": raw})
    if where:
        sch = schema_mod.load(res, c)
        for expr in where:
            query.append(_resolve_where(c, res, sch, expr, explain))
    from fsv import query as Q
    query = _merge_date_ranges(query)
    query = _merge_same_conditions(query)
    if not query:
        return None, query, explain
    advanced = or_grouping or Q._adv_needed(query)
    payload, _adv, clean = Q.serialize(res.name, query, or_grouping,
                                       Q.runtime_workspace(sch if where else None) if advanced else None)
    return payload, (query if advanced else clean), explain


def _saved_views(c: Any, res: Resource) -> list[Any]:
    """The web's saved views as :class:`fsv.views.SavedView` (``[]`` when the
    account will not list them)."""
    from fsv import views as V
    try:
        raw = _load_views(c, res)
    except SessionError:
        raise
    except Exception:  # noqa: BLE001 - views are optional (role/plan)
        return []
    out = []
    for v in raw or []:
        if not isinstance(v, dict) or v.get("id") in (None, ""):
            continue
        marked = bool(v.get("default"))
        out.append(V.SavedView(str(v["id"]), str(v.get("name") or v["id"]),
                               system=marked and v.get("user_id") is None, marked_default=marked, raw=v))
    return out


def _pick_view(c: Any, res: Resource, view: str | None, use_default: bool) -> Any:
    from fsv import views as V
    saved = _saved_views(c, res)
    if view:
        try:
            return V.find_view(saved, view)
        except ValueError as e:
            if not saved:  # cannot list views here: pass the id through as given
                return V.SavedView(view, view)
            _err(str(e))
    if use_default:
        return V.default_view(saved)
    return None


DUE_BY_PENDING_NOTE = ("Note: Pending tickets have no Due By on this tenant type (the field is hidden and empty "
                       "while Pending), so a Due By filter never returns them; list them with --where Status=Pending.")


def _due_by_without_status(clauses: Any) -> bool:
    """A ticket list predicate that filters on Due By but not on status."""
    if not isinstance(clauses, list):
        return False
    names = {str(c.get("condition") or "") for c in clauses if isinstance(c, dict)}
    return bool(names & {"due_by", "due_by_date"}) and not names & {"status", "status_id"}


def list_resource(
    res: Resource,
    filter_name: Optional[str],
    where: list[str],
    debug: bool,
    per_page: int,
    page: int,
    all_pages: bool,
    format_: OutputFormat | str,
    json_out: bool,
    or_grouping: bool = False,
    pager: bool = True,
    raw_query_hash: str | None = None,
    order_by: str | None = None,
    order_type: SortOrder | str | None = None,
    n_pages: int | None = None,
) -> None:
    """A list page like the web's: a saved view (by name; the server's
    default when neither --view nor filters are given), the site's columns
    and a ``<view> · 1-30 of N · Sort by: ...`` header."""
    from fsv import schema as schema_mod
    from fsv import service
    from fsv import views as V
    from fsv import pages
    c = _client()
    qh, query, explanation = _api(lambda: _build_query_hash(c, res, raw_query_hash, where, or_grouping))
    if debug:
        _show_query_explain(explanation, query, or_grouping)
        return
    from fsv.query import _adv_needed
    advanced = bool(or_grouping or _adv_needed(query))
    for x in explanation:
        if "no strict operator exists" in str(x.get("value")):
            err.print(f"note: {x['field']} {x['value']}", markup=False, highlight=False)
    view = _api(lambda: _pick_view(c, res, filter_name, use_default=not where and not raw_query_hash))
    ot = order_type.value if isinstance(order_type, SortOrder) else order_type
    ob = order_by
    if view is not None and not ob and view.raw:
        settings = _api(lambda: V.view_settings(c, res, view))
        ob = settings.get("order_by") or None
        if ob:
            ot = settings.get("order_type") or ot
    ob = ob or "created_at"
    ot = ot or "desc"
    headers = _api(lambda: V.ticket_headers(c)) if res == TICKETS else []
    fmt = format_.value if isinstance(format_, OutputFormat) else format_
    view_id = view.id if view is not None else None

    def load_items() -> tuple[list[dict], dict[str, Any], int | None, bool]:
        size = 100 if (all_pages or n_pages is not None) else per_page
        if all_pages or n_pages is not None:
            acc: list[dict] = []
            limit = n_pages if n_pages is not None else None
            total, more = None, False
            for p in range(page, page + (limit or 100)):
                batch, data = service.list_page(
                    res, client=c, page=p, per_page=size,
                    filter_name=view_id, order_by=ob, order_type=ot,
                    query_hash=qh, or_grouping=advanced,
                )
                acc.extend(batch)
                total, more = V.list_meta(data, batch, p, size)
                err.print(f"  fetched {len(acc)} so far...", highlight=False)
                if len(batch) < size or not more:
                    break
            return acc, schema_mod.load(res, c), total, more
        items, data = service.list_page(
            res, client=c, page=page, per_page=size,
            filter_name=view_id, order_by=ob, order_type=ot,
            query_hash=qh, or_grouping=advanced,
        )
        total, more = V.list_meta(data, items, page, size)
        return items, schema_mod.load(res, c), total, more

    items, sch, total, more = _api(load_items)
    if where or raw_query_hash:
        from fsv.filter_proof import contradictions
        bad = contradictions(items, query) if not or_grouping else []
        if bad:
            err.print("Warning: returned records contradict the requested predicate; do not use this list for writes. IDs: " + ", ".join(map(str, bad)), markup=False)
        if res.name == "tickets" and _due_by_without_status(query):
            err.print(DUE_BY_PENDING_NOTE, markup=False)
    me = V.current_user(c) if fmt != "json" and not json_out else {}
    tz = V.user_tz(me)
    columns = V.list_columns(res, sch, headers)
    rows = [V.list_row(x, res, columns, sch, me.get("id"), tz) for x in items]
    if json_out or fmt == "json":
        emit_json(items)
        return
    if fmt in ("csv", "tsv"):
        render.emit_delimited(rows, "," if fmt == "csv" else "\t")
        return
    if fmt != "table":
        _err("--output must be table, json, csv, or tsv")
    eff_page = page if not (all_pages or n_pages is not None) else 1
    lp = V.ListPage(res, items, columns, rows, view, eff_page, per_page if eff_page == page and not all_pages and n_pages is None else max(len(items), 1),
                    total, more, ob, ot, V.sort_label(ob, sch))
    head = pages.list_header(lp)
    if where or raw_query_hash:
        head += f"[dim] · filtered: {escape(' '.join(where) or 'query hash')}[/]"
    if _should_page(len(items) * 2 + 2, pager):
        with console.capture() as capture:
            console.print(head, highlight=False)
            console.print(pages.list_table(lp))
        if not _page_text(capture.get()):
            console.print(head, highlight=False)
            console.print(pages.list_table(lp))
    else:
        console.print(head, highlight=False)
        console.print(pages.list_table(lp))
    err.print(f"{len(items)} rows")


def get_resource(res: Resource, id_: str, stats: bool, json_out: bool, format_: OutputFormat | str = "table",
                 tab: str | None = None, all_tabs: bool = False, all_conversations: bool = False) -> None:
    """The record laid out like its web page (header, lifecycle, reporter,
    Details with the Properties panel, and any tab). JSON keeps the record's
    own keys and adds ``view`` with the same sections."""
    from fsv import schema as schema_mod
    from fsv import service
    from fsv import views as V
    from fsv import pages
    cid = _cid(id_, res)
    c = _client()
    fmt = format_.value if isinstance(format_, OutputFormat) else format_

    if stats and res == CHANGES and (json_out or fmt == "json"):
        def load_evidence() -> dict[str, Any]:
            sch = schema_mod.load(res, c)
            evidence = service.get_change_evidence(cid, client=c)
            return {
                **evidence["change"],
                "planning_fields": evidence["planning_fields_by_name"],
                "change_planning_fields": evidence["planning_fields"],
                "planning_field_definitions": service.get_change_planning_field_definitions(sch),
                "main_attachments": evidence["main_attachments"],
                "description_attachment_urls": evidence["description_attachment_urls"],
            }
        emit_json(_api(load_evidence))
        return

    def load() -> Any:
        sch = schema_mod.load(res, c)
        item = service.get_item(res, cid, client=c,
                                includes="requester,stats,phone,feedback,ticket_status,responder"
                                if res == TICKETS else None)
        d = V.load_detail(c, res, cid, record=item, schema=sch, all_conversations=all_conversations)
        wanted: list[str] = []
        if all_tabs:
            wanted = [t.name for t in d.tabs if t.name != "details"]
        elif tab:
            try:
                wanted = [V.resolve_tab(d, tab)]
            except ValueError as e:
                _err(str(e))
        for name in wanted:
            if name != "details":
                d.tab_data[name] = V.load_tab(c, res, d.record, name)
        return d

    d = _api(load)
    if json_out or fmt == "json":
        doc = dict(d.record)
        if d.requested_items is not None:
            doc["requested_items"] = d.requested_items
        doc["view"] = d.to_json()
        emit_json(doc)
        return
    if fmt in ("csv", "tsv"):
        rows = [{"section": "summary", "label": k, "value": v} for k, v in d.summary()]
        rows += [{"section": "properties", "label": p.label, "value": p.value} for p in d.properties]
        render.emit_delimited(rows, "," if fmt == "csv" else "\t")
        return
    active = None if all_tabs else (V.resolve_tab(d, tab) if tab else None)
    pages.print_detail(console, d, tab=active, all_tabs=all_tabs)


def _local_dt(value: str | None) -> datetime | None:
    from datetime import datetime
    if not value:
        return None
    try:
        return datetime.fromisoformat(value.replace("Z", "+00:00")).astimezone()
    except ValueError:
        return None


def _activity_day(dt: datetime | None) -> str:
    from datetime import datetime
    if not dt:
        return "Unknown"
    today = datetime.now().astimezone().date()
    if dt.date() == today:
        return "Today"
    if (today - dt.date()).days == 1:
        return "Yesterday"
    return dt.strftime("%Y-%m-%d")


def _activity_text(value: str | None) -> str:
    return re.sub(r"\s+", " ", strip_html(value)).strip()


def _truncation_notice(total: int, shown: int, partial: bool) -> str | None:
    """'truncated: N more ...' when rows were cut by --limit or pages were capped."""
    hidden = total - shown
    if hidden <= 0 and not partial:
        return None
    more = f"{hidden}{'+' if partial else ''} more" if hidden > 0 else "more on the server"
    return f"truncated: {more} (showing {shown} of {total}{'+' if partial else ''}); use --limit N or --all"


def activity_resource(res: Resource, id_: str, limit: int | None, json_out: bool, format_: OutputFormat | str = "table") -> None:
    from fsv import views
    cid = _cid(id_, res)
    c = _client()
    td = _api(lambda: views.load_tab(c, res, {"id": cid}, "activity"))
    _read_warnings(td)
    rows = td.data or []
    acts = rows if limit is None else rows[:max(limit, 0)]
    notice = _truncation_notice(len(rows), len(acts), bool(td.meta.get("partial")))
    if notice and (json_out or (format_.value if isinstance(format_, OutputFormat) else format_) != "table"):
        # Machine formats keep their stdout shape; the notice goes to stderr.
        err.print(f"[yellow]{notice}[/]", highlight=False)
    flat = [{"day": _activity_day(_local_dt(a.get("created_at"))), "time": (lambda dt: dt.strftime("%H:%M") if dt else "--:--")(_local_dt(a.get("created_at"))), "actor": (a.get("actor") or {}).get("name", "?"), "content": _activity_text(a.get("content"))} for a in acts]
    if _emit_fmt(acts, flat, format_, json_out):
        return
    last_day = None
    for a in acts:
        dt = _local_dt(a.get("created_at"))
        day = _activity_day(dt)
        if day != last_day:
            console.print(f"\n[bold]{day}[/]")
            last_day = day
        actor = (a.get("actor") or {}).get("name", "?")
        when = dt.strftime("%H:%M") if dt else "--:--"
        console.print(f"[dim]{when}[/]  [cyan]{escape(actor)}[/]  {escape(_activity_text(a.get('content')))}")
        for sub_c in a.get("sub_contents") or []:
            console.print(f"      [dim]·[/] {escape(_activity_text(sub_c))}")
    if notice:
        console.print(f"\n[yellow]{escape(notice)}[/]", highlight=False)


def tasks_resource(res: Resource, id_: str, format_: OutputFormat | str = "table", json_out: bool = False) -> None:
    from fsv import views
    cid = _cid(id_, res)
    c = _client()
    td = _api(lambda: views.load_tab(c, res, {"id": cid}, "tasks"))
    _read_warnings(td)
    items = td.data or []
    def _flat(x: dict) -> dict:
        cf = x.get("custom_fields") or {}
        row: dict = {
            "id": x.get("human_display_id") or str(x.get("id")),
            "status": x.get("status_name") or str(x.get("status")),
            "group_agent": (x.get("group") or {}).get("name") or (x.get("agent") or {}).get("name") or "-",
            "due": x.get("task_due_status") or x.get("due_date") or "-",
        }
        if res == CHANGES:
            row["system_env"] = f"{cf.get('system') or '-'} / {cf.get('environment') or '-'}"
        row["title"] = (x.get("title") or "")[:60]
        return row
    if _emit_fmt(items, [_flat(x) for x in items], format_, json_out):
        return
    t = Table()
    t.add_column("TSK", style="cyan")
    t.add_column("Status")
    t.add_column("Group/Agent")
    t.add_column("Due", style="dim")
    if res == CHANGES:
        t.add_column("System/Env")
    t.add_column("Title")
    for x in items:
        row = [
            x.get("human_display_id") or str(x.get("id")),
            x.get("status_name") or str(x.get("status")),
            (x.get("group") or {}).get("name") or (x.get("agent") or {}).get("name") or "-",
            x.get("task_due_status") or x.get("due_date") or "-",
        ]
        if res == CHANGES:
            cf = x.get("custom_fields") or {}
            row.append(f"{cf.get('system') or '-'} / {cf.get('environment') or '-'}")
        row.append((x.get("title") or "")[:60])
        t.add_row(*row)
    console.print(t)


_ASSET_CATEGORY_SELECT_RE = re.compile(r'<select[^>]*name="ci_type_id"[^>]*>(.*?)</select>', re.I | re.S)
_ASSET_CATEGORY_OPTION_RE = re.compile(r'<option(?:\s+value(?:="([^"]*)")?)?[^>]*>(.*?)</option>', re.I | re.S)
_ASSET_CATEGORY_CACHE: list[dict[str, str]] | None = None


def _prefixed_value(ci: dict[str, Any], prefix: str) -> Any:
    """Value of the CI-type-specific field ``<prefix><ci_type_id>`` without
    hard-coding the tenant's CI-type suffix: prefer the key whose suffix is this
    asset's ci_type_id, else the first non-empty key with the prefix."""
    keys = sorted(k for k in ci if isinstance(k, str) and k.startswith(prefix))
    type_id = str(ci.get("ci_type_id") or "")
    if type_id and f"{prefix}{type_id}" in keys:
        keys.remove(f"{prefix}{type_id}")
        keys.insert(0, f"{prefix}{type_id}")
    for k in keys:
        if ci.get(k) not in (None, ""):
            return ci[k]
    return None


def _asset_display(asset: dict[str, Any]) -> dict[str, Any]:
    ci = asset.get("config_item") if isinstance(asset.get("config_item"), dict) else asset
    return {
        "id": ci.get("display_id") or ci.get("id") or asset.get("id"),
        "name": ci.get("name") or "-",
        "type": ci.get("asset_type") or ci.get("ci_type_name") or "-",
        "used_by": ci.get("user_name") or ci.get("used_by") or "-",
        "location": ci.get("location_name") or "-",
        "state": _prefixed_value(ci, "asset_state_") or "-",
        "serial": _prefixed_value(ci, "serial_number_") or "-",
    }


def _fetch_asset_categories(c: Client | None = None) -> list[dict[str, str]]:
    from fsv.client import get_client
    from html import unescape
    global _ASSET_CATEGORY_CACHE
    if _ASSET_CATEGORY_CACHE is not None:
        return _ASSET_CATEGORY_CACHE
    if c is None:
        c = get_client()
    html = c._client.get(
        f"https://{config.DOMAIN}/cmdb/items",
        headers={"Accept": "text/html,application/xhtml+xml"},
        follow_redirects=True,
    ).text
    out: list[dict[str, str]] = []
    seen: set[str] = set()
    block_match = _ASSET_CATEGORY_SELECT_RE.search(html)
    if not block_match:
        _err("could not load asset categories from /cmdb/items")
    block = block_match.group(1)
    for ci_type_id, raw_label in _ASSET_CATEGORY_OPTION_RE.findall(block):
        label = " ".join(unescape(re.sub(r"<[^>]+>", "", raw_label)).split())
        if not label:
            continue
        key = label.casefold()
        if key in seen:
            continue
        seen.add(key)
        out.append({
            "name": "All assets" if not ci_type_id and key == "all assets" else label,
            "filter": "all_assets" if not ci_type_id else ci_type_id,
            "ci_type_id": ci_type_id,
        })
    _ASSET_CATEGORY_CACHE = out
    return out


def _resolve_asset_category(value: str, c: Client | None = None) -> dict[str, str]:
    raw = " ".join(str(value).strip().split())
    if not raw:
        _err("empty asset category")
    if raw.casefold() in {"all", "all assets"}:
        return {"name": "All assets", "filter": "all_assets", "ci_type_id": ""}
    categories = _fetch_asset_categories(c)
    folded = raw.casefold()
    exact = [cat for cat in categories if cat["name"].casefold() == folded]
    if len(exact) == 1:
        return exact[0]
    if len(exact) > 1:
        shown = ", ".join(cat["name"] for cat in exact[:5])
        _err(f"multiple asset categories match {value!r}: {shown}")
    matches = [cat for cat in categories if folded in cat["name"].casefold()]
    if len(matches) == 1:
        return matches[0]
    if not matches:
        _err(f"unknown asset category: {value!r}. Try --list-categories")
    shown = ", ".join(cat["name"] for cat in matches[:8])
    _err(f"multiple asset categories match {value!r}: {shown}")


def _filter_assets_by_category(items: list[dict[str, Any]], category: dict[str, str] | None) -> list[dict[str, Any]]:
    if not category or category.get("filter") == "all_assets":
        return items
    wanted = category["name"].casefold()
    out: list[dict[str, Any]] = []
    for item in items:
        row = item if "type" in item and "name" in item else _asset_display(item)
        if str(row.get("type") or "").casefold() == wanted:
            out.append(item)
    return out


def _resolve_change_asset_id(
    change_id: int,
    value: str,
    c: Client | None = None,
    associated: bool = False,
    category: dict[str, str] | None = None,
) -> int:
    from fsv.client import get_client
    from fsv.create import get_change_assets, search_assets_for_change
    raw = str(value).strip()
    if raw.isdigit():
        return int(raw)
    if c is None:
        c = get_client()
    folded = raw.casefold()
    if associated:
        rows = _filter_assets_by_category([_asset_display(asset) for asset in get_change_assets(change_id, c)], category)
        exact = [row for row in rows if str(row.get("id") or "") == raw or str(row.get("name") or "").casefold() == folded]
        if len(exact) == 1:
            return int(exact[0]["id"])
        if len(exact) > 1:
            shown = ", ".join(f"{row['name']} [{row['id']}]" for row in exact[:5])
            _err(f"multiple associated assets match {value!r}: {shown}")
        matches = [row for row in rows if folded in str(row.get("name") or "").casefold()]
        if len(matches) == 1:
            return int(matches[0]["id"])
        if not matches:
            suffix = f" in category {category['name']!r}" if category else ""
            _err(f"asset not associated on {format_id({'id': change_id}, CHANGES)}{suffix}: {value!r}")
        shown = ", ".join(f"{row['name']} [{row['id']}]" for row in matches[:5])
        _err(f"multiple associated assets match {value!r}: {shown}")
    assets = search_assets_for_change(change_id, raw, per_page=50, c=c).get("assets") or []
    assets = _filter_assets_by_category(assets, category)
    exact = [asset for asset in assets if str(asset.get("display_id") or "") == raw or str(asset.get("name") or "").casefold() == folded]
    if len(exact) == 1:
        return int(exact[0]["display_id"])
    if len(exact) > 1:
        shown = ", ".join(f"{asset.get('name') or '-'} [{asset.get('display_id') or '?'}]" for asset in exact[:5])
        _err(f"multiple assets match {value!r}: {shown}")
    if len(assets) == 1:
        return int(assets[0]["display_id"])
    if not assets:
        suffix = f" in category {category['name']!r}" if category else ""
        _err(f"no asset match{suffix}: {value}")
    shown = ", ".join(f"{asset.get('name') or '-'} [{asset.get('display_id') or '?'}]" for asset in assets[:5])
    _err(f"multiple assets match {value!r}: {shown}")


def _resolve_change_asset_ids(
    change_id: int,
    values: list[str],
    c: Client | None = None,
    associated: bool = False,
    category: dict[str, str] | None = None,
) -> list[int]:
    from fsv.client import get_client
    if c is None:
        c = get_client()
    out: list[int] = []
    seen: set[int] = set()
    for value in values:
        did = _resolve_change_asset_id(change_id, value, c, associated=associated, category=category)
        if did not in seen:
            seen.add(did)
            out.append(did)
    return out


def _prompt_text(title: str, text: str, default: str = "") -> str | None:
    from prompt_toolkit.shortcuts import input_dialog
    return input_dialog(title=title, text=text, default=default).run()


def _prompt_multi_select(title: str, text: str, values: list[tuple[str, str]]) -> list[str] | None:
    from prompt_toolkit.shortcuts import checkboxlist_dialog
    result = checkboxlist_dialog(title=title, text=text, values=values).run()
    return list(result) if result else result


def _prompt_asset_category(c: Client | None = None) -> dict[str, str] | None:
    from fsv.client import get_client
    if c is None:
        c = get_client()
    value = _prompt_text(
        "Asset category",
        "Category for asset search. Leave empty for All assets.",
        default="All assets",
    )
    if value is None:
        return None
    raw = value.strip() or "All assets"
    return _resolve_asset_category(raw, c)


def _pick_change_assets(change_id: int, c: Client | None = None, category: dict[str, str] | None = None) -> list[int]:
    from fsv.client import get_client
    from fsv.create import search_assets_for_change
    if _no_input() or not sys.stdin.isatty():
        _err("interactive picker requires TTY")
    if c is None:
        c = get_client()
    if category is None:
        category = _prompt_asset_category(c)
        if category is None:
            raise typer.Exit(0)
    prompt = f"Search assets for {format_id({'id': change_id}, CHANGES)}"
    if category and category.get("filter") != "all_assets":
        prompt += f"\nCategory: {category['name']}"
    query = _prompt_text("Associate assets", prompt)
    if query is None:
        raise typer.Exit(0)
    items = search_assets_for_change(change_id, query, per_page=50, c=c).get("assets") or []
    items = _filter_assets_by_category(items, category)
    if not items:
        suffix = f" in category {category['name']!r}" if category else ""
        _err(f"no asset match{suffix}: {query}")
    values = [
        (
            str(item.get("display_id") or ""),
            f"[{item.get('display_id') or '?'}] {item.get('name') or '-'} · {item.get('ci_type_name') or '-'} · {item.get('location_name') or '-'}",
        )
        for item in items
        if item.get("display_id")
    ]
    selected = _prompt_multi_select(
        "Associate assets",
        f"Select asset(s) for {format_id({'id': change_id}, CHANGES)}",
        values,
    )
    if selected is None:
        raise typer.Exit(0)
    return [int(value) for value in selected]


def _pick_change_tickets(change_id: int, c: Client | None = None) -> list[int]:
    from fsv.client import get_client
    from fsv.create import search_change_tickets
    if _no_input() or not sys.stdin.isatty():
        _err("interactive picker requires TTY")
    if c is None:
        c = get_client()
    query = _prompt_text("Associate tickets", f"Search ticket ID or subject for {format_id({'id': change_id}, CHANGES)}")
    if query is None:
        raise typer.Exit(0)
    items = search_change_tickets(query, c)
    if not items:
        _err(f"no ticket match: {query}")
    values = [
        (
            str(item.get("id") or ""),
            f"[{item.get('human_display_id') or format_id(item, TICKETS)}] {item.get('subject') or '-'} · {item.get('status_name') or item.get('status') or '-'}",
        )
        for item in items
        if item.get("id")
    ]
    selected = _prompt_multi_select(
        "Associate tickets",
        f"Select ticket(s) for {format_id({'id': change_id}, CHANGES)}",
        values,
    )
    if selected is None:
        raise typer.Exit(0)
    return [int(value) for value in selected]


def assets_resource(id_: str, search: str | None, add: list[str], remove: list[str], page: int, per_page: int,
                    dry_run: bool, yes: bool, json_out: bool, format_: OutputFormat | str = "table",
                    pick: bool = False, category_name: str | None = None, list_categories: bool = False) -> None:
    from fsv.create import (
        associate_assets,
        dissociate_assets,
        get_change_assets,
        search_assets_for_change,
    )
    actions = sum(1 for active in (search is not None, bool(add), bool(remove), pick, list_categories) if active)
    if actions > 1:
        _err("choose only one action: --search, --add, --remove, --pick, or --list-categories")
    c = _client()
    category = _resolve_asset_category(category_name, c) if category_name else None
    if list_categories:
        items = _fetch_asset_categories(c)
        if _emit_fmt(items, items, format_, json_out):
            return
        t = Table(title="Asset categories")
        t.add_column("Name")
        t.add_column("CI Type ID", style="dim")
        for item in items:
            t.add_row(item.get("name") or "-", item.get("ci_type_id") or "-")
        console.print(t)
        return
    cid = _cid(id_, CHANGES)
    if pick:
        asset_ids = _pick_change_assets(cid, c, category)
        if not asset_ids:
            console.print("cancelled")
            return
        payload = {"action": "associate_assets", "change_id": cid, "asset_ids": asset_ids}
        if dry_run:
            emit_json(payload)
            return
        if not yes:
            if _no_input() or not sys.stdin.isatty():
                _err("pass --yes to associate assets")
            typer.confirm(f"Associate {len(asset_ids)} asset(s) with {format_id({'id': cid}, CHANGES)}?", abort=True)
        _api(lambda: associate_assets(cid, asset_ids, c))
        if json_out:
            emit_json({**payload, "ok": True})
            return
        console.print(f"[green]associated[/] {len(asset_ids)} asset(s) with {format_id({'id': cid}, CHANGES)}")
        return
    if add:
        asset_ids = _resolve_change_asset_ids(cid, add, c, category=category)
        payload = {"action": "associate_assets", "change_id": cid, "asset_ids": asset_ids}
        if dry_run:
            emit_json(payload)
            return
        if not yes:
            if _no_input() or not sys.stdin.isatty():
                _err("pass --yes to associate assets")
            typer.confirm(f"Associate {len(asset_ids)} asset(s) with {format_id({'id': cid}, CHANGES)}?", abort=True)
        _api(lambda: associate_assets(cid, asset_ids, c))
        if json_out:
            emit_json({**payload, "ok": True})
            return
        console.print(f"[green]associated[/] {len(asset_ids)} asset(s) with {format_id({'id': cid}, CHANGES)}")
        return
    if remove:
        asset_ids = _resolve_change_asset_ids(cid, remove, c, associated=True, category=category)
        payload = {"action": "dissociate_assets", "change_id": cid, "asset_display_ids": asset_ids}
        if dry_run:
            emit_json(payload)
            return
        if not yes:
            if _no_input() or not sys.stdin.isatty():
                _err("pass --yes to remove assets")
            typer.confirm(f"Remove {len(asset_ids)} asset(s) from {format_id({'id': cid}, CHANGES)}?", abort=True)
        _api(lambda: dissociate_assets(cid, asset_ids, c))
        if json_out:
            emit_json({**payload, "ok": True})
            return
        console.print(f"[green]removed[/] {len(asset_ids)} asset(s) from {format_id({'id': cid}, CHANGES)}")
        return
    if search is not None:
        data = _api(lambda: search_assets_for_change(cid, search, page, per_page, c))
        items = _filter_assets_by_category(data.get("assets") or [], category)
        flat_rows = [_asset_display(x) for x in items]
        payload = data
        if category:
            payload = {
                **data,
                "assets": items,
                "meta": {**(data.get("meta") or {}), "filtered_count": len(items), "category": category["name"]},
            }
        if _emit_fmt(payload, flat_rows, format_, json_out):
            return
    else:
        items = _filter_assets_by_category(_api(lambda: get_change_assets(cid, c)), category)
        flat_rows = [_asset_display(x) for x in items]
        if _emit_fmt(items, flat_rows, format_, json_out):
            return
    t = Table(title="Assets")
    t.add_column("ID", style="cyan")
    t.add_column("Name")
    t.add_column("Type")
    t.add_column("Used By")
    t.add_column("Location")
    t.add_column("State")
    t.add_column("Serial")
    for item in items:
        row = _asset_display(item)
        t.add_row(*(str(row[k]) for k in ("id", "name", "type", "used_by", "location", "state", "serial")))
    console.print(t)


def asset_categories_resource(format_: OutputFormat | str = "table", json_out: bool = False) -> None:
    items = _fetch_asset_categories(_client())
    if _emit_fmt(items, items, format_, json_out):
        return
    t = Table(title="Asset categories")
    t.add_column("Name")
    t.add_column("CI Type ID", style="dim")
    for item in items:
        t.add_row(item.get("name") or "-", item.get("ci_type_id") or "-")
    console.print(t)


def change_associations_resource(
    id_: str,
    search: str | None,
    add: list[str],
    remove: list[str],
    dry_run: bool,
    yes: bool,
    json_out: bool,
    format_: OutputFormat | str = "table",
    pick: bool = False,
    expect_plan: str | None = None,
    read_only: bool = False,
) -> None:
    from fsv.create import (
        get_change_associations,
        search_change_tickets,
    )

    actions = sum(1 for active in (search is not None, bool(add), bool(remove), pick) if active)
    if actions > 1:
        _err("choose only one action: --search, --add, or --remove")
    cid = _cid(id_, CHANGES)
    if search is not None:
        items = search_change_tickets(search)
        flat_rows = [
            {
                "id": item.get("human_display_id") or str(item.get("id", "-")),
                "subject": item.get("subject") or item.get("title") or "-",
                "status": item.get("status_name") or str(item.get("status", "-")),
            }
            for item in items
        ]
        if _emit_fmt(items, flat_rows, format_, json_out):
            return
        _print_tickets(items, "Ticket search")
        return
    if pick or add or remove:
        from fsv.change_associations import build_plan, send
        if read_only and not dry_run:
            _err("read-only mode: no association request was sent")
        if remove and len(remove) != 1:
            _err("unlink accepts one ticket per reviewed request; review each removal separately")
        ids = (_pick_change_tickets(cid) if pick else
               [_resolve_associated_change_ticket_id(cid, x) for x in remove] if remove else
               [_resolve_change_ticket_id(x) for x in add])
        if not ids:
            console.print("cancelled")
            return
        plan = build_plan(cid, ids, unlink=bool(remove))
        if dry_run:
            emit_json(send(plan, reviewed_hash=None, dry_run=True))
            return
        if read_only:
            _err("read-only mode: no association request was sent")
        err.print_json(data=plan.review())
        if expect_plan is not None and expect_plan != plan.hash:
            _err("association review hash does not match; review again")
        if yes:
            if expect_plan != plan.hash:
                _err("--yes requires the full --expect-plan from a prior association review")
        elif _no_input() or not sys.stdin.isatty():
            _err("sending requires --yes and --expect-plan in no-input mode")
        elif not typer.confirm("Send this exact reviewed association request?", default=False):
            console.print("cancelled")
            return
        result = send(plan, reviewed_hash=plan.hash, read_only=read_only)
        if json_out:
            emit_json(result)
        else:
            console.print(f"Association {result['status']}; complete inventory verified: {result['verified']}")
        return
    assoc = get_change_associations(cid)
    flat_rows = [
        {
            "type": k,
            "id": item.get("human_display_id") or str(item.get("id", "-")),
            "subject": item.get("subject") or item.get("title") or "-",
            "status": item.get("status_name") or str(item.get("status", "-")),
        }
        for k in ("tickets", "problems", "releases") for item in assoc[k]
    ]
    for warning in assoc["warnings"]:
        err.print(warning, markup=False)
    if _emit_fmt(assoc, flat_rows, format_, json_out):
        return
    any_shown = False
    for kind in ("tickets", "problems", "releases"):
        items = assoc[kind]
        if not items:
            continue
        any_shown = True
        t = Table(title=kind.capitalize())
        t.add_column("ID", style="cyan")
        t.add_column("Subject")
        t.add_column("Status")
        for item in items:
            hid = item.get("human_display_id") or str(item.get("id", "-"))
            subj = item.get("subject") or item.get("title") or "-"
            st = item.get("status_name") or str(item.get("status", "-"))
            t.add_row(hid, subj, st)
        console.print(t)
    if not any_shown:
        console.print("no associations" if assoc["complete"] else "association inventory unavailable/incomplete; not an empty list")


def _network_completion_enabled() -> bool:
    try:
        return bool(completion._completion_network())
    except Exception:
        return False


def _ctx_change_id(ctx: typer.Context) -> int | None:
    raw = ctx.params.get("id_") or ctx.params.get("id")
    if not raw:
        return None
    try:
        return parse_id(str(raw), CHANGES)
    except ValueError:
        return None


def _complete_task_id(ctx: typer.Context, incomplete: str) -> Iterable[tuple[str, str]]:
    cid = _ctx_change_id(ctx)
    if cid is None:
        yield (incomplete, "")
        return
    try:
        tasks = _client().int_get(f"changes/{cid}/tasks", {"page": 1, "per_page": 100}).get("tasks") or []
        for task in tasks:
            tid = str(task.get("id") or "")
            hid = str(task.get("human_display_id") or "")
            title = str(task.get("title") or "")
            if (tid and tid.startswith(incomplete)) or (hid and hid.casefold().startswith(incomplete.casefold())):
                yield (tid, f"{hid} · {title}".strip(" ·"))
    except Exception:
        pass
    yield (incomplete, "")


def _asset_candidates(ctx: typer.Context, incomplete: str) -> list[dict[str, Any]]:
    from fsv.create import search_assets_for_change
    cid = _ctx_change_id(ctx)
    if cid is None or len(incomplete.strip()) < 2:
        return []
    try:
        return list((search_assets_for_change(cid, incomplete, per_page=10).get("assets") or []))
    except Exception:
        return []


def _complete_asset_category(incomplete: str) -> Iterable[tuple[str, str]]:
    try:
        categories = _fetch_asset_categories()
    except Exception:
        categories = [{"name": "All assets", "ci_type_id": ""}]
    emitted: set[str] = set()
    probe = incomplete.casefold()
    for item in categories:
        name = str(item.get("name") or "")
        if not name or name in emitted:
            continue
        if not probe or name.casefold().startswith(probe):
            emitted.add(name)
            detail = f"ci_type_id={item.get('ci_type_id')}" if item.get("ci_type_id") else "all"
            yield (name, detail)
    if "All assets" not in emitted and "all assets".startswith(probe):
        yield ("All assets", "all")
    if "all".startswith(probe):
        yield ("all", "All assets")
    yield (incomplete, "")


def _complete_asset_search_for_change(ctx: typer.Context, incomplete: str) -> Iterable[tuple[str, str]]:
    for asset in _asset_candidates(ctx, incomplete):
        did = str(asset.get("display_id") or "")
        name = str(asset.get("name") or "")
        atype = str(asset.get("ci_type_name") or "")
        if name:
            yield (name, f"id={did} · {atype}".strip(" ·"))
    yield (incomplete, "")


def _complete_asset_id_for_change(ctx: typer.Context, incomplete: str) -> Iterable[tuple[str, str]]:
    for asset in _asset_candidates(ctx, incomplete):
        did = str(asset.get("display_id") or "")
        name = str(asset.get("name") or "")
        atype = str(asset.get("ci_type_name") or "")
        if did:
            yield (did, f"{name} · {atype}".strip(" ·"))
    yield (incomplete, "")


def _complete_associated_asset_for_change(ctx: typer.Context, incomplete: str) -> Iterable[tuple[str, str]]:
    from fsv.create import get_change_assets
    cid = _ctx_change_id(ctx)
    if cid is None:
        return
    try:
        assets = get_change_assets(cid)
        for a in assets:
            ci = a.get("config_item", {})
            did = str(ci.get("display_id") or "")
            name = str(ci.get("name") or "")
            if did and did.startswith(incomplete):
                yield (did, name)
    except Exception:
        pass


def _complete_task_status(incomplete: str) -> Iterable[tuple[str, str]]:
    from fsv.task_status import active_choices
    try:
        for ident, label in active_choices(_task_field_defs()):
            if label.casefold().startswith(incomplete.casefold()):
                yield (label, f"id={ident}")
    except ValueError:
        return


def _task_status_value(value: str) -> int:
    from fsv.task_status import resolve
    return resolve(value, _task_field_defs())[0]


def _task_field_defs() -> list[dict[str, Any]]:
    try:
        data = _client().int_get("change_task_fields", {"include_deleted_choices": "true"})
    except Exception:
        return []
    for key in ("change_task_fields", "task_fields", "fields"):
        values = data.get(key)
        if isinstance(values, list):
            return values
    return []


def _task_field_name_matches(name: str, field: str) -> bool:
    return name == field or name == f"cf_{field}"


def _task_field_choice_pairs(field: str) -> list[tuple[str, str]]:
    out: list[tuple[str, str]] = []
    emitted: set[str] = set()
    for item in _task_field_defs():
        if not _task_field_name_matches(str(item.get("name") or ""), field):
            continue
        for choice in item.get("choices") or []:
            cid = str(choice.get("id") or choice.get("display_id") or "")
            label = str(choice.get("value") or choice.get("label") or choice.get("name") or cid)
            for value, detail in ((label, f"id={cid}" if cid else ""), (cid, label)):
                if value and value not in emitted:
                    emitted.add(value)
                    out.append((value, detail))
        break
    return out


def _task_observed_value_pairs(ctx: typer.Context, field: str) -> list[tuple[str, str]]:
    cid = _ctx_change_id(ctx)
    if cid is None:
        return []
    try:
        tasks = _client().int_get(f"changes/{cid}/tasks", {"page": 1, "per_page": 100}).get("tasks") or []
    except Exception:
        return []
    out: list[tuple[str, str]] = []
    emitted: set[str] = set()
    for task in tasks:
        value = (task.get("custom_fields") or {}).get(field)
        if value in (None, ""):
            continue
        text = str(value)
        if text not in emitted:
            emitted.add(text)
            out.append((text, "observed on current change"))
    return out


def _complete_task_custom_field(field: str):
    def complete(ctx: typer.Context, incomplete: str) -> Iterable[tuple[str, str]]:
        emitted: set[str] = set()
        for pairs in (_task_field_choice_pairs(field), _task_observed_value_pairs(ctx, field)):
            for value, detail in pairs:
                if value not in emitted and value.casefold().startswith(incomplete.casefold()):
                    emitted.add(value)
                    yield (value, detail)
        yield (incomplete, "")

    return complete


def _task_choice_value(value: str, field: str) -> int:
    raw = value.strip()
    if raw.isdigit():
        return int(raw)
    mapping: dict[str, int] = {}
    for item in _task_field_defs():
        if not _task_field_name_matches(str(item.get("name") or ""), field):
            continue
        for choice in item.get("choices") or []:
            cid = choice.get("id") or choice.get("display_id")
            label = str(choice.get("value") or choice.get("label") or choice.get("name") or "")
            if cid is not None and label:
                mapping[label.casefold()] = int(cid)
        break
    key = raw.casefold()
    if key not in mapping:
        if mapping:
            choices = ", ".join(sorted(mapping))
            raise ValueError(f"{field} must be numeric ID or one of: {choices}")
        raise ValueError(f"{field} must be numeric ID")
    return mapping[key]


_complete_task_system = _complete_task_custom_field("system")
_complete_task_environment = _complete_task_custom_field("environment")


def _complete_ticket_for_change(incomplete: str) -> Iterable[tuple[str, str]]:
    from fsv.create import search_change_tickets
    if len(incomplete.strip()) < 2:
        yield (incomplete, "")
        return
    try:
        for ticket in search_change_tickets(incomplete):
            hid = str(ticket.get("human_display_id") or format_id(ticket, TICKETS))
            subject = str(ticket.get("subject") or "")
            yield (hid, subject)
    except Exception:
        pass
    yield (incomplete, "")


def _resolve_change_ticket_id(value: str, c: Client | None = None) -> int:
    from fsv.client import get_client
    from fsv.create import search_change_tickets
    raw = str(value).strip()
    try:
        return parse_id(raw, TICKETS)
    except ValueError:
        pass
    if c is None:
        c = get_client()
    tickets = search_change_tickets(raw, c)
    if not tickets:
        _err(f"no ticket match: {value}")
    folded = raw.casefold()
    exact = [
        ticket for ticket in tickets
        if str(ticket.get("human_display_id") or format_id(ticket, TICKETS)).casefold() == folded
        or str(ticket.get("subject") or "").casefold() == folded
    ]
    if len(exact) == 1:
        return int(exact[0]["id"])
    if len(exact) > 1:
        shown = ", ".join(str(ticket.get("human_display_id") or format_id(ticket, TICKETS)) for ticket in exact[:5])
        _err(f"multiple ticket matches for {value!r}: {shown}")
    if len(tickets) == 1:
        return int(tickets[0]["id"])
    shown = ", ".join(str(ticket.get("human_display_id") or format_id(ticket, TICKETS)) for ticket in tickets[:5])
    _err(f"multiple ticket matches for {value!r}: {shown}")


def _resolve_associated_change_ticket_id(change_id: int, value: str, c: Client | None = None) -> int:
    from fsv.client import get_client
    from fsv.create import get_change_associations
    raw = str(value).strip()
    try:
        target_id = parse_id(raw, TICKETS)
    except ValueError:
        target_id = None
    if c is None:
        c = get_client()
    inventory = get_change_associations(change_id, c)
    if not inventory["meta"]["tickets"].get("complete"):
        _err("initiating-ticket inventory unavailable/incomplete; cannot resolve removal")
    tickets = inventory["tickets"]
    if target_id is not None:
        for ticket in tickets:
            if int(ticket.get("id") or 0) == target_id:
                return target_id
    folded = raw.casefold()
    exact = [
        ticket for ticket in tickets
        if str(ticket.get("human_display_id") or format_id(ticket, TICKETS)).casefold() == folded
        or str(ticket.get("subject") or "").casefold() == folded
    ]
    if len(exact) == 1:
        return int(exact[0]["id"])
    if len(exact) > 1:
        shown = ", ".join(str(ticket.get("human_display_id") or format_id(ticket, TICKETS)) for ticket in exact[:5])
        _err(f"multiple associated ticket matches for {value!r}: {shown}")
    matches = [ticket for ticket in tickets if folded in str(ticket.get("subject") or "").casefold()]
    if len(matches) == 1:
        return int(matches[0]["id"])
    if not matches:
        _err(f"ticket not associated on change #{change_id}: {value!r}")
    shown = ", ".join(str(ticket.get("human_display_id") or format_id(ticket, TICKETS)) for ticket in matches[:5])
    _err(f"multiple associated ticket matches for {value!r}: {shown}")


def _ticket_id(value: str) -> int:
    return parse_id(str(value), TICKETS)


def _print_tickets(items: list[dict[str, Any]], title: str = "Tickets") -> None:
    t = Table(title=title)
    t.add_column("ID", style="cyan")
    t.add_column("Subject")
    t.add_column("Status")
    for item in items:
        hid = item.get("human_display_id") or format_id(item, TICKETS)
        t.add_row(hid, item.get("subject") or "-", item.get("status_name") or str(item.get("status", "-")))
    console.print(t)


def _read_warnings(td: Any) -> None:
    if td.missing:
        err.print(escape(td.missing.note))
        if td.data is None and not td.parts:
            raise typer.Exit(1)
    for note in td.notes:
        err.print(escape(note))


def _history_rows(c: Any, res: Resource, cid: int, page: int, per_page: int,
                  all_pages: bool, n_pages: int | None) -> list[dict[str, Any]]:
    from fsv import views
    key = "conversations" if res == TICKETS else "notes"
    rows, meta, miss = views.collect_pages(
        c, f"{res.api_path}/{cid}/{key}", key,
        {"page": page, "per_page": per_page, "include": "user,phone,feedback" if res == TICKETS else "user"},
        max_pages=min(n_pages or (50 if all_pages else 1), 50), what=key,
    )
    if miss and not meta["pages"]:
        err.print(escape(miss.note))
        raise typer.Exit(1)
    if meta.get("partial"):
        err.print(escape(meta["partial"]))
    return rows


def notes_resource(res: Resource, id_: str, page: int, per_page: int, json_out: bool, all_pages: bool = False, n_pages: int | None = None, format_: OutputFormat | str = "table") -> None:
    cid = _cid(id_, res)
    c = _client()
    items = _api(lambda: _history_rows(c, res, cid, page, per_page, all_pages, n_pages))
    flat = [{"id": str(x.get("id", "-")), "created_at": (x.get("created_at") or "-")[:19], "author": ((x.get("user") or {}).get("name") or str(x.get("user_id", "-"))), "visibility": "private" if x.get("private") else "public", "body": x.get("body_text") or strip_html(x.get("body")) or ""} for x in items]
    if _emit_fmt(items, flat, format_, json_out):
        return
    t = Table()
    t.add_column("ID", style="cyan")
    t.add_column("Created", style="dim")
    t.add_column("Author")
    t.add_column("Visibility")
    t.add_column("Body")
    for x in items:
        user = x.get("user") or {}
        body = x.get("body_text") or strip_html(x.get("body"))
        t.add_row(
            str(x.get("id", "-")),
            (x.get("created_at") or "-")[:19],
            user.get("name") or str(x.get("user_id", "-")),
            "private" if x.get("private") else "public",
            body or "",
        )
    console.print(t)


def conversations_resource(id_: str, page: int, per_page: int, json_out: bool, all_pages: bool = False, n_pages: int | None = None, format_: OutputFormat | str = "table") -> None:
    cid = _cid(id_, TICKETS)
    c = _client()
    items = _api(lambda: _history_rows(c, TICKETS, cid, page, per_page, all_pages, n_pages))
    flat = [{"id": str(x.get("id", "-")), "created_at": (x.get("created_at") or "-")[:19], "author": ((x.get("user") or {}).get("name") or str(x.get("user_id", "-"))), "kind": "private note" if x.get("private") else ("incoming" if x.get("incoming") else "public reply"), "body": x.get("body_text") or strip_html(x.get("body")) or ""} for x in items]
    if _emit_fmt(items, flat, format_, json_out):
        return
    t = Table()
    t.add_column("ID", style="cyan")
    t.add_column("Created", style="dim")
    t.add_column("Author")
    t.add_column("Kind")
    t.add_column("Body")
    for x in items:
        user = x.get("user") or {}
        body = x.get("body_text") or strip_html(x.get("body"))
        kind = "private note" if x.get("private") else ("incoming" if x.get("incoming") else "public reply")
        t.add_row(
            str(x.get("id", "-")),
            (x.get("created_at") or "-")[:19],
            user.get("name") or str(x.get("user_id", "-")),
            kind,
            body or "",
        )
    console.print(t)


def ticket_approvals_resource(id_: str, format_: OutputFormat | str = "table", json_out: bool = False) -> None:
    from fsv import service
    cid = _cid(id_, TICKETS)
    c = _client()
    try:
        items = service.get_ticket_approvals(TICKETS, cid, client=c)
    except APIError as e:
        if e.status == 404:
            prefix = id_.split("-", 1)[0].upper() if "-" in id_ else "ticket"
            hint = "ticket approvals not available for this ticket"
            if prefix == "INC":
                hint += "; INC tickets often do not expose approvals, try SR or another approval-capable ticket"
            _err(hint)
        _err_exc(e)
    def _flat(a: dict) -> dict:
        remark = (a.get("remark") or [{}])[0]
        decided = remark.get("updated_at") or a.get("updated_at") or "-"
        return {
            "level": str(a.get("level_id", "-")),
            "approver": (a.get("member") or {}).get("name") or str(a.get("member_id", "-")),
            "status": (a.get("status") or {}).get("name") or "-",
            "decided": decided[:19] if decided != "-" else "-",
            "type": (a.get("type") or {}).get("name") or "-",
            "comment": remark.get("data") or "",
        }
    if _emit_fmt(items, [_flat(a) for a in items], format_, json_out):
        return
    t = Table()
    t.add_column("Level", style="cyan")
    t.add_column("Approver")
    t.add_column("Status")
    t.add_column("Decided")
    t.add_column("Type")
    t.add_column("Comment")
    for a in items:
        member = (a.get("member") or {}).get("name") or str(a.get("member_id", "-"))
        raw_status = (a.get("status") or {}).get("name") or "-"
        decided = ((a.get("remark") or [{}])[0]).get("updated_at") or a.get("updated_at") or "-"
        comment = ((a.get("remark") or [{}])[0]).get("data") or ""
        atype = (a.get("type") or {}).get("name") or "-"
        t.add_row(str(a.get("level_id", "-")), member, raw_status, decided[:19] if decided != "-" else "-", atype, comment)
    console.print(t)


def ticket_associations_resource(id_: str, format_: OutputFormat | str = "table", json_out: bool = False) -> None:
    from fsv import schema as schema_mod
    cid = _cid(id_, TICKETS)
    c = _client()
    tabs = _api(lambda: c.int_get(f"tickets/{cid}/tabs"))
    associated_modules = set()
    for tab in tabs if isinstance(tabs, list) else []:
        if tab.get("name") == "associations":
            associated_modules.update(tab.get("associated_modules") or [])
    from fsv.views import collect_pages

    def associated(key: str, params: dict[str, Any] | None = None) -> list[dict[str, Any]]:
        rows, meta, miss = _api(lambda: collect_pages(c, f"tickets/{cid}/{key}", key, params, what=key))
        if meta.get("partial"):
            err.print(escape(meta["partial"]))
        if miss and not meta["pages"]:
            raise typer.Exit(1)
        return rows

    changes = associated("changes", {"change_type": "change"}) if (not associated_modules or "change" in associated_modules) else []
    change_causes = associated("changes", {"change_type": "change_cause"}) if (not associated_modules or "change_cause" in associated_modules) else []
    problems = associated("problems") if (not associated_modules or "problem" in associated_modules) else []
    payload = {"changes": changes, "change_causes": change_causes, "problems": problems}
    def _flat(kind: str, item: dict) -> dict:
        res_ref = PROBLEMS if kind == "problems" else CHANGES
        return {
            "type": kind,
            "id": str(item.get("human_display_id") or format_id(item, res_ref)),
            "subject": item.get("subject") or "-",
            "status": item.get("status_name") or str(item.get("status", "-")),
            "priority": item.get("priority_name") or schema_mod.PRIORITY.get(item.get("priority") or 0, "-"),
        }
    flat_rows = [_flat(k, x) for k, items in payload.items() for x in items]
    if _emit_fmt(payload, flat_rows, format_, json_out):
        return
    if not any(payload.values()):
        console.print("no associations")
        return
    if changes:
        t = Table(title="Changes")
        t.add_column("ID", style="cyan")
        t.add_column("Subject")
        t.add_column("Status")
        t.add_column("Priority")
        for item in changes:
            t.add_row(str(item.get("human_display_id") or format_id(item, CHANGES)), item.get("subject") or "-", item.get("status_name") or str(item.get("status", "-")), item.get("priority_name") or schema_mod.PRIORITY.get(item.get("priority") or 0, "-"))
        console.print(t)
    if change_causes:
        t = Table(title="Caused by changes")
        t.add_column("ID", style="cyan")
        t.add_column("Subject")
        t.add_column("Status")
        t.add_column("Priority")
        for item in change_causes:
            t.add_row(str(item.get("human_display_id") or format_id(item, CHANGES)), item.get("subject") or "-", item.get("status_name") or str(item.get("status", "-")), item.get("priority_name") or schema_mod.PRIORITY.get(item.get("priority") or 0, "-"))
        console.print(t)
    if problems:
        t = Table(title="Problems")
        t.add_column("ID", style="cyan")
        t.add_column("Subject")
        t.add_column("Status")
        for item in problems:
            t.add_row(str(item.get("human_display_id") or format_id(item, PROBLEMS)), item.get("subject") or "-", item.get("status_name") or str(item.get("status", "-")))
        console.print(t)


def _load_views(c: Any, res: Resource) -> list[dict[str, Any]]:
    data = c.int_get(res.filters_path or "")
    key = next((k for k in data if k.endswith("_filters")), None)
    return data.get(key, []) if key else []


def _resolve_view_name(c: Any, res: Resource, view: str | None) -> str | None:
    if not view:
        return view
    views = _load_views(c, res)
    wanted = view.strip().casefold()
    for item in views:
        view_id = str(item.get("id") or "")
        if view_id.casefold() == wanted:
            return view_id
    for item in views:
        name = str(item.get("name") or "")
        if name.casefold() == wanted:
            return str(item.get("id") or view)
    return view


def views_resource(res: Resource, format_: OutputFormat | str = "table", json_out: bool = False) -> None:
    """The web's saved views by name; ``▸`` marks the one `ls` opens with."""
    from fsv import views as V
    c = _client()
    saved = _api(lambda: _saved_views(c, res))
    default = V.default_view(saved)
    rows = [{**v.to_json(), "opens_by_default": v == default} for v in saved]
    flat = [{"name": r["name"], "id": r["id"], "type": "system" if r["system"] else "custom",
             "default": "yes" if r["opens_by_default"] else ""} for r in rows]
    if _emit_fmt(rows, flat, format_, json_out):
        return
    if not saved:
        err.print(f"no saved views for {res.name} (the account does not list them)")
        return
    t = Table()
    t.add_column("", width=1)
    t.add_column("View")
    t.add_column("Type", style="dim")
    t.add_column("id", style="dim")
    for v in saved:
        t.add_row("▸" if v == default else "", escape(v.name), "system" if v.system else "custom", escape(v.id))
    console.print(t)
    err.print(f"fsv {res.name} ls --view NAME  (exact name or a unique prefix)")


def _fulltext_search(res: Resource, query: str, page: int, json_out: bool, sort: SearchSort, all_pages: bool = False, n_pages: int | None = None, format_: OutputFormat | str = "table") -> None:
    from fsv import service
    c = _client()
    if n_pages is not None or all_pages:
        all_results: list[dict] = []
        total = 0
        p = page
        limit = n_pages if n_pages is not None else None
        fetched = 0
        while True:
            batch, totals = _api(lambda _p=p: service.search_items(query, entity=res.name, sort=sort.value, page=_p, client=c))
            total = totals.get(res.name, len(all_results) + len(batch))
            all_results.extend(batch)
            fetched += 1
            err.print(f"  fetched {len(all_results)} / {total}...", highlight=False)
            if len(batch) < 30 or (limit is not None and fetched >= limit):
                break
            p += 1
        results = all_results
    else:
        results, totals = _api(lambda: service.search_items(query, entity=res.name, sort=sort.value, page=page, client=c))
        total = totals.get(res.name, len(results))
    clean = [{k: v for k, v in r.items() if not k.startswith("_")} for r in results]
    flat = [{"id": r.get("human_display_id") or str(r.get("display_id") or "-"), "subject": (r.get("subject") or "")[:80], "status": r.get("status") or "-", "priority": r.get("priority_label") or "-", "group": r.get("_group") or "-"} for r in results]
    if _emit_fmt(clean, flat, format_, json_out):
        return
    t = Table()
    t.add_column("ID", style="cyan", no_wrap=True)
    t.add_column("Subject")
    t.add_column("Status", style="green")
    t.add_column("Pri")
    t.add_column("Group", style="dim")
    for r in results:
        did = r.get("human_display_id") or str(r.get("display_id") or "-")
        t.add_row(did, (r.get("subject") or "")[:80], r.get("status") or "-", r.get("priority_label") or "-", r.get("_group") or "-")
    console.print(t)
    err.print(f"{len(results)} rows (total: {total})")


def search_resource(res: Resource, query: str, page: int, json_out: bool, sort: SearchSort, all_pages: bool = False, n_pages: int | None = None, format_: OutputFormat | str = "table") -> None:
    _fulltext_search(res, query, page, json_out, sort, all_pages, n_pages, format_)


_SEARCH_TYPE_LABEL = {
    "helpdesk_ticket": "Ticket",
    "itil_problem": "Problem",
    "itil_change": "Change",
    "itil_task": "Task",
    "cmdb_config_item": "Asset",
    "solution_article": "Solution",
}


def _search_clean(s: str | None) -> str:
    from html import unescape
    import html as _html
    return _html.unescape(re.sub(r"<[^>]+>", "", s or "")).strip()


def _search_extract(r: dict[str, Any]) -> tuple[str, str, str, str, str]:
    rt = r.get("result_type") or ""
    label = _SEARCH_TYPE_LABEL.get(rt, rt or "-")
    did = (r.get("ticket_display_id") or r.get("itil_module_display_id")
           or r.get("task_display_id") or r.get("ci_display_id")
           or r.get("display_id") or str(r.get("id", "-")))
    title = _search_clean(r.get("subject") or r.get("name") or r.get("title") or "")
    status = (r.get("ticket_status") or r.get("itil_module_status")
              or r.get("task_status") or r.get("ci_type") or "-")
    grp = (r.get("ticket_group") or r.get("itil_module_group")
           or r.get("task_group") or r.get("owner_name") or r.get("folder_name") or "-")
    return label, str(did), title, str(status), str(grp)


def global_search(query: str, page: int, format_: OutputFormat | str, json_out: bool, sort: SearchSort, all_pages: bool = False, n_pages: int | None = None) -> None:
    c = _client()
    if n_pages is not None or all_pages:
        all_items: list[dict] = []
        p = page
        limit = n_pages if n_pages is not None else None
        fetched = 0
        while True:
            data = _api(lambda _p=p: c.fulltext_search("all", query, page=_p, sort=sort.value))
            batch = [r for r in data.get("results", []) if r.get("result_type")]
            all_items.extend(batch)
            fetched += 1
            err.print(f"  fetched {len(all_items)}...", highlight=False)
            if len(batch) < 30 or (limit is not None and fetched >= limit):
                break
            p += 1
        items = all_items
    else:
        data = _api(lambda: c.fulltext_search("all", query, page=page, sort=sort.value))
        items = [r for r in data.get("results", []) if r.get("result_type")]
    fmt = format_.value if isinstance(format_, OutputFormat) else format_
    if json_out:
        fmt = "json"
    if fmt == "json":
        emit_json(items)
        return
    rows = [
        {"type": label, "id": did, "title": title, "status": status, "group": grp}
        for label, did, title, status, grp in (_search_extract(r) for r in items)
    ]
    if fmt in ("csv", "tsv"):
        import csv as _csv
        delim = "," if fmt == "csv" else "\t"
        writer = _csv.DictWriter(sys.stdout, fieldnames=list(rows[0]) if rows else [], delimiter=delim)
        writer.writeheader()
        writer.writerows(rows)
        return
    t = Table()
    t.add_column("Type", style="magenta", no_wrap=True)
    t.add_column("ID", style="cyan", no_wrap=True)
    t.add_column("Title")
    t.add_column("Status", style="green")
    t.add_column("Group", style="dim")
    for r in rows:
        t.add_row(r["type"], r["id"], r["title"][:70], r["status"], r["group"])
    console.print(t)
    err.print(f"{len(items)} rows")


def url_resource(res: Resource, id_: str) -> None:
    cid = _cid(id_, res)
    console.print(f"https://{config.require_domain()}/a/{res.api_path}/{cid}")


def _problem_planning_update(id_: str, planned_start: Optional[str], planned_end: Optional[str],
                             planned_effort: Optional[str], dry_run: bool, yes: bool, expect_plan: Optional[str],
                             *others: Any) -> None:
    """Problem planned dates/effort: one reviewed PUT of only these keys."""
    import json
    from fsv import problem_properties
    from fsv.reviewed_write import ReviewedWriteError
    if any(o for o in others):
        _err("problem planned dates/effort are reviewed on their own; send other fields separately")
    try:
        c = _client()
        plan = problem_properties.build_plan(c, id_, planned_start=planned_start, planned_end=planned_end,
                                             planned_effort=planned_effort)
        if dry_run:
            typer.echo(json.dumps({"status": "dry_run", **plan.review()}, ensure_ascii=False, indent=2))
            return
        if not yes or expect_plan != plan.hash:
            raise ReviewedWriteError("review with --dry-run, then --yes --expect-plan FULL_HASH")
        result = problem_properties.send(plan, c, reviewed_hash=expect_plan)
        typer.echo(json.dumps(result, ensure_ascii=False, indent=2))
        if not result.get("verified"):
            raise typer.Exit(4)
    except (ValueError, ReviewedWriteError, APIError) as exc:
        typer.echo(str(exc), err=True)
        raise typer.Exit(2) from None


def _problem_update_hash(res: Resource, cid: int, body: dict[str, Any], row: Any) -> str | None:
    """Review hash of a problem update: tenant, target, exact PUT body and the
    stable snapshot of the problem it was reviewed against (None when the
    problem could not be read)."""
    from fsv import journal
    if not isinstance(row, dict) or not config.DOMAIN:
        return None
    return journal.plan_hash({"action": "update", "resource": res.name, "target": cid, "tenant": config.DOMAIN,
                              "method": "PUT", "path": f"{res.api_path}/{cid}", "payload": body,
                              "record": journal.snapshot_hash(row)})


def update_resource(
    res: Resource,
    id_: str,
    status: Optional[str],
    priority: Optional[str],
    agent_id: Optional[str],
    group_id: Optional[str],
    json_out: bool,
    dry_run: bool,
    yes: bool,
    no_input: bool = False,
    set_: Optional[List[str]] = None,
    expect_plan: Optional[str] = None,
) -> None:
    from fsv import schema as schema_mod
    from fsv import service
    cid = _cid(id_, res)
    row: Any = None
    c = _client()
    sch = _api(lambda: schema_mod.load(res, c))
    stale_warning = schema_mod.freshness_warning(res, c, sch) if dry_run else None
    if stale_warning:
        err.print(f"[yellow]warning[/yellow]: {stale_warning}")
    if res.name == "problems":
        # PUT on a deleted (trashed) problem answers 405; refuse before sending
        try:
            row = c.int_get(f"{res.api_path}/{cid}")
        except Exception:  # advisory: the server still refuses (405) a deleted record
            row = None
        row = row.get(res.item_key, row) if isinstance(row, dict) else None
        if isinstance(row, dict) and row.get("deleted") is True:
            _err(f"{format_id({'id': cid}, res)} is deleted (in trash); restore it before updating")
    body: dict[str, Any] = {}
    if status is not None:
        body["status"] = _resolve_update_choice(sch, "status", status)
    if priority is not None:
        body["priority"] = _resolve_update_choice(sch, "priority", priority)
    if agent_id is not None:
        body["responder_id" if res.name == "tickets" else "agent_id"] = int(_resolve_agent(c, agent_id))
    if group_id is not None:
        body["group_id"] = int(_resolve_group(c, group_id))
    for expr in set_ or []:
        if "=" not in expr:
            _err(f"--set requires FIELD=VALUE format, got: {expr!r}")
        field_text, _, raw_value = expr.partition("=")
        field = _find_schema_field(sch, field_text.strip())
        k, v = _resolve_set_field(c, sch, field_text.strip(), raw_value)
        # The form metadata distinguishes custom fields from default fields;
        # tickets and problems nest custom fields in custom_fields (a
        # top-level custom key is rejected with 400). Only changed keys are
        # sent: the problem web form sends null for unchanged ones.
        ftype = str((field or {}).get("field_type") or "")
        is_default = bool((field or {}).get("default_field", ftype.startswith("default_")))
        if res.name in ("tickets", "problems") and not is_default:
            body.setdefault("custom_fields", {})[k] = v
        else:
            body[k] = v
    if not body:
        _err("nothing to update; pass at least one field (--status, --priority, --agent, --group, --set FIELD=VALUE)")
    current = _current_flat(res, cid, c) if dry_run and res.name == "tickets" else None
    risks = _risks_for_item_write(res, cid, body, sch, c, current=current)
    plan_hash = _problem_update_hash(res, cid, body, row) if res.name == "problems" else None
    if expect_plan is not None and (plan_hash is None or expect_plan != plan_hash):
        _err(f"{format_id({'id': cid}, res)} or the update changed since review (or the problem could not be "
             "read); review again with --dry-run. Nothing was sent")
    if dry_run:
        display_id = None
        if res.name == "tickets":
            # Prefer the server's explicit identity; an untyped numeric ID is
            # not evidence that a ticket is an incident or a service request.
            human = (current or {}).get("human_display_id")
            if human and re.fullmatch(r"(?:INC|SR)-[0-9]+", str(human), re.IGNORECASE):
                display_id = str(human)
            elif re.fullmatch(r"(?:INC|SR)-[0-9]+", id_.strip(), re.IGNORECASE):
                display_id = id_.strip().upper()
            else:
                display_id = f"tickets/{cid}"
        _emit_dry_run(res, cid, body, risks=risks, display_id=display_id,
                      warnings=[stale_warning] if stale_warning else None, plan_hash=plan_hash)
        return
    _confirm_writes(risks, format_id({"id": cid}, res), yes, no_input)

    def _update() -> dict[str, Any]:
        try:
            return service.update_item(res, cid, body, client=c)
        except APIError as exc:
            if exc.status == 405:
                raise APIError(405, f"{format_id({'id': cid}, res)} is not editable (deleted or in trash?); "
                                    "nothing was changed", "PUT", f"{res.api_path}/{cid}") from None
            raise
    item = _api(_update)
    if json_out:
        emit_json(item)
        return
    console.print(f"[green]updated[/] {format_id(item, res)}")


DOWNLOADS_DIR = Path("downloads")


def _download_dir(out: Path | None, label: str) -> Path:
    """Attachment downloads go exactly under --dir, or by default under
    ./downloads/<ID>: never a bare ./<ID> directory in the current
    directory (which may be a repository checkout). The chosen directory is
    announced on stderr so JSON stdout stays a manifest."""
    from rich.markup import escape
    target = out if out is not None else DOWNLOADS_DIR / label
    origin = "--dir" if out is not None else "default ./downloads/<ID>; choose with --dir"
    err.print(f"output directory: {escape(str(target))} ({origin})")
    return target


def note_resource(res: Resource, id_: str, body: str, public: bool, dry_run: bool = False,
                  yes: bool = False, no_input: bool = False) -> None:
    from fsv import service
    from fsv.safety import note_risks
    from fsv.errors import UsageError
    cid = _cid(id_, res)
    try:
        payload = service.note_payload(res, body, public)
    except UsageError as exc:
        _err(str(exc))
        return
    risks = note_risks(public)
    problem = res.name == "problems"
    if dry_run:
        _emit_dry_run(res, cid, payload, "add_note", risks=risks,
                      warnings=["visibility: " + service.PROBLEM_NOTE_VISIBILITY] if problem else None)
        return
    _confirm_writes(risks, f"note on {format_id({'id': cid}, res)}", yes, no_input)
    c = _client()
    n = _api(lambda: service.add_note(res, cid, body, public=public, client=c))
    visibility = "" if problem else ("public " if public else "private ")
    suffix = " (visibility: tenant default; not controllable on problem notes)" if problem else ""
    console.print(f"[green]added[/] {visibility}note {n.get('id')}{suffix}")


def reply_resource(res: Resource, id_: str, body: str, dry_run: bool = False,
                   yes: bool = False, no_input: bool = False) -> None:
    from fsv import service
    from fsv.safety import reply_risks
    cid = _cid(id_, res)
    risks = reply_risks()
    if dry_run:
        _emit_dry_run(res, cid, {"body": body}, "reply", risks=risks)
        return
    _confirm_writes(risks, f"reply on {format_id({'id': cid}, res)}", yes, no_input)
    c = _client()
    reply = _api(lambda: service.add_reply(res, cid, body, client=c))
    console.print(f"[green]added[/] reply {reply.get('id', '')}")


auth_app = typer.Typer(
    no_args_is_help=True,
    help="authenticate and manage sessions",
    rich_markup_mode=None if _COMPLETING else "rich",
    epilog="[bold]Examples:[/bold]  fsv auth login  |  fsv auth login --domain acme.freshservice.com  |  fsv auth status",
)


def _resolve_domain(no_input: bool = False) -> bool:
    """Prompt for domain. Returns True if input looked like a URL (always confirm)."""
    default = config.DOMAIN or "acme.freshservice.com"
    if config.DOMAIN and not sys.stdin.isatty():
        err.print(f"Domain: [bold]{config.DOMAIN}[/]")
        return False
    if not config.DOMAIN and _no_input(no_input):
        _err("no Freshservice domain; pass `fsv auth login --domain ...` when using --no-input")
    answer = typer.prompt("Domain", default=default, show_default=True).strip()
    looks_like_url = "://" in (answer or default)
    try:
        config.set_domain(config.normalize_domain(answer or default))
    except ValueError as e:
        _err_exc(e)
    return looks_like_url


def _validate_new_cookies(cookies: dict[str, str]) -> None:
    """Refuse to save cookies the tenant rejects (override with --force)."""
    from fsv.client import Client

    try:
        with Client(cookies=cookies) as probe:
            probe.check_session()
    except SessionError:
        _err("these cookies are expired or invalid for "
             f"{config.DOMAIN}; nothing saved (use --force to save anyway)", 3)
    except Exception as e:
        _err(f"could not verify cookies against {config.DOMAIN}: {e}; nothing saved (use --force to save anyway)", 1)


@auth_app.command("login", epilog="[bold]Examples:[/bold]  fsv auth login --browser  |  pbpaste | fsv auth login --header - --store keychain")
def auth_login(
    domain: Optional[str] = typer.Option(None, "--domain", "-d", help="Freshservice domain or URL, e.g. acme.freshservice.com. Saved before login."),
    header: Optional[str] = typer.Option(None, "--header", "-H", help="Cookie header string. Use '-' to read from stdin."),
    browser: bool = typer.Option(False, "--browser", help="Open an isolated browser, let you finish SSO/MFA, then read HttpOnly cookies via CDP."),
    store: Optional[str] = typer.Option(None, "--store", help="Where to save: 'file' (plain chmod 600), 'argon' (Argon2id + AES-GCM), or 'keychain' (macOS Keychain). Interactive when omitted; default keychain on macOS.", autocompletion=completion.complete_store),
    no_input: bool = typer.Option(False, "--no-input", help="fail instead of prompting"),
    force: bool = typer.Option(False, "--force", help="save cookies even if they fail validation against the tenant"),
) -> None:
    """Save Freshservice session cookies."""
    from fsv.client import get_client, reset_client
    from fsv.session import login_browser, login_interactive, parse_cookie_header, save_cookies, validate
    no_input = _no_input(no_input)
    if store is not None and store not in ("file", "argon", "keychain"):
        _err("--store must be 'file', 'argon', or 'keychain'")
    if browser and header is not None:
        _err("use either --browser or --header, not both")
    if browser and no_input:
        _err("--browser is interactive; do not use --no-input")
    if header is None and not browser and no_input:
        _err("pass --header when using --no-input")
    explicit_domain = domain is not None
    url_pasted = False
    if domain is not None:
        try:
            config.set_domain(domain)
        except ValueError as e:
            _err_exc(e)
    elif not config.DOMAIN:
        url_pasted = _resolve_domain(no_input)
    else:
        if not no_input and sys.stdin.isatty():
            url_pasted = _resolve_domain(no_input)
    if not explicit_domain and sys.stdin.isatty() and url_pasted and not no_input:
        typer.confirm(f"Extracted {config.DOMAIN} — login?", default=True, abort=True)
    try:
        if browser:
            cookies = login_browser()
        elif header is not None:
            if header == "-":
                header = sys.stdin.read().strip()
            cookies = parse_cookie_header(header)
            validate(cookies)
        else:
            cookies = login_interactive()
    except SessionError as e:
        _err_exc(e)
    if not force:
        _validate_new_cookies(cookies)
    if store is None and sys.stdin.isatty() and not no_input:
        store = _choose_store()
    backend = save_cookies(cookies, store)  # type: ignore[arg-type]
    target = "Keychain" if backend == "keychain" else config.display_path(config.SESSION_FILE)
    console.print(f"saved {len(cookies)} cookies to {target}")
    try:
        reset_client()
        c = get_client()
        agent = c.check_session()
        name = f"{agent.get('first_name', '')} {agent.get('last_name', '')}".strip()
        email = agent.get("email", "")
        if agent:
            console.print(f"logged in as: [bold]{name}[/bold] <{email}>")
        else:
            console.print("logged in (identity unavailable: agent profile is not readable)")
        err.print("warming cache in background...", highlight=False)
        _refresh_cache(verbose=False, blocking=False)
    except Exception as e:
        err.print(f"[yellow]warning[/yellow]: saved cookies but could not verify login for {config.DOMAIN}: {e}")
        err.print("If this is the wrong tenant, rerun with `fsv auth login --domain acme.freshservice.com`.")


@auth_app.command("logout", epilog="[bold]Examples:[/bold]  fsv auth logout  |  fsv auth logout --yes")
def auth_logout(
    yes: bool = typer.Option(False, "--yes", "-y", help="skip confirmation prompt"),
    no_input: bool = typer.Option(False, "--no-input", help="fail instead of prompting"),
) -> None:
    """Wipe stored session (file + Keychain + backend pref)."""
    from fsv.session import logout as session_logout
    if not yes:
        domain = config.DOMAIN or "Freshservice"
        if sys.stdin.isatty() and not _no_input(no_input):
            typer.confirm(f"Wipe session for {domain}?", abort=True)
        else:
            _err("pass --yes to confirm non-interactive logout")
    session_logout()
    console.print("session cleared")


@auth_app.command("status", epilog="[bold]Examples:[/bold]  fsv auth status")
def auth_status() -> None:
    """Show current agent identity and session status."""
    from fsv.session import session_age_hours
    age = session_age_hours()
    if age is None:
        _err("no session; run `fsv auth login`")
    c = _client()
    agent, (rem, tot) = _api(lambda: (c.check_session(), c.rate_limit_remaining()))
    name = f"{agent.get('first_name', '')} {agent.get('last_name', '')}".strip()
    email = agent.get("email", "")
    if not agent:
        name, email = "identity unavailable (403)", ""
    dept = ", ".join(agent.get("department_names") or [])
    team = (agent.get("custom_fields") or {}).get("team") or ""
    dept_display = f"{dept}  ({team})" if dept and team else dept or team
    console.print(f"[bold]{name}[/bold]  <{email}>" if email else f"[bold]{name}[/bold]")
    console.print("session:     valid")
    if dept_display:
        console.print(f"department:  {dept_display}")
    age_str = f"{age:.1f}h"
    if age > 8:
        age_str += "  [yellow]⚠ session may be stale[/yellow]"
    console.print(f"session age: {age_str}")
    console.print(f"rate limit:  {rem}/{tot}")


app.add_typer(auth_app, name="auth")

from fsv.integrated_assets_cli import register_search as _register_inventory_search
_inventory_app = typer.Typer(help="Integrated inventory search (read-only).")
_register_inventory_search(_inventory_app, lambda: _client())
app.add_typer(_inventory_app, name="assets")
from fsv.solutions_cli import register as _register_solutions
_solutions_app = typer.Typer(help="Draft knowledge solutions.")
_register_solutions(_solutions_app, lambda: _client())
app.add_typer(_solutions_app, name="solutions")
from fsv.templates_cli import register_templates as _register_templates
_register_templates(app, lambda: _client())


HELP_TOPICS = {
    "auth": "Login: fsv auth login --browser --domain acme.freshservice.com; fsv auth status. Scripts: use fsv auth login --domain ... --header ... --store file.",
    "workflow": "Daily flow: fsv changes ls --where status=Open; fsv changes get CHN-1234 --internal; fsv changes update CHN-1234 --set 'field=value'; fsv changes download CHN-1234 --all.",
    "fields": "Discover fields with fsv changes fields, fsv changes fields --choices status, and fsv changes lookup requester alice@example.com. Prefer dedicated flags for default fields; use --set for custom fields.",
    "scripting": "Use --json for create/update/clone/download. Use --output json/csv/tsv for list/search commands. Use --no-input in CI. Use --dry-run before updates. Do not parse rich table output.",
    "comments": "Tickets support reply and reviewed add-note (see docs/ticket-notes.md). Changes/problems use add-note. Use --public only when notes should be visible outside private workflow.",
}


@app.command("help")
def help_topic(
    topic: Optional[str] = typer.Argument(None, help="topic: auth|workflow|fields|scripting|comments", autocompletion=completion.complete_help_topic(HELP_TOPICS)),
) -> None:
    """Show topic help."""
    if topic is None:
        t = Table(title="help topics")
        t.add_column("Topic", style="cyan")
        t.add_column("Use")
        for name in HELP_TOPICS:
            t.add_row(name, f"fsv help {name}")
        console.print(t)
        console.print("Use `fsv COMMAND --help` for command reference.")
        return
    key = topic.casefold()
    if key not in HELP_TOPICS:
        _err(f"unknown help topic {topic!r}; choices: {', '.join(HELP_TOPICS)}")
    console.print(HELP_TOPICS[key])


completion_app = typer.Typer(
    no_args_is_help=True,
    help="shell completion helpers",
    epilog="[bold]Examples:[/bold]  fsv completion install  |  fsv cache refresh  |  fsv completion doctor",
)


def _refresh_cache(verbose: bool = True, blocking: bool = True) -> None:
    """Populate all local caches (schema, filters, groups).

    When *blocking* is False, spawns background threads and returns immediately.
    """
    from concurrent.futures import ThreadPoolExecutor, as_completed
    from fsv import schema as schema_mod
    from fsv.cache import save as _cache_save
    from fsv.client import Client
    import threading
    resources = list(REGISTRY.values())

    def _fetch_schema(r: Resource) -> tuple[str, int] | None:
        c = Client()
        try:
            t0 = time.time()
            sch = schema_mod.load(r, c, force=True)
            return (r.name, len(sch.get("fields") or []))
        finally:
            c.close()

    def _fetch_filters(r: Resource) -> tuple[str, int] | None:
        if not r.filters_path:
            return None
        c = Client()
        try:
            data = c.int_get(r.filters_path)
            key = next((k for k in data if k.endswith("_filters")), None)
            filters = data.get(key, []) if key else []
            p = config.filters_cache_path(r.name)
            _cache_save(p, "filters", {"filters": filters})
            return (f"{r.name} views", len(filters))
        finally:
            c.close()

    def _fetch_groups() -> tuple[str, int] | None:
        c = Client()
        try:
            data = c.int_get("bootstrap/agents_groups")
            groups = data.get("groups", [])
            p = config.groups_cache_path()
            _cache_save(p, "groups", {"groups": groups})
            return ("groups", len(groups))
        finally:
            c.close()

    tasks: list[tuple[str, Callable[[], tuple[str, int] | None]]] = []
    for r in resources:
        tasks.append((f"schema:{r.name}", lambda r=r: _fetch_schema(r)))
        if r.filters_path:
            tasks.append((f"filters:{r.name}", lambda r=r: _fetch_filters(r)))
    tasks.append(("groups", _fetch_groups))

    if not blocking:
        for label, fn in tasks:
            threading.Thread(target=fn, daemon=True).start()
        return

    if verbose:
        t = Table(title="cache refresh")
        t.add_column("resource", style="bold")
        t.add_column("count", justify="right")
        with err.status("syncing...", spinner="dots"):
            with ThreadPoolExecutor(max_workers=7) as pool:
                futures = {pool.submit(fn): label for label, fn in tasks}
                for future in as_completed(futures):
                    try:
                        result = future.result()
                    except Exception:
                        continue
                    if result:
                        name, count = result
                        t.add_row(name, str(count))
        console.print(t)
    else:
        with ThreadPoolExecutor(max_workers=7) as pool:
            futures = [pool.submit(fn) for _, fn in tasks]
            for future in as_completed(futures):
                try:
                    future.result()
                except Exception:
                    pass


def _completion_shell(shell: str | None = None) -> str:
    if shell:
        return shell
    env_shell = Path(os.environ.get("SHELL") or "").name
    if env_shell in {"bash", "zsh", "fish", "powershell", "pwsh"}:
        return env_shell
    _err("could not detect shell; pass one of: bash, zsh, fish, powershell, pwsh")


@completion_app.command("show", epilog="[bold]Examples:[/bold]  fsv completion show  |  fsv completion show fish")
def completion_show(
    shell: Optional[str] = typer.Argument(None, help="bash|zsh|fish|powershell|pwsh", autocompletion=completion.complete_shell),
) -> None:
    """Show shell completion script."""
    from fsv.completion_gen import build_script

    shell_name = _completion_shell(shell)
    prog_name = Path(sys.argv[0]).name
    sys.stdout.write(build_script(shell_name, prog_name))


@completion_app.command("install", epilog="[bold]Examples:[/bold]  fsv completion install  |  fsv completion install zsh")
def completion_install(
    shell: Optional[str] = typer.Argument(None, help="bash|zsh|fish|powershell|pwsh", autocompletion=completion.complete_shell),
) -> None:
    """Install shell completion for current shell."""
    from fsv.completion_gen import build_script

    shell_name = _completion_shell(shell)
    prog_name = Path(sys.argv[0]).name
    script = build_script(shell_name, prog_name)

    if shell_name == "fish":
        fish_dir = Path.home() / ".config" / "fish" / "completions"
        fish_dir.mkdir(parents=True, exist_ok=True)
        path = fish_dir / f"{prog_name}.fish"
        path.write_text(script)
        console.print(f"fish completion installed in {path}")
    elif shell_name in ("bash", "zsh"):
        # Write standalone script; append source line to shell config
        script_path = Path.home() / f".{prog_name}-complete.{shell_name}"
        script_path.write_text(script)
        if shell_name == "bash":
            rc_file = Path.home() / ".bashrc"
        else:
            rc_file = Path.home() / ".zshrc"
        source_line = f"\n. {script_path}\n"
        existing = rc_file.read_text() if rc_file.exists() else ""
        if str(script_path) not in existing:
            with rc_file.open("a") as f:
                f.write(source_line)
        console.print(f"{shell_name} completion installed in {script_path}")
        console.print(f"sourced from {rc_file}")
    else:
        # powershell/pwsh: fall back to typer
        from typer._completion_shared import install as _install_completion
        installed_shell, path = _install_completion(shell=shell_name, prog_name=prog_name)
        console.print(f"{installed_shell} completion installed in {path}")

    console.print("restart shell: exec $SHELL")


@completion_app.command("refresh", help="Alias for `fsv cache refresh`.", epilog="[bold]Examples:[/bold]  fsv completion refresh")
def completion_refresh() -> None:
    """Refresh local schema cache used by completion."""
    cache_refresh()


@completion_app.command("doctor", epilog="[bold]Examples:[/bold]  fsv completion doctor")
def completion_doctor() -> None:
    """Show completion install/cache diagnostics."""
    import shutil
    t = Table(title="completion doctor")
    t.add_column("check")
    t.add_column("value")
    shell = os.environ.get("SHELL") or "?"
    t.add_row("shell", shell)
    t.add_row("fsv path", shutil.which("fsv") or sys.argv[0])
    t.add_row("network", "on" if completion._completion_network() else "off")
    for res in REGISTRY.values():
        candidates = config.schema_cache_candidates(res.name)
        t.add_row(f"schema {res.name}", "ok" if any(p.exists() for p in candidates) else "missing")
    console.print(t)
    console.print("Use TAB after a prefix, e.g. `fsv changes ls --where requester=phu<TAB>`.")


app.add_typer(completion_app, name="completion")


config_app = typer.Typer(
    no_args_is_help=True,
    help="manage fsv settings",
    epilog="[bold]Examples:[/bold]  fsv config set completion.network on",
)


_CONFIG_KEYS: dict[str, str] = {
    "completion.network": "enable remote requester/agent completion [on|off]",
    "defaults.requester": "requester email for `changes create` when --requester is not given",
    "changes.attach_flow": "`changes create` attachments: after-create (default) | before-create (unverified vs the web)",
    "forms.workspace_id": "workspace whose business rules apply (default: the form metadata, else the account)",
    "defaults.sr_requester": "requester email for `sr create` when --requester is not given",
    "tui.read_only": "open the TUI without write actions [on|off]",
}


@config_app.command("list", epilog="[bold]Examples:[/bold]  fsv config list")
def config_list() -> None:
    """List all config keys and current values."""
    cfg = completion._config_load()
    for key, desc in _CONFIG_KEYS.items():
        section, _, prop = key.partition(".")
        val = (cfg.get(section) or {}).get(prop, "(unset)")
        console.print(f"{key} = {val}  [dim]# {desc}[/]")


@config_app.command("get", epilog="[bold]Examples:[/bold]  fsv config get completion.network")
def config_get(
    key: str = typer.Argument(..., help="config key", autocompletion=completion.complete_config_key),
) -> None:
    """Get a single config value."""
    if key not in _CONFIG_KEYS:
        _err(f"unknown key: {key}; supported: {list(_CONFIG_KEYS)}")
    section, _, prop = key.partition(".")
    cfg = completion._config_load()
    val = (cfg.get(section) or {}).get(prop, "(unset)")
    console.print(val)


@config_app.command("set", epilog="[bold]Examples:[/bold]  fsv config set completion.network on")
def config_set(
    key: str = typer.Argument(..., help="completion.network, ...", autocompletion=completion.complete_config_key),
    value: str = typer.Argument(..., help="on|off|true|false|1|0", autocompletion=completion.complete_config_value),
) -> None:
    """Set a config value."""
    section, _, prop = key.partition(".")
    if not section or key not in _CONFIG_KEYS:
        _err(f"unknown key: {key}; supported: {list(_CONFIG_KEYS)}")
    cfg = config.load_config()
    cfg.setdefault(section, {})[prop] = value
    config.save_config(cfg)
    console.print(f"{key} = {value}")


_defaults_app = typer.Typer(
    no_args_is_help=True,
    help="manage default --where filters per resource",
    epilog=(
        "[bold]Examples:[/bold]  "
        "fsv config defaults show  |  "
        "fsv config defaults set changes --where agent=me@example.com  |  "
        "fsv config defaults clear changes"
    ),
)


@_defaults_app.command("show", epilog="[bold]Examples:[/bold]  fsv config defaults show")
def defaults_show(
    resource: Optional[str] = typer.Argument(None, help="changes|tickets|problems; omit for all"),
) -> None:
    """Show default --where filters."""
    names = [resource] if resource else list(REGISTRY)
    for name in names:
        exprs = config.get_default_where(name)
        if exprs:
            for expr in exprs:
                console.print(f"{name}: {expr}")
        else:
            console.print(f"{name}: (none)")


@_defaults_app.command("set", epilog="[bold]Examples:[/bold]  fsv config defaults set changes --where agent=me@example.com --where status=Open")
def defaults_set(
    resource: str = typer.Argument(..., help="changes|tickets|problems"),
    where: list[str] = typer.Option(..., "--where", "-w", help="field=value filter; repeat to add multiple"),
) -> None:
    """Set (replace) default --where filters for a resource."""
    if resource not in REGISTRY:
        _err(f"unknown resource {resource!r}; choose: {', '.join(REGISTRY)}")
    config.set_default_where(resource, where)
    console.print(f"defaults.{resource}.where = {where}")


@_defaults_app.command("clear", epilog="[bold]Examples:[/bold]  fsv config defaults clear changes")
def defaults_clear(
    resource: str = typer.Argument(..., help="changes|tickets|problems"),
) -> None:
    """Clear default --where filters for a resource."""
    if resource not in REGISTRY:
        _err(f"unknown resource {resource!r}; choose: {', '.join(REGISTRY)}")
    config.clear_default_where(resource)
    console.print(f"defaults.{resource}.where cleared")


config_app.add_typer(_defaults_app, name="defaults")
app.add_typer(config_app, name="config")


cache_app = typer.Typer(
    no_args_is_help=True,
    help="inspect and manage local cache",
    epilog="[bold]Examples:[/bold]  fsv cache status  |  fsv cache refresh  |  fsv cache clear schema",
)


@cache_app.command("status", epilog="[bold]Examples:[/bold]  fsv cache status")
def cache_status() -> None:
    """Show cache health and TTL."""
    from fsv.cache import load as _l, TTL

    items = []
    for res in REGISTRY.values():
        for kind, label, candidates in (
            ("schema", "schema", config.schema_cache_candidates(res.name)),
            ("filters", "views", config.filters_cache_candidates(res.name)),
        ):
            chosen = None
            doc = None
            stale = True
            for p in candidates:
                doc, stale = _l(p)
                if doc is not None:
                    chosen = p
                    break
            if doc is None or chosen is None:
                continue
            saved = doc.get("saved_at", 0) or chosen.stat().st_mtime
            age = (time.time() - saved) / 3600
            ttl = TTL.get(kind, 3600)
            ttl_s = f"{ttl / 3600:.0f}h" if ttl >= 3600 else f"{ttl / 60:.0f}m"
            items.append((res.name, label, f"{age:.1f}h", f"{'stale' if stale else 'ok'}", ttl_s))
    chosen = None
    doc = None
    stale = True
    for gp in config.groups_cache_candidates():
        doc, stale = _l(gp)
        if doc is not None:
            chosen = gp
            break
    if doc and chosen:
        saved = doc.get("saved_at", 0) or chosen.stat().st_mtime
        age = (time.time() - saved) / 3600
        ttl = TTL.get("groups", 6 * 3600)
        ttl_s = f"{ttl / 3600:.0f}h" if ttl >= 3600 else f"{ttl / 60:.0f}m"
        items.append(("groups", "groups", f"{age:.1f}h", f"{'stale' if stale else 'ok'}", ttl_s))
    t = Table(title="cache status")
    t.add_column("cache")
    t.add_column("kind")
    t.add_column("age")
    t.add_column("state")
    t.add_column("ttl")
    for name, kind, age, state, ttl_s in items:
        t.add_row(name.replace(".json", ""), kind, age, state, ttl_s)
    console.print(t)


@cache_app.command("refresh", epilog="[bold]Examples:[/bold]  fsv cache refresh")
def cache_refresh() -> None:
    """Refresh only stale caches."""
    _refresh_cache(verbose=True, blocking=True)


@cache_app.command("clear", epilog="[bold]Examples:[/bold]  fsv cache clear views  |  fsv cache clear all")
def cache_clear(
    target: Optional[str] = typer.Argument(None, help="schema|views|groups|all", autocompletion=completion.complete_cache_target),
) -> None:
    """Delete cached data."""
    import shutil

    if target is None or target not in {"schema", "views", "filters", "groups", "all"}:
        _err("choose schema|views|groups|all")
    def remove_tree(path: Any) -> bool:
        """Remove a cache directory, tolerating a parallel process that
        removes it (or entries in it) first: a vanished path is cleared."""
        def ignore_missing(_func: Any, _path: Any, exc: BaseException) -> None:
            if not isinstance(exc, FileNotFoundError):
                raise exc
        if not path.exists():
            return False
        shutil.rmtree(path, onexc=ignore_missing)
        return True

    if target in ("schema", "all"):
        if remove_tree(config.SCHEMA_DIR):
            console.print("schema cache cleared")
    if target in ("views", "filters", "all"):
        if remove_tree(config.CONFIG_DIR / "filters"):
            console.print("views cache cleared")
    if target in ("groups", "all"):
        removed = False
        for gp in config.groups_cache_candidates():
            if gp.exists():
                gp.unlink(missing_ok=True)
                removed = True
        if removed:
            console.print("groups cache cleared")


app.add_typer(cache_app, name="cache")

journal_app = typer.Typer(
    no_args_is_help=True,
    help="read the local write journal",
    rich_markup_mode=None if _COMPLETING else "rich",
)


def _journal_rows(rows: list[dict[str, Any]], json_out: bool) -> None:
    import json
    from rich.table import Table
    if json_out:
        for r in rows:
            print(json.dumps(r, ensure_ascii=False))
        return
    if not rows:
        console.print("[dim]journal is empty[/dim]")
        return
    t = Table()
    for col in ("time", "command", "kind", "target", "status", "created", "plan"):
        t.add_column(col)
    for r in rows:
        t.add_row(str(r.get("ts", "")), str(r.get("command") or "-"), str(r.get("kind", "")),
                  str(r.get("target") or "-"), ("ok " if r.get("ok") else "FAIL ") + str(r.get("status") or ""),
                  str(r.get("created_id") or "-"), str(r.get("plan_hash") or "-")[:12])
    console.print(t)


@journal_app.command("ls", epilog="[bold]Examples:[/bold]  fsv journal ls  |  fsv journal ls --json --failed")
def journal_ls(
    limit: int = typer.Option(0, "--limit", "-n", help="only the last N entries (0 = all)"),
    failed: bool = typer.Option(False, "--failed", help="only failed writes"),
    json_out: bool = typer.Option(False, "--json", help="one JSON object per line"),
) -> None:
    """List journaled write requests (oldest first)."""
    from fsv import journal
    rows = [r for r in journal.read() if not failed or not r.get("ok")]
    _journal_rows(rows[-limit:] if limit else rows, json_out)


@journal_app.command("tail", epilog="[bold]Examples:[/bold]  fsv journal tail  |  fsv journal tail -n 5 --json")
def journal_tail(
    limit: int = typer.Option(10, "--limit", "-n", help="number of entries"),
    json_out: bool = typer.Option(False, "--json", help="one JSON object per line"),
) -> None:
    """Show the most recent journaled writes."""
    from fsv import journal
    _journal_rows(journal.read(limit), json_out)


@journal_app.command("path", epilog="[bold]Examples:[/bold]  fsv journal path")
def journal_path_cmd() -> None:
    """Print the journal file location."""
    from fsv import journal
    print(journal.journal_path())


app.add_typer(journal_app, name="journal")


_SITE_TABS = {
    "changes": ("notes", "tasks", "approvals", "assets", "impacted", "associations", "activity", "risk", "devops"),
    "tickets": ("related", "tasks", "assets", "associations", "approvals", "activity", "resolution"),
    "problems": ("notes", "tasks", "assets", "associations", "activity"),
}


def _tab_help(res: Resource) -> str:
    return "show one tab of the page: " + "|".join(_SITE_TABS.get(res.name, ())) + " (only tabs the server shows)"


def _complete_tab(res: Resource) -> Callable[[str], list[str]]:
    def complete(incomplete: str) -> list[str]:
        return [t for t in _SITE_TABS.get(res.name, ()) if t.startswith(incomplete or "")]
    return complete


def _make_subapp(res: Resource) -> typer.Typer:
    _pfx = res.display_prefixes[0]
    sub = typer.Typer(
        no_args_is_help=True,
        help=f"{res.name} commands",
        rich_markup_mode=None if _COMPLETING else "rich",
        epilog=(
            f"[bold]Examples:[/bold]  "
            f"fsv {res.name} ls --where status=Open  |  "
            f"fsv {res.name} ls --where 'requester=me@example.com' --output json  |  "
            f"fsv {res.name} get {_pfx}-1234  |  "
            f"fsv {res.name} get {_pfx}-1234 --internal  |  "
            f"fsv {res.name} update {_pfx}-1234 --status 'In Progress'"
        ),
    )
    singular = res.name[:-1]

    @sub.command("ls", help=f"List {res.name}.", epilog=(
        f"[bold]Examples:[/bold]  "
        f"fsv {res.name} ls  |  "
        f"fsv {res.name} ls --where status=Open  |  "
        f"fsv {res.name} ls --where status=Open --where priority=High  |  "
        f"fsv {res.name} ls --all --output json"
    ))
    def ls(
        filter_name: Optional[str] = typer.Option(None, "--view", help="saved view by name as the web shows it (exact or unique prefix; see `views`); default: the view the server marks default", autocompletion=completion.complete_filter_name(res)),
        where: Optional[list[str]] = typer.Option(None, "--where", "-w", help="filter like the web's filter panel: LABEL=VALUE, e.g. Status=Open, Agents=Me, Created='Last 6 months', 'Environment=(empty)'; internal names work too", autocompletion=completion.complete_where(res)),
        query_hash: Optional[str] = typer.Option(None, "--query-hash", help="raw or URL-encoded Freshservice query_hash JSON array"),
        or_grouping: bool = typer.Option(False, "--or", help="combine --where conditions with OR instead of AND"),
        debug: bool = typer.Option(False, "--debug", help="show resolved query_hash and exit"),
        per_page: int = typer.Option(30, "--per-page", "-n"),
        page: int = typer.Option(1, "--page", "-p"),
        order_by: Optional[str] = typer.Option(None, "--order-by", help="sort field, e.g. created_at", autocompletion=completion.complete_field_names(res)),
        order_type: SortOrder = typer.Option(SortOrder.desc, "--order-type", help="asc | desc", autocompletion=completion.complete_sort_order),
        all_pages: bool = typer.Option(False, "--all", "-a", help="fetch all pages (auto-paginate)"),
        n_pages: Optional[int] = typer.Option(None, "--npages", "-N", help="fetch exactly N pages"),
        format_: OutputFormat = typer.Option(OutputFormat.table, "--output", "-o", help="output format", autocompletion=completion.complete_format),
        json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
        pager: bool = typer.Option(True, "--pager/--no-pager", help="page long table output when stdout is a TTY"),
        no_defaults: bool = typer.Option(False, "--no-defaults", help="ignore default --where filters from config"),
    ) -> None:
        f"""List {res.name}."""
        explicit_where = where or []
        default_where = [] if no_defaults else config.get_default_where(res.name)
        if default_where and explicit_where:
            explicit_fields = {_where_field(e) for e in explicit_where}
            default_where = [e for e in default_where if _where_field(e) not in explicit_fields]
        list_resource(
            res,
            filter_name,
            default_where + explicit_where,
            debug,
            per_page,
            page,
            all_pages,
            format_,
            json_out,
            or_grouping,
            pager,
            query_hash,
            order_by,
            order_type,
            n_pages,
        )

    @sub.command("get", help=f"Show one {singular}.", epilog=(
        f"[bold]Examples:[/bold]  "
        f"fsv {res.name} get {_pfx}-1234  |  "
        f"fsv {res.name} get {_pfx}-1234 --tab activity  |  "
        f"fsv {res.name} get {_pfx}-1234 --all-tabs  |  "
        f"fsv {res.name} get {_pfx}-1234 --json"
    ))
    def get(
        id_: str = typer.Argument(..., metavar="ID"),
        tab: Optional[str] = typer.Option(None, "--tab", "-t", help=_tab_help(res), autocompletion=_complete_tab(res)),
        all_tabs: bool = typer.Option(False, "--all-tabs", help="print the Details tab and every other tab the server shows"),
        all_conversations: bool = typer.Option(False, "--all-conversations", help="every conversation, not only the latest (tickets)"),
        stats: bool = typer.Option(False, "--stats", help="with --json: the record plus planning fields and attachment evidence"),
        format_: OutputFormat = typer.Option(OutputFormat.table, "--output", "-o", help="output format", autocompletion=completion.complete_format),
        json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
    ) -> None:
        f"""Show one {res.name[:-1]} by id."""
        get_resource(res, id_, stats, json_out, format_, tab=tab, all_tabs=all_tabs, all_conversations=all_conversations)

    @sub.command("activity", help=f"List {singular} activity.", epilog=(
        f"[bold]Examples:[/bold]  fsv {res.name} activity {_pfx}-1234  |  fsv {res.name} activity {_pfx}-1234 -l 50 --json"
    ))
    def activity(
        id_: str = typer.Argument(...),
        limit: int = typer.Option(20, "-l", "--limit", help="newest N entries (a 'truncated' notice reports the rest)"),
        all_: bool = typer.Option(False, "--all", help="every loaded entry (overrides --limit)"),
        format_: OutputFormat = typer.Option(OutputFormat.table, "--output", "-o", help="output format", autocompletion=completion.complete_format),
        json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
    ) -> None:
        activity_resource(res, id_, None if all_ else limit, json_out, format_)

    @sub.command("tasks-legacy" if res == TICKETS else "tasks", hidden=res == TICKETS, help=f"List {singular} tasks.", epilog=(
        f"[bold]Examples:[/bold]  fsv {res.name} tasks {_pfx}-1234  |  fsv {res.name} tasks {_pfx}-1234 --json"
    ))
    def tasks(
        id_: str = typer.Argument(...),
        format_: OutputFormat = typer.Option(OutputFormat.table, "--output", "-o", help="output format", autocompletion=completion.complete_format),
        json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
    ) -> None:
        tasks_resource(res, id_, format_, json_out)

    if res == CHANGES:
        from fsv import state_flow
        from fsv.create import (
            associate_ticket,
            delete_task,
            dissociate_ticket,
            get_change_approvals,
            get_change_approval_groups,
            get_change_associations,
            get_task_for_edit,
            search_change_tickets,
            update_task,
        )

        from fsv.change_tasks import register as register_change_tasks
        register_change_tasks(sub)
        from fsv.change_approvals import register as register_change_approvals
        from fsv.change_approval_actions import register as register_change_approval_actions
        register_change_approvals(sub)
        register_change_approval_actions(sub)
        from fsv.change_relationships_cli import register as register_relationships
        register_relationships(sub, lambda: _client())
        from fsv.module_time_cli import register as register_change_time
        register_change_time(sub, lambda: _client(), "changes")
        from fsv.templates_cli import register_windows
        register_windows(sub, lambda: _client())

        @sub.command("delete", help="Soft-delete one exact change (reviewed, destructive).", epilog=(
            "[bold]Examples:[/bold]  fsv changes delete CHN-1234 --dry-run  |  "
            "fsv changes delete CHN-1234 --confirm-destructive --yes --expect-plan FULL_HASH"
        ))
        def change_delete_cmd(
            id_: str = typer.Argument(..., metavar="CHANGE_ID"),
            dry_run: bool = typer.Option(False, "--dry-run", help="print the exact target without deleting"),
            yes: bool = typer.Option(False, "--yes", "-y", help="confirm deletion"),
            confirm_destructive: bool = typer.Option(False, "--confirm-destructive", help="required destructive acknowledgement"),
            expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
            json_out: bool = typer.Option(False, "--json"),
        ) -> None:
            from fsv import change_delete
            from fsv.reviewed_write import ReviewedWriteError
            try:
                c = _client()
                plan = change_delete.build_plan(c, id_)
                if dry_run:
                    emit_json({"status": "dry_run", **change_delete.review(plan, c)})
                    return
                if not yes or expect_plan != plan.hash:
                    raise ReviewedWriteError("review with --dry-run, then --confirm-destructive --yes --expect-plan FULL_HASH")
                result = change_delete.send(plan, c, reviewed_hash=expect_plan, confirm_destructive=confirm_destructive)
                emit_json(result)
                if not result.get("verified"):
                    raise typer.Exit(4)
            except (ValueError, ReviewedWriteError) as exc:
                _err(str(exc))
            except (SessionError, APIError) as e:
                _err_exc(e)

        @sub.command("tasks-update", help="Update a change task.", epilog=(
            "[bold]Examples:[/bold]  "
            "fsv changes tasks-update CHN-1234 987 --status Completed --dry-run  |  "
            "fsv changes tasks-update CHN-1234 987 --group 'Service Desk' --agent me@example.com"
        ))
        def task_update(
            id_: str = typer.Argument(..., metavar="CHANGE_ID"),
            task_id: int = typer.Argument(..., metavar="TASK_ID", autocompletion=_complete_task_id),
            status: Optional[str] = typer.Option(None, "--status", "-s", help="Open/Completed or status ID", autocompletion=_complete_task_status),
            title: Optional[str] = typer.Option(None, "--title", help="task title"),
            group_id: Optional[str] = typer.Option(None, "--group", "--group-id", help="group name or ID", autocompletion=completion.complete_update_group_id),
            agent_id: Optional[str] = typer.Option(None, "--owner", "--agent", "--agent-id", help="agent name/email/user ID", autocompletion=completion.complete_update_agent_id),
            system: Optional[str] = typer.Option(None, "--system", help="task custom field system label or numeric ID", autocompletion=_complete_task_system),
            environment: Optional[str] = typer.Option(None, "--environment", "--env", help="task custom field environment label or numeric ID", autocompletion=_complete_task_environment),
            due_date: Optional[str] = typer.Option(None, "--due-date", help="ISO-8601 due date"),
            planned_start: Optional[str] = typer.Option(None, "--planned-start", help="ISO-8601 planned start"),
            planned_end: Optional[str] = typer.Option(None, "--planned-end", help="ISO-8601 planned end"),
            edit: bool = typer.Option(False, "--edit", "-e", help="open $EDITOR with all task fields"),
            note: Optional[str] = typer.Option(None, "--note", help="plain text note"),
            planned_effort: Optional[str] = typer.Option(None, "--planned-effort"),
            notify_before: Optional[str] = typer.Option(None, "--notify-before", help="runtime label or seconds, not minutes"),
            expected: Optional[str] = typer.Option(None, "--expect-plan"),
            read_only: bool = typer.Option(False, "--read-only"),
            dry_run: bool = typer.Option(False, "--dry-run", help="print resolved payload without updating"),
            yes: bool = typer.Option(False, "--yes", "-y", help="confirm status changes and reassignment"),
            json_out: bool = typer.Option(False, "--json", help="emit JSON"),
            no_input: bool = typer.Option(False, "--no-input", help="fail instead of prompting"),
        ) -> None:
            from fsv import change_task_update
            from fsv.integrated_assets_cli import finish, guard
            def run():
                if edit or due_date is not None:
                    raise ValueError("--edit/--due-date are not captured task-update contracts; use explicit planned dates and reviewed flags")
                inputs = {k: v for k, v in dict(title=title, status=status, owner=agent_id, group=group_id,
                          note=note, planned_start=planned_start, planned_end=planned_end,
                          planned_effort=planned_effort, notify_before=notify_before).items() if v is not None}
                values = {k: v for k, v in dict(cf_system=system, cf_environment=environment).items() if v is not None}
                if values:
                    inputs["values"] = values
                c = _client()
                plan = change_task_update.build_plan(c, id_, task_id, inputs)
                finish(plan, lambda: change_task_update.send(plan, c, reviewed_hash=expected),
                       dry_run=dry_run, yes=yes, expected=expected, read_only=read_only)
            guard(run)

        @sub.command("tasks-delete", help="Delete a change task.", epilog=(
            "[bold]Examples:[/bold]  fsv changes tasks-delete CHN-1234 987 --dry-run  |  fsv changes tasks-delete CHN-1234 987 --yes --expect-plan FULL_HASH"
        ))
        def task_delete(
            id_: str = typer.Argument(..., metavar="CHANGE_ID"),
            task_id: int = typer.Argument(..., metavar="TASK_ID", autocompletion=_complete_task_id),
            dry_run: bool = typer.Option(False, "--dry-run", help="print target without deleting"),
            yes: bool = typer.Option(False, "--yes", "-y", help="confirm deletion"),
            expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
            json_out: bool = typer.Option(False, "--json"),
        ) -> None:
            from fsv import change_task_delete
            from fsv.reviewed_write import ReviewedWriteError
            try:
                c = _client()
                plan = change_task_delete.build_plan(c, id_, task_id)
                if dry_run:
                    emit_json({"status": "dry_run", **plan.review()})
                    return
                if not yes or expect_plan != plan.hash:
                    raise ReviewedWriteError("review with --dry-run, then --yes --expect-plan FULL_HASH")
                result = change_task_delete.send(plan, c, reviewed_hash=expect_plan)
                emit_json(result)
                if not result.get("verified"):
                    raise typer.Exit(4)
            except (ValueError, ReviewedWriteError) as exc:
                _err(str(exc))
            except (SessionError, APIError) as e:
                _err_exc(e)

        @sub.command("approvals", help="List change approvals.", epilog=(
            "[bold]Examples:[/bold]  fsv changes approvals CHN-1234  |  fsv changes approvals CHN-1234 --json"
        ))
        def approvals(
            id_: str = typer.Argument(...),
            groups: bool = typer.Option(False, "--groups", help="read approval-chain groups (IDs, raw statuses, members), instead of legacy records"),
            format_: OutputFormat = typer.Option(OutputFormat.table, "--output", "-o", help="output format", autocompletion=completion.complete_format),
            json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
        ) -> None:
            """List approvals for a change (read-only)."""
            from fsv.change_approvals import ApprovalError
            try:
                cid = _cid(id_, res)
                if groups:
                    items = get_change_approval_groups(cid)
                    flat = [{"group_id": g["id"], "status": g["approval_status"],
                             "member_ids": ", ".join(str(r["member_id"]) for r in g["approvals"])} for g in items]
                    if _emit_fmt(items, flat, format_, json_out):
                        return
                    t = Table("Group ID", "Raw status", "Member IDs")
                    for g in flat:
                        t.add_row(str(g["group_id"]), str(g["status"]), g["member_ids"])
                    console.print(t)
                    return
                items = get_change_approvals(cid)
                def _flat(a: dict) -> dict:
                    remark = (a.get("remark") or [{}])[0]
                    raw_status = (a.get("status") or {}).get("name") or "-"
                    decided = remark.get("updated_at") or "-"
                    return {
                        "level": str(a.get("level_id", "-")),
                        "approver": (a.get("member") or {}).get("name") or str(a.get("member_id", "-")),
                        "status": "peer approved" if raw_status == "peer_responded" else raw_status,
                        "decided": decided[:10] if decided != "-" else "-",
                        "message": remark.get("data") or "",
                    }
                if _emit_fmt(items, [_flat(a) for a in items], format_, json_out):
                    return
                t = Table()
                t.add_column("Level", style="cyan")
                t.add_column("Approver")
                t.add_column("Status")
                t.add_column("Decided")
                t.add_column("Message")
                for a in items:
                    member = (a.get("member") or {}).get("name") or str(a.get("member_id", "-"))
                    raw_status = (a.get("status") or {}).get("name") or "-"
                    status_name = "peer approved" if raw_status == "peer_responded" else raw_status
                    remark = (a.get("remark") or [{}])[0]
                    decided = remark.get("updated_at") or "-"
                    message = remark.get("data") or ""
                    t.add_row(str(a.get("level_id", "-")), member, status_name, decided[:10] if decided != "-" else "-", message)
                console.print(t)
            except (SessionError, APIError, ApprovalError) as e:
                _err_exc(e)

        @sub.command("state", help="Approval lifecycle state flow.", epilog=(
            "[bold]Examples:[/bold]  fsv changes state CHN-1234  |  fsv changes state CHN-1234 --json"
        ))
        def state(
            id_: str = typer.Argument(..., metavar="ID"),
            format_: OutputFormat = typer.Option(OutputFormat.table, "--output", "-o", help="output format", autocompletion=completion.complete_format),
            json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
        ) -> None:
            """Show the ordered approval flow for a change.

            Highlights current position and indicates which gates have been
            passed. Shows approval status inline.
            """
            try:
                cid = _cid(id_, res)
                c = _client()
                data = c.int_get(f"changes/{cid}")
                change = data.get("change", data)
                fmt = format_.value if isinstance(format_, OutputFormat) else format_
                if json_out or fmt == "json":
                    flow_data = state_flow.get_state_flow(change["state_flow_id"], c, change.get("workspace_id"))
                    emit_json({"change": change, "flow": flow_data})
                elif fmt in ("csv", "tsv"):
                    render.emit_delimited([change], "," if fmt == "csv" else "\t")
                else:
                    state_flow.render_flow(change, console)
            except KeyError:
                _err("no state_flow_id on this change")
            except (SessionError, APIError) as e:
                _err_exc(e)

        associations_app = typer.Typer(
            cls=ChangeLegacyGroup,
            invoke_without_command=True,
            no_args_is_help=True,
            help="Manage change ticket associations.",
            rich_markup_mode=None if _COMPLETING else "rich",
            epilog=(
                "[bold]Examples:[/bold]  "
                "fsv changes associations ls CHN-1234  |  "
                "fsv changes associations search CHN-1234 SR-123  |  "
                "fsv changes associations add CHN-1234 SR-123 --dry-run  |  "
                "fsv changes associations remove CHN-1234 SR-123 --yes --expect-plan FULL_HASH  |  "
                "fsv changes associations pick CHN-1234 --dry-run"
            ),
        )

        @associations_app.callback()
        def associations_group(ctx: typer.Context) -> None:
            if ctx.meta.get("fsv_legacy_nested"):
                err.print("[yellow]deprecated[/]: use `fsv changes associations <ls|search|add|remove|pick> ...`", highlight=False)

        @associations_app.command("legacy", hidden=True)
        def associations(
            id_: str = typer.Argument(...),
            search: Optional[str] = typer.Option(None, "--search", "-q", help="search tickets available to associate", autocompletion=_complete_ticket_for_change),
            add: Optional[List[str]] = typer.Option(None, "--add", help="ticket ID(s) or subject text to associate, e.g. SR-565163", autocompletion=_complete_ticket_for_change),
            remove: Optional[List[str]] = typer.Option(None, "--remove", help="ticket ID(s) or subject text to dissociate", autocompletion=_complete_ticket_for_change),
            pick: bool = typer.Option(False, "--pick", help="interactive picker for add flow"),
            dry_run: bool = typer.Option(False, "--dry-run", help="print resolved payload without mutating"),
            yes: bool = typer.Option(False, "--yes", "-y", help="confirm mutation"),
            expect_plan: Optional[str] = typer.Option(None, "--expect-plan", help="full hash from exact association review"),
            read_only: bool = typer.Option(False, "--read-only", help="refuse association writes"),
            format_: OutputFormat = typer.Option(OutputFormat.table, "--output", "-o", help="output format", autocompletion=completion.complete_format),
            json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
        ) -> None:
            try:
                change_associations_resource(id_, search, list(add or []), list(remove or []), dry_run, yes, json_out, format_, pick=pick, **({"expect_plan": expect_plan, "read_only": read_only} if expect_plan is not None or read_only else {}))
            except (SessionError, APIError, ValueError) as e:
                _err_exc(e)

        @associations_app.command("ls", help="List associated tickets for change.", epilog="[bold]Examples:[/bold]  fsv changes associations ls CHN-1234  |  fsv changes associations ls CHN-1234 --json")
        def associations_ls(
            id_: str = typer.Argument(..., metavar="CHANGE_ID"),
            format_: OutputFormat = typer.Option(OutputFormat.table, "--output", "-o", help="output format", autocompletion=completion.complete_format),
            json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
        ) -> None:
            try:
                change_associations_resource(id_, None, [], [], False, False, json_out, format_)
            except (SessionError, APIError, ValueError) as e:
                _err_exc(e)

        @associations_app.command("search", help="Search tickets available to associate.", epilog="[bold]Examples:[/bold]  fsv changes associations search CHN-1234 SR-123  |  fsv changes associations search CHN-1234 Napimpat --output csv")
        def associations_search(
            id_: str = typer.Argument(..., metavar="CHANGE_ID"),
            query: str = typer.Argument(..., metavar="QUERY", autocompletion=_complete_ticket_for_change),
            format_: OutputFormat = typer.Option(OutputFormat.table, "--output", "-o", help="output format", autocompletion=completion.complete_format),
            json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
        ) -> None:
            try:
                change_associations_resource(id_, query, [], [], False, False, json_out, format_)
            except (SessionError, APIError, ValueError) as e:
                _err_exc(e)

        @associations_app.command("add", help="Associate ticket(s) to change.", epilog="[bold]Examples:[/bold]  fsv changes associations add CHN-1234 SR-123 --dry-run  |  fsv changes associations add CHN-1234 SR-123 --yes --expect-plan FULL_HASH")
        def associations_add(
            id_: str = typer.Argument(..., metavar="CHANGE_ID"),
            tickets_: List[str] = typer.Argument(..., metavar="TICKET", autocompletion=_complete_ticket_for_change),
            dry_run: bool = typer.Option(False, "--dry-run", help="print resolved payload without mutating"),
            yes: bool = typer.Option(False, "--yes", "-y", help="confirm mutation"),
            expect_plan: Optional[str] = typer.Option(None, "--expect-plan", help="full hash from exact association review"),
            read_only: bool = typer.Option(False, "--read-only", help="refuse association writes"),
            json_out: bool = typer.Option(False, "--json", help="emit JSON"),
        ) -> None:
            try:
                change_associations_resource(id_, None, list(tickets_), [], dry_run, yes, json_out, **({"expect_plan": expect_plan, "read_only": read_only} if expect_plan is not None or read_only else {}))
            except (SessionError, APIError, ValueError) as e:
                _err_exc(e)

        @associations_app.command("remove", help="Dissociate ticket(s) from change.", epilog="[bold]Examples:[/bold]  fsv changes associations remove CHN-1234 SR-123 --dry-run  |  fsv changes associations remove CHN-1234 SR-123 --yes --expect-plan FULL_HASH")
        def associations_remove(
            id_: str = typer.Argument(..., metavar="CHANGE_ID"),
            tickets_: List[str] = typer.Argument(..., metavar="TICKET", autocompletion=_complete_ticket_for_change),
            dry_run: bool = typer.Option(False, "--dry-run", help="print resolved payload without mutating"),
            yes: bool = typer.Option(False, "--yes", "-y", help="confirm mutation"),
            expect_plan: Optional[str] = typer.Option(None, "--expect-plan", help="full hash from exact association review"),
            read_only: bool = typer.Option(False, "--read-only", help="refuse association writes"),
            json_out: bool = typer.Option(False, "--json", help="emit JSON"),
        ) -> None:
            try:
                change_associations_resource(id_, None, [], list(tickets_), dry_run, yes, json_out, **({"expect_plan": expect_plan, "read_only": read_only} if expect_plan is not None or read_only else {}))
            except (SessionError, APIError, ValueError) as e:
                _err_exc(e)

        @associations_app.command("pick", help="Interactively pick ticket(s) to associate.", epilog="[bold]Examples:[/bold]  fsv changes associations pick CHN-1234 --yes --expect-plan FULL_HASH  |  fsv changes associations pick CHN-1234 --dry-run")
        def associations_pick(
            id_: str = typer.Argument(..., metavar="CHANGE_ID"),
            dry_run: bool = typer.Option(False, "--dry-run", help="print resolved payload without mutating"),
            yes: bool = typer.Option(False, "--yes", "-y", help="confirm mutation"),
            expect_plan: Optional[str] = typer.Option(None, "--expect-plan", help="full hash from exact association review"),
            read_only: bool = typer.Option(False, "--read-only", help="refuse association writes"),
            json_out: bool = typer.Option(False, "--json", help="emit JSON"),
        ) -> None:
            try:
                change_associations_resource(id_, None, [], [], dry_run, yes, json_out, pick=True, **({"expect_plan": expect_plan, "read_only": read_only} if expect_plan is not None or read_only else {}))
            except (SessionError, APIError, ValueError) as e:
                _err_exc(e)

        sub.add_typer(associations_app, name="associations")

    if res.name in {"tickets", "problems", "changes"}:
        from fsv.integrated_assets_cli import register as register_integrated_assets
        register_integrated_assets(sub, res.name, lambda: _client())

    from fsv.list_preferences_cli import register as register_list_preferences
    register_list_preferences(sub, res, lambda: _client())
    if res.name == "changes":
        from fsv.asset_picker_cli import register as register_asset_picker
        register_asset_picker(sub, lambda: _client())
    if res.name == "problems":
        from fsv.problem_cli import register as register_problem
        register_problem(sub, lambda: _client())
        from fsv.solutions_cli import register_links
        register_links(sub, lambda: _client())
    if res.name in {"changes", "problems"}:
        from fsv.planning_cli import register as register_planning
        register_planning(sub, res, lambda: _client())
    if res.name in {"tickets", "changes"}:
        from fsv.watcher_cli import register as register_watchers
        register_watchers(sub, res, lambda: _client())
    if res.name == "tickets":
        from fsv.ticket_tasks_cli import register as register_tasks
        register_tasks(sub, lambda: _client())
        from fsv.ticket_note_mutation_cli import register as register_note_mutation
        register_note_mutation(sub, lambda: _client())
        from fsv.ticket_assignment_cli import register as register_assignment
        register_assignment(sub, lambda: _client())
        from fsv.ticket_merge_cli import register as register_merge
        register_merge(sub, lambda: _client())
        from fsv.ticket_lifecycle_cli import register as register_lifecycle
        register_lifecycle(sub, lambda: _client())
        from fsv.ticket_time_cli import register as register_time
        from fsv.bulk_priority_cli import register as register_bulk_priority
        from fsv.canned_cli import register as register_canned
        from fsv.private_view_cli import register as register_private_views
        register_private_views(sub, lambda: _client())
        register_canned(sub, lambda: _client())
        register_time(sub, lambda: _client())
        register_bulk_priority(sub, lambda: _client())
        from fsv.ticket_bulk_cli import register as register_bulk
        register_bulk(sub, lambda: _client())

    @sub.command("fields", help=f"List discoverable {singular} fields.", epilog=(
        f"[bold]Examples:[/bold]  fsv {res.name} fields  |  fsv {res.name} fields --required  |  fsv {res.name} fields --choices status  |  fsv {res.name} fields --custom"
    ))
    def fields(
        search: Optional[str] = typer.Argument(None, help="field name/label search", autocompletion=completion.complete_field_names(res)),
        default: bool = typer.Option(False, "--default", help="show portable Freshservice fields only"),
        custom: bool = typer.Option(False, "--custom", help="show tenant custom fields only"),
        required: bool = typer.Option(False, "--required", help="show only fields required to create"),
        choices: Optional[str] = typer.Option(None, "--choices", help="show choices for a field", autocompletion=completion.complete_choice_field_names(res)),
        refresh: bool = typer.Option(False, "--refresh"),
        format_: OutputFormat = typer.Option(OutputFormat.table, "--output", "-o", help="output format", autocompletion=completion.complete_format),
        json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
    ) -> None:
        f"""List discoverable {res.name} fields."""
        try:
            fields_resource(res, search, default, custom, choices, refresh, json_out, required, format_)
        except (SessionError, APIError) as e:
            _err_exc(e)
        except typer.Exit:
            raise
        except Exception as e:
            _err_exc(e)

    @sub.command("lookup", help="Look up users, groups, or field choices.", epilog=(
        f"[bold]Examples:[/bold]  fsv {res.name} lookup requester me@example.com  |  fsv {res.name} lookup group desk"
    ))
    def lookup(
        kind: str = typer.Argument(..., help="requester | agent | group | field name/label", autocompletion=completion.complete_lookup_kind(res)),
        query: str = typer.Argument("", help="name/email/text to search", autocompletion=completion.complete_lookup_query(res)),
        refresh: bool = typer.Option(False, "--refresh", help="fetch current schema choices synchronously"),
        format_: OutputFormat = typer.Option(OutputFormat.table, "--output", "-o", help="output format", autocompletion=completion.complete_format),
        json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
    ) -> None:
        f"""Autocomplete users, groups, or field choices."""
        try:
            lookup_resource(res, kind, query, json_out, format_, refresh=refresh)
        except (SessionError, APIError) as e:
            _err_exc(e)
        except typer.Exit:
            raise
        except Exception as e:
            _err_exc(e)

    if res.filters_path:
        @sub.command("views", help="List saved view names.", epilog=(
            f"[bold]Examples:[/bold]  fsv {res.name} views  |  fsv {res.name} ls --view all"
        ))
        def views(
            format_: OutputFormat = typer.Option(OutputFormat.table, "--output", "-o", help="output format", autocompletion=completion.complete_format),
            json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
        ) -> None:
            """List saved view names."""
            views_resource(res, format_, json_out)

    @sub.command("search", help=f"Full-text keyword search {res.name}.", epilog=(
        f"[bold]Examples:[/bold]  fsv {res.name} search 'AcmeApp'  |  fsv {res.name} search 'AcmeApp' --npages 3  |  fsv {res.name} search 'AcmeApp' --all  |  fsv {res.name} search 'data migration' --json"
    ))
    def search(
        query: str = typer.Argument(..., help="free-text keyword(s)"),
        page: int = typer.Option(1, "--page", "-p"),
        sort: SearchSort = typer.Option(SearchSort.relevance, "--sort", help="relevance | created | modified", autocompletion=completion.complete_search_sort),
        format_: OutputFormat = typer.Option(OutputFormat.table, "--output", "-o", help="output format", autocompletion=completion.complete_format),
        json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
        n_pages: Optional[int] = typer.Option(None, "--npages", "-N", help="fetch exactly N pages (30 results each)"),
        all_pages: bool = typer.Option(False, "--all", "-a", help="fetch all pages (auto-paginate)"),
    ) -> None:
        f"""Full-text keyword search {res.name}."""
        search_resource(res, query, page, json_out, sort, all_pages, n_pages, format_)

    @sub.command("url", help=f"Print {singular} browser URL.", epilog=(
        f"[bold]Examples:[/bold]  fsv {res.name} url {_pfx}-1234"
    ))
    def url(id_: str = typer.Argument(..., metavar="ID")) -> None:
        f"""Print the browser URL for a {res.name[:-1]}."""
        url_resource(res, id_)

    if res == CHANGES:
        from fsv import schema as schema_mod
        from fsv import service
        from fsv.create import (
            attach_files_to_change,
            change_clone_data,
            change_template,
            clone_assets,
            clone_planning_fields,
            clone_tasks,
            download_attachment,
            get_change_for_edit,
            match_planning_row,
            planning_server_key,
            resolve_planning_definition,
            set_due_by,
            submit_change,
            update_change,
            update_planning_field,
        )

        @sub.command("update", help="Update change fields.", epilog=(
            f"[bold]Examples:[/bold]  "
            f"fsv changes update {_pfx}-1234 --status 'In Progress'  |  "
            f"fsv changes update {_pfx}-1234 --set 'field name=value' --dry-run  |  "
            f"fsv changes update {_pfx}-1234 --attach report.pdf  |  "
            f"fsv changes update {_pfx}-1234 --planning 'Test Plan' --file plan.docx  |  "
            f"fsv changes update {_pfx}-1234 --status Closed --yes"
        ))
        def update(
            id_: str = typer.Argument(..., metavar="ID"),
            status: Optional[str] = typer.Option(None, "--status", "-s", help="status label or ID", autocompletion=completion.complete_update_choice(res, "status")),
            priority: Optional[str] = typer.Option(None, "--priority", "-p", help="priority label or ID", autocompletion=completion.complete_update_choice(res, "priority")),
            agent_id: Optional[str] = typer.Option(None, "--agent", "--agent-id", help="agent name/email/user ID", autocompletion=completion.complete_update_agent_id),
            group_id: Optional[str] = typer.Option(None, "--group", "--group-id", help="group name or ID", autocompletion=completion.complete_update_group_id),
            edit: bool = typer.Option(False, "--edit", "-e", help="open $EDITOR with all fields"),
            set_: Optional[List[str]] = typer.Option(None, "--set", help="set FIELD=VALUE (repeatable)", autocompletion=completion.complete_set(res)),
            planning: Optional[str] = typer.Option(None, "--planning", help="planning field label/name/id to update", autocompletion=completion.complete_planning_field_names(res)),
            description: Optional[str] = typer.Option(None, "--description", "--desc", help="planning field description HTML/text; use '-' for stdin"),
            files: Optional[List[str]] = typer.Option(None, "--file", "-f", help="file(s) to upload to --planning field"),
            duplicate: str = typer.Option("prompt", "--duplicate", help="same-name attachment behavior: prompt|skip|replace|append", autocompletion=completion.complete_duplicate_mode),
            backup_replaced: Optional[bool] = typer.Option(None, "--backup-replaced/--no-backup-replaced", help="when --duplicate replace, backup old attachment first"),
            backup_name: Optional[str] = typer.Option(None, "--backup-name", help="backup filename for replaced attachment"),
            attach: Optional[List[str]] = typer.Option(None, "--attach", "-a", help="file(s) to attach to main change (Attachments section)"),
            due_by: Optional[str] = typer.Option(None, "--due-by", help="set resolution due date (ISO-8601, e.g. 2026-06-09T18:00:00+07:00)"),
            dry_run: bool = typer.Option(False, "--dry-run", help="print resolved payload without updating"),
            yes: bool = typer.Option(False, "--yes", "-y", help="confirm status changes, reassignment and other notifying writes"),
            json_out: bool = typer.Option(False, "--json", help="emit JSON"),
            no_input: bool = typer.Option(False, "--no-input", help="fail instead of prompting"),
            from_: Optional[str] = typer.Option(None, "--from", help="YAML draft with answers to change (labels or internal names)"),
            preset: Optional[str] = typer.Option(None, "--preset", help="preset name or path (partial answers)"),
            expect_plan: Optional[str] = typer.Option(None, "--expect-plan", help="refuse unless the computed plan hash matches"),
            ack: bool = typer.Option(False, "--ack", help="acknowledge blocking rule messages"),
        ) -> None:
            """Update fields on a change.

            --set, --from, --preset and --edit run the edit form's business
            rules and send only the fields that change (review shows old ->
            new). Legacy --status/--priority/--agent/--group flags use the same
            reviewed, hash-bound form path."""
            try:
                cid = _cid(id_, res)
                no_input = _no_input(no_input)
                # no flags at all in a terminal: the pre-filled interactive edit form
                open_form = not no_input and not any((status, priority, agent_id, group_id, edit, set_, planning,
                                                      description, files, attach, due_by, from_, preset))
                if open_form:
                    from fsv.change_cli import form_tty
                    from fsv.change_flow import interactive
                    open_form = interactive() and form_tty()
                form_flags = edit or set_ or from_ or preset or open_form or any(
                    v is not None for v in (status, priority, agent_id, group_id))
                if form_flags and due_by is None and planning is None:
                    from fsv.change_cli import FlowExit, update_command
                    from fsv.forms import FormError
                    try:
                        update_command(
                            _client, cid, from_file=from_, sets=list(set_ or []), preset=preset, edit=edit,
                            attach=list(attach or []),
                            legacy={"status": status, "priority": priority, "agent": agent_id, "group": group_id},
                            duplicate=duplicate, dry_run=dry_run, as_json=json_out, yes=yes,
                            expect_plan=expect_plan, ack=ack, no_input=no_input, status_label=None,
                            subject=format_id({"id": cid}, res), form=open_form,
                        )
                    except FlowExit as e:
                        _flow_exit(e)
                    except FormError as e:
                        _err(str(e), 2)
                    return
                quick_flags = bool(attach or set_ or any(v is not None for v in [status, priority, agent_id, group_id]))
                modes = sum(1 for active in (due_by is not None, planning is not None, edit, quick_flags) if active)
                if modes > 1:
                    _err("choose one update mode: --due-by, --planning, --edit, or field/attachment flags")
                if no_input and duplicate == "prompt" and (files or attach):
                    _err("--duplicate prompt disabled by --no-input; use --duplicate skip|replace|append")

                if due_by is not None:
                    if dry_run:
                        _emit_dry_run(res, cid, {"due_by": due_by}, "set_due_by")
                        return
                    set_due_by(cid, due_by)
                    console.print(f"[green]updated[/] #{cid} resolution due → {due_by}")
                    return

                if planning is not None:
                    planning_def = resolve_planning_definition(  # by label first: names differ per account
                        planning, _api(lambda: schema_mod.load(res, _client())).get("fields", []))
                    planning_id = planning_server_key(planning_def)
                    desc = sys.stdin.read() if description == "-" else description
                    if dry_run:
                        _emit_dry_run(res, cid, {"planning_field": planning_id, "description": desc, "files": list(files or []), "duplicate": duplicate}, "update_planning_field")
                        return
                    updated = update_planning_field(
                        cid,
                        planning_id,
                        description=desc,
                        file_paths=list(files or []),
                        duplicate=duplicate,
                        backup_replaced=backup_replaced,
                        backup_name=backup_name,
                        label=str(planning_def.get("label") or planning),
                    )
                    if json_out:
                        emit_json(updated)
                    elif updated.get("_fsv_noop"):
                        skipped = ", ".join(updated.get("skipped") or [])
                        console.print(f"[yellow]no changes[/] planning field {planning_id!r}" + (f" ({skipped} already attached)" if skipped else ""))
                    else:
                        n_files = len(files or [])
                        suffix = []
                        if desc is not None:
                            suffix.append("description")
                        if n_files:
                            suffix.append(f"{n_files} file(s)")
                        console.print(f"[green]updated[/] planning field {planning_id!r}" +
                                      (f" ({', '.join(suffix)})" if suffix else ""))
                    return

                if edit:
                    current = get_change_for_edit(cid)
                    body = _edit_body(current, f"Editing change #{cid}", no_input)
                    c = _client()
                    sch = _api(lambda: schema_mod.load(res, c))
                    risks = _risks_for_item_write(res, cid, body, sch, c, current=current)
                    if dry_run:
                        _emit_dry_run(res, cid, body, risks=risks)
                        return
                    _confirm_writes(risks, format_id({"id": cid}, res), yes, no_input)
                    updated = update_change(cid, body)
                    if json_out:
                        emit_json(updated)
                    else:
                        console.print(f"[green]updated[/] {format_id(updated, res)}")
                    return

                if attach or set_ or any(v is not None for v in [status, priority, agent_id, group_id]):
                    c = _client()
                    sch = _api(lambda: schema_mod.load(res, c))
                    quick: dict[str, Any] = {}
                    if status is not None:
                        quick["status"] = _resolve_update_choice(sch, "status", status)
                    if priority is not None:
                        quick["priority"] = _resolve_update_choice(sch, "priority", priority)
                    if agent_id is not None:
                        quick["agent_id"] = int(_resolve_agent(c, agent_id))
                    if group_id is not None:
                        quick["group_id"] = int(_resolve_group(c, group_id))
                    for expr in set_ or []:
                        if "=" not in expr:
                            _err(f"--set requires FIELD=VALUE format, got: {expr!r}")
                        field_text, _, raw_value = expr.partition("=")
                        k, v = _resolve_set_field(c, sch, field_text.strip(), raw_value)
                        quick[k] = v
                    risks = _risks_for_item_write(res, cid, quick, sch, c)
                    if not dry_run:
                        _confirm_writes(risks, format_id({"id": cid}, res), yes, no_input)
                    attach_result: dict[str, Any] | None = None
                    if attach:
                        if dry_run:
                            quick["attachments"] = {"preserve_existing": True, "upload_files": list(attach), "duplicate": duplicate}
                        else:
                            attach_result = attach_files_to_change(
                                cid,
                                list(attach),
                                c,
                                duplicate=duplicate,
                                backup_replaced=backup_replaced,
                                backup_name=backup_name,
                            )
                    if dry_run:
                        _emit_dry_run(res, cid, quick, risks=risks)
                        return
                    if quick:
                        updated = update_change(cid, quick)
                        if json_out:
                            emit_json(updated)
                            return
                        console.print(f"[green]updated[/] {format_id(updated, res)}")
                        return
                    if attach_result and attach_result.get("_fsv_noop"):
                        skipped = ", ".join(attach_result.get("skipped") or [])
                        console.print(f"[yellow]no changes[/] {format_id({'id': cid}, res)}" + (f" ({skipped} already attached)" if skipped else ""))
                        return
                    if attach_result:
                        if json_out:
                            emit_json(attach_result)
                            return
                        console.print(f"[green]updated[/] {format_id({'id': cid}, res)}")
                        return

                _err("nothing to update; use --set, --from, --attach, --planning, --edit, or pass "
                     "--status/--priority/--agent/--group (in a terminal, no flags opens the edit form)")
            except (SessionError, APIError) as e:
                _err_exc(e)
            except (SystemExit, typer.Exit):
                raise
            except Exception as e:
                _err_exc(e)

        @sub.command("download", help="Download change attachments.", epilog=(
            "[bold]Examples:[/bold]  "
            "fsv changes download CHN-1234  |  "
            "fsv changes download CHN-1234 --all --dir ./evidence  |  "
            "fsv changes download CHN-1234 --planning 'Test Plan'"
        ))
        def download(
            id_: str = typer.Argument(..., metavar="ID"),
            planning: Optional[List[str]] = typer.Option(None, "--planning", help="planning field label/name/id (repeatable)", autocompletion=completion.complete_planning_field_names(res)),
            all_planning: bool = typer.Option(False, "--all-planning", help="download all planning field attachments"),
            main_attachments: bool = typer.Option(False, "--attachments", "--main-attachments", help="download main change attachments"),
            description_attachments: bool = typer.Option(False, "--description-attachments", help="download attachment links found in description HTML"),
            all_: bool = typer.Option(False, "--all", help="download planning, main, and description attachments"),
            out: Optional[Path] = typer.Option(None, "--dir", "--out", help="output directory (default: ./downloads/CHN-<id>)"),
            force: bool = typer.Option(False, "--force", help="overwrite existing files"),
            json_out: bool = typer.Option(False, "--json", help="emit JSON manifest"),
        ) -> None:
            try:
                cid = _cid(id_, res)
                c = _client()
                if all_:
                    all_planning = True
                    main_attachments = True
                    description_attachments = True
                if not planning and not all_planning and not main_attachments and not description_attachments:
                    all_planning = True
                out_dir = _download_dir(out, f"CHN-{cid}")
                results: list[dict[str, Any]] = []

                evidence: dict[str, Any] | None = None
                if all_planning or planning:
                    evidence = service.get_change_evidence(cid, client=c)
                    fields = evidence["planning_fields"]
                    by_name = evidence["planning_fields_by_name"]
                    selected: list[str] = []
                    if all_planning:
                        selected.extend(str(f.get("name")) for f in fields if f.get("name") and f.get("attachments"))
                    for value in planning or []:
                        defn = resolve_planning_definition(value, _api(lambda: schema_mod.load(res, c)).get("fields", []))
                        row = match_planning_row(defn, fields)
                        selected.append(str((row or {}).get("name") or planning_server_key(defn)))
                    for field_id in dict.fromkeys(selected):
                        field = by_name.get(field_id)
                        attachments = (field or {}).get("attachments") or []
                        if not attachments:
                            results.append({"source": f"planning:{field_id}", "status": "no attachments"})
                            continue
                        for att in attachments:
                            item = download_attachment(att, out_dir, force=force, c=c)
                            item["source"] = f"planning:{field_id}"
                            item["status"] = "skipped" if item.get("skipped") else "downloaded"
                            results.append(item)

                if main_attachments or description_attachments:
                    evidence = evidence or service.get_change_evidence(cid, client=c)
                if main_attachments:
                    for att in (evidence or {}).get("main_attachments") or []:
                        item = download_attachment(att, out_dir, force=force, c=c)
                        item["source"] = "attachments"
                        item["status"] = "skipped" if item.get("skipped") else "downloaded"
                        results.append(item)
                if description_attachments:
                    for url in (evidence or {}).get("description_attachment_urls") or []:
                        att_id = url.rstrip("/").split("/")[-1].split("?")[0]
                        item = download_attachment({"canonical_url": url, "name": f"attachment-{att_id}"}, out_dir, force=force, c=c)
                        item["source"] = "description"
                        item["status"] = "skipped" if item.get("skipped") else "downloaded"
                        results.append(item)

                if json_out:
                    emit_json(results)
                    return
                t = Table(title=f"Downloaded attachments for CHN-{cid}")
                t.add_column("Status", style="cyan")
                t.add_column("Source")
                t.add_column("File")
                t.add_column("Size", justify="right")
                for item in results:
                    t.add_row(str(item.get("status", "")), str(item.get("source", "")), str(item.get("name") or item.get("path") or ""), str(item.get("size", "")))
                console.print(t)
                console.print(f"[green]output[/] {out_dir}")
            except (SessionError, APIError) as e:
                _err_exc(e)
            except (SystemExit, typer.Exit):
                raise
            except Exception as e:
                _err_exc(e)
    else:
        if res == TICKETS:
            from fsv.ticket_notes import register as register_ticket_notes
            register_ticket_notes(sub)

            @sub.command("create", help="Create an incident using the ticket form and business rules.")
            def ticket_create(
                device: Optional[List[int]] = typer.Option(None, "--device", min=1, help="integrated Device inventory ID (repeatable)"),
                from_: Optional[str] = typer.Option(None, "--from", help="YAML draft"),
                set_: Optional[List[str]] = typer.Option(None, "--set", help="answer LABEL=VALUE (repeatable)"),
                preset: Optional[str] = typer.Option(None, "--preset", help="draft preset"),
                template: Optional[str] = typer.Option(None, "--template", help="ticket template name or ID: prefills the form; --set/--from still win"),
                edit: bool = typer.Option(False, "--edit", "-e", help="edit YAML in a terminal"),
                requester: Optional[str] = typer.Option(None, "--requester", help="requester email or exact name"),
                cc: Optional[List[str]] = typer.Option(None, "--cc", help="plain CC email (repeatable)"),
                attach: Optional[List[str]] = typer.Option(None, "--attach", help="reviewed local file (repeatable)"),
                max_bytes: Optional[int] = typer.Option(None, "--max-bytes", min=1, help="explicit total attachment size limit in bytes; replaces the tenant-published limit (required to send when the tenant publishes none)"),
                dry: bool = typer.Option(False, "--dry-run", help="review and show the payload without sending"),
                format_: OutputFormat = typer.Option(OutputFormat.table, "--output", "-o", help="table or json"),
                json_out: bool = typer.Option(False, "--json", help="emit JSON"),
                yes: bool = typer.Option(False, "--yes", "-y", help="confirm create"),
                expect_plan: Optional[str] = typer.Option(None, "--expect-plan", help="reviewed plan hash prefix"),
                allow_duplicate: bool = typer.Option(False, "--allow-duplicate", help="allow a known completed duplicate"),
                ack: bool = typer.Option(False, "--ack", help="acknowledge blocking rule messages"),
                no_input: bool = typer.Option(False, "--no-input", help="never prompt"),
            ) -> None:
                from fsv.change_flow import FlowExit
                from fsv.ticket_cli import create_command
                try:
                    create_command(_client, from_file=from_, sets=list(set_ or []), preset=preset, edit=edit,
                                   requester=requester, dry_run=dry, as_json=json_out or format_ == OutputFormat.json,
                                   yes=yes, expect_plan=expect_plan, allow_duplicate=allow_duplicate, ack=ack,
                                   no_input=_no_input(no_input), cc=list(cc or []), attach=list(attach or []),
                                   max_bytes=max_bytes, devices=list(device or []), template=template)
                except FlowExit as e:
                    _flow_exit(e)
                except (SessionError, APIError) as e:
                    _err_exc(e)
                except (SystemExit, typer.Exit):
                    raise
                except Exception as e:
                    _err_exc(e)

            @sub.command("approvals", help="List ticket approvals for approval-capable tickets; some INC tickets may return 404.", epilog=(
                f"[bold]Examples:[/bold]  fsv {res.name} approvals SR-1234  |  fsv {res.name} approvals SR-1234 --json"
            ))
            def approvals(
                id_: str = typer.Argument(...),
                format_: OutputFormat = typer.Option(OutputFormat.table, "--output", "-o", help="output format", autocompletion=completion.complete_format),
                json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
            ) -> None:
                ticket_approvals_resource(id_, format_, json_out)

            @sub.command("associations", help="List ticket associations.", epilog=(
                f"[bold]Examples:[/bold]  fsv {res.name} associations {_pfx}-1234  |  fsv {res.name} associations {_pfx}-1234 --json"
            ))
            def associations(
                id_: str = typer.Argument(...),
                format_: OutputFormat = typer.Option(OutputFormat.table, "--output", "-o", help="output format", autocompletion=completion.complete_format),
                json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
            ) -> None:
                ticket_associations_resource(id_, format_, json_out)

        @sub.command("update", help=f"Update {singular} fields.", epilog=(
            f"[bold]Examples:[/bold]  "
            f"fsv {res.name} update {_pfx}-1234 --status 'In Progress'  |  "
            f"fsv {res.name} update {_pfx}-1234 --priority High --dry-run  |  "
            f"fsv {res.name} update {_pfx}-1234 --status Closed --yes"
        ))
        def update(
            id_: str = typer.Argument(..., metavar="ID"),
            status: Optional[str] = typer.Option(None, "--status", "-s", help="status label or ID", autocompletion=completion.complete_update_choice(res, "status")),
            priority: Optional[str] = typer.Option(None, "--priority", "-p", help="priority label or ID", autocompletion=completion.complete_update_choice(res, "priority")),
            agent_id: Optional[str] = typer.Option(None, "--agent", "--agent-id", help="agent name/email/user ID", autocompletion=completion.complete_update_agent_id),
            group_id: Optional[str] = typer.Option(None, "--group", "--group-id", help="group name or ID", autocompletion=completion.complete_update_group_id),
            set_: Optional[List[str]] = typer.Option(None, "--set", help="set FIELD=VALUE (repeatable)", autocompletion=completion.complete_set(res)),
            tag: Optional[List[str]] = typer.Option(None, "--tag", help="tickets only: replace all tags with these values (repeatable)"),
            planned_start: Optional[str] = typer.Option(None, "--planned-start", help="tickets and problems: ISO date/time with explicit UTC offset"),
            planned_end: Optional[str] = typer.Option(None, "--planned-end", help="tickets and problems: ISO date/time with explicit UTC offset"),
            planned_effort: Optional[str] = typer.Option(None, "--planned-effort", help="problems only: 1h30m or 01:30 (sent as '1h 30m'), when the problem form has the field"),
            reviewed: bool = typer.Option(False, "--reviewed", help="tickets only: evaluate --set and core flags as a hash-bound property plan"),
            ack: bool = typer.Option(False, "--ack", help="tickets --reviewed only: acknowledge blocking rule messages (bound to the review hash)"),
            expect_plan: Optional[str] = typer.Option(None, "--expect-plan", help="full reviewed hash (ticket property update, problem update)"),
            dry_run: bool = typer.Option(False, "--dry-run", help="print resolved payload without updating"),
            yes: bool = typer.Option(False, "--yes", "-y", help="confirm status changes, reassignment and other notifying writes"),
            json_out: bool = typer.Option(False, "--json", help="emit JSON"),
            no_input: bool = typer.Option(False, "--no-input", help="fail instead of prompting"),
        ) -> None:
            """Update basic fields."""
            if res == PROBLEMS and (planned_start is not None or planned_end is not None or planned_effort is not None):
                _problem_planning_update(id_, planned_start, planned_end, planned_effort, dry_run, yes, expect_plan,
                                         status, priority, agent_id, group_id, set_, tag, reviewed)
                return
            if planned_effort is not None:
                _err("--planned-effort is supported only for problems")
            if reviewed or tag or planned_start is not None or planned_end is not None:
                import json
                from fsv.ticket_properties import PropertyError, build_plan, send
                try:
                    if res != TICKETS:
                        raise PropertyError("--tag and planned dates are supported only for tickets")
                    answers = {}
                    for expr in set_ or []:
                        name, sep, value = expr.partition("=")
                        if not sep or not name.strip() or name.strip() in answers:
                            raise PropertyError("--set requires unique FIELD=VALUE answers")
                        answers[name.strip()] = value
                    for name, value in (("status", status), ("priority", priority), ("agent", agent_id), ("group", group_id)):
                        if value is not None:
                            if name in answers:
                                raise PropertyError("duplicate property answer: " + name)
                            answers[name] = value
                    c = _client()
                    plan = build_plan(c, id_, tags=list(tag) if tag else None,
                                      planned_start=planned_start, planned_end=planned_end, answers=answers, ack=ack)
                    if dry_run:
                        typer.echo(json.dumps({"status": "dry_run", **plan.review()}, ensure_ascii=False, indent=2))
                        return
                    typer.echo(json.dumps(plan.review(), ensure_ascii=False, indent=2), err=True)
                    if expect_plan is not None and expect_plan != plan.hash:
                        raise PropertyError("review hash changed; review again")
                    if yes:
                        if expect_plan != plan.hash:
                            raise PropertyError("--yes requires the full --expect-plan from a prior review")
                    elif _no_input(no_input):
                        raise PropertyError("sending requires --yes and --expect-plan in no-input mode")
                    elif not typer.confirm("Update these exact ticket properties?", default=False):
                        typer.echo("Cancelled; no update attempted.")
                        return
                    result = send(plan, c, reviewed_hash=plan.hash)
                    typer.echo(json.dumps(result, ensure_ascii=False, indent=2))
                except (ValueError, PropertyError) as exc:
                    typer.echo(str(exc), err=True)
                    raise typer.Exit(2) from None
                return
            if expect_plan and res != PROBLEMS:
                _err("--expect-plan requires --reviewed, --tag, --planned-start or --planned-end")
            if ack:
                _err("--ack requires tickets --reviewed")
            update_resource(res, id_, status, priority, agent_id, group_id, json_out, dry_run, yes, no_input, set_,
                            expect_plan=expect_plan)

        _threads_key = "conversations" if not res.has_notes else "notes"
        _threads_flag = "--conversations" if not res.has_notes else "--notes"
        _threads_help = "download attachments from conversations/replies" if not res.has_notes else "download attachments from notes"

        @sub.command("download", help=f"Download {singular} attachments.", epilog=(
            f"[bold]Examples:[/bold]  fsv {res.name} download {_pfx}-1234  |  "
            f"fsv {res.name} download {_pfx}-1234 --all --dir ./evidence"
        ))
        def download_generic(
            id_: str = typer.Argument(..., metavar="ID"),
            main_attachments: bool = typer.Option(False, "--attachments", "--main-attachments", help="download main attachments"),
            threads: bool = typer.Option(False, _threads_flag, help=_threads_help),
            description_attachments: bool = typer.Option(False, "--description-attachments", help="download attachment links found in description HTML"),
            all_: bool = typer.Option(False, "--all", help="download all attachments from all sources"),
            out: Optional[Path] = typer.Option(None, "--dir", "--out", help="output directory (default: ./downloads/<ID>)"),
            force: bool = typer.Option(False, "--force", help="overwrite existing files"),
            json_out: bool = typer.Option(False, "--json", help="emit JSON manifest"),
        ) -> None:
            try:
                from fsv.create import download_attachment
                cid = _cid(id_, res)
                c = _client()
                if all_:
                    main_attachments = True
                    threads = True
                    description_attachments = True
                if not main_attachments and not threads and not description_attachments:
                    main_attachments = True
                pfx = res.display_prefixes[0]
                out_dir = _download_dir(out, f"{pfx}-{cid}")
                results: list[dict[str, Any]] = []

                raw = c.int_get(f"{res.api_path}/{cid}", params={"include": "requester,stats"})
                item_data = raw.get(res.item_key, raw)

                if main_attachments:
                    for att in item_data.get("attachments") or []:
                        r = download_attachment(att, out_dir, force=force, c=c)
                        r["source"] = "attachments"
                        r["status"] = "skipped" if r.get("skipped") else "downloaded"
                        results.append(r)

                if threads:
                    from fsv.views import collect_pages
                    thread_rows, thread_meta, _ = collect_pages(
                        c, f"{res.api_path}/{cid}/{_threads_key}", _threads_key,
                        {"page": 1, "per_page": 30}, what=_threads_key,
                    )
                    if not thread_meta["complete"]:
                        results.append({"source": _threads_key, "status": "incomplete", "complete": False,
                                        "note": thread_meta["partial"], "meta": thread_meta})
                        err.print(escape(thread_meta["partial"]))
                    for thread in thread_rows:
                        for att in thread.get("attachments") or []:
                            r = download_attachment(att, out_dir, force=force, c=c)
                            r["source"] = _threads_key
                            r["status"] = "skipped" if r.get("skipped") else "downloaded"
                            results.append(r)

                if description_attachments:
                    from html import unescape as _html_unescape
                    html = _html_unescape(str(item_data.get("description") or ""))
                    urls: list[str] = []
                    for m in re.finditer(r"(?:https?://[^\"'<>\s]+)?/helpdesk/attachments/(\d+)(?:\?[^\"'<>\s]+)?", html):
                        url = m.group(0)
                        if url.startswith("/"):
                            url = f"https://{config.DOMAIN}{url}"
                        if url not in urls:
                            urls.append(url)
                    for url in urls:
                        att_id = url.rstrip("/").split("/")[-1].split("?")[0]
                        r = download_attachment({"canonical_url": url, "name": f"attachment-{att_id}"}, out_dir, force=force, c=c)
                        r["source"] = "description"
                        r["status"] = "skipped" if r.get("skipped") else "downloaded"
                        results.append(r)

                if json_out:
                    emit_json(results)
                    return
                t = Table(title=f"Downloaded attachments for {pfx}-{cid}")
                t.add_column("Status", style="cyan")
                t.add_column("Source")
                t.add_column("File")
                t.add_column("Size", justify="right")
                for r in results:
                    t.add_row(str(r.get("status", "")), str(r.get("source", "")), str(r.get("name") or r.get("path") or ""), str(r.get("size", "")))
                console.print(t)
                console.print(f"[green]output[/] {out_dir}")
            except (SessionError, APIError) as e:
                _err_exc(e)

    if res == CHANGES:
        @sub.command("clone", help="Clone a change.", epilog=(
            f"[bold]Examples:[/bold]  "
            f"fsv changes clone {_pfx}-1234  |  "
            f"fsv changes clone {_pfx}-1234 --dry-run  |  "
            f"fsv changes clone {_pfx}-1234 --with-tasks --with-planning"
        ))
        def clone(
            id_: str = typer.Argument(..., metavar="ID", help="change ID to clone (pre-fills form)"),
            no_edit: bool = typer.Option(False, "--no-edit", help="create directly without editing"),
            dry: bool = typer.Option(False, "--dry-run", help="print template to stdout and list side effects on stderr; do not create"),
            with_tasks: bool = typer.Option(False, "--with-tasks", help="clone tasks from source change (owners and past dates are dropped)"),
            keep_task_owners: bool = typer.Option(False, "--keep-task-owners", help="with --with-tasks: keep task owners and past dates (notifies people)"),
            with_assets: bool = typer.Option(False, "--with-assets", help="associate same assets as source change"),
            with_planning: bool = typer.Option(False, "--with-planning", help="clone planning fields (text + attachments)"),
            yes: bool = typer.Option(False, "--yes", "-y", help="confirm --with-tasks"),
            json_out: bool = typer.Option(False, "--json", help="emit JSON"),
            no_input: bool = typer.Option(False, "--no-input", help="fail instead of prompting"),
        ) -> None:
            """Clone a change: pre-fills form and opens $EDITOR.

            The new ID is printed as soon as the change exists. If a later step
            (tasks/assets/planning) fails, the change is NOT rolled back and the
            command exits non-zero: fix the failed step by hand, do not rerun clone.
            """
            from fsv.create import clone_side_effects
            from fsv.errors import exit_code_for
            from fsv.safety import clone_risks
            try:
                cid = _cid(id_, res)
                data = change_clone_data(cid)
                if dry:
                    emit_json(data)
                    if with_tasks or with_assets or with_planning:
                        fx = clone_side_effects(cid)
                        err.print("[bold]side effects if run without --dry-run:[/bold]", highlight=False)
                        if with_tasks:
                            keep = "kept" if keep_task_owners else "dropped"
                            err.print(f"  - {fx['tasks']} task(s) created ({fx['tasks_with_owner']} with an owner; owners/past dates {keep})", highlight=False)
                            err.print("  - tenant workflows may add their own tasks to the new change", highlight=False)
                        if with_assets:
                            err.print(f"  - {fx['assets']} asset(s) associated", highlight=False)
                        if with_planning:
                            err.print(f"  - {fx['planning_fields']} planning field(s) copied, {fx['planning_attachments']} attachment(s) re-uploaded", highlight=False)
                    return
                _confirm_writes(clone_risks(with_tasks), f"clone of {format_id({'id': cid}, res)}", yes, no_input)
                body = data if no_edit else _edit_body(data, f"Cloning change #{cid}", no_input)
                created = submit_change(body)
                cid_out = created.get("id", "?")
                label = format_id(created, res)
                if not json_out:
                    console.print(f"[green]created[/] {label}  {res.portal_url}/{cid_out}")
                else:
                    err.print(f"created {label}", highlight=False)
                if cid_out == "?":
                    err.print("[yellow]warning[/yellow]: change created but the response had no id; see `fsv journal tail`", highlight=False)
                steps: dict[str, str] = {}
                failures: list[BaseException] = []

                def _step(name: str, fn: Callable[[], str]) -> None:
                    try:
                        steps[name] = "ok"
                        err.print(f"  {fn()}", highlight=False)
                    except (SessionError, APIError, ValueError, RuntimeError) as e:
                        steps[name] = f"failed: {e}"
                        failures.append(e)
                        err.print(f"[red]step failed[/red]: {name}: {e}", highlight=False)

                if cid_out != "?":
                    if with_tasks:
                        dropped: list[str] = []

                        def _tasks() -> str:
                            tasks = clone_tasks(cid, cid_out, keep_owners=keep_task_owners, dropped=dropped)
                            return f"cloned {len(tasks)} task(s)"
                        _step("tasks", _tasks)
                        for note in sorted(set(dropped)):
                            err.print(f"  note: {note}", highlight=False)
                        err.print("  note: tenant workflows may have added their own tasks; compare with the source before relying on the copy", highlight=False)
                    if with_assets:
                        _step("assets", lambda: f"associated {len(clone_assets(cid, cid_out))} asset(s)")
                    if with_planning:
                        _step("planning", lambda: f"cloned {len(clone_planning_fields(cid, cid_out))} planning field(s)")
                if json_out:
                    out = dict(created)
                    if steps:
                        out["fsv_clone_steps"] = steps
                    emit_json(out)
                if failures:
                    err.print(f"[red]error[/red]: {label} WAS created but {len(failures)} step(s) failed; do not rerun clone (it would duplicate the change). Fix the failed step(s) on {label} directly.", highlight=False)
                    raise typer.Exit(exit_code_for(failures[0]))
            except (SessionError, APIError) as e:
                _err_exc(e)
            except (SystemExit, typer.Exit):
                raise
            except Exception as e:
                _err_exc(e)

        @sub.command("create", help="Create a change through the web form's own fields and rules.", epilog=(
            "[bold]Examples:[/bold]  "
            "fsv changes create --from change.yaml --dry-run  |  "
            "fsv changes create --from change.yaml --yes --expect-plan 3f2a9c1e0b7d  |  "
            "fsv changes create --preset acme-normal --set 'Subject=Rotate keys' --edit  |  "
            "fsv changes create --dry-run > draft.yaml  |  "
            "fsv changes create --subject 'Patch DB' --set 'Change Type=Normal' --requester jane@example.com --dry-run"
        ))
        def create(
            device: Optional[List[int]] = typer.Option(None, "--device", min=1, help="integrated Device inventory ID (repeatable)"),
            from_: Optional[str] = typer.Option(None, "--from", help="draft YAML (answers by label, description_md, attachments, planning)"),
            set_: Optional[List[str]] = typer.Option(None, "--set", help="answer LABEL=VALUE (label or internal name; repeatable)", autocompletion=completion.complete_set(res)),
            preset: Optional[str] = typer.Option(None, "--preset", help="preset name (config presets/NAME.yaml) or path"),
            edit: bool = typer.Option(False, "--edit", "-e", help="advanced: edit an annotated YAML draft in $EDITOR (default in a terminal: the interactive form)"),
            requester: Optional[str] = typer.Option(None, "--requester", help="requester email or exact name (default: config defaults.requester)"),
            attach: Optional[List[str]] = typer.Option(None, "--attach", "-a", help="file to attach (uploaded only after you confirm; repeatable)"),
            max_bytes: Optional[int] = typer.Option(None, "--max-bytes", min=1, help="total byte limit for reviewed before-create uploads"),
            attach_flow: Optional[str] = typer.Option(None, "--attach-flow", help="after-create (default: create, upload, PUT ids) | before-create (upload, send ids in the create body); config changes.attach_flow"),
            dry: bool = typer.Option(False, "--dry-run", help="print the review, exact payload and plan hash; send nothing"),
            format_: OutputFormat = typer.Option(OutputFormat.table, "--output", "-o", help="table|json (json: {plan_hash, payload, review, warnings})", autocompletion=completion.complete_format),
            json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
            yes: bool = typer.Option(False, "--yes", "-y", help="send without the interactive review (required without a terminal)"),
            expect_plan: Optional[str] = typer.Option(None, "--expect-plan", help="refuse unless the computed plan hash matches (prefix >= 12 chars)"),
            allow_duplicate: bool = typer.Option(False, "--allow-duplicate", help="send even if the journal shows this exact plan in the last 24h"),
            ack: bool = typer.Option(False, "--ack", help="acknowledge blocking rule messages"),
            no_input: bool = typer.Option(False, "--no-input", help="never prompt or open an editor"),
            subject: Optional[str] = typer.Option(None, "--subject", help="subject (same as --set Subject=...)"),
            description: Optional[str] = typer.Option(None, "--description", "--desc", help="description HTML/text; '-' for stdin"),
            status: Optional[str] = typer.Option(None, "--status", "-s", help="status on create (default: the form default; others need confirmation)", autocompletion=completion.complete_update_choice(res, "status")),
            priority: Optional[str] = typer.Option(None, "--priority", "-p", help="priority label", autocompletion=completion.complete_update_choice(res, "priority")),
            agent_id: Optional[str] = typer.Option(None, "--agent", "--agent-id", help="agent name/email/ID", autocompletion=completion.complete_update_agent_id),
            group_id: Optional[str] = typer.Option(None, "--group", "--group-id", help="group name or ID", autocompletion=completion.complete_update_group_id),
            no_validate: bool = typer.Option(False, "--no-validate", help="skip the client-side required-field check (the server still checks)"),
            optional: bool = typer.Option(False, "--optional", "-O", hidden=True, help="(ignored; the draft lists every visible field)"),
            all_fields: bool = typer.Option(False, "--all", hidden=True, help="(ignored; the draft lists every visible field)"),
        ) -> None:
            """Create a new change.

            Every input (YAML draft, --set, preset, the flags) is replayed
            through the form's own fields and business rules, then shown on a
            review screen. Status is never asked: the form default is sent.
            Attachments upload only after you confirm."""
            from fsv.change_cli import FlowExit, create_command
            from fsv.forms import FormError
            try:
                desc = sys.stdin.read() if description == "-" else description
                create_command(
                    _client,
                    from_file=from_, sets=list(set_ or []), preset=preset, edit=edit, requester=requester,
                    attach=list(attach or []),
                    legacy={"subject": subject, "description": desc, "status": status, "priority": priority,
                            "agent": agent_id, "group": group_id},
                    dry_run=dry, as_json=json_out or format_ == OutputFormat.json, yes=yes,
                    expect_plan=expect_plan, allow_duplicate=allow_duplicate, ack=ack,
                    no_input=_no_input(no_input), no_validate=no_validate, portal_url=res.portal_url,
                    attach_flow=attach_flow, devices=list(device or []), max_bytes=max_bytes,
                )
            except FlowExit as e:
                _flow_exit(e)
            except FormError as e:
                _err(str(e), 2)
            except (SessionError, APIError) as e:
                _err_exc(e)
            except (SystemExit, typer.Exit):
                raise
            except Exception as e:
                _err_exc(e)

    if res.has_notes:
        @sub.command("notes", help=f"List {singular} notes.", epilog=(
            f"[bold]Examples:[/bold]  "
            f"fsv {res.name} notes {_pfx}-1234  |  "
            f"fsv {res.name} notes {_pfx}-1234 --json"
        ))
        def notes(
            id_: str = typer.Argument(...),
            page: int = typer.Option(1, "--page", "-p"),
            per_page: int = typer.Option(30, "--per-page", "-n"),
            format_: OutputFormat = typer.Option(OutputFormat.table, "--output", "-o", help="output format", autocompletion=completion.complete_format),
            json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
            all_pages: bool = typer.Option(False, "--all", "-a", help="fetch all pages (auto-paginate)"),
            n_pages: Optional[int] = typer.Option(None, "--npages", "-N", help="fetch exactly N pages"),
        ) -> None:
            """List notes using the browser-session internal endpoint."""
            notes_resource(res, id_, page, per_page, json_out, all_pages, n_pages, format_)

        @sub.command("add-note", help=f"Add {singular} note.", epilog=(
            f"[bold]Note:[/bold] Changes/problems use add-note; ticket notes have a separate reviewed flow (docs/ticket-notes.md).  "
            f"[bold]Examples:[/bold]  "
            f"fsv {res.name} add-note {_pfx}-1234 'Checked deployment evidence'"
            + (f"  |  fsv {res.name} add-note {_pfx}-1234 'Visible update' --public" if res.name != "problems" else
               "  (problem notes have no visibility key: the tenant default applies)")
        ))
        def add_note(
            id_: str = typer.Argument(...),
            body: str = typer.Argument(..., help="HTML or plain text"),
            public: bool = typer.Option(False, "--public", help="changes only: public note (problem notes have no visibility key)"),
            dry_run: bool = typer.Option(False, "--dry-run", help="print the note without adding it"),
            yes: bool = typer.Option(False, "--yes", "-y", help="confirm a public note"),
            no_input: bool = typer.Option(False, "--no-input", help="fail instead of prompting"),
        ) -> None:
            note_resource(res, id_, body, public, dry_run, yes, no_input)
    else:
        @sub.command("conversations", help="List ticket conversations/replies.", epilog=(
            f"[bold]Examples:[/bold]  "
            f"fsv {res.name} conversations {_pfx}-1234  |  "
            f"fsv {res.name} conversations {_pfx}-1234 --json"
        ))
        def conversations(
            id_: str = typer.Argument(...),
            page: int = typer.Option(1, "--page", "-p"),
            per_page: int = typer.Option(30, "--per-page", "-n"),
            format_: OutputFormat = typer.Option(OutputFormat.table, "--output", "-o", help="output format", autocompletion=completion.complete_format),
            json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
            all_pages: bool = typer.Option(False, "--all", "-a", help="fetch all pages (auto-paginate)"),
            n_pages: Optional[int] = typer.Option(None, "--npages", "-N", help="fetch exactly N pages"),
        ) -> None:
            conversations_resource(id_, page, per_page, json_out, all_pages, n_pages, format_)

        @sub.command("reply", help="Add a ticket reply.", epilog=(
            f"[bold]Note:[/bold] Reply sends a public response; for reviewed ticket notes use add-note (docs/ticket-notes.md).  "
            f"[bold]Examples:[/bold]  "
            f"fsv {res.name} reply {_pfx}-1234 'Working on this'"
        ))
        def reply(
            id_: str = typer.Argument(...),
            body: str = typer.Argument(...),
            cc: Optional[List[str]] = typer.Option(None, "--cc", help="CC email from user lookup (repeatable)"),
            bcc: Optional[List[str]] = typer.Option(None, "--bcc", help="BCC plain email from user lookup (repeatable; requires --sender)"),
            attach: Optional[List[str]] = typer.Option(None, "--attach", help="local attachment (repeatable; requires full review and --sender)"),
            inline_image: Optional[List[str]] = typer.Option(None, "--inline-image", help="local JPEG/PNG/GIF/WebP shown inside the reply body (repeatable; requires --sender; reviewed upload)"),
            max_bytes: Optional[int] = typer.Option(None, "--max-bytes", min=1, help="explicit total attachment size limit in bytes; replaces the tenant-published limit (required to send when the tenant publishes none)"),
            json_out: bool = typer.Option(False, "--json"),
            sender: Optional[str] = typer.Option(None, "--sender", help="explicit configured support mailbox; not inferred"),
            set_status: Optional[str] = typer.Option(None, "--set-status", help="send and set status (Pending, Resolved or Closed; runtime requirements apply)"),
            ack: bool = typer.Option(False, "--ack", help="with --set-status Resolved/Closed: acknowledge blocking rule messages (bound to the review hash)"),
            expect_plan: Optional[str] = typer.Option(None, "--expect-plan", help="full hash from prior reply review"),
            dry_run: bool = typer.Option(False, "--dry-run", help="print the reply without sending it"),
            yes: bool = typer.Option(False, "--yes", "-y", help="confirm sending the reply"),
            no_input: bool = typer.Option(False, "--no-input", help="fail instead of prompting"),
        ) -> None:
            """Add a reply (conversation) to a ticket."""
            if ack and set_status is None:
                typer.echo("--ack acknowledges blocking rule messages of a --set-status Resolved/Closed transition; "
                           "pass it together with --set-status", err=True)
                raise typer.Exit(2)
            if inline_image:
                from fsv import inline_images
                from fsv.reviewed_write import ReviewedWriteError
                try:
                    if attach or cc or bcc or set_status:
                        raise ReviewedWriteError("--inline-image with --attach/--cc/--bcc/--set-status is not a captured "
                                                 "combination; nothing was sent")
                    client = _client()
                    rebuild = lambda: inline_images.plan(client, id_, body, list(inline_image), kind="reply",
                                                         sender=sender, max_bytes=max_bytes)
                    inline_images.finish(rebuild(), client, dry_run=dry_run, yes=yes, expected=expect_plan, rebuild=rebuild)
                    return
                except (ValueError, ReviewedWriteError) as exc:
                    typer.echo(str(exc), err=True)
                    raise typer.Exit(2) from None
            if attach:
                from fsv import attachments
                from fsv.attachments_cli import finish
                from fsv.reviewed_write import ReviewedWriteError
                try:
                    if cc or bcc or set_status:
                        raise ReviewedWriteError("combined attachment/recipient/status variant not proven")
                    client = _client()
                    rebuild = lambda: attachments.conversation_plan(client, id_, body, list(attach), kind="reply", sender=sender,
                                                                    max_bytes=max_bytes)
                    finish(rebuild(), client, dry_run=dry_run, yes=yes, expected=expect_plan, rebuild=rebuild)
                    return
                except (ValueError, ReviewedWriteError) as exc:
                    typer.echo(str(exc), err=True)
                    raise typer.Exit(2) from None
            if not cc and not bcc and sender is None and set_status is None and expect_plan is None and not ack:
                reply_resource(res, id_, body, dry_run, yes, no_input)
                return
            import json
            from fsv.ticket_conversation import ConversationError, build_plan, recipients, send
            try:
                c = _client()
                recipients(c, list(bcc or []))
                plan = build_plan(c, id_, body, cc=recipients(c, list(cc or [])), bcc=tuple(bcc or ()),
                                  sender=sender, set_status=set_status, ack=ack)
                if dry_run:
                    typer.echo(json.dumps({"status": "dry_run", **plan.review()}, ensure_ascii=False, indent=2))
                    return
                typer.echo(json.dumps(plan.review(), ensure_ascii=False, indent=2), err=True)
                if expect_plan is not None and expect_plan != plan.hash:
                    raise ConversationError("review hash changed; review again")
                if yes:
                    if expect_plan != plan.hash:
                        raise ConversationError("--yes requires the full --expect-plan from a prior reply review")
                elif _no_input(no_input):
                    raise ConversationError("sending requires --yes and --expect-plan in no-input mode")
                elif not typer.confirm("Send this exact PUBLIC reply?", default=False):
                    typer.echo("Cancelled; no reply sent.")
                    return
                result = send(plan, c, reviewed_hash=plan.hash, public_confirmed=True)
                typer.echo(json.dumps(result, ensure_ascii=False, indent=2))
            except (ValueError, ConversationError) as exc:
                typer.echo(str(exc), err=True)
                raise typer.Exit(2) from None

        @sub.command("forward", help="Review a whole-ticket or exact-conversation forward.")
        def forward(
            id_: str = typer.Argument(..., metavar="ID"),
            body: str = typer.Argument(...),
            conversation: Optional[int] = typer.Option(None, "--conversation", min=1, help="exact source conversation ID in this ticket"),
            to: List[str] = typer.Option(..., "--to", help="plain recipient email (repeatable)"),
            sender: str = typer.Option(..., "--sender", help="configured support mailbox; not inferred"),
            output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o", help="output format (JSON only)"),
            json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
            dry_run: bool = typer.Option(False, "--dry-run"),
            yes: bool = typer.Option(False, "--yes"),
            expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
        ) -> None:
            import json
            from fsv.ticket_conversation import ConversationError, build_plan, recipients, send
            try:
                c = _client()
                recipients(c, to)
                plan = build_plan(c, id_, body, kind="forward", notify=tuple(to), sender=sender, conversation_id=conversation)
                if dry_run:
                    typer.echo(json.dumps({"status": "dry_run", **plan.review()}, indent=2))
                    return
                if not yes or expect_plan != plan.hash:
                    raise ConversationError("review with --dry-run, then pass --yes and the full --expect-plan hash")
                typer.echo(json.dumps(send(plan, c, reviewed_hash=expect_plan, public_confirmed=True), indent=2))
            except (ValueError, ConversationError) as exc:
                typer.echo(str(exc), err=True)
                raise typer.Exit(2) from None

    return sub


def _resolve(name: str) -> Resource:
    name = name.lower().rstrip("s") + "s"
    res = REGISTRY.get(name)
    if not res:
        raise typer.BadParameter(f"unknown resource '{name}'; expected: {list(REGISTRY)}")
    return res


@app.command("search", help="Global search across all entities (tickets, problems, changes, tasks, assets, solutions).", epilog=(
    "[bold]Examples:[/bold]  fsv search 'AcmeApp'  |  fsv search 'AcmeApp' --npages 3  |  fsv search 'AcmeApp' --all  |  fsv search 'billing service' --json"
))
def global_search_cmd(
    query: str = typer.Argument(..., help="Free-text term, e.g. 'AcmeApp'"),
    page: int = typer.Option(1, "--page", "-p"),
    sort: SearchSort = typer.Option(SearchSort.relevance, "--sort", help="relevance | created | modified", autocompletion=completion.complete_search_sort),
    format_: OutputFormat = typer.Option(OutputFormat.table, "--output", "-o", help="output format", autocompletion=completion.complete_format),
    json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
    n_pages: Optional[int] = typer.Option(None, "--npages", "-N", help="fetch exactly N pages (30 results each)"),
    all_pages: bool = typer.Option(False, "--all", "-a", help="fetch all pages (auto-paginate)"),
) -> None:
    """Full-text search across every entity type via /search/all."""
    try:
        global_search(query, page, format_, json_out, sort, all_pages, n_pages)
    except (SessionError, APIError) as e:
        _err_exc(e)
    except typer.Exit:
        raise
    except Exception as e:
        _err_exc(e)


# ── setup wizard ──────────────────────────────────────────────


def _setup_show_status() -> None:
    from fsv.session import session_age_hours, current_backend

    table = Table(title="Current status", show_header=False)
    table.add_column("Setting", style="bold")
    table.add_column("Value")

    table.add_row("domain", config.DOMAIN or "[dim](not set)[/dim]")

    age = session_age_hours()
    if age is not None:
        backend = current_backend()
        table.add_row("auth", f"[green]✓[/green] {backend}, {age:.1f}h ago")
    else:
        table.add_row("auth", "[red]✗[/red] not logged in")

    installed_shells: list[str] = []
    if (Path.home() / ".config" / "fish" / "completions" / "fsv.fish").exists():
        installed_shells.append("fish")
    for sh in ("bash", "zsh"):
        if (Path.home() / f".fsv-complete.{sh}").exists():
            installed_shells.append(sh)
    if installed_shells:
        table.add_row("completion", f"[green]✓[/green] {', '.join(installed_shells)}")
    else:
        table.add_row("completion", "[red]✗[/red] not installed")

    network = (config.load_config().get("completion") or {}).get("network", "[dim](unset)[/dim]")
    table.add_row("network completion", str(network))

    for name in REGISTRY:
        exprs = config.get_default_where(name)
        val = ", ".join(exprs) if exprs else "[dim](none)[/dim]"
        table.add_row(f"defaults.{name}", val)

    console.print(table)


def _setup_domain() -> None:
    console.print("\n[bold]── Domain ──[/bold]")
    _resolve_domain()
    console.print(f"domain set to [bold]{config.DOMAIN}[/bold]")


def _setup_auth() -> None:
    console.print("\n[bold]── Authentication ──[/bold]")
    if not config.DOMAIN:
        console.print("domain required first")
        _resolve_domain()

    from fsv.client import get_client, reset_client
    from fsv.session import login_interactive, save_cookies

    try:
        cookies = login_interactive()
    except SessionError as e:
        _err_exc(e)

    _validate_new_cookies(cookies)
    store = _choose_store()
    backend = save_cookies(cookies, store)
    target = "Keychain" if backend == "keychain" else config.display_path(config.SESSION_FILE)
    console.print(f"saved {len(cookies)} cookies to {target}")

    try:
        reset_client()
        c = get_client()
        agent = c.check_session()
        name = f"{agent.get('first_name', '')} {agent.get('last_name', '')}".strip()
        email = agent.get("email", "")
        if agent:
            console.print(f"logged in as: [bold]{name}[/bold] <{email}>")
        else:
            console.print("logged in (identity unavailable: agent profile is not readable)")
        err.print("warming cache in background...", highlight=False)
        _refresh_cache(verbose=False, blocking=False)
    except Exception as e:
        err.print(f"[yellow]warning[/yellow]: saved but could not verify: {e}")


def _setup_completion_install() -> None:
    console.print("\n[bold]── Shell completion ──[/bold]")
    from fsv.completion_gen import build_script

    shell_name = _shell_from_env()
    if not shell_name:
        err.print("[yellow]could not detect shell; skipping[/yellow]")
        return

    prog_name = Path(sys.argv[0]).name

    if shell_name == "fish":
        comp_path = Path.home() / ".config" / "fish" / "completions" / f"{prog_name}.fish"
    elif shell_name in ("bash", "zsh"):
        comp_path = Path.home() / f".{prog_name}-complete.{shell_name}"
    else:
        comp_path = None

    if comp_path and comp_path.exists():
        console.print(f"already installed: {comp_path}")
        if not typer.confirm("Reinstall?", default=False):
            return
    else:
        if not typer.confirm(f"Install {shell_name} completion?", default=True):
            return

    script = build_script(shell_name, prog_name)

    if shell_name == "fish":
        fish_dir = Path.home() / ".config" / "fish" / "completions"
        fish_dir.mkdir(parents=True, exist_ok=True)
        path = fish_dir / f"{prog_name}.fish"
        path.write_text(script)
        console.print(f"installed: {path}")
    elif shell_name in ("bash", "zsh"):
        script_path = Path.home() / f".{prog_name}-complete.{shell_name}"
        script_path.write_text(script)
        rc_file = Path.home() / (".bashrc" if shell_name == "bash" else ".zshrc")
        source_line = f"\n. {script_path}\n"
        existing = rc_file.read_text() if rc_file.exists() else ""
        if str(script_path) not in existing:
            with rc_file.open("a") as f:
                f.write(source_line)
        console.print(f"installed: {script_path}")
        console.print(f"sourced from: {rc_file}")

    console.print("restart shell: exec $SHELL")


def _setup_network_completion() -> None:
    console.print("\n[bold]── Network completion ──[/bold]")
    cfg = config.load_config()
    current = (cfg.get("completion") or {}).get("network", "(unset)")
    console.print(f"current: {current}")
    console.print("when on, TAB fetches live requester/agent names")

    answer = typer.prompt("Enable? [on/off]", default="on").strip().lower()
    if answer not in ("on", "off"):
        err.print("[yellow]expected on/off; skipping[/yellow]")
        return

    cfg.setdefault("completion", {})["network"] = answer
    config.save_config(cfg)
    console.print(f"completion.network = {answer}")


def _setup_defaults() -> None:
    console.print("\n[bold]── Default filters ──[/bold]")
    console.print("set default --where filters for every `ls` query")
    console.print("examples: status=Open, agent=alice@example.com, requester=john@example.com")

    for name in REGISTRY:
        current = config.get_default_where(name)
        hint = f" [{', '.join(current)}]" if current else ""
        answer = typer.prompt(
            f"\n{name} --where (comma-sep, enter to skip){hint}",
            default="",
            show_default=False,
        ).strip()

        if not answer:
            continue

        exprs = [e.strip() for e in answer.split(",") if e.strip()]
        if exprs:
            config.set_default_where(name, exprs)
            console.print(f"  defaults.{name}.where = {exprs}")


_SETUP_ITEMS = [
    ("Domain", "Freshservice instance URL"),
    ("Authentication", "Login with session cookies"),
    ("Shell completion", "Install tab completion"),
    ("Network completion", "Remote requester/agent lookup"),
    ("Default filters", "Default --where per resource"),
    ("All", "Run full setup"),
]


@app.command("setup", epilog="[bold]Examples:[/bold]  fsv setup")
def setup() -> None:
    """Interactive setup wizard."""
    if not sys.stdin.isatty():
        _err("setup requires an interactive terminal")

    _setup_show_status()
    console.print()

    table = Table(show_header=False)
    table.add_column("#", justify="right", style="bold cyan")
    table.add_column("Item", style="bold")
    table.add_column("Description")
    for i, (name, desc) in enumerate(_SETUP_ITEMS, 1):
        table.add_row(str(i), name, desc)
    console.print(table)

    while True:
        answer = typer.prompt("Choose", default="6").strip()
        if answer.isdigit() and 1 <= int(answer) <= len(_SETUP_ITEMS):
            choice = int(answer)
            break
        err.print(f"[red]error[/red]: choose 1-{len(_SETUP_ITEMS)}")

    run_all = choice == 6

    if choice == 1 or run_all:
        _setup_domain()
    if choice == 2 or run_all:
        _setup_auth()
    if choice == 3 or run_all:
        _setup_completion_install()
    if choice == 4 or run_all:
        _setup_network_completion()
    if choice == 5 or run_all:
        _setup_defaults()

    console.print("\n[green]✓[/green] setup complete")


@app.command()
def tui(
    read_only: bool = typer.Option(False, "--read-only", help="disable write actions (status, notes, replies)"),
) -> None:
    """Launch interactive TUI."""
    from fsv.tui import launch
    launch(read_only=read_only)


for r in REGISTRY.values():
    app.add_typer(_make_subapp(r), name=r.name)

from fsv.attachments_cli import register as register_attachments  # noqa: E402
register_attachments(app, lambda: _client())

from fsv.form_cli import form_app  # noqa: E402

app.add_typer(form_app, name="form")

# --- sr sub-app (cli_sr) ---
try:
    from fsv.cli_sr import app as sr_app  # noqa: E402
except ImportError:  # service-request commands not installed yet
    sr_app = None
if sr_app is not None:
    app.add_typer(sr_app, name="sr")
# --- end sr sub-app ---


if __name__ == "__main__":
    app()
