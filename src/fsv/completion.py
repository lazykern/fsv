from __future__ import annotations

import json
import os
import re
from pathlib import Path
from typing import TYPE_CHECKING, Any, Iterable

if TYPE_CHECKING:
    import typer
elif not os.environ.get("FSV_THIN_COMPLETE"):
    import typer

from fsv.cache import load as _cache_load
from fsv.config import (
    CONFIG_DIR as _CFG,
    filters_cache_candidates,
    groups_cache_candidates,
    schema_cache_candidates,
    schema_cache_path,
)
from fsv.resources import Resource


def _config_load() -> dict[str, Any]:
    p = _CFG / "config.json"
    if not p.exists():
        return {}
    try:
        return json.loads(p.read_text())
    except Exception:
        return {}


def _completion_network() -> bool:
    if os.environ.get("FSV_COMPLETION_NETWORK") == "1":
        return True
    cfg = _config_load()
    return cfg.get("completion", {}).get("network") in ("on", True, "true", "1")

FALLBACK_FIELDS = [
    ("requester", "Requester", "default_requester", "default"),
    ("agent", "Agent", "default_agent", "default"),
    ("group", "Group", "default_group", "default"),
    ("status", "Status", "default_status", "default"),
    ("priority", "Priority", "default_priority", "default"),
    ("created_at", "Created", "default_date", "default"),
    ("updated_at", "Updated", "default_date", "default"),
]

FALLBACK_CHOICES = {
    "status": ["Open", "Pending", "Resolved", "Closed"],
    "priority": ["Low", "Medium", "High", "Urgent"],
    "impact": ["Low", "Medium", "High"],
    "risk": ["Low", "Medium", "High", "Very High"],
}

DATE_EXAMPLES = [
    "2025-01-01T00:00:00+07:00",
    "2025-12-31T23:59:59+07:00",
]

OPERATORS = (">=", "<=", "!=", "~=", "=", ">", "<")


def _schema_path(res: Resource) -> Path:
    return schema_cache_path(res.name)


def _cached_fields(res: Resource) -> list[dict[str, Any]]:
    doc = None
    for path in schema_cache_candidates(res.name):
        doc, _stale = _cache_load(path)
        if doc is not None:
            break
    if doc is None:
        return []
    data = doc.get("data") or doc.get("fields")
    fields = data if isinstance(data, list) else (doc.get("fields") or [])
    return fields


def _field_scope(f: dict[str, Any]) -> str:
    return "default" if f.get("default_field") else "custom"


def _field_name(f: dict[str, Any]) -> str:
    return str(f.get("name") or "")


def _field_label(f: dict[str, Any]) -> str:
    return str(f.get("label") or f.get("name") or "")


def _field_help(f: dict[str, Any]) -> str:
    return f"{_field_label(f)} · {f.get('field_type') or '?'} · {_field_scope(f)}"


def _fields_or_fallback(res: Resource) -> list[dict[str, Any]]:
    fields = _cached_fields(res)
    if fields:
        return fields
    return [
        {"name": name, "label": label, "field_type": ftype, "default_field": scope == "default"}
        for name, label, ftype, scope in FALLBACK_FIELDS
    ]


def _match(text: str, incomplete: str) -> bool:
    return text.casefold().startswith(incomplete.casefold())


def _field_matches(f: dict[str, Any], incomplete: str) -> bool:
    return _match(_field_name(f), incomplete) or _match(_field_label(f), incomplete)


def _find_field(res: Resource, name: str) -> dict[str, Any] | None:
    raw = name
    scope = None
    if ":" in name:
        maybe, rest = name.split(":", 1)
        if maybe in ("default", "custom"):
            scope, raw = maybe, rest
    folded = raw.casefold()
    fields = _fields_or_fallback(res)
    if scope == "default":
        fields = [f for f in fields if f.get("default_field")]
    elif scope == "custom":
        fields = [f for f in fields if not f.get("default_field")]
    exact = [
        f for f in fields
        if _field_name(f).casefold() == folded or _field_label(f).casefold() == folded
    ]
    if exact:
        return exact[0]
    matches = [f for f in fields if raw.casefold() in _field_name(f).casefold() or raw.casefold() in _field_label(f).casefold()]
    return matches[0] if len(matches) == 1 else None


