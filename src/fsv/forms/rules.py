"""Business-rules engine: the client-side form rules Freshservice runs in the
browser (``GET /api/_/ws/2/business_rules``), evaluated in Python.

Two evaluation models are implemented (``RulesEngine(model=...)``):

``"event"`` (the web's own model, the default)
    1. On load, every rule is evaluated in array order against the load
       values (defaults on a new form, the record on an edit form); a
       reversible rule that does not match applies its reverse.
    2. Each user change is a *change event* that evaluates every rule again,
       in array order, on the current state (``event_scope="all"``). A
       matching rule applies its actions; a reversible rule that does not
       match applies its reverse every time (``event_reverse="always"``).
       ``event_scope="referenced"`` runs only the rules whose conditions read
       the changed field; ``event_reverse="transition"`` reverses only when a
       rule goes from matching to not matching (the earlier semantics).
    3. ``populate`` restricts the choices of a dropdown (see below); it
       fires no change event.
    Answers that arrive without an order (YAML, ``--set``) are replayed as
    change events in web field order (``position``: a person filling the
    form top to bottom). Setting a field to the value it already has is not
    a change event.

``"replay"`` (pure function of the values)
    Every evaluation starts from the metadata and applies *every* rule in
    order; a later rule overrides an earlier one.

``populate`` (``RulesEngine(populate=...)``):

``"restrict"`` (default, what the web does)
    The action's ``values`` (choice ``display_id``s; for nested fields a
    tree ``[{display_id, nested_options}]``) become the only choices the
    dropdown offers (plus the empty choice). The field's value is NOT set.
    A reversible rule that stops matching restores the full choice list.
``"set"`` (legacy)
    The first value is filled in as the field's value. In the replay model
    it never overwrites a user value and feeds back until it settles.

In both models a matching rule applies its actions; a rule that does not
match applies the reverse (show<->hide, mandate<->unmandate,
disable<->enable) only if ``is_reversible``. Several entries may share one
rule id (the endpoint returns a reversible and a non-reversible entry); they
are separate rules evaluated in order.
"""
from __future__ import annotations

from dataclasses import dataclass, field as dc_field, replace
from typing import Any, Collection, Iterable, Mapping, Sequence

from fsv.forms.errors import FormRuleError, UnsupportedRuleError
from fsv.forms.spec import CHOICE_KINDS, CUSTOM_PREFIXES, Choice, Field, FormSpec, norm

SUPPORTED_OPERATORS = frozenset({"is", "is_not", "includes", "does_not_include", "selected", "not_selected"})
SUPPORTED_ACTIONS = frozenset({"show", "hide", "mandate", "unmandate", "populate", "disable", "enable", "message"})
# ``evaluate_on`` values that read a form field (the module's own name) ...
FORM_SOURCES = frozenset({"change", "ticket", "problem", "release", "service_request", "form"})
# ... and those that read a person record from the rule context.
USER_SOURCES: dict[str, str] = {"current_user": "user", "requester": "requester", "requested_for": "requested_for"}
# ... and those that read the context's ``system`` mapping (e.g. ``portal``:
# the id of the portal the form is opened in).
CONTEXT_SOURCES: dict[str, str] = {**USER_SOURCES, "system": "system"}
# ... and the saved record the edit form was opened on (``current_entity``):
# its values as loaded, not the live form answers. They come from
# ``RuleContext.entity`` or, when that is absent, the load values (``base``).
ENTITY_SOURCES: dict[str, str] = {"current_entity": "entity"}
SUPPORTED_EVALUATE_ON = FORM_SOURCES | frozenset(CONTEXT_SOURCES) | frozenset(ENTITY_SOURCES)
# Action targets that are not form fields but form-level switches.
PSEUDO_TARGETS = frozenset({"attachment_ids"})  # general request attachments (SR)
# Rule targets that are UI widgets rather than form fields, mapped to the
# payload keys that would carry them; a hide of one blocks a write sending them.
WIDGET_PAYLOAD_KEYS: dict[str, set[str]] = {
    "config_item_ids": {"device_ids", "config_item_ids", "assets"},
    "tags": {"tags"},
    "approval_request": {"approval_request", "approvals"},
    "add_child_ticket": {"child_tickets"},
    "add_service_item": {"requested_items"},
    "edit_service_item": {"requested_items"},
}
# Record state that is not a form field but is known to be empty on a new record.
NEW_RECORD_EMPTY = frozenset({"tags", "approval_status", "approval_request"})
# fsv fills agent-side forms, never the self-service portal.
AGENT_SYSTEM_CONTEXT: Mapping[str, Any] = {"portal": False}
_REVERSE = {"show": "hide", "hide": "show", "mandate": "unmandate", "unmandate": "mandate", "disable": "enable",
            "enable": "disable", "populate": "restore"}
_KIND = {"show": "visible", "hide": "visible", "mandate": "required", "unmandate": "required",
         "disable": "disabled", "enable": "disabled"}
_POSITIVE = frozenset({"show", "mandate", "disable"})
_MODES = {"new": "new_form", "edit": "edit_form"}
MODELS = ("event", "replay")
# What ``populate`` does: restrict the dropdown's choices (web behaviour,
# default) or set the value (legacy). See the module docstring.
POPULATE_MODES = ("restrict", "set")
DEFAULT_POPULATE = "restrict"
# Default model. Chosen by scoring every variant against captured web forms
# (see docs/forms.md, "Choosing the model").
DEFAULT_MODEL = "event"
MAX_PASSES = 8
# How to treat several entries with the same rule id in one response:
#   "exact" (default) drop entries identical to an earlier one, keep the rest
#           in order (the endpoint returns a reversible and a non-reversible
#           entry per id with different action lists; both are applied).
#   "first"/"last" keep one entry per id (first position; first/last content).
#   "none"  keep everything.
DEDUPE_POLICIES = ("exact", "first", "last", "none")
# Event model: which rules a change event runs. "referenced": the rules whose
# conditions read the changed field; "all": every rule, in array order.
EVENT_SCOPES = ("referenced", "all")
# Defaults chosen by scoring every variant against live-web captures (see
# docs/forms.md, "Choosing the model"): every change re-runs every rule, and a
# reversible rule that does not match applies its reverse each time.
DEFAULT_EVENT_SCOPE = "all"
DEFAULT_EVENT_REVERSE = "always"


def is_empty(value: Any) -> bool:
    return value is None or value == "" or (isinstance(value, (list, tuple, set, frozenset)) and len(value) == 0)


# ---------------------------------------------------------------------------
# data model


