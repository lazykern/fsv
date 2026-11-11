"""Captured canned-response reads and local insertion into a reviewed reply.

Detail payload keys are not a portable template schema: the caller selects an
explicit JSON pointer from `show`. No placeholders, attachments or signatures
are expanded, and the complete source is rechecked before sending.
"""
from __future__ import annotations

from dataclasses import replace
import re
from typing import Any

from fsv import journal
from fsv.watchers import positive_id
from fsv.ticket_conversation import ConversationError, ConversationPlan, build_plan


def folders(client: Any) -> Any:
    return client.int_get("canned_response_folders")


def responses(client: Any, folder_id: int) -> Any:
    return client.int_get(f"canned_response_folders/{positive_id(folder_id)}/canned_responses")


def show(client: Any, response_id: int) -> Any:
    return client.int_get(f"canned_responses/{positive_id(response_id)}")


def content(raw: Any, pointer: str) -> str:
    """Select a string using RFC 6901 object pointers, never a guessed key."""
    if not pointer.startswith("/"):
        raise ValueError("content pointer must start with /; inspect canned show first")
    value = raw
    for part in pointer[1:].split("/"):
        if re.search(r"~(?![01])", part):
            raise ValueError("invalid JSON pointer escape")
        key = part.replace("~1", "/").replace("~0", "~")
        if not isinstance(value, dict) or key not in value:
            raise ValueError("content pointer does not identify a source field")
        value = value[key]
    if not isinstance(value, str) or not value.strip():
        raise ValueError("selected canned content must be nonempty text/HTML")
    if re.search(r"\{\{|\{%|\$\{|\[\[", value):
        raise ValueError("unresolved canned placeholders require the web editor")
    return value


def reply_plan(client: Any, target: str, response_id: int, pointer: str, *, sender: str) -> ConversationPlan:
    response_id = positive_id(response_id)
    raw = show(client, response_id)
    plan = build_plan(client, target, content(raw, pointer), body_format="html", sender=sender)
    return replace(plan, canned_id=response_id, canned_pointer=pointer,
                   canned_hash=journal.plan_hash(raw))


def freshen(plan: ConversationPlan, client: Any) -> None:
    raw = show(client, plan.canned_id)
    if journal.plan_hash(raw) != plan.canned_hash or content(raw, plan.canned_pointer) != plan.payload["body"]:
        raise ConversationError("canned source changed; review again")


def scenarios(client: Any) -> Any:
    return client.int_get("scenario_automations")


def scenario(client: Any, scenario_id: int) -> dict[str, Any]:
    """Select exact inventory identity; no uncaptured detail endpoint."""
    raw = scenarios(client)
    rows = raw.get("scenario_automations") if isinstance(raw, dict) else raw
    if not isinstance(rows, list):
        raise ValueError("scenario inventory is not a list; inspect scenarios list")
    matches = [row for row in rows if isinstance(row, dict) and str(row.get("id")) == str(positive_id(scenario_id))]
    if len(matches) != 1:
        raise ValueError("scenario identity is missing or ambiguous in this inventory")
    return matches[0]
