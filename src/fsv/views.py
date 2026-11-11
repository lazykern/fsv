"""Site-like list and detail views, built only from what the tenant returns.

The agent web shows a change, ticket or problem the same way for every
account; what differs is the metadata behind it (saved views, form fields
and their order, business rules, lifecycle, planning fields, the tab list).
This module fetches that metadata through the same internal endpoints the
web uses and turns it into plain data that both faces render:

* lists: saved views by name (``<module>-filters``), the site's columns
  (``Subject · Requester · [State ·] Status · Priority · Assigned to ·
  [Department]``) and a header like ``<view> · 1-30 of N · Sort by: ...``;
* detail pages: header + status badge + due, the lifecycle bar, the reporter
  line, Description, Planning (changes) or Requested items and
  Conversations (tickets), a summary card, Properties in form order with
  rule-driven visibility, and every tab the server lists.

Portability: nothing here assumes a feature exists. Workspaces, business
rules, the change lifecycle, planning fields, the tabs list, risk/devops,
the service catalog and the current user's profile are all optional: a
403/404 (or an empty answer) leaves that section out with a short note and
never fails the page. Only an expired session (``SessionError``) stops it.

No rendering happens here (see :mod:`fsv.pages`).
"""
from __future__ import annotations

import datetime as dt
import html
import re
from dataclasses import dataclass, field
from typing import Any, Callable, Mapping, Sequence

from fsv import schema as schema_mod
from fsv.errors import APIError, SessionError
from fsv.resources import Resource, format_id

MODULE_OF = {"changes": "change", "tickets": "ticket", "problems": "problem"}

# The include lists the web's list pages send.
LIST_INCLUDE = {
    "changes": "change_status,agent,requester,group,department",
    "tickets": "stats,responder,requester,ticket_states,ticket_status,group",
}

# Ticket list "State" column (``state`` on each row) as the site words it.
TICKET_STATES = {"new": "New", "elapsed": "Response Due", "overdue": "Overdue"}

# Ticket ``source`` when the form has no Source choices.
SOURCES = {1: "Email", 2: "Portal", 3: "Phone", 4: "Chat", 5: "Feedback Widget", 6: "Yammer",
           7: "AWS CloudWatch", 8: "PagerDuty", 9: "Walk-up", 10: "Slack"}

# Change/ticket ``approval_status`` when the record carries no name.
APPROVAL_STATUS = {1: "Approved", 2: "Rejected", 3: "Requested", 4: "Not requested", 5: "Cancelled"}

SORT_LABELS = {"created_at": "Created Date", "updated_at": "Last Modified", "due_by": "Due by",
               "fr_due_by": "First response due", "display_id": "ID"}

# Site tab names -> labels. Unknown names are shown title-cased.
TAB_LABELS = {
    "details": "Details", "devops": "Devops", "risk": "Risk", "notes": "Notes", "tasks": "Tasks",
    "assets": "Assets", "impacted_services": "Impacted services", "associations": "Associations",
    "approvals": "Approvals", "activity": "Activity", "related_tickets": "Related tickets",
    "resolution_notes": "Resolution", "time_entries": "Time entries",
}
# What users may type for a tab.
TAB_ALIASES = {
    "impacted": "impacted_services", "impacted-services": "impacted_services", "related": "related_tickets",
    "related-tickets": "related_tickets", "resolution": "resolution_notes", "activities": "activity",
    "detail": "details",
}
# Shown inside the Details tab on the site, not as a tab of its own.
_DETAILS_PARTS = {"requested_items", "details", "conversations"}
# Tabs fsv can render when the server will not say which exist.
FALLBACK_TABS = {
    "changes": ("notes", "tasks", "approvals", "activity"),
    "tickets": ("tasks", "approvals", "activity", "resolution_notes"),
    "problems": ("notes", "tasks", "activity"),
}



# --------------------------------------------------------------------------
# optional fetching


@dataclass
class Missing:
    """A section the tenant did not provide (feature off, role, plan)."""

    what: str
    status: int | None = None

    @property
    def note(self) -> str:
        if self.status == 403:
            return f"{self.what}: not permitted for your role (403)"
        if self.status == 404:
            return f"{self.what}: not available on this account (404)"
        if self.status:
            return f"{self.what}: unavailable ({self.status})"
        return f"{self.what}: unavailable"

    def to_json(self) -> dict[str, Any]:
        return {"unavailable": True, "status": self.status, "note": self.note}


def optional_get(c: Any, path: str, params: Mapping[str, Any] | None = None,
                 what: str = "") -> tuple[Any, Missing | None]:
    """``GET /api/_/<path>``; any failure but an expired session becomes a
    :class:`Missing` instead of an exception."""
    try:
        return c.int_get(path, dict(params) if params else None), None
    except SessionError:
        raise
    except APIError as e:
        return None, Missing(what or path, e.status)
    except Exception:  # noqa: BLE001 - an optional section never breaks the page
        return None, Missing(what or path)


def workspace_get(c: Any, path: str, params: Mapping[str, Any] | None, workspace: Any,
                  what: str = "") -> tuple[Any, Missing | None]:
    """``ws/<workspace>/<path>`` when the record lives in a workspace, else (or
    when that 404s, e.g. a single-workspace account) the plain ``<path>``."""
    if workspace not in (None, ""):
        data, miss = optional_get(c, f"ws/{workspace}/{path}", params, what)
        if miss is None or miss.status != 404:
            return data, miss
    return optional_get(c, path, params, what)


def current_user(c: Any) -> dict[str, Any]:
    """The signed-in agent (``/api/v2/agents/me``), once per client; ``{}``
    when the role may not read it."""
    cached = getattr(c, "_fsv_me", None)
    if isinstance(cached, dict):
        return cached
    me: dict[str, Any] = {}
    try:
        got = c.me()
        me = dict(got) if isinstance(got, Mapping) else {}
    except SessionError:
        raise
    except Exception:  # noqa: BLE001
        me = {}
    if me and not me.get("name"):
        me["name"] = " ".join(str(me.get(k) or "") for k in ("first_name", "last_name")).strip()
    try:
        c._fsv_me = me
    except Exception:  # noqa: BLE001
        pass
    return me


def user_tz(user: Mapping[str, Any] | None) -> dt.tzinfo:
    """The user's profile time zone (``FSV_TZ`` wins), else the system one."""
    from fsv.forms.tz import resolve_timezone
    try:
        return resolve_timezone(None, user if user and user.get("time_zone") else None)[0]
    except Exception:  # noqa: BLE001
        return dt.datetime.now().astimezone().tzinfo or dt.timezone.utc


# --------------------------------------------------------------------------
# dates and text


def _now() -> dt.datetime:
    return dt.datetime.now(dt.timezone.utc)


