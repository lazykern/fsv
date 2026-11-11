"""Form specification: the fields a Freshservice form shows, built from the
same metadata the web UI uses.

Two sources are supported:

* module form fields (``GET /api/_/change_form_fields``, ``/api/_/ticket_form_fields``
  and the ``/api/v2/*_form_fields`` variants): :meth:`FormSpec.from_form_fields`
* a service-catalog item (``GET /api/_/catalog/service-items/<id>``):
  :meth:`FormSpec.from_service_item`

The spec is pure data: no HTTP, no rules. Business rules live in
:mod:`fsv.forms.rules`, user answers in :mod:`fsv.forms.state`.
"""
from __future__ import annotations

import html
import json
import re
from dataclasses import dataclass, field as dc_field, replace
from typing import Any, Iterable, Mapping, Sequence

from fsv.forms.errors import UnknownFieldError

# Field kinds understood by the engine.
KINDS = frozenset(
    {
        "text",
        "textarea",
        "richtext",
        "number",
        "checkbox",
        "dropdown",
        "multi_select",
        "date",
        "datetime",
        "lookup_user",
        "lookup_group",
        "lookup_agent",
        "lookup",  # any other lookup entity (e.g. maintenance window)
        "department",
        "nested",
        "attachment",
        "notice",
    }
)

CHOICE_KINDS = frozenset({"dropdown", "multi_select", "department", "nested"})
# Content-only blocks (static notes, guides): shown on the form, never an
# input. Field types seen or documented for them; any other ``field_type``
# naming content/static/rich-text blocks is treated the same way.
CONTENT_FIELD_TYPES = frozenset({
    "custom_content", "content", "nested_content", "default_content", "custom_static_rich_text",
    "static_rich_text", "custom_static_text", "static_content",
})
_CONTENT_TYPE = re.compile(r"(^|_)(content|static)(_|$)")


def is_content_type(field_type: str) -> bool:
    """A form-field type that is a static content block, not an input."""
    ft = str(field_type or "").lower()
    return ft in CONTENT_FIELD_TYPES or bool(_CONTENT_TYPE.search(ft))
LOOKUP_KINDS = frozenset({"lookup_user", "lookup_group", "lookup_agent", "lookup"})
INPUTLESS_KINDS = frozenset({"notice"})

# Module form-field ``field_type`` -> kind.
_DEFAULT_KINDS: dict[str, str] = {
    "default_requester": "lookup_user",
    "default_department": "department",
    "default_group": "lookup_group",
    "default_agent": "lookup_agent",
    "default_subject": "text",
    "default_description": "richtext",
    "default_planned_start_date": "datetime",
    "default_planned_end_date": "datetime",
    "default_change_window": "lookup",
    "default_category": "nested",
    "default_planned_effort": "text",
    "default_change_type": "dropdown",
    "default_impact": "dropdown",
    "default_priority": "dropdown",
    "default_risk": "dropdown",
    "default_status": "dropdown",
    "default_urgency": "dropdown",
    "default_source": "dropdown",
    "default_ticket_type": "dropdown",
    # Planning fields (not on the create form; position is null).
    "default_change_reason": "richtext",
    "default_change_impact": "richtext",
    "default_change_plan": "richtext",
    "default_backout_plan": "richtext",
}
_CUSTOM_KINDS: dict[str, str] = {
    "custom_text": "text",
    "custom_paragraph": "textarea",
    "custom_number": "number",
    "custom_decimal": "number",
    "custom_checkbox": "checkbox",
    "custom_dropdown": "dropdown",
    "custom_multi_select_dropdown": "multi_select",
    "custom_date": "datetime",
    "custom_content": "notice",
    "nested_field": "nested",
    "custom_lookup": "lookup_user",
    "custom_multi_lookup": "lookup_user",
    "planning_field": "richtext",
}
# Default dropdowns the web pre-selects with their first visible choice
# (e.g. Change Type, Impact, Priority, Risk, Urgency, Status, Source).
FIRST_CHOICE_DEFAULT_TYPES = frozenset(
    {
        "default_change_type",
        "default_impact",
        "default_priority",
        "default_risk",
        "default_urgency",
        "default_status",
        "default_source",
    }
)
# lookup_config.data_source value used for user lookups.
_USER_LOOKUP_SOURCE = 5