@dataclass(frozen=True)
class RuleContext:
    """Who is filling which form: decides which rules apply and what
    ``current_user`` / ``requester`` conditions read.

    ``workspace_id`` may be one id or a collection (a user in workspaces 1
    and 2); ``None`` disables workspace filtering. ``user`` is the
    ``user`` object of the business_rules response. ``system`` holds the
    values ``evaluate_on: "system"`` conditions read (e.g.
    ``{"portal": <portal id>}``).
    """

    module: str | None = None
    form: str = "new"  # "new" | "edit"
    user_type: str = "agent"
    workspace_id: Any = None
    item_id: Any = None
    user: Mapping[str, Any] | None = None
    requester: Mapping[str, Any] | None = None
    requested_for: Mapping[str, Any] | None = None
    system: Mapping[str, Any] | None = None
    # the saved record (``evaluate_on: "current_entity"``); ``None``: use the
    # evaluation's load values
    entity: Mapping[str, Any] | None = None

    def workspaces(self) -> set[str] | None:
        if self.workspace_id is None:
            return None
        ws = self.workspace_id
        items = ws if isinstance(ws, Collection) and not isinstance(ws, (str, bytes)) else [ws]
        return {str(w) for w in items}


@dataclass(frozen=True)
class Condition:
    name: str  # raw name as written in the rule
    field: str | None  # API name (None if unknown or not a form field)
    operator: str
    values: tuple[str, ...] = ()
    source: str = "form"  # "form" | "entity" | "user" | "requester" | "requested_for" | "system"


@dataclass(frozen=True)
class ConditionGroup:
    mode: str  # "all" | "any"
    items: tuple["Condition | ConditionGroup", ...] = ()


@dataclass(frozen=True)
class Target:
    """One action target. ``api`` is the form field (None when unknown);
    ``base``/``custom`` give the server-side name (``<base>_<account>`` for
    custom fields) used in ``br_validation_excludes``."""

    api: str | None
    raw: str
    base: str
    custom: bool


@dataclass(frozen=True)
class Action:
    property: str
    targets: tuple[str, ...]  # API names of known fields; message texts for "message"
    values: tuple[Any, ...] = ()
    all_targets: tuple[Target, ...] = ()


@dataclass(frozen=True)
class Rule:
    id: Any
    name: str
    condition: ConditionGroup
    actions: tuple[Action, ...]
    reversible: bool = False
    form_type: str = "both"
    user_type: str = "both"
    is_validation: bool = False
    index: int = 0
    fields: frozenset[str] = frozenset()  # form fields its conditions read
    unconditional: bool = False

    def applies_to(self, mode: str, user_type: str) -> bool:
        ft = _MODES.get(mode, mode)
        if self.form_type not in ("both", ft):
            return False
        return self.user_type in ("both", user_type) or user_type == "any"


@dataclass(frozen=True)
class Message:
    text: str
    rule_id: Any = None
    rule_name: str = ""
    blocking: bool = False

    @property
    def key(self) -> tuple[str, str]:
        return (str(self.rule_id), self.text)


@dataclass(frozen=True)
class Decision:
    """One rule action applied to one field property."""

    rule_id: Any
    rule_name: str
    action: str  # show/hide/mandate/unmandate/disable/enable/populate/restore
    matched: bool  # False: reverse action of a reversible rule that did not match
    event: str | None = None  # field whose change triggered it; None = load / replay
    rule_index: int = -1

    def describe(self) -> str:
        how = "" if self.matched else " (reversed: rule does not match)"
        on = f" on change of {self.event}" if self.event else ""
        return f"{self.action} by rule {self.rule_name!r} (id {self.rule_id}){how}{on}"


@dataclass(frozen=True)
class Conflict:
    """Matching rules that disagree on one field property; ``winner`` is the
    decision in effect (the last one applied)."""

    field: str
    property: str  # visible | required | disabled
    decisions: tuple[Decision, ...]
    winner: Decision

    def describe(self) -> str:
        rules = "; ".join(f"{d.action} by {d.rule_name!r} (id {d.rule_id})" for d in self.decisions)
        return f"{self.field}.{self.property}: {rules} -> {self.winner.action} wins ({self.winner.rule_name!r})"


@dataclass
class RuleGate:
    """Outcome of :meth:`RulesEngine.write_gate`."""

    blockers: list[str] = dc_field(default_factory=list)
    warnings: list[str] = dc_field(default_factory=list)
    unknown_rules: list[Any] = dc_field(default_factory=list)

    def as_json(self) -> dict[str, Any]:
        return {"blockers": list(self.blockers), "warnings": list(self.warnings),
                "unknown_rules": list(self.unknown_rules)}


@dataclass(frozen=True)
class Explanation:
    field: str
    label: str
    property: str  # visible | required | disabled
    value: bool
    source: str  # rule | metadata | section | nested | parent
    decision: Decision | None = None

    def __str__(self) -> str:
        state = self.property if self.value else f"not {self.property}"
        if self.decision is not None:
            return f"{self.label}: {state} ({self.decision.describe()})"
        why = {
            "metadata": "form metadata",
            "section": "its section's choice is not selected",
            "nested": "the upper level has no options for it",
            "parent": "the field it belongs to is hidden",
        }.get(self.source, self.source)
        return f"{self.label}: {state} ({why})"


@dataclass(frozen=True)
class ExcludeName:
    base: str
    custom: bool

    def name(self, account_id: str | None) -> str:
        if not self.custom:
            return self.base
        if not account_id:
            raise ValueError(f"custom field {self.base!r} needs the account id")
        return f"{self.base}_{account_id}"


EXCLUDE_KEYS = ("optional", "disabled", "hidden")


@dataclass(frozen=True)
class ChoiceRestriction:
    """The choices a ``populate`` action leaves on offer for one dropdown.

    ``allowed`` maps each offered choice (normalised choice value) to the
    restriction of its nested options: ``None`` means all of them. The
    empty choice (``--``) is always offered; the field's value is not set.
    """

    allowed: Mapping[str, "ChoiceRestriction | None"]
    rule_id: Any = None
    rule_name: str = ""

    def offers(self, value: Any) -> bool:
        return norm(value) in self.allowed

    def below(self, value: Any) -> "ChoiceRestriction | None":
        return self.allowed.get(norm(value))


