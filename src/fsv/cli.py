from __future__ import annotations

import os
import re
import sys
import time
from enum import Enum
from pathlib import Path
from typing import Any, Callable, Iterable, List, Optional, TypeVar
from urllib.parse import unquote

import typer

from fsv import completion, config, render
from fsv.errors import APIError, SessionError
from fsv.config import CONFIG_DIR, SCHEMA_DIR
from fsv.render import console, emit_json, err, strip_html
from fsv.resources import CHANGES, PROBLEMS, TICKETS, REGISTRY, Resource, format_id, parse_id

_COMPLETING = bool(os.environ.get('_FSV_COMPLETE'))

if not _COMPLETING:
    from rich.markup import escape
    from rich.table import Table
else:
    escape = str  # type: ignore[assignment]
    Table = None  # type: ignore[assignment,misc]

app = typer.Typer(
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
TERMINAL_STATUS_WORDS = {"closed", "resolved", "rejected", "cancelled", "canceled"}


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


def _err(msg: str, code: int = 1) -> None:
    err.print(f"[red]error[/red]: {msg}")
    raise typer.Exit(code) from None


def _api(fn: Callable[[], T]) -> T:
    try:
        t0 = time.time() if VERBOSE else 0.0
        result = fn()
        if VERBOSE:
            err.print(f"[dim]api {(time.time()-t0)*1000:.0f}ms[/]")
        return result
    except (SessionError, APIError, ValueError) as e:
        _err(str(e))


def _client():
    from fsv.client import get_client
    from fsv.session import session_age_hours
    if session_age_hours() is None:
        err.print("[red]error[/red]: no session; run `fsv auth login`")
        raise SystemExit(1)
    try:
        return get_client()
    except (SessionError, RuntimeError) as e:
        err.print(f"[red]error[/red]: {e}")
        raise SystemExit(1)


def _edit_body(data: dict[str, Any], hint: str, no_input: bool = False) -> dict[str, Any]:
    """Open $EDITOR with JSON data, return edited body."""
    from fsv.editor import edit_json, EditorAbort
    if _no_input(no_input):
        _err("editor disabled by --no-input; use --dry-run/--dry or provide non-interactive flags")
    from fsv.editor import edit_json, EditorAbort
    try:
        return edit_json(data, hint)
    except EditorAbort as e:
        _err(str(e))


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
        _err(str(e))


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
    t.add_column("choices", style="dim")
    for f in fields:
        ch = f.get("choices") or []
        t.add_row(
            f.get("name", "?"),
            f.get("label", "?"),
            f.get("field_type", "?"),
            _field_scope(f),
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
        if json_out:
            emit_json(items)
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
    if json_out:
        emit_json(fields)
        return
    _show_fields_table(fields)


def lookup_resource(res: Resource, kind: str, query: str, json_out: bool) -> None:
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
        sch = _api(lambda: schema_mod.load(res, c))
        f = _find_schema_field(sch, kind)
        if not f:
            _err(f"unknown lookup kind or field: {kind}")
        items = [x for x in f.get("choices") or [] if not query or query.casefold() in str(x.get("value") or x.get("label") or x.get("name") or x.get("id")).casefold()]
    if json_out:
        emit_json(items)
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
    return loaded


def _pick_lookup(kind: str, value: str, results: list[dict[str, Any]], exact_keys: tuple[str, ...]) -> dict[str, Any]:
    if not results:
        _err(f"no {kind} match: {value}")
    folded = value.casefold()
    for item in results:
        if any(str(item.get(k) or "").casefold() == folded for k in exact_keys):
            return item
    if len(results) == 1:
        return results[0]
    choices = ", ".join(str(x.get("details") or x.get("value") or x.get("id")) for x in results[:5])
    _err(f"multiple {kind} matches for {value}: {choices}")


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
    folded = value.casefold()
    groups = [
        g for g in data.get("groups", [])
        if folded in str(g.get("name") or "").casefold() or str(g.get("id") or "") == value
    ]
    item = _pick_lookup("group", value, groups, ("name", "id"))
    group_id = item.get("id")
    if group_id is None:
        _err(f"group match has no id: {value}")
    return str(group_id)


def _resolve_update_choice(sch: dict[str, Any], field_name: str, raw: str) -> int | str:
    from fsv import schema as schema_mod
    if raw.isdigit():
        return int(raw)
    f = _find_schema_field(sch, field_name)
    if f:
        folded = raw.casefold()
        for c in f.get("choices") or []:
            label = str(c.get("value") or c.get("name") or c.get("label") or "")
            if label.casefold() == folded:
                return c.get("id", c.get("value", raw))
    fallback = {"priority": schema_mod.PRIORITY}.get(field_name, {})
    for cid, label in fallback.items():
        if label.casefold() == raw.casefold():
            return cid
    choices = []
    if f:
        choices = [str(c.get("value") or c.get("name") or c.get("label") or c.get("id")) for c in f.get("choices") or []]
    if field_name == "priority" and not choices:
        choices = list(schema_mod.PRIORITY.values())
    suffix = f"; choices: {', '.join(choices[:12])}" if choices else "; use numeric ID"
    _err(f"unknown {field_name}: {raw!r}{suffix}")


def _status_label(sch: dict[str, Any], status: int | str) -> str:
    from fsv import schema as schema_mod
    label = schema_mod.choice_label("status", status, sch)
    return label if label != "-" else str(status)


def _is_terminal_status(label: str) -> bool:
    folded = label.casefold()
    return any(word in folded for word in TERMINAL_STATUS_WORDS)


def _confirm_terminal_status(res: Resource, cid: int, sch: dict[str, Any], status: int | str, yes: bool, no_input: bool = False) -> None:
    label = _status_label(sch, status)
    if not _is_terminal_status(label) or yes:
        return
    if _no_input(no_input) or not sys.stdin.isatty():
        _err(f"pass --yes to confirm setting {format_id({'id': cid}, res)} status to {label}")
    typer.confirm(f"Set {format_id({'id': cid}, res)} status to {label}?", abort=True)


def _emit_dry_run(res: Resource, cid: int, body: dict[str, Any], action: str = "update") -> None:
    emit_json({"dry_run": True, "action": action, "id": format_id({"id": cid}, res), "body": body})


def _strip_quotes(value: str) -> str:
    value = value.strip()
    if len(value) >= 2 and value[0] == value[-1] and value[0] in ('"', "'"):
        return value[1:-1]
    return value


def _parse_where(expr: str) -> tuple[str, str, str]:
    m = re.match(r"^(.+?)(>=|<=|!=|~=|=|>|<)(.*)$", expr.strip())
    if not m:
        _err(f"invalid --where {expr!r}; expected FIELD=VALUE")
    field, op, value = m.groups()
    return field.strip(), op, _strip_quotes(value)


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
        return name, raw_value.lower() in ("true", "1", "yes", "on")

    # multi-select → list
    if "multi_select" in ftype:
        parts = [p.strip() for p in raw_value.split(",") if p.strip()]
        return name, [_choice_value(f, p, type_) for p in parts]

    # dropdown with choices
    choices = f.get("choices") or []
    if choices:
        return name, _choice_value(f, raw_value, type_)

    # text / date / number — pass through
    return name, raw_value


def _where_operator(op: str) -> str:
    if op == "=":
        return "is_in"
    if op == "!=":
        return "is_not_in"
    if op in (">", ">=", "<", "<="):
        _err(f"operator {op!r} not supported; inequalities only work for date fields (created_at, updated_at, etc.). Use = or !=")
    if op == "~=":
        _err(f"operator ~= (contains) not supported; use = or !=")
    return "is_in"


def _query_item(condition: str, operator: str, value: Any, type_: str, explain: list[dict[str, Any]], field: str, raw: str) -> dict[str, Any]:
    explain.append({"field": field, "condition": condition, "operator": operator, "type": type_, "value": raw})
    return {"value": value, "condition": condition, "operator": operator, "type": type_}


def _resolve_where(c: Any, res: Resource, sch: dict[str, Any], expr: str, explain: list[dict[str, Any]]) -> dict[str, Any]:
    field_text, op, raw = _parse_where(expr)
    pseudo_dates = {"created_at", "updated_at", "due_by", "planned_start_date", "planned_end_date"}
    if field_text in pseudo_dates:
        if op in (">", ">="):
            value = {"from": raw}
        elif op in ("<", "<="):
            value = {"to": raw}
        elif op in ("=", "!="):
            value = {"from": raw, "to": raw}
        else:
            _err(f"unsupported date operator {op!r}; use = != >= <= > <")
        return _query_item(field_text, "custom_date", value, "default", explain, field_text, raw)
    if field_text.endswith("_id"):
        if op in (">", "<", ">=", "<=", "~="):
            _err(f"operator {op!r} not supported on ID fields; use = or !=")
        operator = "is_in" if op == "=" else "is_not_in"
        val: Any = int(raw) if raw.isdigit() else raw
        return _query_item(field_text, operator, [val], "default", explain, field_text, raw)

    f = _find_schema_field(sch, field_text)
    if not f:
        _err(f"field not found: {field_text}; run `fsv {res.name} fields`")
    name = f.get("name") or field_text
    type_ = _field_api_type(f)
    ftype = str(f.get("field_type") or "")
    operator = _where_operator(op)

    if name == "requester":
        req = _resolve_requester(c, raw)
        return _query_item("requester_id", operator, [{"id": req["id"], "name": req["name"]}], "default", explain, name, f"{req['name']} ({req['id']})")
    if name == "agent":
        agent_id = _resolve_agent(c, raw)
        condition = "responder_id" if res.name == "tickets" else "agent_id"
        return _query_item(condition, operator, [agent_id], "default", explain, name, agent_id)
    if name == "group":
        val = _choice_value(f, raw, "default")
        return _query_item("group_id", operator, [str(val)], "default", explain, name, str(val))

    if f.get("choices") or "dropdown" in ftype or ftype in ("default_status", "default_priority"):
        val = _choice_value(f, raw, type_)
        return _query_item(name, operator, [val], type_, explain, name, str(val))
    val = int(raw) if raw.isdigit() and ftype.endswith("number") else raw
    return _query_item(name, operator, [val], type_, explain, name, raw)


def _merge_date_ranges(query: list[dict[str, Any]]) -> list[dict[str, Any]]:
    ranges: dict[tuple[str, str], dict[str, Any]] = {}
    out: list[dict[str, Any]] = []
    for item in query:
        value = item.get("value")
        if isinstance(value, dict) and ("from" in value or "to" in value):
            key = (item.get("condition"), item.get("type"))
            if key not in ranges:
                merged = dict(item)
                merged["value"] = {}
                ranges[key] = merged
                out.append(merged)
            ranges[key]["value"].update(value)
        else:
            out.append(item)
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
    console.print(f"api: internal /api/_/ with query_hash ({len(query)} conditions, {grouping})")


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
        query.extend(_load_query_hash(raw))
        explain.append({"field": "query_hash", "condition": "raw", "operator": "raw", "type": "raw", "value": raw})
    if where:
        sch = schema_mod.load(res, c)
        for expr in where:
            query.append(_resolve_where(c, res, sch, expr, explain))
    query = _merge_date_ranges(query)
    if not query:
        return None, query, explain
    if or_grouping:
        payload = json.dumps({"any": query}, separators=(",", ":"))
        return payload, query, explain
    return json.dumps(query, separators=(",", ":")), query, explain


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
    from fsv import schema as schema_mod
    from fsv import service
    c = _client()
    qh, query, explanation = _api(lambda: _build_query_hash(c, res, raw_query_hash, where, or_grouping))
    if debug:
        _show_query_explain(explanation, query, or_grouping)
        return
    if or_grouping and res != CHANGES:
        _err(f"OR grouping not supported for {res.name}; use AND filters")
    ot = order_type.value if isinstance(order_type, SortOrder) else order_type

    def load_items() -> tuple[list[dict], dict[str, Any]]:
        if all_pages or n_pages is not None:
            acc: list[dict] = []
            limit = n_pages if n_pages is not None else None
            for p in range(page, page + (limit or 100)):
                batch, _ = service.list_items(
                    res, client=c, page=p, per_page=100,
                    filter_name=filter_name, order_by=order_by, order_type=ot,
                    query_hash=qh, or_grouping=or_grouping,
                )
                acc.extend(batch)
                err.print(f"  fetched {len(acc)} so far...", highlight=False)
                if len(batch) < 100:
                    break
            return acc, schema_mod.load(res, c)
        items, _ = service.list_items(
            res, client=c, page=page, per_page=per_page,
            filter_name=filter_name, order_by=order_by, order_type=ot,
            query_hash=qh, or_grouping=or_grouping,
        )
        return items, schema_mod.load(res, c)

    items, sch = _api(load_items)
    fmt = format_.value if isinstance(format_, OutputFormat) else format_
    if not json_out and fmt == "table" and _should_page(len(items), pager):
        with console.capture() as capture:
            console.print(render.list_table(items, res, sch))
        if not _page_text(capture.get()):
            console.print(render.list_table(items, res, sch))
    else:
        _emit_items(items, res, sch, format_, json_out)
    if not json_out and fmt == "table":
        err.print(f"{len(items)} rows")


def get_resource(res: Resource, id_: str, stats: bool, json_out: bool) -> None:
    from fsv import schema as schema_mod
    from fsv import service
    cid = _cid(id_, res)
    c = _client()

    def load_item() -> tuple[dict[str, Any], dict[str, Any], list[dict[str, Any]]]:
        sch = schema_mod.load(res, c)
        if stats and res == CHANGES:
            evidence = service.get_change_evidence(cid, client=c)
            item = {
                **evidence["change"],
                "planning_fields": evidence["planning_fields_by_name"],
                "change_planning_fields": evidence["planning_fields"],
                "planning_field_definitions": service.get_change_planning_field_definitions(sch),
                "main_attachments": evidence["main_attachments"],
                "description_attachment_urls": evidence["description_attachment_urls"],
            }
        else:
            item = service.get_item(res, cid, client=c)
        requested_items = service.get_requested_items(cid, client=c) if res == TICKETS else []
        return item, sch, requested_items

    item, sch, requested_items = _api(load_item)
    if requested_items:
        item = {**item, "requested_items": requested_items}
    if json_out:
        emit_json(item)
        return
    render.detail_panel(item, res, sch)
    if requested_items:
        render.requested_items_panel(requested_items)


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


def activity_resource(res: Resource, id_: str, limit: int, json_out: bool) -> None:
    from fsv import service
    from fsv.create import get_change_activities
    cid = _cid(id_, res)
    c = _client()
    acts = _api(lambda: get_change_activities(cid, c) if res == CHANGES else service.get_activities(res, cid, client=c))[:limit]
    if json_out:
        emit_json(acts)
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


def tasks_resource(res: Resource, id_: str, format_: OutputFormat | str = "table", json_out: bool = False) -> None:
    from fsv import service
    cid = _cid(id_, res)
    c = _client()
    items = _api(lambda: service.get_tasks(res, cid, client=c))
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


def _asset_display(asset: dict[str, Any]) -> dict[str, Any]:
    ci = asset.get("config_item") if isinstance(asset.get("config_item"), dict) else asset
    return {
        "id": ci.get("display_id") or ci.get("id") or asset.get("id"),
        "name": ci.get("name") or "-",
        "type": ci.get("asset_type") or ci.get("ci_type_name") or "-",
        "used_by": ci.get("user_name") or ci.get("used_by") or "-",
        "location": ci.get("location_name") or "-",
        "state": ci.get("asset_state_16000510422") or "-",
        "serial": ci.get("serial_number_16000510422") or "-",
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
    cid = _cid(id_, CHANGES)
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
    if pick:
        asset_ids = _pick_change_assets(cid, c, category)
        if not asset_ids:
            console.print("cancelled")
            return
        if dry_run:
            emit_json({"action": "associate_assets", "change_id": cid, "asset_ids": asset_ids})
            return
        if not yes:
            if _no_input() or not sys.stdin.isatty():
                _err("pass --yes to associate assets")
            typer.confirm(f"Associate {len(asset_ids)} asset(s) with {format_id({'id': cid}, CHANGES)}?", abort=True)
        _api(lambda: associate_assets(cid, asset_ids, c))
        console.print(f"[green]associated[/] {len(asset_ids)} asset(s) with {format_id({'id': cid}, CHANGES)}")
        return
    if add:
        asset_ids = _resolve_change_asset_ids(cid, add, c, category=category)
        if dry_run:
            emit_json({"action": "associate_assets", "change_id": cid, "asset_ids": asset_ids})
            return
        if not yes:
            if _no_input() or not sys.stdin.isatty():
                _err("pass --yes to associate assets")
            typer.confirm(f"Associate {len(asset_ids)} asset(s) with {format_id({'id': cid}, CHANGES)}?", abort=True)
        _api(lambda: associate_assets(cid, asset_ids, c))
        console.print(f"[green]associated[/] {len(asset_ids)} asset(s) with {format_id({'id': cid}, CHANGES)}")
        return
    if remove:
        asset_ids = _resolve_change_asset_ids(cid, remove, c, associated=True, category=category)
        if dry_run:
            emit_json({"action": "dissociate_assets", "change_id": cid, "asset_display_ids": asset_ids})
            return
        if not yes:
            if _no_input() or not sys.stdin.isatty():
                _err("pass --yes to remove assets")
            typer.confirm(f"Remove {len(asset_ids)} asset(s) from {format_id({'id': cid}, CHANGES)}?", abort=True)
        _api(lambda: dissociate_assets(cid, asset_ids, c))
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
    for value, help_text in (("Open", "id=1"), ("Completed", "id=2"), ("1", "Open"), ("2", "Completed")):
        if value.casefold().startswith(incomplete.casefold()):
            yield (value, help_text)
    yield (incomplete, "")


def _task_status_value(value: str) -> int:
    raw = value.strip()
    if raw.isdigit():
        return int(raw)
    mapping = {"open": 1, "completed": 2, "complete": 2, "done": 2, "closed": 2}
    key = raw.casefold()
    if key not in mapping:
        raise ValueError("task status must be Open/Completed or numeric ID")
    return mapping[key]


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
    tickets = get_change_associations(change_id, c).get("tickets", [])
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


def notes_resource(res: Resource, id_: str, page: int, per_page: int, json_out: bool, all_pages: bool = False, n_pages: int | None = None) -> None:
    from fsv import service
    cid = _cid(id_, res)
    c = _client()
    if all_pages or n_pages is not None:
        limit = n_pages if n_pages is not None else None
        acc: list[dict] = []
        for p in range(page, page + (limit or 100)):
            batch = _api(lambda _p=p: service.get_notes(res, cid, client=c, page=_p, per_page=per_page))
            acc.extend(batch)
            err.print(f"  fetched {len(acc)}...", highlight=False)
            if len(batch) < per_page or (limit is not None and p - page + 1 >= limit):
                break
        items = acc
    else:
        items = _api(lambda: service.get_notes(res, cid, client=c, page=page, per_page=per_page))
    if json_out:
        emit_json(items)
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


def conversations_resource(id_: str, page: int, per_page: int, json_out: bool, all_pages: bool = False, n_pages: int | None = None) -> None:
    from fsv import service
    cid = _cid(id_, TICKETS)
    c = _client()
    if all_pages or n_pages is not None:
        limit = n_pages if n_pages is not None else None
        acc: list[dict] = []
        for p in range(page, page + (limit or 100)):
            batch = _api(lambda _p=p: service.get_notes(TICKETS, cid, client=c, page=_p, per_page=per_page))
            acc.extend(batch)
            err.print(f"  fetched {len(acc)}...", highlight=False)
            if len(batch) < per_page or (limit is not None and p - page + 1 >= limit):
                break
        items = acc
    else:
        items = _api(lambda: service.get_notes(TICKETS, cid, client=c, page=page, per_page=per_page))
    if json_out:
        emit_json(items)
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
    items = _api(lambda: service.get_ticket_approvals(TICKETS, cid, client=c))
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
    changes = _api(lambda: c.int_get(f"tickets/{cid}/changes", {"change_type": "change"})).get("changes", []) if (not associated_modules or "change" in associated_modules) else []
    change_causes = _api(lambda: c.int_get(f"tickets/{cid}/changes", {"change_type": "change_cause"})).get("changes", []) if (not associated_modules or "change_cause" in associated_modules) else []
    problems = _api(lambda: c.int_get(f"tickets/{cid}/problems")).get("problems", []) if (not associated_modules or "problem" in associated_modules) else []
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


def filters_resource(res: Resource) -> None:
    c = _client()
    data = _api(lambda: c.int_get(res.filters_path or ""))
    key = next((k for k in data if k.endswith("_filters")), None)
    t = Table()
    t.add_column("id", style="cyan")
    t.add_column("name")
    for f in data.get(key, []) if key else []:
        t.add_row(str(f.get("id") or "-"), str(f.get("name") or "-"))
    console.print(t)


def _search_dsl_to_where(query: str) -> tuple[list[str], bool]:
    text = query.strip()
    if not text:
        _err("empty search query")
    parts = re.split(r"\s+(AND|OR)\s+", text, flags=re.IGNORECASE)
    clauses = [parts[i].strip() for i in range(0, len(parts), 2)]
    joins = [parts[i].upper() for i in range(1, len(parts), 2)]
    if not clauses or any(not c for c in clauses):
        _err(f"invalid search query: {query}")
    if joins and any(j != joins[0] for j in joins):
        _err("mixed AND/OR not supported for changes/problems search; use one join type")
    where: list[str] = []
    for clause in clauses:
        if ":" not in clause:
            _err(f"invalid search clause: {clause!r}; expected field:value")
        field, raw_value = clause.split(":", 1)
        field = field.strip()
        value = raw_value.strip()
        op = "="
        for candidate in (">=", "<=", "!=", ">", "<"):
            if value.startswith(candidate):
                op = candidate
                value = value[len(candidate):].strip()
                break
        if len(value) >= 2 and value[0] == value[-1] and value[0] in {'"', "'"}:
            value = value[1:-1]
        if not field or not value:
            _err(f"invalid search clause: {clause!r}")
        where.append(f"{field}{op}{value}")
    return where, bool(joins and joins[0] == "OR")


def _fulltext_search(res: Resource, query: str, page: int, json_out: bool, sort: SearchSort, all_pages: bool = False, n_pages: int | None = None) -> None:
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
    if json_out:
        emit_json([{k: v for k, v in r.items() if not k.startswith("_")} for r in results])
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


def search_resource(res: Resource, query: str, page: int, json_out: bool, sort: SearchSort, all_pages: bool = False, n_pages: int | None = None) -> None:
    _fulltext_search(res, query, page, json_out, sort, all_pages, n_pages)


def filter_resource(res: Resource, query: str, per_page: int, page: int, format_: OutputFormat | str, json_out: bool, all_pages: bool = False, n_pages: int | None = None) -> None:
    from fsv import schema as schema_mod
    if res in (CHANGES, PROBLEMS):
        where, or_grouping = _search_dsl_to_where(query)
        list_resource(
            res,
            per_page=per_page,
            page=page,
            all_pages=all_pages,
            filter_name=None,
            where=where,
            debug=False,
            format_=format_,
            json_out=json_out,
            or_grouping=or_grouping,
            pager=True,
            n_pages=n_pages,
        )
        return
    c = _client()
    if all_pages or n_pages is not None:
        limit = n_pages if n_pages is not None else None
        acc: list[dict] = []
        sch = _api(lambda: schema_mod.load(res, c))
        total = 0
        for p in range(page, page + (limit or 100)):
            data = _api(lambda _p=p: c.v2_get(f"{res.api_path}/filter", params={"query": f'"{query}"', "per_page": per_page, "page": _p}))
            batch = data.get(res.list_key, [])
            total = data.get("total", total)
            acc.extend(batch)
            err.print(f"  fetched {len(acc)} / {total}...", highlight=False)
            if len(batch) < per_page or (limit is not None and p - page + 1 >= limit):
                break
        _emit_items(acc, res, sch, format_, json_out)
        if not json_out and format_ == "table":
            err.print(f"{len(acc)} rows (total: {total})")
        return
    data, sch = _api(lambda: (
        c.v2_get(f"{res.api_path}/filter", params={"query": f'"{query}"', "per_page": per_page, "page": page}),
        schema_mod.load(res, c),
    ))
    items = data.get(res.list_key, [])
    total = data.get("total", len(items))
    _emit_items(items, res, sch, format_, json_out)
    if not json_out and format_ == "table":
        err.print(f"{len(items)} rows (total: {total})")


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
) -> None:
    from fsv import schema as schema_mod
    from fsv import service
    cid = _cid(id_, res)
    c = _client()
    sch = _api(lambda: schema_mod.load(res, c))
    body: dict[str, Any] = {}
    if status is not None:
        body["status"] = _resolve_update_choice(sch, "status", status)
        _confirm_terminal_status(res, cid, sch, body["status"], yes, no_input)
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
        k, v = _resolve_set_field(c, sch, field_text.strip(), raw_value)
        body[k] = v
    if not body:
        _err("nothing to update; pass at least one field (--status, --priority, --agent, --group, --set FIELD=VALUE)")
    if dry_run:
        _emit_dry_run(res, cid, body)
        return
    item = _api(lambda: service.update_item(res, cid, body, client=c))
    if json_out:
        emit_json(item)
        return
    console.print(f"[green]updated[/] {format_id(item, res)}")


def note_resource(res: Resource, id_: str, body: str, public: bool) -> None:
    from fsv import service
    cid = _cid(id_, res)
    c = _client()
    n = _api(lambda: service.add_note(res, cid, body, public=public, client=c))
    visibility = "public" if public else "private"
    console.print(f"[green]added[/] {visibility} note {n.get('id')}")


def reply_resource(res: Resource, id_: str, body: str) -> None:
    from fsv import service
    cid = _cid(id_, res)
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
    default = config.DOMAIN or "yourcompany.freshservice.com"
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
        _err(str(e))
    return looks_like_url


@auth_app.command("login", epilog="[bold]Examples:[/bold]  fsv auth login  |  pbpaste | fsv auth login --header - --store keychain")
def auth_login(
    domain: Optional[str] = typer.Option(None, "--domain", "-d", help="Freshservice domain or URL, e.g. acme.freshservice.com. Saved before login."),
    header: Optional[str] = typer.Option(None, "--header", "-H", help="Cookie header string. Use '-' to read from stdin."),
    store: Optional[str] = typer.Option(None, "--store", help="Where to save: 'file' (plain chmod 600), 'argon' (Argon2id + AES-GCM), or 'keychain' (macOS Keychain). Interactive when omitted; default keychain on macOS.", autocompletion=completion.complete_store),
    no_input: bool = typer.Option(False, "--no-input", help="fail instead of prompting"),
) -> None:
    """Save Freshservice session cookies."""
    from fsv.client import get_client, reset_client
    from fsv.session import login_interactive, parse_cookie_header, save_cookies, validate
    no_input = _no_input(no_input)
    if store is not None and store not in ("file", "argon", "keychain"):
        _err("--store must be 'file', 'argon', or 'keychain'")
    if header is None and no_input:
        _err("pass --header when using --no-input")
    explicit_domain = domain is not None
    url_pasted = False
    if domain is not None:
        try:
            config.set_domain(domain)
        except ValueError as e:
            _err(str(e))
    elif not config.DOMAIN:
        url_pasted = _resolve_domain(no_input)
    else:
        if not no_input and sys.stdin.isatty():
            url_pasted = _resolve_domain(no_input)
    if not explicit_domain and sys.stdin.isatty() and url_pasted and not no_input:
        typer.confirm(f"Extracted {config.DOMAIN} — login?", default=True, abort=True)
    try:
        if header is not None:
            if header == "-":
                header = sys.stdin.read().strip()
            cookies = parse_cookie_header(header)
            validate(cookies)
        else:
            cookies = login_interactive()
    except SessionError as e:
        _err(str(e))
    if store is None and sys.stdin.isatty() and not no_input:
        store = _choose_store()
    backend = save_cookies(cookies, store)  # type: ignore[arg-type]
    target = "Keychain" if backend == "keychain" else "~/.config/fsv/session.json"
    console.print(f"saved {len(cookies)} cookies to {target}")
    try:
        reset_client()
        c = get_client()
        agent = c.me()
        name = f"{agent.get('first_name', '')} {agent.get('last_name', '')}".strip()
        email = agent.get("email", "")
        console.print(f"logged in as: [bold]{name}[/bold] <{email}>")
        err.print("warming cache in background...", highlight=False)
        _refresh_cache(verbose=False, blocking=False)
    except Exception as e:
        err.print(f"[yellow]warning[/yellow]: saved cookies but could not verify login for {config.DOMAIN}: {e}")
        err.print("If this is the wrong tenant, rerun with `fsv auth login --domain yourcompany.freshservice.com`.")


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
    agent, (rem, tot) = _api(lambda: (c.me(), c.rate_limit_remaining()))
    name = f"{agent.get('first_name', '')} {agent.get('last_name', '')}".strip()
    email = agent.get("email", "")
    dept = ", ".join(agent.get("department_names") or [])
    team = (agent.get("custom_fields") or {}).get("team") or ""
    dept_display = f"{dept}  ({team})" if dept and team else dept or team
    console.print(f"[bold]{name}[/bold]  <{email}>")
    if dept_display:
        console.print(f"department:  {dept_display}")
    age_str = f"{age:.1f}h"
    if age > 8:
        age_str += "  [yellow]⚠ session may be stale[/yellow]"
    console.print(f"session age: {age_str}")
    console.print(f"rate limit:  {rem}/{tot}")


app.add_typer(auth_app, name="auth")


HELP_TOPICS = {
    "auth": "Login: fsv auth login --domain yourcompany.freshservice.com; fsv auth status. Scripts: use fsv auth login --domain ... --header ... --store file.",
    "workflow": "Daily flow: fsv changes ls --where status=Open; fsv changes get CHN-1234 --internal; fsv changes update CHN-1234 --set 'field=value'; fsv changes download CHN-1234 --all.",
    "fields": "Discover fields with fsv changes fields, fsv changes fields --choices status, and fsv changes lookup requester alice@example.com. Prefer dedicated flags for default fields; use --set for custom fields.",
    "scripting": "Use --json or --output csv/tsv for scripts. Use --no-input in CI. Use --dry-run before updates. Do not parse rich table output.",
    "comments": "Tickets use reply. Changes/problems use add-note. Use --public only when notes should be visible outside private workflow.",
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
            return (f"{r.name} filters", len(filters))
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
        _err(f"unknown key: {key}; supported: {list(possible)}")
    cfg = completion._config_load()
    sec = cfg.setdefault(section, {})
    sec[prop] = value
    import json
    p = CONFIG_DIR / "config.json"
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(cfg, indent=2))
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
        for kind, candidates in (("schema", config.schema_cache_candidates(res.name)), ("filters", config.filters_cache_candidates(res.name))):
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
            items.append((res.name, kind, f"{age:.1f}h", f"{'stale' if stale else 'ok'}", ttl_s))
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


@cache_app.command("clear", epilog="[bold]Examples:[/bold]  fsv cache clear schema  |  fsv cache clear all")
def cache_clear(
    target: Optional[str] = typer.Argument(None, help="schema|filters|groups|all", autocompletion=completion.complete_cache_target),
) -> None:
    """Delete cached data."""
    import shutil
    import shutil

    if target == "schema" or target == "all":
        if SCHEMA_DIR.exists():
            shutil.rmtree(SCHEMA_DIR)
            console.print("schema cache cleared")
    if target == "filters" or target == "all":
        fd = CONFIG_DIR / "filters"
        if fd.exists():
            shutil.rmtree(fd)
            console.print("filters cache cleared")
    if target in ("groups", "all"):
        removed = False
        for gp in config.groups_cache_candidates():
            if gp.exists():
                gp.unlink()
                removed = True
        if removed:
            console.print("groups cache cleared")
    if target is None:
        _err("choose schema|filters|groups|all")


app.add_typer(cache_app, name="cache")


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
        filter_name: Optional[str] = typer.Option(None, "--view", help="saved view name, e.g. new_and_my_open", autocompletion=completion.complete_filter_name(res)),
        where: Optional[list[str]] = typer.Option(None, "--where", "-w", help="field=value filters, e.g. requester=me@example.com or status=Open", autocompletion=completion.complete_where(res)),
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
        f"fsv {res.name} get {_pfx}-1234 --json"
    ))
    def get(
        id_: str = typer.Argument(..., metavar="ID"),
        stats: bool = typer.Option(False, "--stats", help="include stats + planning_fields"),
        json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
    ) -> None:
        f"""Show one {res.name[:-1]} by id."""
        get_resource(res, id_, stats, json_out)

    @sub.command("activity", help=f"List {singular} activity.", epilog=(
        f"[bold]Examples:[/bold]  fsv {res.name} activity {_pfx}-1234  |  fsv {res.name} activity {_pfx}-1234 -l 50 --json"
    ))
    def activity(
        id_: str = typer.Argument(...),
        limit: int = typer.Option(20, "-l", "--limit"),
        json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
    ) -> None:
        activity_resource(res, id_, limit, json_out)

    @sub.command("tasks", help=f"List {singular} tasks.", epilog=(
        f"[bold]Examples:[/bold]  fsv {res.name} tasks {_pfx}-1234  |  fsv {res.name} tasks {_pfx}-1234 --json"
    ))
    def tasks(
        id_: str = typer.Argument(...),
        format_: OutputFormat = typer.Option(OutputFormat.table, "--output", "-o", help="output format", autocompletion=completion.complete_format),
        json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
    ) -> None:
        from fsv import state_flow
        from fsv.create import (
            associate_ticket,
            delete_task,
            dissociate_ticket,
            get_change_approvals,
            get_change_associations,
            get_task_for_edit,
            search_change_tickets,
            update_task,
        )
        tasks_resource(res, id_, format_, json_out)

    if res == CHANGES:
        @sub.command("assets", help="List, search, or associate change assets.", epilog=(
            "[bold]Examples:[/bold]  "
            "fsv changes assets CHN-1234  |  "
            "fsv changes assets CHN-1234 --search app  |  "
            "fsv changes assets CHN-1234 --search EDP --category 'Application Portfolio'  |  "
            "fsv changes assets CHN-1234 --add 456 --dry-run  |  "
            "fsv changes assets CHN-1234 --add 'EDP' --category 'Application Portfolio' --yes  |  "
            "fsv changes assets CHN-1234 --pick --category 'Application Portfolio' --yes  |  "
            "fsv changes assets CHN-1234 --list-categories"
        ))
        def assets(
            id_: str = typer.Argument(..., metavar="CHANGE_ID"),
            search: Optional[str] = typer.Option(None, "--search", "-q", help="search assets available to associate", autocompletion=_complete_asset_search_for_change),
            add: Optional[List[str]] = typer.Option(None, "--add", help="asset display ID(s) or names to associate", autocompletion=_complete_asset_id_for_change),
            remove: Optional[List[str]] = typer.Option(None, "--remove", help="asset display ID(s) or names to remove", autocompletion=_complete_associated_asset_for_change),
            page: int = typer.Option(1, "--page", "-p"),
            per_page: int = typer.Option(30, "--per-page", "-n"),
            category: Optional[str] = typer.Option(None, "--category", help="asset category/type label from Freshservice UI", autocompletion=_complete_asset_category),
            list_categories: bool = typer.Option(False, "--list-categories", help="show available asset categories from /cmdb/items"),
            pick: bool = typer.Option(False, "--pick", help="interactive picker for add flow; prompts for category first when omitted"),
            dry_run: bool = typer.Option(False, "--dry-run", help="print resolved payload without mutating"),
            yes: bool = typer.Option(False, "--yes", "-y", help="confirm add/remove mutation"),
            format_: OutputFormat = typer.Option(OutputFormat.table, "--output", "-o", help="output format", autocompletion=completion.complete_format),
            json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
        ) -> None:
            try:
                assets_resource(
                    id_,
                    search,
                    list(add or []),
                    list(remove or []),
                    page,
                    per_page,
                    dry_run,
                    yes,
                    json_out,
                    format_,
                    pick=pick,
                    category_name=category,
                    list_categories=list_categories,
                )
            except (SessionError, APIError, ValueError) as e:
                _err(str(e))

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
            agent_id: Optional[str] = typer.Option(None, "--agent", "--agent-id", help="agent name/email/user ID", autocompletion=completion.complete_update_agent_id),
            system: Optional[str] = typer.Option(None, "--system", help="task custom field system label or numeric ID", autocompletion=_complete_task_system),
            environment: Optional[str] = typer.Option(None, "--environment", "--env", help="task custom field environment label or numeric ID", autocompletion=_complete_task_environment),
            due_date: Optional[str] = typer.Option(None, "--due-date", help="ISO-8601 due date"),
            planned_start: Optional[str] = typer.Option(None, "--planned-start", help="ISO-8601 planned start"),
            planned_end: Optional[str] = typer.Option(None, "--planned-end", help="ISO-8601 planned end"),
            edit: bool = typer.Option(False, "--edit", "-e", help="open $EDITOR with all task fields"),
            dry_run: bool = typer.Option(False, "--dry-run", help="print resolved payload without updating"),
            json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
            no_input: bool = typer.Option(False, "--no-input", help="fail instead of prompting"),
        ) -> None:
            """Update a task on a change. Use --edit for full edit or flags for quick update."""
            try:
                cid = _cid(id_, res)
                if edit:
                    current = get_task_for_edit(cid, task_id)
                    body = _edit_body(current, f"Editing task {task_id} on change #{cid}", no_input)
                    if dry_run:
                        emit_json({"action": "update_task", "change_id": cid, "task_id": task_id, "body": body})
                        return
                    updated = update_task(cid, task_id, body)
                else:
                    body: dict[str, Any] = {}
                    custom_fields: dict[str, Any] = {}
                    if status is not None:
                        body["status"] = _task_status_value(status)
                    if title is not None:
                        body["title"] = title
                    if group_id is not None:
                        body["group_id"] = int(_resolve_group(_client(), group_id))
                    if agent_id is not None:
                        body["owner_id"] = int(_resolve_agent(_client(), agent_id))
                    if due_date is not None:
                        body["due_date"] = due_date
                    if planned_start is not None:
                        body["planned_start_date"] = planned_start
                    if planned_end is not None:
                        body["planned_end_date"] = planned_end
                    if system is not None:
                        custom_fields["system"] = _task_choice_value(system, "system")
                    if environment is not None:
                        custom_fields["environment"] = _task_choice_value(environment, "environment")
                    if custom_fields:
                        body["custom_fields"] = custom_fields
                    if not body:
                        _err("nothing to update; use --edit or pass task fields")
                        return
                    if dry_run:
                        emit_json({"action": "update_task", "change_id": cid, "task_id": task_id, "body": body})
                        return
                    updated = update_task(cid, task_id, body)
                if json_out:
                    emit_json(updated)
                else:
                    console.print(f"[green]updated[/] task {task_id} on #{cid}")
            except (SessionError, APIError, ValueError) as e:
                _err(str(e))

        @sub.command("tasks-delete", help="Delete a change task.", epilog=(
            "[bold]Examples:[/bold]  fsv changes tasks-delete CHN-1234 987 --dry-run  |  fsv changes tasks-delete CHN-1234 987 --yes"
        ))
        def task_delete(
            id_: str = typer.Argument(..., metavar="CHANGE_ID"),
            task_id: int = typer.Argument(..., metavar="TASK_ID", autocompletion=_complete_task_id),
            dry_run: bool = typer.Option(False, "--dry-run", help="print target without deleting"),
            yes: bool = typer.Option(False, "--yes", "-y", help="confirm deletion"),
        ) -> None:
            try:
                cid = _cid(id_, res)
                payload = {"action": "delete_task", "change_id": cid, "task_id": task_id}
                if dry_run:
                    emit_json(payload)
                    return
                if not yes:
                    if _no_input() or not sys.stdin.isatty():
                        _err("pass --yes to delete task")
                    typer.confirm(f"Delete task {task_id} from {format_id({'id': cid}, res)}?", abort=True)
                _api(lambda: delete_task(cid, task_id))
                console.print(f"[green]deleted[/] task {task_id} from #{cid}")
            except (SessionError, APIError) as e:
                _err(str(e))

        @sub.command("approvals", help="List change approvals.", epilog=(
            "[bold]Examples:[/bold]  fsv changes approvals CHN-1234  |  fsv changes approvals CHN-1234 --json"
        ))
        def approvals(
            id_: str = typer.Argument(...),
            format_: OutputFormat = typer.Option(OutputFormat.table, "--output", "-o", help="output format", autocompletion=completion.complete_format),
            json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
        ) -> None:
            """List approvals for a change (read-only)."""
            try:
                cid = _cid(id_, res)
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
            except (SessionError, APIError) as e:
                _err(str(e))

        @sub.command("state", help="Approval lifecycle state flow.", epilog=(
            "[bold]Examples:[/bold]  fsv changes state CHN-1234  |  fsv changes state CHN-1234 --json"
        ))
        def state(
            id_: str = typer.Argument(..., metavar="ID"),
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
                if json_out:
                    flow_data = state_flow.get_state_flow(change["state_flow_id"], c)
                    emit_json({"change": change, "flow": flow_data})
                else:
                    state_flow.render_flow(change, console)
            except KeyError:
                _err("no state_flow_id on this change")
            except (SessionError, APIError) as e:
                _err(str(e))

        @sub.command("associations", help="List, search, or manage associated tickets.", epilog=(
            "[bold]Examples:[/bold]  "
            "fsv changes associations CHN-1234  |  "
            "fsv changes associations CHN-1234 --search SR-123  |  "
            "fsv changes associations CHN-1234 --add SR-123 --dry-run  |  "
            "fsv changes associations CHN-1234 --add 'Napimpat' --yes  |  "
            "fsv changes associations CHN-1234 --pick --yes"
        ))
        def associations(
            id_: str = typer.Argument(...),
            search: Optional[str] = typer.Option(None, "--search", "-q", help="search tickets available to associate", autocompletion=_complete_ticket_for_change),
            add: Optional[List[str]] = typer.Option(None, "--add", help="ticket ID(s) or subject text to associate, e.g. SR-565163", autocompletion=_complete_ticket_for_change),
            remove: Optional[List[str]] = typer.Option(None, "--remove", help="ticket ID(s) or subject text to dissociate", autocompletion=_complete_ticket_for_change),
            pick: bool = typer.Option(False, "--pick", help="interactive picker for add flow"),
            dry_run: bool = typer.Option(False, "--dry-run", help="print resolved payload without mutating"),
            yes: bool = typer.Option(False, "--yes", "-y", help="confirm mutation"),
            format_: OutputFormat = typer.Option(OutputFormat.table, "--output", "-o", help="output format", autocompletion=completion.complete_format),
            json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
        ) -> None:
            """List, search, or manage associated tickets for a change.

            --search sr-565163      find an initiating ticket
            --add SR-565163        associate a ticket
            --remove SR-565163     dissociate a ticket
            """
            try:
                actions = sum(1 for active in (search is not None, bool(add), bool(remove), pick) if active)
                if actions > 1:
                    _err("choose only one action: --search, --add, or --remove")
                cid = _cid(id_, res)
                if search:
                    items = search_change_tickets(search)
                    if json_out:
                        emit_json(items)
                    else:
                        _print_tickets(items, "Ticket search")
                    return
                if pick:
                    ids = _pick_change_tickets(cid)
                    if not ids:
                        console.print("cancelled")
                        return
                    if dry_run:
                        emit_json({"action": "associate_tickets", "change_id": cid, "ticket_ids": ids})
                        return
                    if not yes:
                        if _no_input() or not sys.stdin.isatty():
                            _err("pass --yes to confirm")
                        typer.confirm(f"Associate {len(ids)} ticket(s) with #{cid}?", abort=True)
                    associate_ticket(cid, ids)
                    console.print(f"[green]associated[/] {len(ids)} ticket(s) with #{cid}")
                    return
                if add:
                    ids = [_resolve_change_ticket_id(x) for x in add]
                    if dry_run:
                        emit_json({"action": "associate_tickets", "change_id": cid, "ticket_ids": ids})
                        return
                    if not yes:
                        if _no_input() or not sys.stdin.isatty():
                            _err("pass --yes to confirm")
                        typer.confirm(f"Associate {len(ids)} ticket(s) with #{cid}?", abort=True)
                    associate_ticket(cid, ids)
                    console.print(f"[green]associated[/] {len(ids)} ticket(s) with #{cid}")
                    return
                if remove:
                    ids = [_resolve_associated_change_ticket_id(cid, x) for x in remove]
                    if dry_run:
                        emit_json({"action": "dissociate_tickets", "change_id": cid, "ticket_ids": ids})
                        return
                    if not yes:
                        if _no_input() or not sys.stdin.isatty():
                            _err("pass --yes to confirm")
                        typer.confirm(f"Dissociate {len(ids)} ticket(s) from #{cid}?", abort=True)
                    for tid in ids:
                        dissociate_ticket(cid, tid)
                    console.print(f"[green]dissociated[/] {len(ids)} ticket(s) from #{cid}")
                    return
                assoc = get_change_associations(cid)
                flat_rows = [
                    {"type": k, "id": item.get("human_display_id") or str(item.get("id", "-")),
                     "subject": item.get("subject") or item.get("title") or "-",
                     "status": item.get("status_name") or str(item.get("status", "-"))}
                    for k, itms in assoc.items() for item in itms
                ]
                if _emit_fmt(assoc, flat_rows, format_, json_out):
                    return
                any_shown = False
                for kind, items in assoc.items():
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
                    console.print("no associations")
            except (SessionError, APIError, ValueError) as e:
                _err(str(e))

    @sub.command("fields", help=f"List discoverable {singular} fields.", epilog=(
        f"[bold]Examples:[/bold]  fsv {res.name} fields  |  fsv {res.name} fields --choices status  |  fsv {res.name} fields --custom"
    ))
    def fields(
        search: Optional[str] = typer.Argument(None, help="field name/label search", autocompletion=completion.complete_field_names(res)),
        default: bool = typer.Option(False, "--default", help="show portable Freshservice fields only"),
        custom: bool = typer.Option(False, "--custom", help="show tenant custom fields only"),
        choices: Optional[str] = typer.Option(None, "--choices", help="show choices for a field", autocompletion=completion.complete_choice_field_names(res)),
        refresh: bool = typer.Option(False, "--refresh"),
        json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
    ) -> None:
        f"""List discoverable {res.name} fields."""
        try:
            fields_resource(res, search, default, custom, choices, refresh, json_out)
        except (SessionError, APIError) as e:
            _err(str(e))
        except typer.Exit:
            raise
        except Exception as e:
            _err(str(e))

    @sub.command("lookup", help="Look up users, groups, or field choices.", epilog=(
        f"[bold]Examples:[/bold]  fsv {res.name} lookup requester me@example.com  |  fsv {res.name} lookup group desk"
    ))
    def lookup(
        kind: str = typer.Argument(..., help="requester | agent | group | field name/label", autocompletion=completion.complete_lookup_kind(res)),
        query: str = typer.Argument("", help="name/email/text to search", autocompletion=completion.complete_lookup_query(res)),
        json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
    ) -> None:
        f"""Autocomplete users, groups, or field choices."""
        try:
            lookup_resource(res, kind, query, json_out)
        except (SessionError, APIError) as e:
            _err(str(e))
        except typer.Exit:
            raise
        except Exception as e:
            _err(str(e))

    if res.filters_path:
        @sub.command("filters", help="List saved filter names.", epilog=(
            f"[bold]Examples:[/bold]  fsv {res.name} filters"
        ))
        def filters() -> None:
            """List saved filter names."""
            filters_resource(res)

    @sub.command("search", help=f"Full-text keyword search {res.name}.", epilog=(
        f"[bold]Examples:[/bold]  fsv {res.name} search 'EDP'  |  fsv {res.name} search 'EDP' --npages 3  |  fsv {res.name} search 'EDP' --all  |  fsv {res.name} search 'data migration' --json"
    ))
    def search(
        query: str = typer.Argument(..., help="free-text keyword(s)"),
        page: int = typer.Option(1, "--page", "-p"),
        sort: SearchSort = typer.Option(SearchSort.relevance, "--sort", help="relevance | created | modified", autocompletion=completion.complete_search_sort),
        json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
        n_pages: Optional[int] = typer.Option(None, "--npages", "-N", help="fetch exactly N pages (30 results each)"),
        all_pages: bool = typer.Option(False, "--all", "-a", help="fetch all pages (auto-paginate)"),
    ) -> None:
        f"""Full-text keyword search {res.name}."""
        search_resource(res, query, page, json_out, sort, all_pages, n_pages)

    @sub.command("filter", help=f"Filter {res.name} by field query (DSL).", epilog=(
        f"[bold]Examples:[/bold]  fsv {res.name} filter 'status:Open'  |  fsv {res.name} filter 'status:Open' --all  |  fsv {res.name} filter 'status:Open AND priority:High' --json"
    ))
    def filter_cmd(
        query: str = typer.Argument(..., help='DSL query, e.g. "status:Open AND priority:High"', autocompletion=completion.complete_search_dsl),
        per_page: int = typer.Option(30, "--per-page", "-n"),
        page: int = typer.Option(1, "--page", "-p"),
        format_: OutputFormat = typer.Option(OutputFormat.table, "--output", "-o", help="output format", autocompletion=completion.complete_format),
        json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
        all_pages: bool = typer.Option(False, "--all", "-a", help="fetch all pages (auto-paginate)"),
        n_pages: Optional[int] = typer.Option(None, "--npages", "-N", help="fetch exactly N pages"),
    ) -> None:
        f"""Filter {res.name} using Freshservice query DSL."""
        filter_resource(res, query, per_page, page, format_, json_out, all_pages, n_pages)

    @sub.command("url", help=f"Print {singular} browser URL.", epilog=(
        f"[bold]Examples:[/bold]  fsv {res.name} url {_pfx}-1234"
    ))
    def url(id_: str = typer.Argument(..., metavar="ID")) -> None:
        f"""Print the browser URL for a {res.name[:-1]}."""
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
            resolve_planning_field,
            set_due_by,
            submit_change,
            update_change,
            update_planning_field,
        )
        url_resource(res, id_)

    if res == CHANGES:
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
            yes: bool = typer.Option(False, "--yes", "-y", help="confirm terminal status transitions"),
            json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
            no_input: bool = typer.Option(False, "--no-input", help="fail instead of prompting"),
        ) -> None:
            """Update fields on a change."""
            try:
                cid = _cid(id_, res)
                no_input = _no_input(no_input)
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
                    planning_id = resolve_planning_field(planning, _api(lambda: schema_mod.load(res, _client())).get("fields", []))
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
                    if dry_run:
                        _emit_dry_run(res, cid, body)
                        return
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
                        _confirm_terminal_status(res, cid, sch, quick["status"], yes, no_input)
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
                        _emit_dry_run(res, cid, quick)
                        return
                    if quick:
                        updated = update_change(cid, quick)
                        if json_out:
                            emit_json(updated)
                        else:
                            console.print(f"[green]updated[/] {format_id(updated, res)}")
                        return
                    if attach_result and attach_result.get("_fsv_noop"):
                        skipped = ", ".join(attach_result.get("skipped") or [])
                        console.print(f"[yellow]no changes[/] {format_id({'id': cid}, res)}" + (f" ({skipped} already attached)" if skipped else ""))
                        return
                    if attach_result:
                        if json_out:
                            emit_json(attach_result)
                        else:
                            console.print(f"[green]updated[/] {format_id({'id': cid}, res)}")
                        return

                _err("nothing to update; use --edit, --attach, --planning, --set, or pass --status/--priority/--agent/--group")
            except (SessionError, APIError) as e:
                _err(str(e))
            except (SystemExit, typer.Exit):
                raise
            except Exception as e:
                _err(str(e))

        @sub.command("download", help="Download change attachments.", epilog=(
            "[bold]Examples:[/bold]  "
            "fsv changes download CHN-1234  |  "
            "fsv changes download CHN-1234 --all --out ./evidence  |  "
            "fsv changes download CHN-1234 --planning 'Test Plan'"
        ))
        def download(
            id_: str = typer.Argument(..., metavar="ID"),
            planning: Optional[List[str]] = typer.Option(None, "--planning", help="planning field label/name/id (repeatable)", autocompletion=completion.complete_planning_field_names(res)),
            all_planning: bool = typer.Option(False, "--all-planning", help="download all planning field attachments"),
            main_attachments: bool = typer.Option(False, "--attachments", "--main-attachments", help="download main change attachments"),
            description_attachments: bool = typer.Option(False, "--description-attachments", help="download attachment links found in description HTML"),
            all_: bool = typer.Option(False, "--all", help="download planning, main, and description attachments"),
            out: Optional[Path] = typer.Option(None, "--out", "-o", help="output directory (default: ./CHN-<id>)"),
            force: bool = typer.Option(False, "--force", help="overwrite existing files"),
            json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
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
                out_dir = out or Path(f"CHN-{cid}")
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
                        field_id = resolve_planning_field(value, _api(lambda: schema_mod.load(res, c)).get("fields", []))
                        selected.append(field_id)
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
                _err(str(e))
            except (SystemExit, typer.Exit):
                raise
            except Exception as e:
                _err(str(e))
    else:
        if res == TICKETS:
            @sub.command("approvals", help="List ticket approvals.", epilog=(
                f"[bold]Examples:[/bold]  fsv {res.name} approvals {_pfx}-1234  |  fsv {res.name} approvals {_pfx}-1234 --json"
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
            dry_run: bool = typer.Option(False, "--dry-run", help="print resolved payload without updating"),
            yes: bool = typer.Option(False, "--yes", "-y", help="confirm terminal status transitions"),
            json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
            no_input: bool = typer.Option(False, "--no-input", help="fail instead of prompting"),
        ) -> None:
            """Update basic fields."""
            update_resource(res, id_, status, priority, agent_id, group_id, json_out, dry_run, yes, no_input, set_)

        _threads_key = "conversations" if not res.has_notes else "notes"
        _threads_flag = "--conversations" if not res.has_notes else "--notes"
        _threads_help = "download attachments from conversations/replies" if not res.has_notes else "download attachments from notes"

        @sub.command("download", help=f"Download {singular} attachments.", epilog=(
            f"[bold]Examples:[/bold]  fsv {res.name} download {_pfx}-1234  |  "
            f"fsv {res.name} download {_pfx}-1234 --all --out ./evidence"
        ))
        def download_generic(
            id_: str = typer.Argument(..., metavar="ID"),
            main_attachments: bool = typer.Option(False, "--attachments", "--main-attachments", help="download main attachments"),
            threads: bool = typer.Option(False, _threads_flag, help=_threads_help),
            description_attachments: bool = typer.Option(False, "--description-attachments", help="download attachment links found in description HTML"),
            all_: bool = typer.Option(False, "--all", help="download all attachments from all sources"),
            out: Optional[Path] = typer.Option(None, "--out", "-o", help="output directory"),
            force: bool = typer.Option(False, "--force", help="overwrite existing files"),
            json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
        ) -> None:
            try:
                cid = _cid(id_, res)
                c = _client()
                if all_:
                    main_attachments = True
                    threads = True
                    description_attachments = True
                if not main_attachments and not threads and not description_attachments:
                    main_attachments = True
                pfx = res.display_prefixes[0]
                out_dir = out or Path(f"{pfx}-{cid}")
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
                    sub_data = c.int_get(f"{res.api_path}/{cid}/{_threads_key}")
                    for thread in sub_data.get(_threads_key, []):
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
                _err(str(e))

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
            dry: bool = typer.Option(False, "--dry-run", help="print template to stdout, do not create"),
            with_tasks: bool = typer.Option(False, "--with-tasks", help="clone tasks from source change"),
            with_assets: bool = typer.Option(False, "--with-assets", help="associate same assets as source change"),
            with_planning: bool = typer.Option(False, "--with-planning", help="clone planning fields (text + attachments)"),
            json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
            no_input: bool = typer.Option(False, "--no-input", help="fail instead of prompting"),
        ) -> None:
            """Clone a change — pre-fills form and opens $EDITOR."""
            try:
                cid = _cid(id_, res)
                data = change_clone_data(cid)
                if dry:
                    emit_json(data)
                    return
                if no_edit:
                    created = submit_change(data)
                else:
                    body = _edit_body(data, f"Cloning change #{cid}", no_input)
                    created = submit_change(body)
                cid_out = created.get("id", "?")
                if with_tasks and cid_out != "?":
                    tasks = clone_tasks(cid, cid_out)
                    err.print(f"  cloned {len(tasks)} task(s)", highlight=False)
                if with_assets and cid_out != "?":
                    assets = clone_assets(cid, cid_out)
                    err.print(f"  associated {len(assets)} asset(s)", highlight=False)
                if with_planning and cid_out != "?":
                    planning = clone_planning_fields(cid, cid_out)
                    err.print(f"  cloned {len(planning)} planning field(s)", highlight=False)
                if json_out:
                    emit_json(created)
                else:
                    console.print(f"[green]created[/] {format_id(created, res)}  {res.portal_url}/{cid_out}")
            except (SessionError, APIError) as e:
                _err(str(e))
            except (SystemExit, typer.Exit):
                raise
            except Exception as e:
                _err(str(e))

        @sub.command("create", help="Create a change.", epilog=(
            "[bold]Examples:[/bold]  "
            "fsv changes create  |  "
            "fsv changes create --optional  |  "
            "fsv changes create --dry-run"
        ))
        def create(
            optional: bool = typer.Option(False, "--optional", "-o", help="include optional fields in template"),
            all_fields: bool = typer.Option(False, "--all", "-a", help="include all fields in template"),
            dry: bool = typer.Option(False, "--dry-run", help="print template to stdout, do not create"),
            json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
            no_input: bool = typer.Option(False, "--no-input", help="fail instead of prompting"),
        ) -> None:
            """Create a new change via $EDITOR.

            Opens $EDITOR with a JSON template.
            Edit, save, and close to submit."""
            try:
                level = "all" if all_fields else ("optional" if optional else "required")
                template = change_template(level)
                if dry:
                    emit_json(template)
                    return
                body = _edit_body(template, "Creating new change", no_input)
                created = submit_change(body)
                cid = created.get("id", "?")
                if json_out:
                    emit_json(created)
                else:
                    console.print(f"[green]created[/] {format_id(created, res)}  {res.portal_url}/{cid}")
            except (SessionError, APIError) as e:
                _err(str(e))
            except (SystemExit, typer.Exit):
                raise
            except Exception as e:
                _err(str(e))

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
            json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
            all_pages: bool = typer.Option(False, "--all", "-a", help="fetch all pages (auto-paginate)"),
            n_pages: Optional[int] = typer.Option(None, "--npages", "-N", help="fetch exactly N pages"),
        ) -> None:
            """List notes using the browser-session internal endpoint."""
            notes_resource(res, id_, page, per_page, json_out, all_pages, n_pages)

        @sub.command("add-note", help=f"Add {singular} note.", epilog=(
            f"[bold]Note:[/bold] Changes/problems use add-note; tickets use reply.  "
            f"[bold]Examples:[/bold]  "
            f"fsv {res.name} add-note {_pfx}-1234 'Checked deployment evidence'  |  "
            f"fsv {res.name} add-note {_pfx}-1234 'Visible update' --public"
        ))
        def add_note(
            id_: str = typer.Argument(...),
            body: str = typer.Argument(..., help="HTML or plain text"),
            public: bool = typer.Option(False, "--public"),
        ) -> None:
            note_resource(res, id_, body, public)
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
            json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
            all_pages: bool = typer.Option(False, "--all", "-a", help="fetch all pages (auto-paginate)"),
            n_pages: Optional[int] = typer.Option(None, "--npages", "-N", help="fetch exactly N pages"),
        ) -> None:
            conversations_resource(id_, page, per_page, json_out, all_pages, n_pages)

        @sub.command("reply", help="Add a ticket reply.", epilog=(
            f"[bold]Note:[/bold] Tickets use reply; changes/problems use add-note.  "
            f"[bold]Examples:[/bold]  "
            f"fsv {res.name} reply {_pfx}-1234 'Working on this'"
        ))
        def reply(
            id_: str = typer.Argument(...),
            body: str = typer.Argument(...),
        ) -> None:
            """Add a reply (conversation) to a ticket."""
            reply_resource(res, id_, body)

    return sub


def _resolve(name: str) -> Resource:
    name = name.lower().rstrip("s") + "s"
    res = REGISTRY.get(name)
    if not res:
        raise typer.BadParameter(f"unknown resource '{name}'; expected: {list(REGISTRY)}")
    return res


@app.command("search", help="Global search across all entities (tickets, problems, changes, tasks, assets, solutions).", epilog=(
    "[bold]Examples:[/bold]  fsv search 'EDP'  |  fsv search 'EDP' --npages 3  |  fsv search 'EDP' --all  |  fsv search 'data platform' --json"
))
def global_search_cmd(
    query: str = typer.Argument(..., help="Free-text term, e.g. 'EDP'"),
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
        _err(str(e))
    except typer.Exit:
        raise
    except Exception as e:
        _err(str(e))


@app.command()
def tui() -> None:
    """Launch interactive TUI."""
    from fsv.tui import launch
    launch()


for r in REGISTRY.values():
    app.add_typer(_make_subapp(r), name=r.name)


if __name__ == "__main__":
    app()