def parse_dt(value: Any) -> dt.datetime | None:
    if value in (None, ""):
        return None
    if isinstance(value, dt.datetime):
        return value if value.tzinfo else value.replace(tzinfo=dt.timezone.utc)
    try:
        out = dt.datetime.fromisoformat(str(value).replace("Z", "+00:00"))
    except ValueError:
        return None
    return out if out.tzinfo else out.replace(tzinfo=dt.timezone.utc)


def _is_date_only(value: Any) -> bool:
    return isinstance(value, str) and bool(re.fullmatch(r"\d{4}-\d{2}-\d{2}", value.strip()))


def fmt_when(value: Any, tz: dt.tzinfo | None = None, *, year: bool = True) -> str:
    """``Fri, 27 Nov 2026 14:07`` in ``tz`` (``Mon, 9 Nov 14:07`` without the
    year); date-only values keep no time; unparseable values are returned."""
    if value in (None, ""):
        return ""
    if _is_date_only(value):
        d = dt.date.fromisoformat(str(value).strip())
        return f"{d:%a}, {d.day} {d:%b %Y}"
    when = parse_dt(value)
    if when is None:
        return str(value)
    when = when.astimezone(tz) if tz else when.astimezone()
    yr = f" {when:%Y}" if year else ""
    return f"{when:%a}, {when.day} {when:%b}{yr} {when:%H:%M}"


def relative(value: Any, now: dt.datetime | None = None) -> str:
    """``2 days ago`` / ``in 3 hours`` (the web's wording)."""
    when = parse_dt(value)
    if when is None:
        return ""
    now = now or _now()
    secs = (now - when).total_seconds()
    future = secs < 0
    s = abs(secs)
    if s < 45:
        text = "a few seconds"
    elif s < 90:
        text = "a minute"
    elif s < 45 * 60:
        text = f"{round(s / 60)} minutes"
    elif s < 90 * 60:
        text = "an hour"
    elif s < 22 * 3600:
        text = f"{round(s / 3600)} hours"
    elif s < 36 * 3600:
        text = "a day"
    elif s < 26 * 86400:
        text = f"{round(s / 86400)} days"
    elif s < 45 * 86400:
        text = "a month"
    elif s < 320 * 86400:
        text = f"{round(s / (30 * 86400))} months"
    elif s < 548 * 86400:
        text = "a year"
    else:
        text = f"{round(s / (365 * 86400))} years"
    return f"in {text}" if future else f"{text} ago"


def span(seconds: float | int | None) -> str:
    """``2w 1d`` / ``3h 20m`` / ``45m`` (two most significant units)."""
    if seconds is None:
        return ""
    s = int(abs(seconds))
    units = (("w", 7 * 86400), ("d", 86400), ("h", 3600), ("m", 60))
    for i, (name, size) in enumerate(units):
        n, rest = divmod(s, size)
        if not n:
            continue
        if i + 1 < len(units):
            sub_name, sub_size = units[i + 1]
            sub = round(rest / sub_size)
            if sub * sub_size >= size:  # rounded up into the bigger unit
                n, sub = n + 1, 0
            return f"{n}{name} {sub}{sub_name}" if sub else f"{n}{name}"
        return f"{n}{name}"
    return "0m"


def html_to_text(value: Any) -> str:
    """Readable text from rich-text HTML: paragraphs and list items keep
    their line breaks; scripts, styles and tags are dropped."""
    if value in (None, ""):
        return ""
    s = str(value)
    s = re.sub(r"(?is)<(script|style)\b.*?</\1>", "", s)
    s = re.sub(r"(?i)<br\s*/?>", "\n", s)
    s = re.sub(r"(?i)<li\b[^>]*>", "\n• ", s)
    s = re.sub(r"(?i)</(p|div|li|tr|h[1-6]|ul|ol|table|blockquote)>", "\n", s)
    s = re.sub(r"<[^>]+>", "", s)
    s = html.unescape(s).replace("\xa0", " ")
    lines = [re.sub(r"[ \t]+", " ", ln).strip() for ln in s.splitlines()]
    out: list[str] = []
    for ln in lines:
        if not ln and (not out or not out[-1]):
            continue
        out.append(ln)
    return "\n".join(out).strip()


def one_line(value: Any) -> str:
    return " ".join(html_to_text(value).split())


# --------------------------------------------------------------------------
# saved views


@dataclass(frozen=True)
class SavedView:
    id: str
    name: str
    system: bool = False
    marked_default: bool = False
    raw: Mapping[str, Any] = field(default_factory=dict, compare=False, hash=False)

    def to_json(self) -> dict[str, Any]:
        return {"id": self.id, "name": self.name, "system": self.system, "default": self.marked_default}


def _view_list(data: Any) -> list[dict[str, Any]]:
    if isinstance(data, list):
        return [x for x in data if isinstance(x, Mapping)]
    if isinstance(data, Mapping):
        for k, v in data.items():
            if isinstance(v, list) and (k.endswith("filters") or k.endswith("views")):
                return [x for x in v if isinstance(x, Mapping)]
    return []


def load_views(c: Any, res: Resource) -> tuple[list[SavedView], Missing | None]:
    """The web's saved views for ``res`` in server order."""
    if not res.filters_path:
        return [], None
    data, miss = optional_get(c, res.filters_path, None, "saved views")
    if miss:
        return [], miss
    out = []
    for v in _view_list(data):
        vid = v.get("id")
        if vid in (None, ""):
            continue
        marked = bool(v.get("default"))
        out.append(SavedView(str(vid), str(v.get("name") or vid), system=marked and v.get("user_id") is None,
                             marked_default=marked, raw=dict(v)))
    return out, None


def default_view(views: Sequence[SavedView]) -> SavedView | None:
    """The view the web opens with: the first one the server marks default."""
    for v in views:
        if v.marked_default:
            return v
    return views[0] if views else None


def _fold(s: Any) -> str:
    return " ".join(str(s or "").split()).casefold()


def find_view(views: Sequence[SavedView], text: str) -> SavedView:
    """By exact id, exact name, or unique name prefix (case-insensitive)."""
    want = _fold(text)
    for v in views:
        if _fold(v.id) == want:
            return v
    exact = [v for v in views if _fold(v.name) == want]
    if len(exact) == 1:
        return exact[0]
    pref = exact or [v for v in views if _fold(v.name).startswith(want)]
    if len(pref) == 1:
        return pref[0]
    if not pref:
        pref = [v for v in views if want and want in _fold(v.name)]
        if len(pref) == 1:
            return pref[0]
    names = ", ".join(repr(v.name) for v in (pref or views)[:12])
    if pref:
        raise ValueError(f"view {text!r} is ambiguous: {names}")
    raise ValueError(f"no view named {text!r}; views: {names}" if views else f"no view named {text!r}")