@dataclass
class EvaluatedForm:
    visible: list[str]  # ordered, inputs and notices
    required: set[str]  # visible and required
    disabled: set[str]
    values: dict[str, Any]  # effective values (user + populated)
    populated: dict[str, Any]  # api name -> id of the rule that populated it
    messages: list[Message] = dc_field(default_factory=list)
    matched: list[Any] = dc_field(default_factory=list)  # ids of matching rules
    model: str = "replay"
    # fields rules unmandated / disabled / hid, in the order the actions ran
    excludes: dict[str, list[ExcludeName]] = dc_field(default_factory=lambda: {k: [] for k in EXCLUDE_KEYS})
    decisions: dict[tuple[str, str], Decision] = dc_field(default_factory=dict)
    conflicts: list[Conflict] = dc_field(default_factory=list)
    # user answers refused because a rule disabled the field: (field, decision)
    refused: list[tuple[str, Decision | None]] = dc_field(default_factory=list)
    # user answers a later populate replaced: field -> (user value, rule id)
    overridden: dict[str, tuple[Any, Any]] = dc_field(default_factory=dict)
    attachments_required: bool | None = None  # rule override for item attachments
    account_id: str | None = None
    spec: FormSpec | None = dc_field(default=None, repr=False, compare=False)
    # populate="restrict": field (top level) -> choices still on offer, and
    # the populate decision in effect (also kept for restores)
    restricted: dict[str, ChoiceRestriction] = dc_field(default_factory=dict)
    restricted_by: dict[str, Decision] = dc_field(default_factory=dict)
    # visible fields whose value is not on offer any more: field -> (value, decision)
    unavailable: dict[str, tuple[Any, Decision | None]] = dc_field(default_factory=dict)
    populate_mode: str = DEFAULT_POPULATE

    def __post_init__(self) -> None:
        self._vis = set(self.visible)

    def choices(self, name: str) -> tuple[Choice, ...]:
        """Choices the dropdown offers now: nested levels follow the upper
        levels, and a ``populate`` restriction keeps only its choices."""
        if self.spec is None:
            raise ValueError("choices() needs the FormSpec (evaluate through a RulesEngine)")
        return offered_choices(self.spec, self.spec.field(name), self.values, self.restricted)

    def restriction_for(self, name: str) -> tuple[ChoiceRestriction | None, Decision | None]:
        """The restriction that applies to ``name`` (its top-level field's)
        and the decision that set it."""
        if self.spec is None:
            return None, None
        top = _nested_chain(self.spec, self.spec.field(name))[0].api_name
        return self.restricted.get(top), self.restricted_by.get(top)

    def is_visible(self, name: str) -> bool:
        return name in self._vis

    def is_required(self, name: str) -> bool:
        return name in self.required

    @property
    def blocking_messages(self) -> list[Message]:
        """Messages of ``is_validation`` rules: the web blocks submit until
        they are dealt with, so fsv requires an explicit acknowledgement."""
        return [m for m in self.messages if m.blocking]

    def decision(self, name: str, prop: str) -> Decision | None:
        return self.decisions.get((name, prop))

    def explain(self, names: Iterable[str] | None = None) -> list[Explanation]:
        """Why each field is (not) visible, required and disabled."""
        spec = self.spec
        if spec is None:
            raise ValueError("explain() needs the FormSpec (evaluate through a RulesEngine)")
        wanted = list(names) if names is not None else [f.api_name for f in spec.ordered()]
        out: list[Explanation] = []
        for n in wanted:
            f = spec.field(n)
            n = f.api_name
            vis = self.is_visible(n)
            d = self.decisions.get((n, "visible"))
            src = "rule" if d is not None else "metadata"
            if not vis and (d is None or d.action == "show"):
                # visible by rules/metadata but gated by structure
                if f.parent and not self.is_visible(f.parent):
                    src, d = "parent", None
                elif spec.section_parent(n) is not None:
                    src, d = "section", None
                elif f.is_nested_level:
                    src, d = "nested", None
            out.append(Explanation(n, f.label, "visible", vis, src, d))
            if vis and f.kind != "notice":
                d = self.decisions.get((n, "required"))
                out.append(Explanation(n, f.label, "required", n in self.required, "rule" if d else "metadata", d))
            d = self.decisions.get((n, "disabled"))
            if d is not None or n in self.disabled:
                out.append(Explanation(n, f.label, "disabled", n in self.disabled, "rule" if d else "metadata", d))
            r, d = self.restriction_for(n)
            if d is not None and f.kind in CHOICE_KINDS:
                out.append(Explanation(n, f.label, "choices restricted", r is not None, "rule", d))
        return out


# ---------------------------------------------------------------------------
# engine