def _choice_text(c: dict[str, Any]) -> str:
    return str(c.get("value") or c.get("name") or c.get("label") or c.get("id") or "")


def _choices_for(res: Resource, field: str) -> list[str]:
    f = _find_field(res, field)
    if not f:
        return FALLBACK_CHOICES.get(field, [])
    choices = [_choice_text(c) for c in f.get("choices") or []]
    choices = [x for x in choices if x]
    return choices or FALLBACK_CHOICES.get(_field_name(f), [])


def _where_parts(incomplete: str) -> tuple[str, str, str] | None:
    for op in OPERATORS:
        if op in incomplete:
            left, right = incomplete.split(op, 1)
            return left.strip(), op, right
    return None


def complete_field_names(res: Resource):
    def complete(incomplete: str) -> Iterable[tuple[str, str]]:
        for f in _fields_or_fallback(res):
            name = _field_name(f)
            if name and _field_matches(f, incomplete):
                yield (name, _field_help(f))
        yield (incomplete, "")
    return complete


def complete_choice_field_names(res: Resource):
    def complete(incomplete: str) -> Iterable[tuple[str, str]]:
        for f in _fields_or_fallback(res):
            name = _field_name(f)
            if not name or not _field_matches(f, incomplete):
                continue
            choices = f.get("choices") or []
            if choices:
                yield (name, f"{_field_help(f)} · {len(choices)} choices")
        yield (incomplete, "")
    return complete


def complete_planning_field_names(res: Resource):
    def complete(incomplete: str) -> Iterable[tuple[str, str]]:
        for f in _fields_or_fallback(res):
            name = _field_name(f)
            label = _field_label(f)
            ftype = str(f.get("field_type") or "")
            planning_types = {"planning_field", "default_change_reason", "default_change_impact", "default_change_plan", "default_backout_plan"}
            if ftype not in planning_types:
                continue
            if name and (_match(name, incomplete) or _match(label, incomplete)):
                yield (label or name, name)
                yield (name, label)
        yield (incomplete, "")
    return complete


def complete_lookup_kind(res: Resource):
    def complete(incomplete: str) -> Iterable[tuple[str, str]]:
        base = [
            ("requester", "requester autocomplete"),
            ("agent", "agent autocomplete"),
            ("group", "group lookup"),
        ]
        for value, help_text in base:
            if _match(value, incomplete):
                yield (value, help_text)
        for f in _fields_or_fallback(res):
            name = _field_name(f)
            if name and _field_matches(f, incomplete):
                yield (name, _field_help(f))
        yield (incomplete, "")
    return complete


def complete_lookup_query(res: Resource):
    def complete(ctx: typer.Context, incomplete: str) -> Iterable[tuple[str, str]]:
        kind = str(ctx.params.get("kind") or "")
        if kind in ("requester", "requesters"):
            for value, detail in _remote_user_values("requesters", incomplete):
                yield (value, detail)
            yield (incomplete, "")
            return
        if kind in ("agent", "agents"):
            for value, detail in _remote_user_values("agents", incomplete):
                yield (value, detail)
            yield (incomplete, "")
            return
        if kind in ("group", "groups"):
            for g in _cached_groups():
                name = str(g.get("name") or "")
                desc = str(g.get("description") or "")
                if name and _match(name, incomplete):
                    yield (name, desc)
            yield (incomplete, "")
            return
        if not kind:
            yield (incomplete, "")
            return
        for choice in _choices_for(res, kind):
            if _match(choice, incomplete):
                yield (choice, kind)
        yield (incomplete, "")
    return complete


def _completion_case(candidate: str, query: str) -> str:
    if candidate.startswith(query):
        return candidate
    if candidate.casefold().startswith(query.casefold()):
        return query + candidate[len(query):]
    return candidate