def view_settings(c: Any, res: Resource, view: SavedView) -> dict[str, Any]:
    """The view's own sort and conditions (``<filters>/<id>``); ``{}`` if hidden."""
    if not res.filters_path:
        return {}
    data, miss = optional_get(c, f"{res.filters_path}/{view.id}", None, "view settings")
    if miss or not isinstance(data, Mapping):
        return {}
    for v in data.values():
        if isinstance(v, Mapping):
            return dict(v)
    return dict(data)


# --------------------------------------------------------------------------
# lists


@dataclass(frozen=True)
class Column:
    key: str
    label: str


_COLUMN_KEYS = {
    "changes": ("subject", "requester", "status", "priority", "assigned_to", "department"),
    "tickets": ("subject", "requester", "state", "status", "priority", "assigned_to"),
    "problems": ("subject", "requester", "status", "priority", "assigned_to", "department"),
}
_CHROME_LABELS = {"subject": "Subject", "requester": "Requester", "state": "State", "status": "Status",
                  "priority": "Priority", "assigned_to": "Assigned to", "department": "Department"}
# tickets/headers names -> column keys
_HEADER_KEYS = {"ticket_subject": "subject", "subject": "subject", "requester": "requester", "state": "state",
                "status": "status", "priority": "priority", "assigned_to": "assigned_to",
                "department": "department"}


def _schema_label(schema: Mapping[str, Any] | None, name: str) -> str | None:
    for f in (schema or {}).get("fields") or []:
        if f.get("name") == name and f.get("label"):
            return html.unescape(str(f["label"]))
    return None


def list_columns(res: Resource, schema: Mapping[str, Any] | None = None,
                 headers: Sequence[Mapping[str, Any]] | None = None) -> list[Column]:
    """The site's list columns. Labels come from the tenant's form (or the
    ticket list headers) when it has them."""
    if headers:
        picked = sorted((h for h in headers if h.get("selected") and h.get("name") != "display_id"),
                        key=lambda h: (h.get("position") or 0))
        cols: list[Column] = []
        seen: set[str] = set()
        for h in picked:
            key = _HEADER_KEYS.get(str(h.get("name")), str(h.get("name")))
            if key in seen:
                continue
            seen.add(key)
            cols.append(Column(key, html.unescape(str(h.get("label") or key))))
        if cols:
            if "subject" not in seen:
                cols.insert(0, Column("subject", _schema_label(schema, "subject") or "Subject"))
            return cols
    out = []
    for key in _COLUMN_KEYS.get(res.name, _COLUMN_KEYS["changes"]):
        label = _schema_label(schema, key) if key in ("subject", "requester", "status", "priority", "department") else None
        out.append(Column(key, label or _CHROME_LABELS[key]))
    return out


def _name(obj: Any) -> str:
    if isinstance(obj, Mapping):
        return str(obj.get("name") or obj.get("value") or obj.get("email") or "").strip()
    return ""


def choice_name(schema: Mapping[str, Any] | None, field_name: str, value: Any) -> str:
    """Label of a dropdown value from the form (``""`` when unknown)."""
    if value in (None, ""):
        return ""
    for f in (schema or {}).get("fields") or []:
        if f.get("name") != field_name:
            continue
        for ch in f.get("choices") or []:
            if str(ch.get("id")) == str(value) or str(ch.get("value")) == str(value):
                return html.unescape(str(ch.get("value") or ch.get("name") or ch.get("label") or value))
    return ""


def status_name(item: Mapping[str, Any], schema: Mapping[str, Any] | None) -> str:
    for key in ("change_status", "ticket_status", "problem_status", "summary_status"):
        n = _name(item.get(key))
        if n:
            return n
    return (str(item.get("status_name") or "") or choice_name(schema, "status", item.get("status"))
            or str(item.get("status") if item.get("status") is not None else ""))


def priority_name(item: Mapping[str, Any], schema: Mapping[str, Any] | None) -> str:
    v = item.get("priority")
    return (str(item.get("priority_name") or "") or choice_name(schema, "priority", v)
            or schema_mod.PRIORITY.get(v, "") or ("" if v is None else str(v)))


def department_name(item: Mapping[str, Any], schema: Mapping[str, Any] | None) -> str:
    return (_name(item.get("department")) or str(item.get("department_name") or "")
            or choice_name(schema, "department", item.get("department_id")))


def requester_name(item: Mapping[str, Any]) -> str:
    return _name(item.get("requester")) or str(item.get("requester_name") or "")


def agent_of(item: Mapping[str, Any]) -> tuple[Any, str]:
    for key in ("agent", "responder", "owner"):
        obj = item.get(key)
        if isinstance(obj, Mapping) and (obj.get("name") or obj.get("id") is not None):
            return obj.get("id", item.get("agent_id") or item.get("responder_id")), _name(obj)
    aid = item.get("agent_id") or item.get("responder_id")
    nm = str(item.get("owner_name") or item.get("responder_name") or item.get("agent_name") or "")
    return aid, nm


def group_name_of(item: Mapping[str, Any], schema: Mapping[str, Any] | None = None) -> str:
    return (_name(item.get("group")) or str(item.get("group_name") or "")
            or choice_name(schema, "group", item.get("group_id")))


def assigned_to(item: Mapping[str, Any], me_id: Any = None, schema: Mapping[str, Any] | None = None) -> str:
    """``Group / Agent`` with ``(Me)`` on the signed-in agent, ``None`` when unset."""
    aid, agent = agent_of(item)
    if agent and me_id is not None and aid is not None and str(aid) == str(me_id):
        agent += " (Me)"
    grp = group_name_of(item, schema).strip()
    if grp:
        return f"{grp} / {agent or 'None'}"
    return agent or "None"


def ticket_state(item: Mapping[str, Any]) -> str:
    st = item.get("state")
    if isinstance(st, Mapping):
        st = st.get("name") or st.get("state")
    if not st:
        return ""
    return TICKET_STATES.get(str(st), str(st).replace("_", " ").capitalize())


def cell(item: Mapping[str, Any], key: str, res: Resource, schema: Mapping[str, Any] | None = None,
         me_id: Any = None, tz: dt.tzinfo | None = None) -> str:
    if key == "subject":
        return str(item.get("subject") or "")
    if key == "requester":
        return requester_name(item)
    if key == "status":
        return status_name(item, schema)
    if key == "priority":
        return priority_name(item, schema)
    if key == "assigned_to":
        return assigned_to(item, me_id, schema)
    if key == "department":
        return department_name(item, schema)
    if key == "state":
        return ticket_state(item)
    if key in ("group", "group_id"):
        return group_name_of(item, schema)
    if key in ("agent", "responder", "agent_id", "responder_id"):
        return agent_of(item)[1]
    raw = item.get(key)
    if raw is None:
        raw = (item.get("custom_fields") or {}).get(key)
    if raw is None and key.endswith("_id"):
        raw = item.get(key)
    if isinstance(raw, str) and parse_dt(raw) and ("T" in raw or _is_date_only(raw)):
        return fmt_when(raw, tz)
    label = choice_name(schema, key, raw) if not isinstance(raw, (list, dict)) else ""
    if label:
        return label
    if isinstance(raw, Mapping):
        return _name(raw)
    if isinstance(raw, list):
        return ", ".join(_name(x) or str(x) for x in raw)
    if raw is True:
        return "Yes"
    if raw is False:
        return "No"
    return "" if raw is None else str(raw)