# Service-catalog ("formserv") field type codes -> kind.
SR_TYPE_KINDS: dict[int, str] = {
    1001: "text",
    1003: "dropdown",
    1007: "date",  # datetime unless field_options.date_only == "true"
    1008: "textarea",
    1014: "attachment",
    1024: "notice",
}

# Rule/DOM name aliases -> API field name.
_NAME_ALIASES: dict[str, str] = {
    "department_id": "department",
    "group_id": "group",
    "agent_id": "agent",
    "responder_id": "agent",
    "requester_id": "requester",
    "change_window_id": "change_window",
    "type": "ticket_type",
    "description_html": "description",
}
_PREFIXES = ("customFields.", "customField.", "custom_fields.", "custom_field.")
CUSTOM_PREFIXES = _PREFIXES
# ``/lookup_choices?field_name=<name>_<account id>&module=...`` in lookup configs.
_LOOKUP_FIELD_NAME = re.compile(r"field_name=([A-Za-z0-9_]+?)_(\d+)(?:&|$)")


def norm(value: Any) -> str:
    """Normalise a string for matching: collapse whitespace, casefold."""
    return " ".join(str(value).split()).casefold()


def camel_to_snake(name: str) -> str:
    if not any(ch.isupper() for ch in name):
        return name
    return re.sub(r"(?<!^)(?=[A-Z])", "_", name).lower()


_TAG = re.compile(r"<[^>]+>")
_BLOCK = re.compile(r"<\s*(br|/p|/div|/li)\s*/?>", re.I)


def plain_text(markup: str) -> str:
    """Rough HTML -> text for notices, placeholders and help text."""
    if not markup:
        return ""
    text = html.unescape(markup)
    if "<" in text:
        text = _BLOCK.sub("\n", text)
        text = _TAG.sub("", text)
        text = html.unescape(text)
    lines = [" ".join(line.split()) for line in text.replace("\xa0", " ").splitlines()]
    return "\n".join(line for line in lines if line).strip()


@dataclass(frozen=True)
class Choice:
    """One selectable option.

    ``value`` is what the form stores (and what custom fields send);
    ``id`` is what default fields send (e.g. ``priority: 2``).
    ``description`` is the long meaning shown by the web (parsed from the
    field's help text when it lists ``Label - meaning || ...``).
    """

    value: str
    label: str
    id: Any = None
    display_id: Any = None
    description: str = ""
    visible: bool = True
    children: tuple["Choice", ...] = ()

    def keys(self) -> set[str]:
        out = {norm(self.value), norm(self.label)}
        if self.id is not None:
            out.add(norm(self.id))
        if self.display_id is not None:
            out.add(norm(self.display_id))
        return out


@dataclass(frozen=True)
class Section:
    """Fields revealed when the controlling dropdown has one of ``choices``."""

    choices: tuple[str, ...]  # normalised choice values
    fields: tuple[str, ...]  # api names


@dataclass
class Field:
    api_name: str
    label: str
    kind: str
    choices: tuple[Choice, ...] = ()
    required_base: bool = False
    position: float | None = None
    helptext: str = ""
    hint: bool = False
    placeholder: str = ""
    description: str = ""
    default: Any = None
    is_custom: bool = False
    field_type: str = ""
    id: Any = None
    multi: bool = False
    visible_base: bool = True
    on_form: bool = True
    content: str = ""  # notice HTML
    parent: str | None = None  # controlling dropdown (section) or upper nested level
    level: int = 1  # nested level (1 = top)
    levels: tuple[str, ...] = ()  # api names of deeper nested levels (on the top field)
    sections: tuple[Section, ...] = ()
    date_only: bool = False
    lookup_entity: str | None = None
    lookup_link: str | None = None
    allowed_file_types: tuple[str, ...] = ()
    max_files: int | None = None
    _display_ids: dict[str, Choice] | None = dc_field(default=None, init=False, repr=False, compare=False)

    @property
    def help(self) -> str:
        return self.helptext

    @property
    def is_input(self) -> bool:
        """False for content-only blocks (notes, guides): never answered,
        listed, required or sent."""
        return self.kind not in INPUTLESS_KINDS

    @property
    def is_nested_level(self) -> bool:
        return self.kind == "nested" and self.level > 1

    def choice(self, raw: Any, choices: Sequence[Choice] | None = None) -> Choice | None:
        """Find a choice by value/label (normalised exact), else by id/display id."""
        pool = self.choices if choices is None else choices
        key = norm(raw)
        for c in pool:
            if norm(c.value) == key or norm(c.label) == key:
                return c
        for c in pool:
            if (c.id is not None and norm(c.id) == key) or (c.display_id is not None and norm(c.display_id) == key):
                return c
        return None

    def choice_by_display_id(self, display_id: Any) -> Choice | None:
        """The choice with this ``display_id`` (what ``populate`` actions use)."""
        if self._display_ids is None:
            self._display_ids = {norm(c.display_id): c for c in self.choices if c.display_id is not None}
        return self._display_ids.get(norm(display_id))