class RulesEngine:
    """Evaluates business rules for one :class:`FormSpec`.

    ``strict`` (default True) raises :class:`UnsupportedRuleError` (a
    ``FormRuleError``) for a rule with an unknown operator, action or
    ``evaluate_on``; ``strict=False`` is the lenient mode that skips such
    rules and lists them in ``unsupported``.
    """

    def __init__(
        self,
        spec: FormSpec,
        rules: Any = None,
        *,
        module: str | None = None,
        user_type: str = "agent",
        item_id: Any = None,
        workspace_id: Any = None,
        user: Mapping[str, Any] | None = None,
        context: RuleContext | None = None,
        dedupe: str = "exact",
        strict: bool = True,
        model: str | None = None,
        same_value_fires: bool = False,
        event_reverse: str | None = None,
        populate: str | None = None,
        system: Mapping[str, Any] | None = None,
        event_scope: str | None = None,
    ) -> None:
        if dedupe not in DEDUPE_POLICIES:
            raise ValueError(f"dedupe must be one of {DEDUPE_POLICIES}")
        model = model or DEFAULT_MODEL
        if model not in MODELS:
            raise ValueError(f"model must be one of {MODELS}")
        raw_user = rules.get("user") if isinstance(rules, Mapping) else None
        if context is None:
            context = RuleContext(
                module=module,
                user_type=user_type,
                item_id=item_id,
                workspace_id=workspace_id,
                user=user if user is not None else raw_user,
                system=system,
            )
        elif system is not None and context.system is None:
            context = replace(context, system=system)
        populate = populate or DEFAULT_POPULATE
        if populate not in POPULATE_MODES:
            raise ValueError(f"populate must be one of {POPULATE_MODES}")
        self.populate = populate
        self.context = context
        self.spec = spec
        self.module = context.module
        self.user_type = context.user_type
        self.item_id = context.item_id
        self.user = context.user
        self.dedupe = dedupe
        self.strict = strict
        self.model = model
        self.same_value_fires = same_value_fires
        event_reverse = event_reverse or DEFAULT_EVENT_REVERSE
        event_scope = event_scope or DEFAULT_EVENT_SCOPE
        if event_reverse not in ("transition", "always"):
            raise ValueError("event_reverse must be 'transition' or 'always'")
        self.event_reverse = event_reverse
        if event_scope not in EVENT_SCOPES:
            raise ValueError(f"event_scope must be one of {EVENT_SCOPES}")
        self.event_scope = event_scope
        self.warnings: list[str] = []
        self.unsupported: list[str] = []
        self.skipped: list[str] = []
        self.duplicates: list[Any] = []
        self._entity: Mapping[str, Any] | None = context.entity
        self.rules: list[Rule] = self._load(rules)
        order = spec.ordered()
        n = len(order)
        self._pos: dict[str, float] = {f.api_name: i for i, f in enumerate(order)}
        for f in spec.fields:
            self._pos.setdefault(f.api_name, n + (f.position or 0))

    # -- loading ----------------------------------------------------------
    def _load(self, data: Any) -> list[Rule]:
        if isinstance(data, Mapping):
            raw = data.get("business_rules", data.get("business_rule")) or []
        else:
            raw = list(data or [])
        listing = [r for r in raw if "condition_data" not in r and "action_data" not in r]
        if listing:
            raise FormRuleError(
                f"{len(listing)} business rule(s) have no conditions or actions: this is the rules listing, "
                "not the form's rules. Fetch them with fsv.forms.business_rules_query(...) (ember_api=true)."
            )
        kept: list[Mapping[str, Any]] = []
        for r in raw:
            why = self._filter(r)
            if why:
                self.skipped.append(f"{r.get('name')!s}: {why}")
                continue
            kept.append(r)
        kept = self._dedupe(kept)
        out: list[Rule] = []
        for r in kept:
            rule = self._parse_rule(r, len(out))
            if rule is not None:
                out.append(rule)
        return out

    def _filter(self, r: Mapping[str, Any]) -> str | None:
        ctx = self.context
        if r.get("deleted"):
            return "deleted"
        if str(r.get("state", "active")) != "active":
            return f"state {r.get('state')}"
        if str(r.get("rule_type", "client_side")) != "client_side":
            return f"rule_type {r.get('rule_type')} (server-side rules are not evaluated)"
        if ctx.module and r.get("module_type") not in (None, ctx.module):
            return f"module {r.get('module_type')}"
        if r.get("item_id") not in (None, "") and str(r.get("item_id")) != str(ctx.item_id):
            return f"item {r.get('item_id')}"  # rules of one catalog item apply to that item only
        ws = ctx.workspaces()
        if ws is not None and r.get("workspace_id") is not None and str(r.get("workspace_id")) not in ws:
            return f"workspace {r.get('workspace_id')}"
        return None

    def _dedupe(self, rules: list[Mapping[str, Any]]) -> list[Mapping[str, Any]]:
        seen: dict[Any, int] = {}
        for i, r in enumerate(rules):
            rid = r.get("id")
            if rid is None:
                continue
            if rid in seen:
                self.duplicates.append(rid)
            seen.setdefault(rid, i)
        if self.dedupe == "none" or not self.duplicates:
            return rules
        out: list[Mapping[str, Any]] = []
        if self.dedupe == "exact":
            for r in rules:
                if r.get("id") is not None and any(o.get("id") == r.get("id") and o == r for o in out):
                    continue
                out.append(r)
            return out
        position: dict[Any, int] = {}
        for r in rules:
            rid = r.get("id")
            if rid is None or rid not in position:
                if rid is not None:
                    position[rid] = len(out)
                out.append(r)
            elif self.dedupe == "last":
                out[position[rid]] = r  # keep first position, last content
        return out

    def _parse_rule(self, r: Mapping[str, Any], index: int) -> Rule | None:
        name = str(r.get("name") or r.get("id")).strip()
        try:
            raw_cond = r.get("condition_data")
            cond = self._parse_group(raw_cond or {}, name)
            actions = self._parse_actions((r.get("action_data") or {}).get("action") or [], name)
        except _Skip as e:
            msg = (
                f"form has a rule fsv doesn't understand: {name!r} (id {r.get('id')}): {e}. "
                "Fill this form in the web UI, or evaluate with strict=False to skip the rule."
            )
            if self.strict:
                raise UnsupportedRuleError(msg, r.get("id"), name) from None
            self.unsupported.append(msg)
            self.warnings.append(msg)
            return None
        return Rule(
            id=r.get("id"),
            name=name,
            condition=cond,
            actions=actions,
            reversible=bool(r.get("is_reversible")),
            form_type=str(r.get("form_type") or "both"),
            user_type=str(r.get("user_type") or "both"),
            is_validation=bool(r.get("is_validation")),
            index=index,
            fields=frozenset(_condition_fields(cond)),
            unconditional=not cond.items,
        )

    def _parse_group(self, data: Mapping[str, Any], rule: str) -> ConditionGroup:
        if not isinstance(data, Mapping) or not data:
            return ConditionGroup("all", ())
        modes = [k for k in ("all", "any") if k in data]
        if len(modes) != 1:
            raise _Skip(f"condition must have exactly one of all/any, got {sorted(data)}")
        mode = modes[0]
        items: list[Condition | ConditionGroup] = []
        for c in data.get(mode) or []:  # "all": null / "any": null means "always"
            if isinstance(c, Mapping) and ("all" in c or "any" in c) and "operator" not in c:
                items.append(self._parse_group(c, rule))
                continue
            op = str(c.get("operator") or "")
            if op not in SUPPORTED_OPERATORS:
                raise _Skip(f"unsupported operator {op!r}")
            ev_on = c.get("evaluate_on")
            ev_key = "" if ev_on is None else str(ev_on)
            if ev_key and ev_key not in SUPPORTED_EVALUATE_ON:
                raise _Skip(f"unsupported evaluate_on {ev_on!r}")
            raw_name = str(c.get("name") or "")
            val = c.get("value")
            vals = tuple(str(v) for v in val) if isinstance(val, (list, tuple)) else (() if val is None else (str(val),))
            if ev_key in ENTITY_SOURCES:
                items.append(Condition(raw_name, self.spec.resolve_name(raw_name), op, vals, ENTITY_SOURCES[ev_key]))
                continue
            if ev_key in CONTEXT_SOURCES:
                attr = raw_name.split(".")[-1]
                items.append(Condition(raw_name, attr, op, vals, CONTEXT_SOURCES[ev_key]))
                continue
            api = self.spec.resolve_name(raw_name)
            if api is None:
                self.warnings.append(f"rule {rule!r}: condition on unknown field {raw_name!r}")
            items.append(Condition(raw_name, api, op, vals, "form"))
        return ConditionGroup(mode, tuple(items))

    def _target(self, raw: str) -> Target:
        api = self.spec.resolve_name(raw)
        stripped, had_prefix = raw, False
        for p in CUSTOM_PREFIXES:
            if raw.startswith(p):
                stripped, had_prefix = raw[len(p):], True
                break
        if api is not None:
            f = self.spec.by_name[api]
            if f.is_custom:
                return Target(api, raw, api, True)
            return Target(api, raw, stripped, False)
        return Target(None, raw, stripped, had_prefix)

    def _parse_actions(self, data: Iterable[Mapping[str, Any]], rule: str) -> tuple[Action, ...]:
        out: list[Action] = []
        for a in data:
            prop = str(a.get("property") or "")
            if prop not in SUPPORTED_ACTIONS:
                raise _Skip(f"unsupported action {prop!r}")
            names = a.get("name")
            names = [names] if isinstance(names, str) else list(names or [])
            vals = a.get("values", a.get("value"))
            vals = tuple(vals) if isinstance(vals, (list, tuple)) else (() if vals is None else (vals,))
            if prop == "message":
                out.append(Action(prop, tuple(str(n) for n in names)))
                continue
            targets: list[Target] = []
            for n in names:  # an empty name list is a no-op
                t = self._target(str(n))
                if t.api is None and t.base not in PSEUDO_TARGETS:
                    self.warnings.append(f"rule {rule!r}: {prop} on unknown field {n!r} ignored")
                targets.append(t)
            out.append(Action(prop, tuple(t.api for t in targets if t.api), vals, tuple(targets)))
        return tuple(out)

    # -- evaluation -------------------------------------------------------
    def rules_for(self, mode: str = "new") -> list[Rule]:
        return [r for r in self.rules if r.applies_to(mode, self.user_type)]

    def unavailable_dependencies(self, mode: str = "new") -> list[str]:
        """Unresolved fields or context needed by applicable rules.

        Inspect all branches, not just those reached by boolean evaluation.
        A present key with an empty value is known; a missing key is not.
        Callers preparing writes should treat any result as a blocker.
        """
        problems: list[str] = []

        def add(message: str) -> None:
            if message not in problems:
                problems.append(message)

        def inspect(group: ConditionGroup, rule: Rule) -> None:
            for condition in group.items:
                if isinstance(condition, ConditionGroup):
                    inspect(condition, rule)
                elif condition.source == "form":
                    if condition.field is None:
                        add(f"rule {rule.name!r}: unknown condition field {condition.name!r}")
                elif condition.source == "entity":
                    if condition.field is None and self._entity_lookup(condition)[0] is False:
                        add(f"rule {rule.name!r}: unavailable record (current_entity) field {condition.name!r}")
                else:
                    values = getattr(self.context, condition.source, None)
                    if values is None or condition.field not in values:
                        add(f"rule {rule.name!r}: unavailable {condition.source} context field {condition.field!r}")

        for rule in self.rules_for(mode):
            inspect(rule.condition, rule)
            for action in rule.actions:
                for target in action.all_targets:
                    if target.api is None and target.base not in PSEUDO_TARGETS:
                        add(f"rule {rule.name!r}: unknown action field {target.raw!r}")
        return problems

    def _condition_gaps(self, group: ConditionGroup, mode: str) -> list[str]:
        """Condition dependencies of one rule that cannot be evaluated."""
        out: list[str] = []
        for c in group.items:
            if isinstance(c, ConditionGroup):
                out.extend(self._condition_gaps(c, mode))
            elif c.source == "form":
                if c.field is None and not (mode == "new" and c.name.split(".")[-1] in NEW_RECORD_EMPTY):
                    out.append(f"field {c.name!r}")
            elif c.source == "entity":
                if c.field is None and self._entity_lookup(c)[0] is False:
                    out.append(f"current_entity {c.name!r}")
            else:
                values = getattr(self.context, c.source, None)
                if values is None or c.field not in values:
                    out.append(f"{c.source} {c.field!r}")
        return out

    def write_gate(self, mode: str, evaluated: EvaluatedForm, user_fields: Iterable[str] = (),
                   sent_keys: Iterable[str] = ()) -> RuleGate:
        """Decide which rule gaps block a write and which only warn.

        A rule whose condition cannot be evaluated is *unknown*. It blocks
        only when it could hide, disable or require a field the user sets
        or that is required (or could require a field nobody fills);
        otherwise it is a warning naming the rule id. An action on a
        target that is not a form field (a UI widget such as tags or the
        associated-items panel) blocks when it could hide something the
        write sends (``sent_keys``) or require something fsv cannot fill.
        User answers a rule hid are refused, never dropped silently.
        Writes must treat ``blockers`` as fatal and show ``warnings``.
        """
        user = set(user_fields)
        sent = set(sent_keys)
        required = set(evaluated.required) | {f.api_name for f in self.spec.fields if f.required_base}
        filled = {k for k, v in evaluated.values.items() if not is_empty(v)}
        gate = RuleGate()
        for rule in self.rules_for(mode):
            label = f"rule {rule.id} {rule.name!r}"
            gaps = self._condition_gaps(rule.condition, mode)
            could_match = bool(gaps) or self._match_group(rule.condition, evaluated.values)
            known: set[str] = set()
            mandates: set[str] = set()
            for action in rule.actions:
                if action.property in ("message", "populate"):
                    continue
                props = {action.property}
                if rule.reversible:
                    props.add(_REVERSE.get(action.property, action.property))
                for target in action.all_targets:
                    if target.api is not None:
                        known.add(target.api)
                        if "mandate" in props:
                            mandates.add(target.api)
                        continue
                    if target.base in PSEUDO_TARGETS:
                        continue
                    aliases = {target.base, target.raw} | WIDGET_PAYLOAD_KEYS.get(target.base, set())
                    if not could_match:
                        continue
                    if "mandate" in props:
                        gate.blockers.append(f"{label}: could require {target.raw!r}, which is not a form field fsv "
                                            "fills (unavailable fields or context)")
                    elif props & {"hide", "disable"} and aliases & sent:
                        gate.blockers.append(f"{label}: could {action.property} {target.raw!r}, which this write sends")
                    else:
                        gate.warnings.append(f"{label}: {action.property} on {target.raw!r} (not a form field; "
                                             "nothing fsv sends) ignored")
            if gaps:
                # fields this write sends (answers and defaults) or must fill
                risky = sorted(known & (user | required | filled)) + sorted(mandates - filled - user - required)
                what = ", ".join(dict.fromkeys(gaps))
                if risky:
                    names = ", ".join(self.spec.by_name[n].label for n in dict.fromkeys(risky))
                    gate.blockers.append(f"{label}: unavailable fields or context ({what}); it could change {names}")
                else:
                    gate.warnings.append(f"{label}: unavailable fields or context ({what}); it touches no field "
                                         "this write sends or requires")
                gate.unknown_rules.append(rule.id)
        for name in sorted(user, key=lambda n: self._pos.get(n, 1 << 30)):
            if name in evaluated._vis or is_empty(evaluated.values.get(name)):
                continue
            f = self.spec.by_name.get(name)
            if f is None or f.kind == "notice":
                continue
            d = evaluated.decision(name, "visible")
            if d is not None:
                why = f"rule {d.rule_id} {d.rule_name!r}"
            else:
                sec = self.spec.section_parent(name)
                parent = sec[0] if sec is not None else self.spec.by_name.get(f.parent or "")
                why = (f"the form: its section/parent field {parent.label!r} is hidden or not on the right choice"
                       if parent is not None else "the form metadata")
            gate.blockers.append(f"{f.label}: hidden by {why}; refusing to drop the value you passed")
        gate.blockers = list(dict.fromkeys(gate.blockers))
        gate.warnings = list(dict.fromkeys(gate.warnings))
        return gate

    def position_order(self, names: Iterable[str]) -> list[str]:
        """Field names sorted in web form order (the replay contract)."""
        return sorted(names, key=lambda n: (self._pos.get(n, 1 << 30), n))

    def evaluate(
        self,
        values: Mapping[str, Any],
        mode: str = "new",
        *,
        base: Mapping[str, Any] | None = None,
        order: Sequence[str] | None = None,
        model: str | None = None,
        events: Sequence[tuple[str, Any]] | None = None,
    ) -> EvaluatedForm:
        """Evaluate the rules.

        ``base`` are the load values (defaults, or the record being edited);
        ``values`` the user's answers (API names). In the event model the
        answers are replayed as change events in ``order`` (default: web
        field order); names missing from ``order`` follow in field order.
        ``events`` replaces that with an explicit click history
        (``[(field, value), ...]``, repeats allowed); the replay model only
        uses its final values.
        """
        model = model or self.model
        if model not in MODELS:
            raise ValueError(f"model must be one of {MODELS}")
        rules = self.rules_for(mode)
        base = {k: v for k, v in (base or {}).items() if not is_empty(v)}
        if self.context.entity is None:
            # current_entity reads the record as loaded, never the live answers
            self._entity = dict(base) if mode == "edit" else {}
        if model == "replay":
            user = dict(base)
            user.update(values)  # an empty answer clears a load value
            user = {k: v for k, v in user.items() if not is_empty(v)}
            st = self._evaluate_replay(rules, user, {k for k, v in values.items() if not is_empty(v)})
        elif events is not None:
            st = self._evaluate_events(rules, base, list(events))
        else:
            seq = self._event_order(values, order)
            st = self._evaluate_events(rules, base, [(n, values.get(n)) for n in seq])
        return self._finish(st, model)

    def _event_order(self, values: Mapping[str, Any], order: Sequence[str] | None) -> list[str]:
        if order is None:
            return self.position_order(values)
        seen: list[str] = []
        for n in order:
            if n in values and n not in seen:
                seen.append(n)
        return seen + self.position_order(n for n in values if n not in seen)

    # replay model -------------------------------------------------------
    def _evaluate_replay(self, rules: Sequence[Rule], user: Mapping[str, Any], user_only: set[str]) -> "_State":
        user = {k: v for k, v in user.items() if not is_empty(v)}
        populated: dict[str, Any] = {}
        st: _State | None = None
        for _ in range(MAX_PASSES):
            eff = dict(user)
            for k, v in populated.items():
                eff.setdefault(k, v)
            st = self._fresh(eff)
            for rule in rules:
                self._apply(rule, self._match_group(rule.condition, st.values), st, None, fixed=user)
            new_pop = {k: st.values[k] for k in st.populated}
            if new_pop == populated:
                break
            populated = new_pop
        else:  # pragma: no cover - defensive
            self.warnings.append("populate rules did not settle; result may be unstable")
        assert st is not None
        for n in user_only:
            if n in st.dis and n in user:
                st.refused.append((n, st.why.get((n, "disabled"))))
        return st

    # event model --------------------------------------------------------
    def _evaluate_events(
        self, rules: Sequence[Rule], base: Mapping[str, Any], events: Sequence[tuple[str, Any]]
    ) -> "_State":
        st = self._fresh(dict(base))
        for rule in rules:  # load
            self._apply(rule, self._match_group(rule.condition, st.values), st, None, fixed=None)
        user_set: set[str] = set()
        for name, value in events:
            if name in st.dis:
                st.refused.append((name, st.why.get((name, "disabled"))))
                continue
            old = st.values.get(name)
            same = _same(old, value)
            if is_empty(value):
                st.values.pop(name, None)
            else:
                st.values[name] = value
            user_set.add(name)
            st.overridden.pop(name, None)
            if same and not self.same_value_fires:
                continue  # selecting the current value fires no change event
            st.populated.pop(name, None)
            changed = {name} | self._clear_lower_levels(name, st)
            for rule in rules:
                if self.event_scope == "referenced" and not rule.fields & changed:
                    continue
                ok = self._match_group(rule.condition, st.values)
                if not ok and self.event_reverse == "transition" and rule.index not in st.last_match:
                    continue  # did not match before either: nothing to undo
                self._apply(rule, ok, st, name, fixed=None, user_set=user_set)
        return st

    def _clear_lower_levels(self, name: str, st: "_State") -> set[str]:
        """A nested parent changed: its lower levels are cleared (web behaviour)."""
        cleared: set[str] = set()
        f = self.spec.by_name.get(name)
        while f is not None and f.kind == "nested":
            child = next((c for c in self.spec.fields if c.parent == f.api_name and c.is_nested_level), None)
            if child is None:
                break
            if child.api_name in st.values:
                st.values.pop(child.api_name)
                st.populated.pop(child.api_name, None)
                cleared.add(child.api_name)
            f = child
        return cleared

    # shared -------------------------------------------------------------
    def _fresh(self, values: dict[str, Any]) -> "_State":
        return _State(
            vis={n: f.visible_base for n, f in self.spec.by_name.items()},
            req={n: f.required_base for n, f in self.spec.by_name.items()},
            values=values,
        )

    def _apply(
        self,
        rule: Rule,
        ok: bool,
        st: "_State",
        event: str | None,
        *,
        fixed: Mapping[str, Any] | None,
        user_set: set[str] | None = None,
    ) -> None:
        """Apply ``rule`` (matched or not) to the state.

        ``fixed``: replay model, values populate must not overwrite.
        ``user_set``: event model, user answers so far (overwrites recorded).
        """
        if ok:
            st.last_match[rule.index] = rule.id
        else:
            st.last_match.pop(rule.index, None)
        has_msg = any(a.property == "message" for a in rule.actions)
        if has_msg:
            st.messages.pop(rule.index, None)
        for a in rule.actions:
            prop = a.property if ok else (_REVERSE.get(a.property) if rule.reversible else None)
            if prop is None:
                continue
            if self.populate == "restrict" and a.property == "populate":
                self._restrict(rule, a, ok, st, event)
                continue
            if prop == "restore":
                continue  # populate="set": a reverse leaves the value alone
            if prop == "message":
                st.messages[rule.index] = [Message(t, rule.id, rule.name, rule.is_validation) for t in a.targets]
                continue
            if prop == "populate":
                for t in a.all_targets:
                    if t.api is None:
                        continue
                    if fixed is not None and t.api in fixed:
                        continue  # replay model: never overwrite user input
                    v = self._populate_value(self.spec.by_name[t.api], a.values, rule)
                    if v is None:
                        continue
                    if user_set is not None and t.api in user_set and not _same(st.values.get(t.api), v):
                        st.overridden[t.api] = (st.values.get(t.api), rule.id)
                    st.values[t.api] = v
                    st.populated[t.api] = rule.id
                continue
            d = Decision(rule.id, rule.name, prop, ok, event, rule.index)
            for t in a.all_targets:
                if t.api is None:
                    if t.base in PSEUDO_TARGETS and prop in ("mandate", "unmandate"):
                        st.pseudo_req[t.base] = prop == "mandate"
                        st.touch(prop, t)
                    continue  # fields not on the form are not tracked
                st.touch(prop, t)
                n = t.api
                if prop == "show":
                    st.vis[n] = True
                elif prop == "hide":
                    st.vis[n] = False
                elif prop == "mandate":
                    st.req[n] = True
                elif prop == "unmandate":
                    st.req[n] = False
                elif prop == "disable":
                    st.dis.add(n)
                elif prop == "enable":
                    st.dis.discard(n)
                kind = _KIND[prop]
                st.why[(n, kind)] = d
                if ok:
                    st.matched_decisions.setdefault((n, kind), []).append(d)

    def _restrict(self, rule: Rule, a: Action, ok: bool, st: "_State", event: str | None) -> None:
        """populate="restrict": a matching rule limits the choices; a
        reversible rule that does not match restores them."""
        d = Decision(rule.id, rule.name, "populate" if ok else "restore", ok, event, rule.index)
        for t in a.all_targets:
            if t.api is None:
                continue
            f = self.spec.by_name[t.api]
            if not ok:
                st.restricted.pop(t.api, None)
                st.restricted_by[t.api] = d
                continue
            r = self._restriction(f, a.values, rule)
            if r is not None:
                st.restricted[t.api] = r
                st.restricted_by[t.api] = d

    def _restriction(self, f: Field, values: Sequence[Any], rule: Rule) -> ChoiceRestriction | None:
        if f.kind not in CHOICE_KINDS:
            self.warnings.append(f"rule {rule.name!r}: populate on {f.api_name!r}, which has no choices, ignored")
            return None

        def build(pool: Sequence[Choice], vals: Sequence[Any]) -> dict[str, ChoiceRestriction | None]:
            out: dict[str, ChoiceRestriction | None] = {}
            for raw in vals:
                kids = None
                if isinstance(raw, Mapping):
                    kids = raw.get("nested_options")
                    raw = raw.get("display_id", raw.get("id", raw.get("value")))
                ch = _choice_in(pool, raw)
                if ch is None:
                    self.warnings.append(
                        f"rule {rule.name!r}: populate value {raw!r} is not a choice of {f.api_name!r}; ignored")
                    continue
                # null / empty nested_options: every nested option stays on offer
                out[norm(ch.value)] = ChoiceRestriction(build(ch.children, kids), rule.id, rule.name) if kids else None
            return out

        allowed = build(f.choices, values)
        if not allowed:
            if values:
                self.warnings.append(f"rule {rule.name!r}: populate on {f.api_name!r} offers no known choice; ignored")
            return None
        return ChoiceRestriction(allowed, rule.id, rule.name)

    def _finish(self, st: "_State", model: str) -> EvaluatedForm:
        visible: list[str] = []
        shown: set[str] = set()
        values = st.values
        for f in self.spec.ordered():
            if not st.vis.get(f.api_name, False):
                continue
            if f.parent:
                if f.parent not in shown:
                    continue
                sec = self.spec.section_parent(f.api_name)
                if sec is not None:
                    parent, choices = sec
                    cur = values.get(parent.api_name)
                    cur_items = cur if isinstance(cur, (list, tuple)) else ([] if is_empty(cur) else [cur])
                    if not any(norm(x) in choices for x in cur_items):
                        continue
                if f.is_nested_level and not self.spec.choices_for(f, values):
                    continue
            visible.append(f.api_name)
            shown.add(f.api_name)
        required = {n for n in visible if st.req.get(n) and self.spec.by_name[n].kind != "notice"}
        conflicts: list[Conflict] = []
        for (n, kind), ds in st.matched_decisions.items():
            polarities = {d.action in _POSITIVE for d in ds}
            if len(polarities) > 1:
                uniq: list[Decision] = []
                for d in ds:
                    if all((u.rule_id, u.action) != (d.rule_id, d.action) for u in uniq):
                        uniq.append(d)
                conflicts.append(Conflict(n, kind, tuple(uniq), st.why[(n, kind)]))
        messages = [m for i in sorted(st.messages) for m in st.messages[i]]
        excludes = self._excludes(st)
        unavailable: dict[str, tuple[Any, Decision | None]] = {}
        if st.restricted:
            for n in visible:
                f = self.spec.by_name[n]
                v = values.get(n)
                if f.kind not in CHOICE_KINDS or is_empty(v):
                    continue
                top = _nested_chain(self.spec, f)[0].api_name
                if top not in st.restricted:
                    continue
                pool = offered_choices(self.spec, f, values, st.restricted)
                items = v if isinstance(v, (list, tuple)) else [v]
                if any(f.choice(x, pool) is None for x in items):
                    unavailable[n] = (v, st.restricted_by.get(top))
        return EvaluatedForm(
            visible=visible,
            required=required,
            disabled=set(st.dis),
            values=dict(values),
            populated=dict(st.populated),
            messages=messages,
            matched=[st.last_match[i] for i in sorted(st.last_match)],
            model=model,
            excludes=excludes,
            decisions=dict(st.why),
            conflicts=conflicts,
            refused=list(st.refused),
            overridden=dict(st.overridden),
            attachments_required=st.pseudo_req.get("attachment_ids"),
            account_id=self.spec.account_id,
            spec=self.spec,
            restricted=dict(st.restricted),
            restricted_by=dict(st.restricted_by),
            unavailable=unavailable,
            populate_mode=self.populate,
        )

    def _excludes(self, st: "_State") -> dict[str, list[ExcludeName]]:
        """``br_validation_excludes`` lists. Order: first time a rule action
        (or its reverse) touched the field's property. Membership: the final
        state. ``optional`` holds only statically required fields that the
        rules leave not required (those are the server checks to skip)."""
        item = self.spec.item
        hidden = [x for n, x in st.reg["visible"].items() if not st.vis.get(n, True)]
        optional = []
        for n, x in st.reg["required"].items():
            if n in PSEUDO_TARGETS:
                static = bool(item and item.attachment_mandatory)
                now = st.pseudo_req.get(n, static)
            else:
                static = self.spec.by_name[n].required_base
                now = bool(st.req.get(n))
            if static and not now:
                optional.append(x)
        disabled = [x for n, x in st.reg["disabled"].items() if n in st.dis]
        # A custom nested field a rule hid takes its lower levels with it: the
        # web lists them after the rule actions, as hidden and as optional.
        names_h = {(x.base, x.custom) for x in hidden}
        names_o = {(x.base, x.custom) for x in optional}
        for n in list(st.reg["visible"]):
            f = self.spec.by_name.get(n)
            if f is None or st.vis.get(n, True) or f.kind != "nested" or not f.is_custom or f.is_nested_level:
                continue
            for lvl in f.levels:
                lf = self.spec.by_name.get(lvl)
                if lf is None or not lf.on_form:
                    continue
                key = (lvl, True)
                if key not in names_h:
                    hidden.append(ExcludeName(lvl, True))
                    names_h.add(key)
                if key not in names_o and not st.req.get(lvl):
                    optional.append(ExcludeName(lvl, True))
                    names_o.add(key)
        return {"optional": optional, "disabled": disabled, "hidden": hidden}

    # -- matching ---------------------------------------------------------
    def _match_group(self, g: ConditionGroup, eff: Mapping[str, Any]) -> bool:
        if not g.items:
            return True  # no conditions ("all": null): always matches
        results = (self._match_group(c, eff) if isinstance(c, ConditionGroup) else self._match(c, eff) for c in g.items)
        return all(results) if g.mode == "all" else any(results)

    def _person(self, source: str) -> Mapping[str, Any] | None:
        person = getattr(self.context, source, None)
        if person is None:
            what = "system values (RuleContext.system)" if source == "system" else f"the {source} record"
            msg = f"rules read {what} but none was given; their conditions see empty values"
            if msg not in self.warnings:
                self.warnings.append(msg)
        return person

    def _entity_lookup(self, c: Condition) -> tuple[bool, Any]:
        """(found, value) of a ``current_entity`` condition in the record."""
        entity = self._entity or {}
        names = [c.field, c.name, c.name.split(".")[-1]]
        for n in names:
            if n and n in entity:
                return True, entity[n]
        return False, None

    def _match(self, c: Condition, eff: Mapping[str, Any]) -> bool:
        if c.source == "entity":
            if self._entity is None:
                msg = "rules read the saved record (current_entity) but none was given; their conditions see empty values"
                if msg not in self.warnings:
                    self.warnings.append(msg)
            v = self._entity_lookup(c)[1]
            f = self.spec.by_name.get(c.field) if c.field else None
        elif c.source != "form":
            person = self._person(c.source) or {}
            v = person.get(c.field or "")
            f = None
        else:
            f = self.spec.by_name.get(c.field) if c.field else None
            v = eff.get(c.field) if c.field else None
        if c.operator in ("selected", "not_selected"):
            if f is not None and f.kind == "checkbox":
                sel = v is True
            else:
                sel = not is_empty(v) and v is not False
            return sel if c.operator == "selected" else not sel
        wanted = {norm(x) for x in c.values}
        # single value: is/includes = value in list; multi-select: any-of
        hit = any(keys & wanted for keys in self._value_keys(f, v, eff))
        if c.operator in ("is", "includes"):
            return hit
        return not hit  # is_not / does_not_include: none-of (an empty field matches)

    def _value_keys(self, f: Field | None, v: Any, eff: Mapping[str, Any]) -> list[set[str]]:
        if is_empty(v):
            return []
        items = list(v) if isinstance(v, (list, tuple, set, frozenset)) else [v]
        out: list[set[str]] = []
        pool = self.spec.choices_for(f, eff) if f is not None else ()
        for item in items:
            if isinstance(item, bool):
                out.append({"true", "yes", "1"} if item else {"false", "no", "0"})
                continue
            keys = {norm(item)}
            for attr in ("id", "display", "email"):
                if hasattr(item, attr) and getattr(item, attr) not in (None, ""):
                    keys.add(norm(getattr(item, attr)))
            if f is not None and pool:
                ch = f.choice(item, pool)
                if ch is not None:
                    keys |= ch.keys()
            out.append(keys)
        return out

    def _populate_value(self, f: Field, values: Sequence[Any], rule: Rule) -> Any:
        if not values:
            return None
        if f.kind in ("dropdown", "department", "nested", "multi_select"):
            picked: list[Choice] = []
            for raw in values:
                ch = _choice_by_any(f, raw)
                if ch is None:
                    self.warnings.append(f"rule {rule.name!r}: populate value {raw!r} is not a choice of {f.api_name!r}")
                    return None
                picked.append(ch)
            if f.kind == "multi_select":
                return tuple(c.value for c in picked)
            return picked[0].value
        if f.kind == "checkbox":
            return str(values[0]).lower() in ("true", "1", "yes")
        return values[0] if len(values) == 1 else tuple(values)