def list_row(item: Mapping[str, Any], res: Resource, columns: Sequence[Column],
             schema: Mapping[str, Any] | None = None, me_id: Any = None,
             tz: dt.tzinfo | None = None) -> dict[str, str]:
    row = {"id": format_id(dict(item), res)}
    for col in columns:
        row[col.key] = cell(item, col.key, res, schema, me_id, tz)
    return row


def sort_label(order_by: str | None, schema: Mapping[str, Any] | None = None) -> str:
    if not order_by:
        return SORT_LABELS["created_at"]
    if order_by in SORT_LABELS:
        return SORT_LABELS[order_by]
    base = order_by[:-3] if order_by.endswith("_id") else order_by
    return _schema_label(schema, base) or _schema_label(schema, order_by) or base.replace("_", " ").capitalize()


@dataclass
class ListPage:
    resource: Resource
    items: list[dict[str, Any]]
    columns: list[Column]
    rows: list[dict[str, str]]
    view: SavedView | None
    page: int
    per_page: int
    total: int | None
    has_next: bool
    order_by: str
    order_type: str
    sort: str
    notes: list[str] = field(default_factory=list)

    @property
    def title(self) -> str:
        return self.view.name if self.view else f"All {self.resource.name}"

    def header(self) -> str:
        n = len(self.items)
        if n:
            lo = (self.page - 1) * self.per_page + 1
            rng = f"{lo}-{lo + n - 1}"
        else:
            rng = "0"
        if self.total is not None and self.total >= n:
            of = f" of {self.total}"
        else:
            of = f" of {n}+" if self.has_next else f" of {n}"
        return f"{self.title} · {rng}{of} · Sort by: {self.sort}"


def list_meta(data: Mapping[str, Any], items: Sequence[Any], page: int, per_page: int) -> tuple[int | None, bool]:
    meta = data.get("meta") if isinstance(data.get("meta"), Mapping) else {}
    total = meta.get("total_count", meta.get("count", data.get("total")))
    has_next = meta.get("has_next")
    if has_next is None:
        has_next = meta.get("has_more")
    if has_next is None:
        has_next = len(items) >= per_page
    try:
        total = int(total) if total is not None else None
    except (TypeError, ValueError):
        total = None
    if total is not None and total < len(items) + (page - 1) * per_page:
        total = None
    return total, bool(has_next)


def ticket_headers(c: Any) -> list[dict[str, Any]]:
    data, miss = optional_get(c, "tickets/headers", None, "list headers")
    if miss or not isinstance(data, Mapping):
        return []
    rows = data.get("headers")
    return [h for h in rows if isinstance(h, Mapping)] if isinstance(rows, list) else []


# --------------------------------------------------------------------------
# detail


@dataclass
class Tab:
    name: str
    label: str
    count: int | None = None

    def to_json(self) -> dict[str, Any]:
        return {"name": self.name, "label": self.label, "count": self.count}


def canonical_tab(name: str) -> str:
    n = str(name or "").strip().lower().replace(" ", "_")
    return TAB_ALIASES.get(n, TAB_ALIASES.get(n.replace("_", "-"), n))


def tab_label(name: str) -> str:
    return TAB_LABELS.get(name) or name.replace("_", " ").capitalize()


def parse_tabs(data: Any) -> list[Tab]:
    rows = data if isinstance(data, list) else (data or {}).get("tabs") if isinstance(data, Mapping) else None
    tabs = [Tab("details", tab_label("details"))]
    for t in rows or []:
        if not isinstance(t, Mapping) or not t.get("name"):
            continue
        name = str(t["name"])
        if name in _DETAILS_PARTS:
            continue
        cnt = t.get("count")
        tabs.append(Tab(name, tab_label(name), cnt if isinstance(cnt, int) else None))
    return tabs


@dataclass
class LifecycleState:
    id: Any
    name: str
    mark: str  # done | current | todo


@dataclass
class Lifecycle:
    name: str
    states: list[LifecycleState]

    def to_json(self) -> dict[str, Any]:
        return {"name": self.name, "states": [{"id": s.id, "name": s.name, "mark": s.mark} for s in self.states]}


def load_lifecycle(c: Any, record: Mapping[str, Any]) -> tuple[Lifecycle | None, Missing | None]:
    """The change's lifecycle (state flow) with the current state marked;
    ``(None, None)`` when the account has none."""
    sfid = record.get("state_flow_id")
    if not sfid:
        return None, None
    data, miss = workspace_get(c, f"change_management/{sfid}", {"include": "state_info"},
                               record.get("workspace_id"), "lifecycle")
    if miss:
        return None, miss
    flow = data.get("state_flow", data) if isinstance(data, Mapping) else {}
    states = flow.get("state_list") if isinstance(flow, Mapping) else None
    if not states:
        return None, None
    cur = record.get("status")
    visited = {str(s) for s in record.get("state_traversal") or []}
    out = []
    for s in states:
        if not isinstance(s, Mapping):
            continue
        sid = s.get("id")
        name = str(s.get("value") or s.get("name") or sid)
        mark = "current" if str(sid) == str(cur) else "done" if str(sid) in visited else "todo"
        out.append(LifecycleState(sid, name, mark))
    if not any(s.mark == "current" for s in out):
        return None, None
    return Lifecycle(str(flow.get("name") or ""), out), None


@dataclass
class PlanningEntry:
    name: str
    label: str
    help: str
    text: str
    files: list[str]

    @property
    def empty(self) -> bool:
        return not self.text and not self.files

    def to_json(self) -> dict[str, Any]:
        return {"name": self.name, "label": self.label, "help": self.help, "content": self.text,
                "files": self.files, "empty": self.empty}


PLANNING_TYPES = {"planning_field", "default_change_reason", "default_change_impact", "default_change_plan",
                  "default_backout_plan"}


def _file_names(rows: Any) -> list[str]:
    out = []
    for a in rows or []:
        if isinstance(a, Mapping):
            out.append(str(a.get("name") or a.get("content_file_name") or a.get("file_name") or a.get("id")))
    return out