@dataclass
class ServiceItem:
    """Service-catalog item metadata needed to place a request."""

    id: Any
    display_id: Any
    name: str
    short_description: str = ""
    category_name: str = ""
    description_html: str = ""
    allow_attachments: bool = True
    attachment_mandatory: bool = False
    allow_quantity: bool = False
    subject_template: str = ""

    def render_subject(self, values: Mapping[str, Any]) -> str:
        """Fill ``{{custom_field.cf_<name>}}`` placeholders from field values."""

        def repl(m: re.Match[str]) -> str:
            key = m.group(1)
            for cand in (key, key[3:] if key.startswith("cf_") else key):
                if cand in values and values[cand] not in (None, ""):
                    return str(values[cand])
            return ""

        return re.sub(r"\{\{\s*custom_field\.([\w]+)\s*\}\}", repl, self.subject_template or "").strip()


@dataclass
class FormSpec:
    module: str  # "change" | "ticket" | "service_request" | other module name
    fields: list[Field]
    item: ServiceItem | None = None
    warnings: list[str] = dc_field(default_factory=list)
    # Freshservice account id: custom fields are known to the server as
    # ``<name>_<account id>`` (e.g. in ``br_validation_excludes``). Derived
    # from lookup links / formserv ``account_id`` when the metadata has it.
    account_id: str | None = None

    def __post_init__(self) -> None:
        self.by_name: dict[str, Field] = {f.api_name: f for f in self.fields}
        self.by_id: dict[str, Field] = {str(f.id): f for f in self.fields if f.id is not None}

    # -- lookup -----------------------------------------------------------
    def find(self, name: str) -> Field | None:
        """Find by API name, rule/DOM name, or label (normalised exact)."""
        if name in self.by_name:
            return self.by_name[name]
        canonical = self.resolve_name(name)
        if canonical:
            return self.by_name[canonical]
        key = norm(name)
        hits = [f for f in self.fields if norm(f.label) == key]
        if len(hits) == 1:
            return hits[0]
        return None

    def field(self, name: str) -> Field:
        f = self.find(name)
        if f is None:
            key = norm(name)
            sugg = [f.label for f in self.fields if key and key in norm(f.label)][:5]
            raise UnknownFieldError(name, sugg)
        return f

    def resolve_name(self, name: str) -> str | None:
        """Map a business-rule / web DOM field name to the API name.

        Handles ``customFields.x`` / ``customField.sr_x`` prefixes, camelCase
        DOM names (``plannedStartDate``), and ``*_id`` foreign keys
        (``department_id`` -> ``department``). Returns None when unknown.
        """
        if not name:
            return None
        n = str(name)
        for p in _PREFIXES:
            if n.startswith(p):
                n = n[len(p):]
                break
        cands = [n, camel_to_snake(n)]
        for c in list(cands):
            if c in _NAME_ALIASES:
                cands.append(_NAME_ALIASES[c])
            if c.startswith("sr_"):
                cands.append(c[3:])
            if c.endswith("_id"):
                cands.append(c[:-3])
        for c in cands:
            if c in self.by_name:
                return c
        return None

    # -- structure --------------------------------------------------------
    def ordered(self) -> list[Field]:
        """Fields in web order: by position, with section children and nested
        levels placed right after the field that controls them."""
        children: dict[str, list[Field]] = {}
        top: list[Field] = []
        for f in self.fields:
            if not f.on_form:
                continue
            if f.parent and f.parent in self.by_name:
                children.setdefault(f.parent, []).append(f)
            else:
                top.append(f)
        top.sort(key=lambda f: (f.position is None, f.position or 0))
        out: list[Field] = []

        def emit(f: Field) -> None:
            out.append(f)
            kids = children.get(f.api_name, [])
            kids.sort(key=lambda k: (k.level, k.position is None, k.position or 0))
            for k in kids:
                emit(k)

        for f in top:
            emit(f)
        return out

    def inputs(self) -> list[Field]:
        """:meth:`ordered` without content-only blocks: the fields a person
        can answer."""
        return [f for f in self.ordered() if f.is_input]

    def section_parent(self, name: str) -> tuple[Field, tuple[str, ...]] | None:
        """(controlling field, normalised choices) if ``name`` sits in a section."""
        f = self.by_name.get(name)
        if not f or not f.parent:
            return None
        parent = self.by_name.get(f.parent)
        if not parent:
            return None
        for s in parent.sections:
            if name in s.fields:
                return parent, s.choices
        return None

    def choices_for(self, f: Field, values: Mapping[str, Any]) -> tuple[Choice, ...]:
        """Choices available now (nested levels depend on the upper levels)."""
        if not f.is_nested_level:
            return f.choices
        chain: list[Field] = []
        cur: Field | None = f
        while cur is not None and cur.is_nested_level:
            parent = self.by_name.get(cur.parent or "")
            if parent is None:
                return ()
            chain.append(parent)
            cur = parent
        pool: tuple[Choice, ...] = chain[-1].choices if chain else ()
        for upper in reversed(chain):
            picked = values.get(upper.api_name)
            if picked in (None, ""):
                return ()
            c = upper.choice(picked, pool)
            if c is None:
                return ()
            pool = c.children
        return pool

    # -- builders ---------------------------------------------------------
    @classmethod
    def from_form_fields(cls, data: Any, module: str = "change") -> "FormSpec":
        """Build from a module form-fields response (internal or v2 API)."""
        raw = _unwrap_fields(data)
        warnings: list[str] = []
        fields: list[Field] = []
        for f in raw:
            fields.extend(_fields_from_form_field(f, warnings))
        spec = cls(module=module, fields=fields, warnings=warnings, account_id=_account_from_form_fields(raw))
        _link_ticket_sections(spec, raw)
        return spec

    @classmethod
    def from_service_item(cls, data: Mapping[str, Any]) -> "FormSpec":
        """Build from ``GET /api/_/catalog/service-items/<id>``."""
        si = data.get("service_item", data) if isinstance(data, Mapping) else data
        warnings: list[str] = []
        fields: list[Field] = []
        id_to_name: dict[str, str] = {}
        pending_sections: list[tuple[Field, list[dict[str, Any]]]] = []

        def walk(items: Iterable[Mapping[str, Any]], parent: str | None) -> None:
            for c in items:
                if c.get("deleted"):
                    continue
                f, sections = _field_from_sr(c, parent, warnings)
                fields.append(f)
                id_to_name[str(c.get("id"))] = f.api_name
                if sections:
                    pending_sections.append((f, sections))
                walk(c.get("fields") or [], f.api_name)

        walk(si.get("custom_fields") or [], None)
        for f, sections in pending_sections:
            out = []
            for s in sections:
                names = tuple(id_to_name[str(i)] for i in s.get("fields", []) if str(i) in id_to_name)
                out.append(Section(choices=tuple(norm(x) for x in s.get("choices", [])), fields=names))
            f.sections = tuple(out)
        configs = si.get("configs") or {}
        item = ServiceItem(
            id=si.get("id"),
            display_id=si.get("display_id"),
            name=str(si.get("name") or ""),
            short_description=str(si.get("short_description") or ""),
            category_name=str(si.get("category_name") or ""),
            description_html=str(si.get("description") or ""),
            allow_attachments=bool(si.get("allow_attachments", True)),
            attachment_mandatory=bool(si.get("attachment_mandatory", configs.get("attachment_mandatory", False))),
            allow_quantity=bool(si.get("allow_quantity", False)),
            subject_template=str(configs.get("subject") or ""),
        )
        return cls(
            module="service_request", fields=fields, item=item, warnings=warnings, account_id=_account_from_item(si)
        )


