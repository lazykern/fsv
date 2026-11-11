"""Service-layer helpers for TUI write actions (no Textual imports).

Everything here is synchronous and talks to ``fsv.service`` through the module
attribute, so tests can substitute the service functions. Writes go through the
shared client, which journals every non-GET request, so no extra recording is
needed here.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from fsv import config, safety, schema as schema_mod, service
from fsv.errors import APIError, SessionError
from fsv.resources import Resource

WRITABLE_STATUS = ("tickets", "changes")


def resolve_read_only(flag: bool = False) -> bool:
    """CLI flag wins; otherwise the ``tui.read_only`` config option."""
    if flag:
        return True
    try:
        cfg = config.load_config().get("tui") or {}
    except Exception:
        return False
    if not isinstance(cfg, dict):
        return False
    val = cfg.get("read_only")
    if isinstance(val, str):
        return val.strip().lower() in ("on", "true", "1", "yes")
    return bool(val)


def can_change_status(resource: Resource) -> bool:
    return resource.name in WRITABLE_STATUS


def can_note(resource: Resource) -> bool:
    return resource.name in ("tickets", "changes", "problems") and resource.has_notes


def can_reply(resource: Resource) -> bool:
    return resource.name == "tickets"


def status_choices(schema: dict[str, Any]) -> list[tuple[Any, str]]:
    """(id, label) pairs for the status field, in schema order."""
    f = schema_mod.field("status", schema) if schema.get("fields") is not None else None
    out: list[tuple[Any, str]] = []
    for c in (f or {}).get("choices") or []:
        cid = c.get("id", c.get("value"))
        label = c.get("value") or c.get("name") or str(cid)
        if cid is not None:
            out.append((cid, str(label)))
    return out


@dataclass
class ConfirmPlan:
    title: str
    lines: list[str]
    risks: list[str] = field(default_factory=list)


def status_plan(resource: Resource, item: dict[str, Any], schema: dict[str, Any],
                new_id: Any, new_label: str) -> ConfirmPlan:
    old = service.resolve_status(item, resource, schema)
    risks = safety.write_risks(
        resource.name, {"status": new_id}, current=item,
        status_label=lambda v: schema_mod.choice_label("status", v, schema) or str(v),
    )
    lines = [f"Status: {old}  ->  {new_label}"]
    lines += risks or ["No side-effect warnings for this change."]
    if resource.name == "changes":
        lines.append("Changing a change's status can send approval requests and fire workflows.")
    return ConfirmPlan("Confirm status change", lines, risks)


def note_plan(resource: Resource, public: bool, reply: bool) -> ConfirmPlan | None:
    """None when no confirmation is needed (private note)."""
    risks = safety.reply_risks() if reply else safety.note_risks(public)
    if not risks:
        return None
    return ConfirmPlan("Confirm reply" if reply else "Confirm public note", list(risks), risks)


def apply_status(resource: Resource, item_id: int, new_id: Any) -> dict[str, Any]:
    return service.update_item(resource, item_id, {"status": new_id})


def apply_note(resource: Resource, item_id: int, body: str, public: bool) -> dict[str, Any]:
    return service.add_note(resource, item_id, body, public=public)


def apply_reply(resource: Resource, item_id: int, body: str) -> dict[str, Any]:
    return service.add_reply(resource, item_id, body)


def clean_error(exc: BaseException) -> str:
    """One-line message suitable for a toast."""
    if isinstance(exc, SessionError):
        return str(exc) or "Session expired. Log in again."
    if isinstance(exc, APIError):
        body = exc.body
        if isinstance(body, dict):
            body = body.get("description") or body.get("message") or body.get("errors") or body
        text = " ".join(str(body).split())
        return f"Server rejected the request (HTTP {exc.status}): {text[:200]}"
    return " ".join(str(exc).split())[:240] or type(exc).__name__
