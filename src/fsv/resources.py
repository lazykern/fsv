from __future__ import annotations

import re
from dataclasses import dataclass, field


@dataclass(frozen=True)
class Resource:
    name: str                       # canonical key
    api_path: str                   # url path segment
    list_key: str                   # JSON list field
    item_key: str                   # JSON detail field
    form_fields_path: str           # v2 schema endpoint
    form_fields_key: str            # JSON key in schema response
    filters_path: str | None        # /api/_/<this>
    display_prefixes: tuple[str, ...]
    sub_resources: tuple[str, ...]
    list_include: str               # csv for ?include= on list endpoint
    has_notes: bool = True

    @property
    def portal_url(self) -> str:
        from fsv.config import DOMAIN
        return f"https://{DOMAIN}/a/{self.api_path}"


CHANGES = Resource(
    name="changes",
    api_path="changes",
    list_key="changes",
    item_key="change",
    form_fields_path="change_form_fields",
    form_fields_key="change_fields",
    filters_path="change-filters",
    display_prefixes=("CHN",),
    sub_resources=("notes", "tasks", "activities", "approvals", "time_entries"),
    list_include="change_status,agent,requester,group",
)

TICKETS = Resource(
    name="tickets",
    api_path="tickets",
    list_key="tickets",
    item_key="ticket",
    form_fields_path="ticket_form_fields",
    form_fields_key="ticket_fields",
    filters_path="ticket_filters",
    display_prefixes=("INC", "SR"),
    sub_resources=("conversations", "tasks", "activities", "time_entries"),
    list_include="ticket_status,responder,requester,group",
    has_notes=False,
)

PROBLEMS = Resource(
    name="problems",
    api_path="problems",
    list_key="problems",
    item_key="problem",
    form_fields_path="problem_form_fields",
    form_fields_key="problem_fields",
    filters_path="problem-filters",
    display_prefixes=("PRB",),
    sub_resources=("notes", "tasks", "activities", "time_entries"),
    list_include="problem_status,agent,requester,group",
)

REGISTRY: dict[str, Resource] = {r.name: r for r in (CHANGES, TICKETS, PROBLEMS)}


def parse_id(s: str, resource: Resource) -> int:
    """Accept INC-123, SR-123, CHN-123, PRB-123, or bare 123."""
    s = s.strip()
    m = re.match(r"(?:([A-Z]+)-)?(\d+)$", s, re.IGNORECASE)
    if not m:
        raise ValueError(f"invalid id: {s}")
    prefix, num = m.group(1), m.group(2)
    if prefix:
        prefix = prefix.upper()
        if prefix not in resource.display_prefixes:
            raise ValueError(f"prefix {prefix} not valid for {resource.name} (expect {resource.display_prefixes})")
    return int(num)


def format_id(item: dict, resource: Resource) -> str:
    hid = item.get("human_display_id")
    if hid:
        return hid
    did = item.get("display_id") or item.get("id")
    prefix = resource.display_prefixes[0]
    return f"{prefix}-{did}"