# ---------------------------------------------------------------------------
# module form fields


def _unwrap_fields(data: Any) -> list[Mapping[str, Any]]:
    if isinstance(data, list):
        return data
    if isinstance(data, Mapping):
        for key in ("change_fields", "ticket_fields", "problem_fields", "release_fields", "fields", "form_fields"):
            if isinstance(data.get(key), list):
                return data[key]
        for v in data.values():
            if isinstance(v, list) and v and isinstance(v[0], Mapping) and "field_type" in v[0]:
                return v
    raise ValueError("unrecognised form-fields payload")


def _account_from_form_fields(raw: Sequence[Mapping[str, Any]]) -> str | None:
    """Account id from ``lookup_config.link`` (``field_name=<name>_<id>``)."""
    found: set[str] = set()
    for f in raw:
        cfg = f.get("lookup_config") or {}
        link = str(cfg.get("link") or "") if isinstance(cfg, Mapping) else ""
        m = _LOOKUP_FIELD_NAME.search(link)
        if m and m.group(1) == str(f.get("name")):
            found.add(m.group(2))
    return found.pop() if len(found) == 1 else None


def _account_from_item(si: Mapping[str, Any]) -> str | None:
    found: set[str] = set()

    def walk(items: Iterable[Mapping[str, Any]]) -> None:
        for c in items or []:
            if c.get("account_id") not in (None, ""):
                found.add(str(c["account_id"]))
            walk(c.get("fields") or [])

    walk(si.get("custom_fields") or [])
    if si.get("account_id") not in (None, ""):
        found.add(str(si["account_id"]))
    return found.pop() if len(found) == 1 else None


