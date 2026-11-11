"""Business-rules engine: the client-side form rules Freshservice runs in the
browser (``GET /api/_/ws/2/business_rules``), evaluated in Python.

Two evaluation models are implemented (``RulesEngine(model=...)``):

``"event"`` (the web's own model, the default)
    1. On load, every rule is evaluated in array order against the load
       values (defaults on a new form, the record on an edit form); a
       reversible rule that does not match applies its reverse.
    2. Each user change of field F is a *change event*: only the rules whose
       conditions reference F are evaluated, in array order. A matching rule
       applies its actions; a reversible rule is reversed only when it goes
       from matching to not matching (``event_reverse="transition"``;
       ``"always"`` reverses on every non-match).
    3. ``populate`` sets a value but does not cascade: it fires no change
       event for the field it filled.
    Answers that arrive without an order (YAML, ``--set``) are replayed as
    change events in web field order (``position``: a person filling the
    form top to bottom). Setting a field to the value it already has is not
    a change event.

``"replay"`` (pure function of the values)
    Every evaluation starts from the metadata and applies *every* rule in
    order; a later rule overrides an earlier one. ``populate`` never
    overwrites a value the user set and feeds back until it settles.

In both models a matching rule applies its actions; a rule that does not
match applies the reverse (show<->hide, mandate<->unmandate,
disable<->enable) only if ``is_reversible``. Several entries may share one
rule id (the endpoint returns a reversible and a non-reversible entry); they
are separate rules evaluated in order.
"""
from __future__ import annotations

from dataclasses import dataclass, field as dc_field
from typing import Any, Collection, Iterable, Mapping, Sequence

from fsv.forms.errors import UnsupportedRuleError
from fsv.forms.spec import CUSTOM_PREFIXES, Choice, Field, FormSpec, norm

SUPPORTED_OPERATORS = frozenset({"is", "is_not", "includes", "does_not_include", "selected", "not_selected"})
SUPPORTED_ACTIONS = frozenset({"show", "hide", "mandate", "unmandate", "populate", "disable", "message"})
# ``evaluate_on`` values that read a form field (the module's own name) ...
FORM_SOURCES = frozenset({"change", "ticket", "problem", "release", "service_request", "form"})
# ... and those that read a person record from the rule context.
USER_SOURCES: dict[str, str] = {"current_user": "user", "requester": "requester", "requested_for": "requested_for"}
SUPPORTED_EVALUATE_ON = FORM_SOURCES | frozenset(USER_SOURCES)
# Action targets that are not form fields but form-level switches.
PSEUDO_TARGETS = frozenset({"attachment_ids"})  # general request attachments (SR)
_REVERSE = {"show": "hide", "hide": "show", "mandate": "unmandate", "unmandate": "mandate", "disable": "enable"}
_KIND = {"show": "visible", "hide": "visible", "mandate": "required", "unmandate": "required",
         "disable": "disabled", "enable": "disabled"}
_POSITIVE = frozenset({"show", "mandate", "disable"})
_MODES = {"new": "new_form", "edit": "edit_form"}
MODELS = ("event", "replay")
# Default model. Chosen by scoring both models against the captured web
# forms (see docs/forms.md, "Choosing the model").
DEFAULT_MODEL = "event"
MAX_PASSES = 8
# How to treat several entries with the same rule id in one response:
#   "exact" (default) drop entries identical to an earlier one, keep the rest
#           in order (the endpoint returns a reversible and a non-reversible
#           entry per id with different action lists; both are applied).
#   "first"/"last" keep one entry per id (first position; first/last content).
#   "none"  keep everything.
DEDUPE_POLICIES = ("exact", "first", "last", "none")


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
    ``user`` object of the business_rules response.
    """

    module: str | None = None
    form: str = "new"  # "new" | "edit"
    user_type: str = "agent"
    workspace_id: Any = None
    item_id: Any = None
    user: Mapping[str, Any] | None = None
    requester: Mapping[str, Any] | None = None
    requested_for: Mapping[str, Any] | None = None

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
    source: str = "form"  # "form" | "user" | "requester" | "requested_for"


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
    action: str  # show/hide/mandate/unmandate/disable/enable
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

    def __post_init__(self) -> None:
        self._vis = set(self.visible)

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
        event_reverse: str = "transition",
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
            )
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
        if event_reverse not in ("transition", "always"):
            raise ValueError("event_reverse must be 'transition' or 'always'")
        self.event_reverse = event_reverse
        self.warnings: list[str] = []
        self.unsupported: list[str] = []
        self.skipped: list[str] = []
        self.duplicates: list[Any] = []
        self.rules: list[Rule] = self._load(rules)
        order = spec.ordered()
        n = len(order)
        self._pos: dict[str, float] = {f.api_name: i for i, f in enumerate(order)}
        for f in spec.fields:
            self._pos.setdefault(f.api_name, n + (f.position or 0))

    # -- loading ----------------------------------------------------------
    def _load(self, data: Any) -> list[Rule]:
        raw = data.get("business_rules", []) if isinstance(data, Mapping) else list(data or [])
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
            if ev_key in USER_SOURCES:
                attr = raw_name.split(".")[-1]
                items.append(Condition(raw_name, attr, op, vals, USER_SOURCES[ev_key]))
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
                if not rule.fields & changed:
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
            msg = f"rules read the {source} record but none was given; its conditions see empty values"
            if msg not in self.warnings:
                self.warnings.append(msg)
        return person

    def _match(self, c: Condition, eff: Mapping[str, Any]) -> bool:
        if c.source != "form":
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
    refused: list[tuple[str, Decision | None]] = dc_field(default_factory=list)
    overridden: dict[str, tuple[Any, Any]] = dc_field(default_factory=dict)

    def touch(self, prop: str, t: Target) -> None:
        key = t.api or t.base
        self.reg[_KIND[prop]].setdefault(key, ExcludeName(t.base, t.custom))
