"""Create-request payload builders matching the bodies the Freshservice web
UI sends:

* change:  ``POST /api/_/changes``
* ticket:  ``POST /api/_/tickets``
* service request: ``POST /api/_/catalog/service-items/<display_id>/place-request``

Only visible fields with a value are sent; hidden values are dropped. Status
is never sent on create unless the caller passes ``status=`` explicitly.
Files must be uploaded first (``POST /api/_/attachments``); the builders
only take the resulting attachment ids.
"""
from __future__ import annotations

import datetime as dt
import html
from typing import Any, Mapping, Sequence

from fsv.forms.errors import PayloadError
from fsv.forms.rules import is_empty
from fsv.forms.spec import Field, FormSpec, ServiceItem
from fsv.forms.state import FormState, LookupValue

# The web form also sends ``br_validation_excludes``: a token the page builds
# (it looks like a signed JWT). Its meaning, and whether the server requires
# it, is unknown. We never fabricate it: callers may pass a value they
# obtained some other way; otherwise the key is omitted.
# TODO(forms): confirm with one real create whether the key is needed.
BR_VALIDATION_EXCLUDES = "br_validation_excludes"

# Default (non-custom) fields whose API key differs from the field name.
CHANGE_KEYS: dict[str, str] = {
    "department": "department_id",
    "group": "group_id",
    "agent": "agent_id",
    "change_window": "change_window_id",
}
TICKET_KEYS: dict[str, str] = {
    "department": "department_id",
    "group": "group_id",
    "agent": "responder_id",
    "ticket_type": "type",
}
# Default dropdowns sent by their label rather than their numeric id.
_SEND_VALUE_TYPES = frozenset({"default_ticket_type"})
# Never sent from form state: requester becomes ``email``; status only via
# the explicit ``status=`` argument.
_NEVER_FROM_STATE = frozenset({"requester", "status"})

_CHANGE_ORDER = (
    "subject",
    "description",
    "status",
    "priority",
    "change_type",
    "risk",
    "impact",
    "group_id",
    "department_id",
    "agent_id",
    "planned_start_date",
    "planned_end_date",
)
_TICKET_ORDER = ("subject", "description", "department_id", "status", "priority", "urgency", "impact", "source")


def format_requester(email: str, name: str | None = None) -> str:
    """``Name <email>`` as the web sends it, or the bare email."""
    email = (email or "").strip()
    if "@" not in email:
        raise PayloadError([f"requester email {email!r} is not an email address"])
    name = (name or "").strip()
    return f"{name} <{email}>" if name else email


def text_to_html(text: str) -> str:
    """Wrap plain text in the minimal HTML the description field expects.
    Text that already looks like HTML is returned unchanged."""
    s = str(text)
    if s.lstrip().startswith("<"):
        return s
    paras = [p for p in s.replace("\r\n", "\n").split("\n\n")]
    body = "".join(f"<div>{html.escape(p).replace(chr(10), '<br>')}</div>" for p in paras if p.strip())
    return body or "<div></div>"


def encode_value(spec: FormSpec, f: Field, value: Any, values: Mapping[str, Any]) -> Any:
    """Form value -> JSON value for the API."""
    k = f.kind
    if k in ("dropdown", "department", "nested"):
        ch = f.choice(value, spec.choices_for(f, values))
        if ch is None:
            raise PayloadError([f"{f.label}: {value!r} is not a valid choice"])
        if not f.is_custom and k != "nested" and f.field_type not in _SEND_VALUE_TYPES and ch.id is not None:
            return ch.id
        return ch.value
    if k == "multi_select":
        return [str(v) for v in value]
    if k == "checkbox":
        return bool(value)
    if k == "date":
        return value.isoformat() if isinstance(value, dt.date) else str(value)
    if k == "datetime":
        return value.isoformat(timespec="seconds") if isinstance(value, dt.datetime) else str(value)
    if k in ("lookup_user", "lookup_group", "lookup_agent", "lookup"):
        if isinstance(value, (list, tuple)):
            return [_lookup_id(f, v) for v in value]
        return _lookup_id(f, value)
    if k == "richtext":
        return text_to_html(value)
    return value


def _lookup_id(f: Field, v: Any) -> Any:
    if isinstance(v, LookupValue):
        return v.id
    if isinstance(v, int):
        return v
    raise PayloadError([f"{f.label}: {v!r} is not resolved to an id"])


def _problems(state: FormState, skip: Sequence[str] = ()) -> list[str]:
    out = [f"{f.label} is required" for f in state.missing_required() if f.api_name not in skip]
    out.extend(f"blocked by rule: {m.text}" for m in state.messages if m.blocking)
    return out


def _check(state: FormState, allow_incomplete: bool, skip: Sequence[str] = ()) -> None:
    if allow_incomplete:
        return
    problems = _problems(state, skip)
    if problems:
        raise PayloadError(problems)


