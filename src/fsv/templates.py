"""Ticket templates (read-only) and agent notification entries.

Captured: ``GET /api/_/ticket-templates`` -> ``{"ticket_templates": [{id, name,
type, workspace_id}], "meta": {"max_template_count": N}}`` (N is the current
count, not a cap) and ``GET /api/_/ticket-templates/<id>`` ->
``{"ticket_template": {id, name, template_data: {subject, status, source,
priority, urgency, impact, description, custom_fields, ...}}}``. Applying a
template in the new-ticket form only prefills; fsv does the same and every
value still goes through the form's fields, business rules and review.
Template create/delete use classic admin form posts and are not implemented.
"""
from __future__ import annotations

from typing import Any

from fsv.drafts import Draft

# Default template_data keys that map 1:1 onto new-ticket form fields.
PREFILL_KEYS = ("subject", "status", "source", "priority", "urgency", "impact", "category", "sub_category",
                "item_category")


class TemplateError(ValueError):
    pass


def _positive(value: Any) -> int | None:
    if isinstance(value, bool):
        return None
    if isinstance(value, int) and value > 0:
        return value
    if isinstance(value, str) and value.isascii() and value.isdigit() and int(value) > 0:
        return int(value)
    return None


def list_templates(client: Any) -> list[dict[str, Any]]:
    raw = client.int_get("ticket-templates")
    rows = raw.get("ticket_templates") if isinstance(raw, dict) else None
    if not isinstance(rows, list) or not all(isinstance(r, dict) and _positive(r.get("id")) and
                                             isinstance(r.get("name"), str) for r in rows):
        raise TemplateError("ticket template list unavailable or malformed")
    ids = [_positive(r["id"]) for r in rows]
    if len(set(ids)) != len(ids):
        raise TemplateError("ticket template list has duplicate ids")
    return rows


def find(client: Any, ident: str) -> dict[str, Any]:
    rows = list_templates(client)
    wanted = ident.strip()
    found = [r for r in rows if str(r["id"]) == wanted] or \
        [r for r in rows if r["name"].strip().casefold() == wanted.casefold()]
    if len(found) != 1:
        names = ", ".join(r["name"] for r in rows[:12]) or "none"
        raise TemplateError(f"no unique ticket template {ident!r}; templates: {names}")
    return found[0]


def show(client: Any, ident: str) -> dict[str, Any]:
    tid = _positive(find(client, ident)["id"])
    raw = client.int_get(f"ticket-templates/{tid}")
    row = raw.get("ticket_template") if isinstance(raw, dict) else None
    if not isinstance(row, dict) or _positive(row.get("id")) != tid or not isinstance(row.get("template_data"), dict):
        raise TemplateError("exact ticket template unavailable")
    return row


def to_draft(template: dict[str, Any]) -> tuple[Draft, list[str], list[str]]:
    """(draft, applied keys, ignored keys). Empty values are not prefilled."""
    data = template["template_data"]
    answers: dict[str, Any] = {}
    applied, ignored = [], []
    for key, value in data.items():
        if value in (None, "", [], {}):
            continue
        if key in PREFILL_KEYS and isinstance(value, (str, int)) and not isinstance(value, bool):
            answers[key] = value
            applied.append(key)
        elif key == "custom_fields" and isinstance(value, dict):
            for name, item in value.items():
                if item in (None, "", [], {}):
                    continue
                if isinstance(item, (str, int, bool)):
                    answers[name] = item
                    applied.append(name)
                else:
                    ignored.append(name)
        elif key != "description":
            ignored.append(key)
    description = data.get("description")
    if isinstance(description, str) and description.strip():
        applied.append("description")
    return (Draft(answers=answers, description_html=description if isinstance(description, str) and
                  description.strip() else None), applied, sorted(ignored))


def notify_entry(client: Any, text: str) -> str:
    """``Name <email>`` for one active agent from the agent autocomplete
    (the note editor's "Notify to" picker). Exact name or email only."""
    from fsv.ticket_notes import _valid_recipient
    wanted = text.strip().casefold()
    if not wanted:
        raise TemplateError("--notify-agent needs a name or email")
    rows = client.autocomplete("agents", text.strip(), {"qf": "name"}) if "@" not in text else \
        client.autocomplete("agents", text.strip())
    found = {}
    for row in rows if isinstance(rows, list) else []:
        if not isinstance(row, dict) or row.get("active") is False or row.get("deleted"):
            continue
        name = row.get("value") or row.get("name")
        emails = [v for v in (row.get("email"), row.get("details"), row.get("id")) if isinstance(v, str) and "@" in v]
        if not isinstance(name, str) or len({e.casefold() for e in emails}) != 1:
            continue
        email = emails[0]
        if name.strip().casefold() == wanted or email.casefold() == wanted:
            found[email.casefold()] = f"{name.strip()} <{email}>"
    if len(found) != 1:
        raise TemplateError(f"no unique active agent {text!r} for --notify-agent; use the exact name or email")
    entry = next(iter(found.values()))
    if not _valid_recipient(entry):
        raise TemplateError(f"agent entry for {text!r} is not a valid notification address")
    return entry