def load_planning(c: Any, record: Mapping[str, Any], schema: Mapping[str, Any] | None
                  ) -> tuple[list[PlanningEntry] | None, Missing | None]:
    """Planning fields in form order with their content; ``(None, None)``
    when the account defines none."""
    defs = [f for f in (schema or {}).get("fields") or [] if str(f.get("field_type") or "") in PLANNING_TYPES]
    if not defs:
        return None, None
    data, miss = optional_get(c, f"changes/{record.get('display_id') or record.get('id')}/planning-fields",
                              None, "planning")
    if miss:
        return None, miss
    rows = data.get("change_planning_fields") if isinstance(data, Mapping) else data
    by_name = {str(r.get("name")): r for r in rows or [] if isinstance(r, Mapping)}
    out = []
    for d in defs:
        name = str(d.get("name") or "")
        v = by_name.get(name) or {}
        text = html_to_text(v.get("description") or v.get("description_html") or "") or str(v.get("description_text") or "").strip()
        out.append(PlanningEntry(name, html.unescape(str(d.get("label") or name)),
                                 one_line(d.get("helptext") or d.get("description") or ""), text,
                                 _file_names(v.get("attachments"))))
    return out, None


@dataclass
class Prop:
    name: str
    label: str
    value: str
    required: bool = False

    def to_json(self) -> dict[str, Any]:
        return {"name": self.name, "label": self.label, "value": self.value, "required": self.required}


_SKIP_PROPS = {"subject", "description"}


def _record_for_form(res: Resource, record: Mapping[str, Any]) -> dict[str, Any]:
    rec = dict(record)
    if res.name == "tickets":
        rec.setdefault("agent_id", rec.get("responder_id"))
        if rec.get("ticket_type") is None and rec.get("type"):
            rec["ticket_type"] = rec["type"]
    return rec


def load_edit_rules(c: Any, module: str, record: Mapping[str, Any], raw_fields: Sequence[Mapping[str, Any]]
                    ) -> tuple[dict[str, Any] | None, str | None]:
    """Edit-form business rules (cached); ``(None, note)`` when the account
    has none or will not show them."""
    from fsv import formmeta
    from fsv.forms import business_rules_query

    ws = record.get("workspace_id")
    if ws in (None, ""):
        ws = formmeta.workspace_from_fields(raw_fields)
    key = f"{module}-rules-edit_form-ws{ws if ws not in (None, '') else 'none'}"
    try:
        cached = formmeta._cached(key)
    except Exception:  # noqa: BLE001
        cached = None
    if isinstance(cached, Mapping):
        return dict(cached), None
    q = business_rules_query(module, workspace_id=ws if ws not in (None, "") else 0, form_type="edit",
                             module_id=record.get("display_id") or record.get("id"))
    data, miss = workspace_get(c, "business_rules", dict(q.params), ws, "business rules")
    if miss:
        return None, "static field visibility (" + miss.note + ")"
    try:
        rules = formmeta.normalize_rules(data)
    except SessionError:
        raise
    except Exception:  # noqa: BLE001
        return None, "static field visibility (business rules unreadable)"
    if not rules.get("business_rules"):
        return None, None
    try:
        formmeta._save(key, "form_rules", rules)
    except Exception:  # noqa: BLE001
        pass
    return rules, None


def _prop_value(state: Any, f: Any, tz: dt.tzinfo | None) -> str:
    v = state.evaluated.values.get(f.api_name)
    if v in (None, "", [], (), {}):
        return ""
    if f.kind in ("date", "datetime") and isinstance(v, str):
        return fmt_when(v, tz) if f.kind == "datetime" else fmt_when(v[:10] if _is_date_only(v[:10]) else v, tz)
    if isinstance(v, (list, tuple)) and v and all(hasattr(x, "display") for x in v):
        return ", ".join(str(x.display or x.id) for x in v)
    if hasattr(v, "display") and hasattr(v, "id"):
        return str(v.display or v.id)
    try:
        return one_line(state.display(f.api_name)) if f.kind in ("textarea", "richtext") else state.display(f.api_name)
    except Exception:  # noqa: BLE001
        return str(v)


def flat_properties(record: Mapping[str, Any], schema: Mapping[str, Any] | None) -> list[Prop]:
    """Fallback when the form cannot be evaluated: schema order, labels, names."""
    out = []
    cf = record.get("custom_fields") or {}
    for f in (schema or {}).get("fields") or []:
        name = str(f.get("name") or "")
        if not name or name in _SKIP_PROPS or str(f.get("field_type")) in PLANNING_TYPES:
            continue
        if f.get("default_field"):
            raw = record.get(f"{name}_id", record.get(name))
        else:
            raw = cf.get(name) if isinstance(cf, Mapping) else None
        val = choice_name(schema, name, raw) if not isinstance(raw, (list, dict)) else ""
        if not val:
            val = _name(raw) if isinstance(raw, Mapping) else (", ".join(_name(x) or str(x) for x in raw)
                                                              if isinstance(raw, list) else
                                                              ("" if raw is None else str(raw)))
        out.append(Prop(name, html.unescape(str(f.get("label") or name)), val, bool(f.get("required"))))
    return out


def load_properties(c: Any, res: Resource, record: Mapping[str, Any], schema: Mapping[str, Any] | None,
                    tz: dt.tzinfo | None = None, user: Mapping[str, Any] | None = None
                    ) -> tuple[list[Prop], str | None]:
    """Form fields in web order, visible per the edit-form rules, with
    display values (names, not ids)."""
    from fsv.forms import FormSpec, FormState, RulesEngine
    from fsv.formmeta import ClientResolver, existing_from_record, name_existing

    raw_fields = list((schema or {}).get("fields") or [])
    if not raw_fields:
        return [], "no form fields"
    module = MODULE_OF.get(res.name, res.name.rstrip("s"))
    try:
        spec = FormSpec.from_form_fields({res.form_fields_key: raw_fields}, module=module)
    except Exception:  # noqa: BLE001
        return flat_properties(record, schema), "form metadata unreadable; fields shown as stored"
    rules, note = load_edit_rules(c, module, record, raw_fields)
    rec = _record_for_form(res, record)
    try:
        engine = RulesEngine(spec, rules or {"business_rules": []}, module=module,
                             workspace_id=record.get("workspace_id"))
        existing = existing_from_record(spec, rec)
        existing = name_existing(existing, spec, rec, ClientResolver(c))
        tzname = (user or {}).get("time_zone") or None
        try:
            state = FormState(spec, engine, mode="edit", existing=existing, tz=tzname)
        except ValueError:
            state = FormState(spec, engine, mode="edit", existing=existing)
    except SessionError:
        raise
    except Exception:  # noqa: BLE001 - never fail the page on one odd rule/value
        return flat_properties(record, schema), "form rules could not be applied; fields shown as stored"
    visible = {f.api_name for f in state.visible_fields()}
    required = set(getattr(state.evaluated, "required", ()) or ())
    out = []
    for f in spec.ordered():
        if f.api_name not in visible or f.api_name in _SKIP_PROPS or f.kind in ("notice", "attachment"):
            continue
        out.append(Prop(f.api_name, f.label, _prop_value(state, f, tz), f.api_name in required))
    return out, note