def _parse_choices(raw: Any) -> tuple[Choice, ...]:
    out: list[Choice] = []
    for c in raw or []:
        if isinstance(c, Mapping):
            value = c.get("value")
            label = c.get("label", value)
            out.append(
                Choice(
                    value=str(value if value is not None else label),
                    label=html.unescape(str(label if label is not None else value)),
                    id=c.get("id"),
                    display_id=c.get("display_id"),
                    description=" ".join(str(c.get("description") or "").split()),
                    visible=c.get("visible", True) is not False,
                    children=_parse_choices(c.get("nested_options")),
                )
            )
        elif isinstance(c, (list, tuple)) and c:
            out.append(Choice(value=str(c[0]), label=str(c[0]), id=c[1] if len(c) > 1 else None))
        else:
            out.append(Choice(value=str(c), label=str(c)))
    return tuple(out)


def split_choice_help(choices: Sequence[Choice], helptext: str) -> tuple[tuple[Choice, ...], str]:
    """Parse ``"Very High - meaning || Low - meaning"`` help text.

    Each ``||`` segment goes to the longest choice label it starts with
    (case-insensitive) followed by optional spaces and ``-``; that becomes the
    choice's description. Returns the choices and the unmatched segments
    (general help text). Choices keep any description they already have."""
    if not helptext or "||" not in helptext:
        return tuple(choices), helptext.strip()
    segs = [" ".join(x.split()) for x in helptext.split("||")]
    names = sorted({" ".join((c.label or c.value).split()): (c.label or c.value) for c in choices
                    if (c.label or c.value)}.items(), key=lambda kv: len(kv[0]), reverse=True)
    found: dict[str, str] = {}
    rest: list[str] = []
    for seg in segs:
        if not seg:
            continue
        low = seg.lower()
        for flat, name in names:
            if low.startswith(flat.lower()):
                m = re.match(r"\s*-\s*(.+)$", seg[len(flat):])
                if m:
                    found.setdefault(norm(name), m.group(1).strip())
                    break
        else:
            rest.append(seg)
    out = []
    for c in choices:
        d = found.get(norm(c.label)) or found.get(norm(c.value))
        out.append(replace(c, description=d) if d else c)
    return tuple(out), " ".join(rest)


def attach_choice_descriptions(choices: Sequence[Choice], helptext: str) -> tuple[Choice, ...]:
    return split_choice_help(choices, helptext)[0]