def _condition_fields(g: ConditionGroup) -> set[str]:
    out: set[str] = set()
    for c in g.items:
        if isinstance(c, ConditionGroup):
            out |= _condition_fields(c)
        elif c.source == "form" and c.field:
            out.add(c.field)
    return out


def _nested_chain(spec: FormSpec, f: Field) -> list[Field]:
    """[top field, level 2, ...] down to ``f`` (just ``[f]`` if not nested)."""
    chain = [f]
    cur = f
    while cur.is_nested_level and cur.parent in spec.by_name:
        cur = spec.by_name[cur.parent]
        chain.insert(0, cur)
    return chain


def offered_choices(
    spec: FormSpec, f: Field, values: Mapping[str, Any], restricted: Mapping[str, ChoiceRestriction]
) -> tuple[Choice, ...]:
    """Choices ``f`` offers given the upper nested levels' values and the
    ``populate`` restrictions in effect."""
    pool = spec.choices_for(f, values)
    chain = _nested_chain(spec, f)
    node: ChoiceRestriction | None = restricted.get(chain[0].api_name)
    for upper in chain[:-1]:
        if node is None:
            return pool
        v = values.get(upper.api_name)
        if is_empty(v) or not node.offers(v):
            return ()
        node = node.below(v)
    if node is None:
        return pool
    return tuple(c for c in pool if node.offers(c.value))