def _remote_user_values(kind: str, query: str) -> list[tuple[str, str]]:
    query = query.strip()
    if not _completion_network() or len(query) < 2:
        return []
    try:
        from fsv.client import get_client

        params = {"all_users": "true"} if kind == "requesters" else None
        rows = get_client().autocomplete(kind, query, params)
    except Exception:
        return []
    out = []
    for row in rows[:10]:
        candidates = [
            str(row.get("email") or ""),
            str(row.get("details") or ""),
            str(row.get("value") or ""),
            str(row.get("name") or ""),
        ]
        value = next((x for x in candidates if x and x.casefold().startswith(query.casefold())), "")
        if not value:
            value = str(row.get("value") or row.get("name") or row.get("email") or "")
        detail = str(row.get("value") or row.get("name") or row.get("email") or row.get("details") or row.get("id") or row.get("user_id") or "")
        if value:
            out.append((_completion_case(value, query), detail))
    return out


def _complete_pairs(incomplete: str, pairs: Iterable[tuple[str, str]]) -> Iterable[tuple[str, str]]:
    for value, help_text in pairs:
        if _match(value, incomplete):
            yield (value, help_text)
    yield (incomplete, "")


def complete_format(incomplete: str) -> Iterable[tuple[str, str]]:
    yield from _complete_pairs(incomplete, (
        ("table", "rich table"),
        ("json", "JSON array"),
        ("csv", "comma-separated values"),
        ("tsv", "tab-separated values"),
    ))


def _cached_filters(res: Resource) -> list[dict[str, str]]:
    if not res.filters_path:
        return []
    doc = None
    for path in filters_cache_candidates(res.name):
        doc, _stale = _cache_load(path)
        if doc is not None:
            break
    if doc is None:
        return []
    data_obj = doc.get("data")
    filters = (data_obj or doc).get("filters") if isinstance(data_obj, dict) else (data_obj or doc.get("filters"))
    if not isinstance(filters, list):
        filters = doc.get("filters") or []
    return [
        {"id": str(f.get("id") or ""), "name": str(f.get("name") or "")}
        for f in filters
        if isinstance(f, dict) and (f.get("id") or f.get("name"))
    ]


def _cached_groups() -> list[dict[str, Any]]:
    doc = None
    for path in groups_cache_candidates():
        doc, _stale = _cache_load(path)
        if doc is not None:
            break
    if doc is None:
        return []
    data_obj = doc.get("data")
    groups = (data_obj or doc).get("groups") if isinstance(data_obj, dict) else (data_obj or doc.get("groups"))
    if not isinstance(groups, list):
        groups = doc.get("groups") or []
    return groups


def complete_store(incomplete: str) -> Iterable[tuple[str, str]]:
    yield from _complete_pairs(incomplete, (
        ("file", "plain JSON, chmod 600"),
        ("argon", "Argon2id + AES-256-GCM encrypted"),
        ("keychain", "macOS Keychain"),
    ))


def complete_filter_name(res: Resource):
    def complete(incomplete: str) -> Iterable[tuple[str, str]]:
        for item in _cached_filters(res):
            view_id = item["id"]
            name = item["name"]
            if view_id and (_match(view_id, incomplete) or _match(name, incomplete)):
                yield (view_id, name or "saved view")
        yield (incomplete, "")
    return complete


def complete_group_query(incomplete: str) -> Iterable[tuple[str, str]]:
    for g in _cached_groups():
        name = str(g.get("name") or "")
        desc = str(g.get("description") or "")
        if name and _match(name, incomplete):
            yield (name, desc)
    yield (incomplete, "")


def complete_update_choice(res: Resource, field_name: str):
    def complete(incomplete: str) -> Iterable[tuple[str, str]]:
        emitted: set[str] = set()
        for f in _fields_or_fallback(res):
            if _field_name(f) == field_name:
                for c in f.get("choices") or []:
                    cid = str(c.get("id") or "")
                    label = str(c.get("value") or c.get("name") or c.get("label") or cid)
                    for value, help_text in ((label, f"id={cid}" if cid else ""), (cid, label)):
                        if value and value not in emitted and _match(value, incomplete):
                            emitted.add(value)
                            yield (value, help_text)
                break
        if field_name in FALLBACK_CHOICES:
            for label in FALLBACK_CHOICES[field_name]:
                if label not in emitted and _match(label, incomplete):
                    emitted.add(label)
                    yield (label, field_name)
        yield (incomplete, "")
    return complete


