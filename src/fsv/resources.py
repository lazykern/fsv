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
    list_include="change_status,agent,requester,group,department",
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
    list_include="stats,responder,requester,ticket_states,ticket_status,group",
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


# Built-in display-id prefixes per record kind. Tenants with custom prefixes
# declare them in the config (``ids.<kind>_prefix``); records carrying their
# own ``human_display_id`` are displayed with it as is.
DEFAULT_KIND_PREFIXES = {"incident": "INC", "service_request": "SR", "change": "CHN", "problem": "PRB"}
KIND_RESOURCES = {"incident": "tickets", "service_request": "tickets", "change": "changes", "problem": "problems"}


def kind_prefixes() -> dict[str, str]:
    """Display-id prefix -> record kind (``incident``, ``service request``,
    ``change``, ``problem``): the built-ins plus configured custom ones."""
    from fsv import config
    out = {prefix: kind.replace("_", " ") for kind, prefix in DEFAULT_KIND_PREFIXES.items()}
    ids = config.load_config().get("ids") or {}
    for kind in DEFAULT_KIND_PREFIXES:
        value = ids.get(f"{kind}_prefix") if isinstance(ids, dict) else None
        if isinstance(value, str) and re.fullmatch(r"[A-Za-z][A-Za-z0-9]*", value.strip()):
            out[value.strip().upper()] = kind.replace("_", " ")
    return out


def parse_id(s: str, resource: Resource) -> int:
    """Accept INC-123, SR-123, CHN-123, PRB-123 (or a configured custom
    prefix of this resource), or bare 123."""
    s = s.strip()
    m = re.match(r"(?:([A-Z]+)-)?(\d+)$", s, re.IGNORECASE)
    if not m:
        raise ValueError(f"invalid id: {s}")
    prefix, num = m.group(1), m.group(2)
    if prefix:
        prefix = prefix.upper()
        custom = () if prefix in resource.display_prefixes else tuple(
            p for p, kind in kind_prefixes().items() if KIND_RESOURCES[kind.replace(" ", "_")] == resource.name)
        if prefix not in resource.display_prefixes and prefix not in custom:
            raise ValueError(f"prefix {prefix} not valid for {resource.name} (expect {' or '.join(resource.display_prefixes)})")
    return int(num)


def format_id(item: dict, resource: Resource) -> str:
    hid = item.get("human_display_id")
    if hid:
        return hid
    did = item.get("display_id") or item.get("id")
    prefix = resource.display_prefixes[0]
    return f"{prefix}-{did}"