def _fields_from_form_field(f: Mapping[str, Any], warnings: list[str]) -> list[Field]:
    ftype = str(f.get("field_type") or "")
    name = str(f.get("name") or "")
    is_default = bool(f.get("default_field", ftype.startswith("default_")))
    kind = _DEFAULT_KINDS.get(ftype) or _CUSTOM_KINDS.get(ftype)
    if kind is None and is_content_type(ftype):
        kind = "notice"
    if kind is None:
        kind = "text"
        warnings.append(f"field {name!r}: unknown field_type {ftype!r}, treated as text")
    lookup_cfg = f.get("lookup_config") or {}
    lookup_entity = None
    if ftype in ("custom_lookup", "custom_multi_lookup"):
        src = lookup_cfg.get("data_source", f.get("data_source"))
        if src not in (None, _USER_LOOKUP_SOURCE):
            kind = "lookup"
            lookup_entity = str(src)
        else:
            lookup_entity = "user"
    elif ftype == "default_change_window":
        lookup_entity = "change_window"
    if kind == "datetime" and (f.get("date_only") is True or str(f.get("date_only")).lower() == "true"):
        kind = "date"
    ht = f.get("helptext")
    helptext = ht if isinstance(ht, str) else ""  # `hint` is a boolean show/hide flag; never text
    choices, helptext = split_choice_help(_parse_choices(f.get("choices")), helptext)
    position = f.get("position")
    required = f.get("required")
    if required is None:
        required = f.get("required_for_agents", False)
    default = None
    if ftype in FIRST_CHOICE_DEFAULT_TYPES:
        first = next((c for c in choices if c.visible), None)
        default = first.value if first else None
    nested_levels = sorted(f.get("nested_fields") or [], key=lambda n: n.get("level") or 0)
    top = Field(
        api_name=name,
        label=html.unescape(str(f.get("label") or name)),
        kind=kind,
        choices=choices,
        required_base=bool(required),
        position=position,
        helptext=helptext,
        hint=bool(f.get("hint")),
        placeholder=plain_text(str(f.get("placeholder") or "")),
        description=str(f.get("description") or ""),
        default=default,
        is_custom=not is_default,
        field_type=ftype,
        id=f.get("id"),
        multi=ftype in ("custom_multi_select_dropdown", "custom_multi_lookup"),
        visible_base=f.get("displayed_to_agents", True) is not False,
        on_form=position is not None and ftype != "planning_field",
        content=html.unescape(str(_content_of(f) if kind == "notice" else (f.get("content") or ""))),
        levels=tuple(str(n.get("name")) for n in nested_levels),
        date_only=kind == "date",
        lookup_entity=lookup_entity,
        lookup_link=lookup_cfg.get("link"),
    )
    out = [top]
    prev = name
    for n in nested_levels:
        lvl_name = str(n.get("name"))
        out.append(
            Field(
                api_name=lvl_name,
                label=html.unescape(str(n.get("label") or lvl_name)),
                kind="nested",
                position=position,
                is_custom=top.is_custom,
                field_type=ftype,
                id=n.get("id"),
                visible_base=top.visible_base,
                on_form=top.on_form,
                parent=prev,
                level=int(n.get("level") or 2),
                placeholder=plain_text(str(n.get("placeholder") or "")),
                description=str(n.get("description") or ""),
            )
        )
        prev = lvl_name
    return out


def _content_of(f: Mapping[str, Any]) -> str:
    """A content block's HTML: ``content``, else ``field_options.content``,
    else its description/help text."""
    opts = f.get("field_options") if isinstance(f.get("field_options"), Mapping) else {}
    for v in (f.get("content"), opts.get("content"), opts.get("content_html"), f.get("description"), f.get("helptext")):
        if isinstance(v, str) and v.strip():
            return v
    return ""


def _link_ticket_sections(spec: FormSpec, raw: Sequence[Mapping[str, Any]]) -> None:
    """Dynamic sections (``sections: [{choice_ids, ticket_field_ids}]``).

    A field flagged ``belongs_to_section`` that no section lists is never
    revealed by a choice: it starts hidden (a rule may still show it)."""
    _link_sections(spec, raw)
    for f in raw:
        if not f.get("belongs_to_section"):
            continue
        fld = spec.by_name.get(str(f.get("name")))
        if fld is not None and not fld.parent and fld.visible_base:
            fld.visible_base = False
            spec.warnings.append(f"field {fld.api_name!r} belongs to a section that no choice reveals; hidden")