def _module_body(
    state: FormState, keys: Mapping[str, str], order: Sequence[str], status: Any
) -> tuple[dict[str, Any], dict[str, Any]]:
    spec = state.spec
    top: dict[str, Any] = {}
    custom: dict[str, Any] = {}
    values = state.evaluated.values
    for name, v in state.payload_values().items():
        if name in _NEVER_FROM_STATE:
            continue
        f = spec.by_name[name]
        if f.kind == "attachment":
            raise PayloadError([f"{f.label}: file fields are not supported on this form"])
        enc = encode_value(spec, f, v, values)
        if f.is_custom:
            custom[name] = enc
        else:
            top[keys.get(name, name)] = enc
    if status is not None:
        top["status"] = _status_id(spec, status)
    ordered = {k: top[k] for k in order if k in top}
    ordered.update({k: v for k, v in top.items() if k not in ordered})
    return ordered, custom


def _status_id(spec: FormSpec, status: Any) -> Any:
    f = spec.by_name.get("status")
    if f is None:
        return status
    ch = f.choice(status)
    if ch is None:
        raise PayloadError([f"status {status!r} is not a valid choice"])
    return ch.id if ch.id is not None else ch.value


def _requester(state: FormState, email: str | None, name: str | None) -> str:
    if not email:
        v = state.values.get("requester")
        if isinstance(v, LookupValue) and v.email:
            email, name = v.email, name or v.display
    if not email:
        raise PayloadError(["requester email is required"])
    return format_requester(email, name)


def build_change_payload(
    state: FormState,
    requester_email: str | None = None,
    requester_name: str | None = None,
    *,
    status: Any = None,
    br_validation_excludes: str | None = None,
    allow_incomplete: bool = False,
) -> dict[str, Any]:
    """Body for ``POST /api/_/changes``."""
    _check(state, allow_incomplete, skip=("requester",))
    top, custom = _module_body(state, CHANGE_KEYS, _CHANGE_ORDER, status)
    body = dict(top)
    body["custom_fields"] = custom
    body["email"] = _requester(state, requester_email, requester_name)
    if br_validation_excludes is not None:
        body[BR_VALIDATION_EXCLUDES] = br_validation_excludes
    return body


def build_ticket_payload(
    state: FormState,
    requester_email: str | None = None,
    requester_name: str | None = None,
    *,
    status: Any = None,
    br_validation_excludes: str | None = None,
    allow_incomplete: bool = False,
) -> dict[str, Any]:
    """Body for ``POST /api/_/tickets``."""
    _check(state, allow_incomplete, skip=("requester",))
    top, custom = _module_body(state, TICKET_KEYS, _TICKET_ORDER, status)
    body = dict(top)
    body["custom_fields"] = custom
    body["email"] = _requester(state, requester_email, requester_name)
    if br_validation_excludes is not None:
        body[BR_VALIDATION_EXCLUDES] = br_validation_excludes
    return body


def build_sr_payload(
    item: ServiceItem | None,
    state: FormState,
    requester_email: str,
    *,
    field_attachments: Mapping[str, Sequence[Any]] | None = None,
    attachments: Sequence[Any] | None = None,
    br_validation_excludes: str | None = None,
    allow_incomplete: bool = False,
) -> dict[str, Any]:
    """Body for ``POST /api/_/catalog/service-items/<display_id>/place-request``.

    ``field_attachments`` maps a file field's API name to uploaded attachment
    ids; ``attachments`` are the general request attachments.
    """
    item = item or state.spec.item
    if item is None:
        raise PayloadError(["not a service-item form"])
    field_attachments = dict(field_attachments or {})
    attachments = list(attachments or [])
    spec = state.spec
    problems: list[str] = []
    visible = {f.api_name for f in state.visible_fields()}
    file_fields = [f for f in spec.fields if f.kind == "attachment"]
    for name in list(field_attachments):
        target = spec.find(name)
        if target is None or target.kind != "attachment":
            problems.append(f"{name!r} is not a file field of this item")
        elif target.api_name != name:
            field_attachments[target.api_name] = field_attachments.pop(name)
    if not allow_incomplete:
        satisfied = [f.api_name for f in file_fields if field_attachments.get(f.api_name)]
        problems.extend(_problems(state, satisfied))
        if item.attachment_mandatory and not attachments:
            problems.append("this item requires at least one attachment")
    if attachments and not item.allow_attachments:
        problems.append("this item does not accept attachments")
    if problems:
        raise PayloadError(problems)

    custom: dict[str, Any] = {}
    values = state.evaluated.values
    for name, v in state.payload_values().items():
        f = spec.by_name[name]
        if f.kind == "attachment":
            continue
        custom[name] = encode_value(spec, f, v, values)
    for f in file_fields:
        ids = field_attachments.get(f.api_name)
        if ids and f.api_name in visible:
            custom[f.api_name] = list(ids)
        elif f.api_name in visible and not is_empty(state.values.get(f.api_name)) and not allow_incomplete:
            raise PayloadError([f"{f.label}: upload the file(s) first and pass their attachment ids"])
    body: dict[str, Any] = {"email": format_requester(requester_email), "custom_fields": custom}
    if br_validation_excludes is not None:
        body[BR_VALIDATION_EXCLUDES] = br_validation_excludes
    body["attachments"] = attachments
    return body