def complete_set(res: Resource):
    def complete(incomplete: str) -> Iterable[tuple[str, str]]:
        incomplete = incomplete.strip("\"'")
        if "=" not in incomplete:
            for f in _fields_or_fallback(res):
                name = _field_name(f)
                label = _field_label(f)
                if not name or not _field_matches(f, incomplete):
                    continue
                if _match(name, incomplete):
                    yield (f"{name}=", _field_help(f))
                else:
                    label_completion = incomplete + label[len(incomplete):]
                    yield (f"{label_completion}=", f"{name} · {f.get('field_type') or '?'} · {_field_scope(f)}")
            return
        field, _, raw_value = incomplete.partition("=")
        f = _find_field(res, field)
        if not f:
            yield (incomplete, "")
            return
        field_name = _field_name(f)
        ftype = str(f.get("field_type") or "")
        if "checkbox" in ftype:
            for v in ("true", "false"):
                if _match(v, raw_value):
                    yield (f"{field}={v}", "")
            yield (incomplete, "")
            return
        if field_name in ("planned_start_date", "planned_end_date", "created_at", "updated_at"):
            for v in DATE_EXAMPLES:
                if _match(v, raw_value):
                    yield (f"{field}={v}", "ISO-8601")
            yield (incomplete, "")
            return
        if "lookup" in ftype:
            if not _completion_network() or len(raw_value.strip()) < 2:
                yield (incomplete, "")
                return
            link = (f.get("lookup_config") or {}).get("link")
            if link:
                try:
                    from fsv.client import get_client
                    rows = get_client().lookup_choices(link, raw_value)
                    # Sort: prefix-matched names/emails first, substring-only after
                    r = raw_value.casefold()
                    rows.sort(
                        key=lambda x: 0
                        if str(x.get("email") or "").casefold().startswith(r)
                        or str(x.get("name") or "").casefold().startswith(r)
                        else 1
                    )
                    shown = 0
                    for row in rows:
                        name_ = str(row.get("name") or "")
                        email_ = str(row.get("email") or "")
                        if email_ and _match(email_, raw_value):
                            suffix = email_[len(raw_value):]
                            yield (f"{field}={raw_value}{suffix}", name_)
                            shown += 1
                        elif name_ and _match(name_, raw_value):
                            suffix = name_[len(raw_value):]
                            yield (f"{field}={raw_value}{suffix}", email_)
                            shown += 1
                        if shown >= 20:
                            break
                except Exception:
                    pass
            else:
                seen: set[str] = set()
                for kind in ("agents", "requesters"):
                    for v, detail in _remote_user_values(kind, raw_value):
                        if v not in seen:
                            seen.add(v)
                            yield (f"{field}={v}", detail)
            yield (incomplete, "")
            return
        choices = [_choice_text(c) for c in f.get("choices") or [] if _choice_text(c)]
        if choices:
            for c in choices:
                if _match(c, raw_value):
                    yield (f"{field}={c}", field_name)
            yield (incomplete, "")
            return
        yield (incomplete, "")
    return complete


def complete_config_key(incomplete: str) -> Iterable[tuple[str, str]]:
    yield from _complete_pairs(incomplete, (
        ("completion.network", "enable remote requester/agent completion [on|off]"),
    ))


def complete_config_value(incomplete: str) -> Iterable[tuple[str, str]]:
    for value in ("on", "off", "true", "false", "1", "0"):
        if _match(value, incomplete):
            yield (value, "")
    yield (incomplete, "")