def _choice_in(pool: Sequence[Choice], raw: Any) -> Choice | None:
    """Populate values are choice ``display_id``s; fall back to id, then value/label."""
    key = norm(raw)
    for attr in ("display_id", "id"):
        for c in pool:
            v = getattr(c, attr)
            if v is not None and norm(v) == key:
                return c
    for c in pool:
        if norm(c.value) == key or norm(c.label) == key:
            return c
    return None


def _choice_by_any(f: Field, raw: Any) -> Choice | None:
    """Populate values are choice ``display_id``s; fall back to id/value."""
    ch = f.choice_by_display_id(raw)
    if ch is not None:
        return ch
    key = norm(raw)
    for c in f.choices:
        if c.id is not None and norm(c.id) == key:
            return c
    return f.choice(raw)


def _same(a: Any, b: Any) -> bool:
    if is_empty(a) and is_empty(b):
        return True
    if isinstance(a, (list, tuple)) and isinstance(b, (list, tuple)):
        return [norm(x) for x in a] == [norm(x) for x in b]
    if isinstance(a, bool) or isinstance(b, bool):
        return a is b
    return a == b or (a is not None and b is not None and type(a) is type(b) and norm(a) == norm(b))


class _Skip(Exception):
    pass


@dataclass
class _State:
    vis: dict[str, bool]
    req: dict[str, bool]
    values: dict[str, Any]
    dis: set[str] = dc_field(default_factory=set)
    populated: dict[str, Any] = dc_field(default_factory=dict)
    messages: dict[int, list[Message]] = dc_field(default_factory=dict)
    last_match: dict[int, Any] = dc_field(default_factory=dict)
    why: dict[tuple[str, str], Decision] = dc_field(default_factory=dict)
    matched_decisions: dict[tuple[str, str], list[Decision]] = dc_field(default_factory=dict)
    # property -> {field: server name}, in first-touch order
    reg: dict[str, dict[str, ExcludeName]] = dc_field(
        default_factory=lambda: {"visible": {}, "required": {}, "disabled": {}}
    )
    pseudo_req: dict[str, bool] = dc_field(default_factory=dict)
    restricted: dict[str, ChoiceRestriction] = dc_field(default_factory=dict)
    restricted_by: dict[str, Decision] = dc_field(default_factory=dict)
    refused: list[tuple[str, Decision | None]] = dc_field(default_factory=list)
    overridden: dict[str, tuple[Any, Any]] = dc_field(default_factory=dict)

    def touch(self, prop: str, t: Target) -> None:
        key = t.api or t.base
        self.reg[_KIND[prop]].setdefault(key, ExcludeName(t.base, t.custom))