# --------------------------------------------------------------------------
# ticket parts


def load_conversations(c: Any, ticket_id: Any, *, all_pages: bool = False, per_page: int = 3
                       ) -> tuple[list[dict[str, Any]], dict[str, Any], Missing | None]:
    """Conversations oldest first; by default only the latest few, like the
    web (``meta.count`` says how many exist)."""
    params = {"include": "user,phone,feedback", "per_page": per_page}
    if not all_pages:
        data, miss = optional_get(c, f"tickets/{ticket_id}/conversations", params, "conversations")
        if miss:
            return [], {}, miss
        rows = list((data or {}).get("conversations") or [])
        meta = dict((data or {}).get("meta") or {})
    else:
        rows, meta, page = [], {}, 1
        while page <= 50:
            data, miss = optional_get(c, f"tickets/{ticket_id}/conversations",
                                      {**params, "per_page": 30, "page": page}, "conversations")
            if miss:
                if page == 1:
                    return [], {}, miss
                break
            batch = list((data or {}).get("conversations") or [])
            rows.extend(batch)
            meta = dict((data or {}).get("meta") or {})
            if not batch or len(batch) < 30 or meta.get("has_more") is False:
                break
            page += 1
        meta["count"] = max(int(meta.get("count") or 0), len(rows))
        meta["has_more"] = False
    seen: set[Any] = set()
    uniq = []
    for r in rows:
        if r.get("id") in seen and r.get("id") is not None:
            continue
        seen.add(r.get("id"))
        uniq.append(r)
    uniq.sort(key=lambda r: str(r.get("created_at") or ""))
    return uniq, meta, None


def conversation_kind(conv: Mapping[str, Any]) -> str:
    if conv.get("private"):
        return "private note"
    if conv.get("source") == 2:
        return "public note"
    if conv.get("incoming"):
        return "replied"
    return "replied"


def source_name(record: Mapping[str, Any], schema: Mapping[str, Any] | None) -> str:
    v = record.get("source")
    if v in (None, ""):
        return ""
    return choice_name(schema, "source", v) or SOURCES.get(v, str(v))


def status_badge(res: Resource, record: Mapping[str, Any], schema: Mapping[str, Any] | None) -> str:
    """Status as the header shows it, ``(late)`` when resolved after due."""
    name = status_name(record, schema)
    if res.name == "tickets":
        stats = record.get("stats") or {}
        done = parse_dt(stats.get("resolved_at") or stats.get("closed_at"))
        due = parse_dt(record.get("due_by"))
        if done and due and done > due:
            name += " (late)"
    return name


def approval_name(record: Mapping[str, Any]) -> str:
    if record.get("approval_status_name"):
        return str(record["approval_status_name"])
    v = record.get("approval_status")
    if v in (None, ""):
        return ""
    return APPROVAL_STATUS.get(v, str(v))


# --------------------------------------------------------------------------
# tabs


@dataclass
class TabData:
    name: str
    label: str
    data: Any = None
    missing: Missing | None = None
    parts: dict[str, Any] = field(default_factory=dict)  # sub-lists (associations, devops)
    notes: list[str] = field(default_factory=list)

    def to_json(self) -> dict[str, Any]:
        out: dict[str, Any] = {"name": self.name, "label": self.label}
        if self.missing:
            out.update(self.missing.to_json())
        if self.data is not None:
            out["data"] = self.data
        if self.parts:
            out["parts"] = self.parts
        if self.notes:
            out["notes"] = self.notes
        return out


def _rows(data: Any, key: str) -> list[dict[str, Any]]:
    if isinstance(data, Mapping):
        v = data.get(key)
        return [x for x in v if isinstance(x, Mapping)] if isinstance(v, list) else []
    return [x for x in data if isinstance(x, Mapping)] if isinstance(data, list) else []


def _parts(c: Any, specs: Sequence[tuple[str, str, Mapping[str, Any] | None, str]], what: str
           ) -> tuple[dict[str, Any], list[str], Missing | None]:
    parts: dict[str, Any] = {}
    notes: list[str] = []
    failures: list[Missing] = []
    for label, path, params, key in specs:
        data, miss = optional_get(c, path, params, label)
        if miss:
            failures.append(miss)
            if miss.status not in (404,):
                notes.append(miss.note)
            continue
        parts[label] = _rows(data, key)
    if not parts and failures:
        return {}, [], Missing(what, failures[0].status)
    return parts, notes, None