def _link_sections(spec: FormSpec, raw: Sequence[Mapping[str, Any]]) -> None:
    for f in raw:
        sections = f.get("sections") or []
        if not sections:
            continue
        ctrl = spec.by_name.get(str(f.get("name")))
        if ctrl is None:
            continue
        by_choice_id = {str(c.id): c for c in ctrl.choices}
        out = []
        for s in sections:
            ids = s.get("ticket_field_ids") or s.get("field_ids") or s.get("change_field_ids") or []
            names = tuple(spec.by_id[str(i)].api_name for i in ids if str(i) in spec.by_id)
            vals = tuple(norm(by_choice_id[str(i)].value) for i in s.get("choice_ids") or [] if str(i) in by_choice_id)
            for n in names:
                spec.by_name[n].parent = ctrl.api_name
            out.append(Section(choices=vals, fields=names))
        ctrl.sections = tuple(out)


# ---------------------------------------------------------------------------
# service catalog items


def _opt_true(opts: Mapping[str, Any], key: str) -> bool:
    return str(opts.get(key, "")).lower() == "true"


def _field_from_sr(
    c: Mapping[str, Any], parent: str | None, warnings: list[str]
) -> tuple[Field, list[dict[str, Any]]]:
    opts = c.get("field_options") or {}
    code = c.get("type")
    name = str(c.get("name") or c.get("id"))
    kind = SR_TYPE_KINDS.get(code if isinstance(code, int) else -1)
    if kind is None:
        kind = "text"
        warnings.append(f"item field {name!r}: unknown type {code!r}, treated as text")
    if kind == "date" and opts.get("date_only") is not None and not _opt_true(opts, "date_only"):
        kind = "datetime"
    raw_label = str(c.get("label") or name)
    content = ""
    label = html.unescape(raw_label)
    if kind == "notice":
        content = html.unescape(raw_label)
        label = str(opts.get("content_name") or "Notice")
    raw_choices = sorted(c.get("choices") or [], key=lambda x: x.get("position") or 0)
    choices = tuple(
        Choice(value=str(x.get("value")), label=str(x.get("label", x.get("value"))), id=x.get("id"),
               description=" ".join(str(x.get("description") or "").split()))
        for x in raw_choices
    )
    ht = c.get("helptext")
    helptext = ht if isinstance(ht, str) else ""  # a boolean `hint` is a show/hide flag, never text
    choices, helptext = split_choice_help(choices, helptext)
    sections: list[dict[str, Any]] = []
    info = opts.get("section_info")
    if info:
        try:
            for s in json.loads(info) if isinstance(info, str) else info:
                sections.append({"fields": s.get("sectionFields") or [], "choices": s.get("sectionChoices") or []})
        except (ValueError, TypeError):
            warnings.append(f"item field {name!r}: unreadable section_info")
    dep_sections = []
    for x in raw_choices:
        dep = (x.get("dependent_ids") or {}).get("field") or []
        if dep:
            dep_sections.append({"fields": dep, "choices": [x.get("value")]})
    if not sections:
        sections = dep_sections
    elif dep_sections:
        # Both sources present: formserv's choices[].dependent_ids is what
        # reveals the child fields; section_info normally mirrors it. Merge
        # anything section_info is missing.
        covered = {(norm(ch), str(fid)) for s in sections for ch in s["choices"] for fid in s["fields"]}
        extra = [s for s in dep_sections
                 if any((norm(s["choices"][0]), str(fid)) not in covered for fid in s["fields"])]
        if extra:
            warnings.append(f"item field {name!r}: section_info and dependent_ids differ; using both")
            sections = sections + extra
    file_types: tuple[str, ...] = ()
    if opts.get("allowed_file_types"):
        try:
            ft = opts["allowed_file_types"]
            file_types = tuple(json.loads(ft) if isinstance(ft, str) else ft)
        except (ValueError, TypeError):
            pass
    max_files = None
    if str(opts.get("max_attachment_limit", "")).isdigit():
        max_files = int(opts["max_attachment_limit"])
    f = Field(
        api_name=name,
        label=label,
        kind=kind,
        choices=choices,
        required_base=bool(c.get("required")),
        position=c.get("position"),
        helptext=helptext,
        hint=c.get("hint") is True,
        placeholder=plain_text(str(c.get("placeholder") or "")),
        is_custom=True,
        field_type=f"formserv:{code}",
        id=c.get("id"),
        visible_base=c.get("visible", True) is not False,
        on_form=True,
        content=content,
        parent=parent,
        date_only=kind == "date",
        allowed_file_types=file_types,
        max_files=max_files,
    )
    return f, sections
