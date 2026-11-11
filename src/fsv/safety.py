"""One place that decides whether a write needs confirmation.

``write_risks`` inspects the FINAL request body (after flags, --set and
--edit are merged) and returns human-readable reasons; an empty list means
the write is safe to send without asking. Labels come from the schema, so no
tenant status ids or names are hard-coded.
"""
from __future__ import annotations

import unicodedata
from typing import Any, Callable

TERMINAL_STATUS_WORDS = {"closed", "resolved", "rejected", "cancelled", "canceled"}
# Status labels known NOT to close a record. A label that is neither known
# terminal nor known open (a localized or custom status) is treated as
# possibly terminal: confirming one extra time beats closing silently.
OPEN_STATUS_WORDS = {"open", "pending", "in progress", "waiting", "on hold", "new", "planning", "awaiting",
                     "assigned", "scheduled", "review", "reopened"}
# Freshservice API default terminal status ids (tickets: 4 Resolved,
# 5 Closed; problems: 3 Closed), used when the label is unknown.
TERMINAL_STATUS_IDS = {"tickets": {"4", "5"}, "problems": {"3"}}
ASSIGNEE_KEYS = ("agent_id", "responder_id", "owner_id")
GROUP_KEYS = ("group_id",)


def norm(text: Any) -> str:
    """NFC-normalized, case-folded, whitespace-collapsed."""
    return " ".join(unicodedata.normalize("NFC", str(text or "")).casefold().split())


def is_terminal_label(label: str) -> bool:
    folded = norm(label)
    return any(word in folded for word in TERMINAL_STATUS_WORDS)


def is_open_label(label: str) -> bool:
    folded = norm(label)
    return not is_terminal_label(label) and any(word in folded for word in OPEN_STATUS_WORDS)


def status_closes(resource: str, value: Any, label: str) -> bool | None:
    """True: the status closes/resolves the record; False: known open;
    None: unknown (e.g. a localized or custom label)."""
    if is_terminal_label(label) or str(value) in TERMINAL_STATUS_IDS.get(resource, set()):
        return True
    if is_open_label(label):
        return False
    return None


def _plain(v: Any) -> Any:
    if isinstance(v, dict) and "id" in v:
        return v["id"]
    return v


def _same(a: Any, b: Any) -> bool:
    return str(_plain(a)) == str(_plain(b))


def write_risks(
    resource: str,
    body: dict[str, Any],
    *,
    current: dict[str, Any] | None = None,
    status_label: Callable[[Any], str] | None = None,
    kind: str = "item",
) -> list[str]:
    """Reasons this write needs confirmation (empty = none).

    resource: "changes" | "tickets" | "problems" | ...; kind: "item" | "task".
    current: the item as stored now (flat); when given, unchanged values are
    not risks. Unknown current => any provided status/assignee counts.
    """
    label_of = status_label or (lambda v: str(v))
    risks: list[str] = []
    cur = current or {}

    if "status" in body and not (current is not None and "status" in cur and _same(body["status"], cur["status"])):
        label = label_of(_plain(body["status"]))
        if kind == "task":
            risks.append(f"task status -> {label}")
        elif resource == "changes":
            # every change status transition can fire approvals/workflows
            risks.append(f"change status -> {label}")
        else:
            what = resource[:-1] if resource.endswith("s") else resource
            closes = status_closes(resource, _plain(body["status"]), label)
            if closes:
                risks.append(f"{what} status -> {label}")
            elif closes is None:
                risks.append(f"{what} status -> {label} (fsv cannot tell whether this status closes it)")

    for key in ASSIGNEE_KEYS:
        if key in body and not (current is not None and key in cur and _same(body[key], cur[key])):
            risks.append(f"reassign agent ({key}={_plain(body[key])})")
            break
    for key in GROUP_KEYS:
        if key in body and not (current is not None and key in cur and _same(body[key], cur[key])):
            risks.append(f"reassign group ({key}={_plain(body[key])})")
            break
    return risks


def note_risks(public: bool) -> list[str]:
    return ["public note: visible to the requester and may send notifications"] if public else []


def reply_risks() -> list[str]:
    return ["ticket reply: sent to the requester by email"]


def clone_risks(with_tasks: bool) -> list[str]:
    if not with_tasks:
        return []
    return ["--with-tasks: creates tasks that can notify their groups and trigger tenant workflows"]