def load_tab(c: Any, res: Resource, record: Mapping[str, Any], name: str) -> TabData:
    """One site tab's content through the endpoints the web calls."""
    name = canonical_tab(name)
    rid = record.get("display_id") or record.get("id")
    base = f"{res.api_path}/{rid}"
    td = TabData(name, tab_label(name))
    ws = record.get("workspace_id")

    def simple(path: str, params: Mapping[str, Any] | None, key: str) -> TabData:
        data, miss = optional_get(c, path, params, td.label)
        td.missing = miss
        if not miss:
            td.data = _rows(data, key)
        return td

    if name == "notes":
        return simple(f"{base}/notes", {"include": "user", "page": 1, "per_page": 30}, "notes")
    if name == "tasks":
        return simple(f"{base}/tasks", {"page": 1, "per_page": 100}, "tasks")
    if name == "approvals":
        return simple(f"{base}/approvals", None, "approvals")
    if name == "activity":
        return simple(f"{base}/activities", None, "activities")
    if name == "assets":
        if res.name == "tickets":
            return simple(f"{base}/associated_assets", None, "associated_assets")
        data, miss = optional_get(c, f"{base}/assets", None, td.label)
        td.missing = miss
        if not miss:
            td.data = _rows(data, "associated_assets") or _rows(data, "assets")
        return td
    if name == "impacted_services":
        data, miss = optional_get(c, f"{base}/assets", {"filter": "impacted_services"}, td.label)
        td.missing = miss
        if not miss:
            td.data = _rows(data, "associated_assets") or _rows(data, "assets")
        return td
    if name == "associations":
        if res.name == "changes":
            specs = [("Releases", f"{base}/releases", {"per_page": 50}, "releases"),
                     ("Tickets", f"{base}/tickets", {"per_page": 50}, "tickets"),
                     ("Archived tickets", f"{base}/tickets", {"filter": "archived", "per_page": 50}, "tickets"),
                     ("Tickets caused by this change", f"{base}/tickets",
                      {"filter": "associated_tickets_caused", "per_page": 50}, "tickets"),
                     ("Problems", f"{base}/problems", {"per_page": 50}, "problems")]
        elif res.name == "tickets":
            specs = [("Changes", f"{base}/changes", {"change_type": "change"}, "changes"),
                     ("Changes that caused this", f"{base}/changes", {"change_type": "change_cause"}, "changes"),
                     ("Problems", f"{base}/problems", None, "problems")]
        else:
            specs = [("Changes", f"{base}/changes", None, "changes"),
                     ("Tickets", f"{base}/tickets", None, "tickets")]
        td.parts, td.notes, td.missing = _parts(c, specs, td.label)
        return td
    if name == "devops":
        specs = [("Work items", f"{base}/work-items", {"page": 1, "per_page": 5}, "work_items"),
                 ("Pull requests", f"{base}/pull-requests", {"page": 1, "per_page": 5}, "pull_requests"),
                 ("Pipeline runs", f"{base}/pipeline-runs", {"page": 1, "per_page": 5}, "pipeline_runs")]
        td.parts, td.notes, td.missing = _parts(c, specs, td.label)
        return td
    if name == "risk":
        data, miss = optional_get(c, f"{base}/risk-analysis", None, td.label)
        td.missing = miss
        if not miss:
            td.data = dict(data) if isinstance(data, Mapping) else {}
            params = {"module": "risk"}
            if ws not in (None, ""):
                params["workspace_id"] = ws
            survey, smiss = optional_get(c, f"{base}/survey", params, "risk survey")
            if not smiss and survey:
                td.parts["survey"] = survey
        return td
    if name == "related_tickets":
        data, miss = optional_get(c, f"{base}/mappings", {"successor_type": "active"}, td.label)
        td.missing = miss
        if not miss and isinstance(data, Mapping):
            pred = data.get("predecessor_ticket")
            td.parts = {"Parent": [pred] if isinstance(pred, Mapping) else [],
                        "Child tickets": _rows(data, "successor_tickets")}
        return td
    if name == "resolution_notes":
        stats = record.get("stats") or {}
        info = {k: stats.get(k) for k in ("resolved_at", "closed_at") if stats.get(k)}
        text = html_to_text(record.get("resolution_notes") or record.get("resolution_notes_html") or "")
        if not text:
            data, miss = optional_get(c, f"{base}/resolution_notes", None, td.label)
            if not miss and isinstance(data, Mapping):
                note = data.get("resolution_note") or data.get("resolution_notes") or data
                if isinstance(note, Mapping):
                    text = html_to_text(note.get("body") or note.get("description") or "")
        if text:
            info["note"] = text
        td.data = info
        return td
    if name == "time_entries":
        return simple(f"{base}/time_entries", None, "time_entries")
    td.missing = Missing(td.label)
    return td


# --------------------------------------------------------------------------
# the whole page


@dataclass
class Detail:
    resource: Resource
    record: dict[str, Any]
    schema: dict[str, Any]
    user: dict[str, Any]
    tz: dt.tzinfo
    tabs: list[Tab]
    tabs_missing: Missing | None = None
    lifecycle: Lifecycle | None = None
    lifecycle_missing: Missing | None = None
    planning: list[PlanningEntry] | None = None
    planning_missing: Missing | None = None
    properties: list[Prop] = field(default_factory=list)
    properties_note: str | None = None
    requested_items: list[dict[str, Any]] | None = None
    requested_items_missing: Missing | None = None
    conversations: list[dict[str, Any]] | None = None
    conversations_meta: dict[str, Any] = field(default_factory=dict)
    conversations_missing: Missing | None = None
    requested_for: str | None = None
    tab_data: dict[str, TabData] = field(default_factory=dict)

    # -- derived ---------------------------------------------------------
    @property
    def id(self) -> str:
        return format_id(self.record, self.resource)

    @property
    def subject(self) -> str:
        return str(self.record.get("subject") or "")

    @property
    def badge(self) -> str:
        return status_badge(self.resource, self.record, self.schema)

    @property
    def me_id(self) -> Any:
        return self.user.get("id")

    def due_text(self) -> str:
        return fmt_when(self.record.get("due_by"), self.tz)

    def tab_names(self) -> list[str]:
        return [t.name for t in self.tabs]

    def reporter(self) -> dict[str, Any]:
        rec = self.record
        name = requester_name(rec) or str(rec.get("created_by_name") or "")
        out = {"name": name, "created_at": rec.get("created_at"),
               "relative": relative(rec.get("created_at")),
               "when": fmt_when(rec.get("created_at"), self.tz, year=False)}
        if self.resource.name == "tickets":
            out["via"] = source_name(rec, self.schema)
            if self.requested_for:
                out["request_for"] = self.requested_for
        return out

    def summary(self) -> list[tuple[str, str]]:
        """The right-hand card above Properties."""
        rec, sch = self.record, self.schema
        rows: list[tuple[str, str]] = [(_schema_label(sch, "status") or "Status", self.badge)]
        pri = priority_name(rec, sch)
        if pri:
            rows.append((_schema_label(sch, "priority") or "Priority", pri))
        due = parse_dt(rec.get("due_by"))
        if due:
            left = (due - _now()).total_seconds()
            remain = f"in {span(left)}" if left > 0 else f"overdue by {span(left)}"
            stats = rec.get("stats") or {}
            if stats.get("resolved_at") or stats.get("closed_at") or rec.get("status") in (5, 6) and self.resource.name == "tickets":
                remain = ""
            label = "Resolution due by" if self.resource.name == "changes" else "Due by"
            rows.append((label, self.due_text() + (f"  ({remain})" if remain else "")))
        if self.resource.name == "changes":
            appr = approval_name(rec)
            if appr:
                rows.append(("Approval", appr))
        for key in ("planned_start_date", "planned_end_date"):
            if rec.get(key):
                rows.append((_schema_label(sch, key) or key.replace("_", " ").capitalize(), fmt_when(rec[key], self.tz)))
        if self.resource.name == "tickets":
            secs = (rec.get("stats") or {}).get("resolution_time_in_secs")
            if secs:
                rows.append(("Time taken", span(secs)))
            req = rec.get("requester") if isinstance(rec.get("requester"), Mapping) else {}
            if req.get("job_title"):
                rows.append(("Requester title", str(req["job_title"])))
        return rows

    def description(self) -> str:
        return html_to_text(self.record.get("description") or "") or str(self.record.get("description_text") or "").strip()

    def attachments(self) -> list[dict[str, Any]]:
        out = []
        for a in list(self.record.get("attachments") or []) + list(self.record.get("cloud_files") or []):
            if isinstance(a, Mapping):
                out.append({"name": str(a.get("name") or a.get("content_file_name") or a.get("filename") or a.get("id")),
                            "size": a.get("size") or a.get("content_file_size")})
        return out

    def notes(self) -> list[str]:
        out = []
        for m in (self.tabs_missing, self.lifecycle_missing, self.planning_missing,
                  self.requested_items_missing, self.conversations_missing):
            if m:
                out.append(m.note)
        if self.properties_note:
            out.append(self.properties_note)
        return out

    def to_json(self) -> dict[str, Any]:
        out: dict[str, Any] = {
            "id": self.id, "subject": self.subject, "status": self.badge, "due_by": self.record.get("due_by"),
            "reporter": self.reporter(),
            "tabs": [t.to_json() for t in self.tabs],
            "lifecycle": self.lifecycle.to_json() if self.lifecycle else None,
            "details": {"description": self.description(), "attachments": self.attachments()},
            "summary": [{"label": k, "value": v} for k, v in self.summary()],
            "properties": [p.to_json() for p in self.properties],
            "notes": self.notes(),
        }
        if self.planning is not None:
            out["details"]["planning"] = [p.to_json() for p in self.planning]
        if self.requested_items is not None:
            out["details"]["requested_items"] = [requested_item_json(i) for i in self.requested_items]
        if self.conversations is not None:
            out["details"]["conversations"] = [conversation_json(cv, self.tz) for cv in self.conversations]
            out["details"]["conversations_total"] = self.conversations_meta.get("count")
        if self.tab_data:
            out["tab_data"] = {k: v.to_json() for k, v in self.tab_data.items()}
        return out