def complete_cache_target(incomplete: str) -> Iterable[tuple[str, str]]:
    yield from _complete_pairs(incomplete, (
        ("schema", "field definitions"),
        ("filters", "saved view names"),
        ("groups", "agent groups"),
        ("all", "everything"),
    ))


def complete_update_agent_id(incomplete: str) -> Iterable[tuple[str, str]]:
    if not _completion_network() or len(incomplete.strip()) < 2:
        yield (incomplete, "")
        return
    for value, detail in _remote_user_values("agents", incomplete):
        yield (value, detail)


def complete_update_group_id(incomplete: str) -> Iterable[tuple[str, str]]:
    for g in _cached_groups():
        gid = str(g.get("id") or "")
        name = str(g.get("name") or "")
        desc = str(g.get("description") or "")
        detail = f"id={gid}" + (f" · {desc}" if desc else "")
        if name and _match(name, incomplete):
            yield (name, detail)
        if gid and _match(gid, incomplete):
            yield (gid, name)
    yield (incomplete, "")


def complete_shell(incomplete: str) -> Iterable[tuple[str, str]]:
    yield from _complete_pairs(incomplete, (
        ("bash", "Bash"),
        ("zsh", "Z shell"),
        ("fish", "Fish shell"),
        ("powershell", "Windows PowerShell"),
        ("pwsh", "PowerShell Core"),
    ))


def complete_sort_order(incomplete: str) -> Iterable[tuple[str, str]]:
    yield from _complete_pairs(incomplete, (
        ("asc", "ascending"),
        ("desc", "descending"),
    ))


def complete_search_sort(incomplete: str) -> Iterable[tuple[str, str]]:
    yield from _complete_pairs(incomplete, (
        ("relevance", "best text match"),
        ("created", "created time"),
        ("modified", "last modified time"),
    ))


def complete_duplicate_mode(incomplete: str) -> Iterable[tuple[str, str]]:
    yield from _complete_pairs(incomplete, (
        ("prompt", "ask on duplicate filename"),
        ("skip", "keep existing, skip new file"),
        ("replace", "replace existing file"),
        ("append", "upload alongside existing file"),
    ))


def complete_help_topic(topics: Iterable[str]):
    topic_list = [(topic, "help topic") for topic in topics]

    def complete(incomplete: str) -> Iterable[tuple[str, str]]:
        yield from _complete_pairs(incomplete, topic_list)

    return complete


def complete_where(res: Resource):
    def complete(incomplete: str) -> Iterable[tuple[str, str]]:
        parts = _where_parts(incomplete)
        if parts is None:
            for f in _fields_or_fallback(res):
                name = _field_name(f)
                if name and _field_matches(f, incomplete):
                    yield (f"{name}=", _field_help(f))
            for name, help_text in (
                ("created_at>=", "created lower bound"),
                ("created_at<=", "created upper bound"),
                ("updated_at>=", "updated lower bound"),
            ):
                if _match(name, incomplete):
                    yield (name, help_text)
            return

        field, op, raw_value = parts
        f = _find_field(res, field)
        field_name = _field_name(f) if f else field
        field_label = _field_label(f) if f else field
        if field_name in ("created_at", "updated_at", "due_by", "planned_start_date", "planned_end_date"):
            for value in DATE_EXAMPLES:
                if _match(value, raw_value):
                    yield (f"{field}{op}{value}", "ISO-8601 timestamp")
            yield (incomplete, "")  # suppress file fallback
            return
        if field_name == "requester":
            for value, detail in _remote_user_values("requesters", raw_value):
                yield (f"{field}{op}{value}", detail)
            yield (incomplete, "")  # suppress file fallback
            return
        if field_name == "agent":
            for value, detail in _remote_user_values("agents", raw_value):
                yield (f"{field}{op}{value}", detail)
            yield (incomplete, "")  # suppress file fallback
            return
        choices = _choices_for(res, field)
        if choices:
            for choice in choices:
                if _match(choice, raw_value):
                    yield (f"{field}{op}{choice}", field_name)
            yield (incomplete, "")  # suppress file fallback
        else:
            yield (incomplete, "")  # suppress file fallback
    return complete