def requested_item_json(item: Mapping[str, Any]) -> dict[str, Any]:
    from fsv.render import _requested_item_rows
    meta = item.get("item") or {}
    stage = item.get("stage")
    return {"name": str(meta.get("name") or meta.get("short_description") or ""),
            "stage": _name(stage) if isinstance(stage, Mapping) else (stage or ""),
            "quantity": item.get("quantity"),
            "answers": [{"label": k, "value": v} for k, v in _requested_item_rows(dict(item))]}


def conversation_json(conv: Mapping[str, Any], tz: dt.tzinfo | None) -> dict[str, Any]:
    return {"id": conv.get("id"), "author": _name(conv.get("user")) or str(conv.get("user_id") or ""),
            "kind": conversation_kind(conv), "private": bool(conv.get("private")),
            "created_at": conv.get("created_at"), "relative": relative(conv.get("created_at")),
            "body": html_to_text(conv.get("body") or "") or str(conv.get("body_text") or ""),
            "attachments": _file_names(conv.get("attachments"))}


def _requested_for(c: Any, record: Mapping[str, Any]) -> str | None:
    rf = record.get("requested_for_id")
    if rf in (None, ""):
        return None
    if str(rf) == str(record.get("requester_id")):
        return "Self"
    data, miss = optional_get(c, f"users/{rf}", None, "requested for")
    if not miss and isinstance(data, Mapping):
        u = data.get("user", data)
        nm = _name(u) if isinstance(u, Mapping) else ""
        if nm:
            return nm
    return f"#{rf}"


def load_detail(c: Any, res: Resource, item_id: Any, *, record: Mapping[str, Any] | None = None,
                schema: Mapping[str, Any] | None = None, all_conversations: bool = False,
                tabs: Sequence[str] = (), loaders: Mapping[str, Callable[..., Any]] | None = None) -> Detail:
    """Everything the site's detail page shows for one record."""
    from fsv import service

    if record is None:
        record = service.get_item(res, item_id, client=c,
                                  includes="requester,stats,phone,feedback,ticket_status,responder"
                                  if res.name == "tickets" else None)
    rec = dict(record)
    if schema is None:
        schema = schema_mod.load(res, c)
    sch = dict(schema or {"fields": []})
    user = current_user(c)
    tz = user_tz(user)
    rid = rec.get("display_id") or rec.get("id") or item_id

    tabs_data, tabs_miss = optional_get(c, f"{res.api_path}/{rid}/tabs", None, "tabs")
    raw_tab_names: set[str] = set()
    if tabs_miss or not isinstance(tabs_data, (list, Mapping)):
        tab_list = [Tab("details", tab_label("details"))] + [Tab(n, tab_label(n)) for n in FALLBACK_TABS.get(res.name, ())]
    else:
        tab_list = parse_tabs(tabs_data)
        rows = tabs_data if isinstance(tabs_data, list) else tabs_data.get("tabs") or []
        raw_tab_names = {str(t.get("name")) for t in rows if isinstance(t, Mapping)}
    d = Detail(res, rec, sch, user, tz, tab_list, tabs_missing=tabs_miss)

    if res.name == "changes":
        d.lifecycle, d.lifecycle_missing = load_lifecycle(c, rec)
        d.planning, d.planning_missing = load_planning(c, rec, sch)
    if res.name == "tickets":
        d.requested_for = _requested_for(c, rec)
        wants_items = "requested_items" in raw_tab_names or tabs_miss is not None
        if wants_items:
            try:
                d.requested_items = service.get_requested_items(rid, client=c)
            except SessionError:
                raise
            except APIError as e:
                d.requested_items_missing = Missing("requested items", e.status)
            except Exception:  # noqa: BLE001
                d.requested_items_missing = Missing("requested items")
            if d.requested_items_missing and d.requested_items_missing.status == 404 and tabs_miss is not None:
                d.requested_items_missing = None
        convs, meta, miss = load_conversations(c, rid, all_pages=all_conversations)
        d.conversations, d.conversations_meta, d.conversations_missing = (None if miss else convs), meta, miss
    d.properties, d.properties_note = load_properties(c, res, rec, sch, tz, user)
    for name in tabs:
        cname = canonical_tab(name)
        if cname == "details":
            continue
        d.tab_data[cname] = load_tab(c, res, rec, cname)
    return d


def resolve_tab(d: Detail, text: str) -> str:
    """Canonical tab name the server offers (or any known one when the
    tabs list is unavailable); ValueError otherwise."""
    name = canonical_tab(text)
    names = d.tab_names()
    if name in names:
        return name
    matches = [n for n in names if n.startswith(name) or tab_label(n).casefold().startswith(text.strip().casefold())]
    if len(matches) == 1:
        return matches[0]
    if d.tabs_missing is not None and name in TAB_LABELS:
        return name
    shown = ", ".join(n for n in names)
    raise ValueError(f"tab {text!r} is not shown for {d.id}; tabs: {shown}")


__all__ = [
    "Column", "Detail", "Lifecycle", "ListPage", "Missing", "PlanningEntry", "Prop", "SavedView", "Tab", "TabData",
    "assigned_to", "canonical_tab", "cell", "current_user", "default_view", "find_view", "fmt_when", "html_to_text",
    "list_columns", "list_meta", "list_row", "load_detail", "load_lifecycle", "load_planning", "load_properties",
    "load_tab", "load_views", "optional_get", "relative", "resolve_tab", "sort_label", "span", "status_badge",
    "ticket_headers", "ticket_state", "user_tz", "view_settings", "workspace_get",
]

