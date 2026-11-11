"""Business-rules engine: the client-side form rules Freshservice runs in the
browser (``GET /api/_/ws/2/business_rules``), evaluated in Python.

Semantics (see docs/forms.md for the full description):

* Rules are filtered by module, ``state == active``, ``rule_type ==
  client_side``, not deleted, ``form_type`` (``both``/``new_form``/``edit_form``)
  and ``user_type`` (``both``/``agent``/``requester``).
* Every evaluation starts from the base state (field ``visible``/``required``
  flags) and applies the rules **in their given order**; a later rule
  overrides an earlier one for the same field + property. The result is a pure
  function of the values, so a reversible rule "undoes" itself simply by not
  matching any more.
* A rule whose condition matches applies its actions. A rule that does not
  match applies the **reverse** of its actions only if ``is_reversible``
  (show<->hide, mandate<->unmandate, disable<->enable; populate/message have
  no reverse).
* ``populate`` fills a field only while the rule matches and only if the user
  has not given that field a value; the populated value is reported with its
  rule id (provenance). Populated values feed back into conditions, so the
  engine iterates to a fixed point.
* ``message`` collects messages; rules flagged ``is_validation`` produce
  blocking messages.
"""
from __future__ import annotations

from dataclasses import dataclass, field as dc_field
from typing import Any, Iterable, Mapping, Sequence

from fsv.forms.errors import UnsupportedRuleError
from fsv.forms.spec import Choice, Field, FormSpec, norm

SUPPORTED_OPERATORS = frozenset({"is", "is_not", "includes", "does_not_include", "selected", "not_selected"})
SUPPORTED_ACTIONS = frozenset({"show", "hide", "mandate", "unmandate", "populate", "disable", "message"})
_REVERSE = {"show": "hide", "hide": "show", "mandate": "unmandate", "unmandate": "mandate", "disable": "enable"}
_MODES = {"new": "new_form", "edit": "edit_form"}
MAX_PASSES = 8
# How to treat several entries with the same rule id in one response:
#   "exact" (default) drop entries identical to an earlier one, keep the rest
#           in order. The business_rules endpoint has been seen returning two
#           *different* entries per id (one reversible, one not, with
#           different action lists); evaluating both in order reproduced the
#           web form exactly, while keeping only one of them did not.
#   "first"/"last" keep one entry per id (first position; first/last content).
#   "none"  keep everything.
DEDUPE_POLICIES = ("exact", "first", "last", "none")


def is_empty(value: Any) -> bool:
    return value is None or value == "" or (isinstance(value, (list, tuple, set, frozenset)) and len(value) == 0)


@dataclass(frozen=True)
class Condition:
    name: str  # raw name as written in the rule
    field: str | None  # API name (None if the field is unknown)
    operator: str
    values: tuple[str, ...] = ()


@dataclass(frozen=True)
class ConditionGroup:
    mode: str  # "all" | "any"
    items: tuple["Condition | ConditionGroup", ...] = ()


@dataclass(frozen=True)
class Action:
    property: str
    targets: tuple[str, ...]  # API names; message texts for "message"
    values: tuple[Any, ...] = ()


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


@dataclass
class EvaluatedForm:
    visible: list[str]  # ordered, inputs and notices
    required: set[str]  # visible and required
    disabled: set[str]
    values: dict[str, Any]  # effective values (user + populated)
    populated: dict[str, Any]  # api name -> id of the rule that populated it
    messages: list[Message] = dc_field(default_factory=list)
    matched: list[Any] = dc_field(default_factory=list)  # ids of matching rules

    def is_visible(self, name: str) -> bool:
        return name in self._vis

    def is_required(self, name: str) -> bool:
        return name in self.required

    def __post_init__(self) -> None:
        self._vis = set(self.visible)


class RulesEngine:
    """Evaluates business rules for one :class:`FormSpec`."""

    def __init__(
        self,
        spec: FormSpec,
        rules: Any = None,
        *,
        module: str | None = None,
        user_type: str = "agent",
        item_id: Any = None,
        dedupe: str = "exact",
        strict: bool = False,
    ) -> None:
        if dedupe not in DEDUPE_POLICIES:
            raise ValueError(f"dedupe must be one of {DEDUPE_POLICIES}")
        self.spec = spec
        self.module = module
        self.user_type = user_type
        self.item_id = item_id
        self.dedupe = dedupe
        self.strict = strict
        self.warnings: list[str] = []
        self.unsupported: list[str] = []
        self.skipped: list[str] = []
        self.duplicates: list[Any] = []
        self.rules: list[Rule] = self._load(rules)

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
        for i, r in enumerate(kept):
            rule = self._parse_rule(r, i)
            if rule is not None:
                out.append(rule)
        return out

    def _filter(self, r: Mapping[str, Any]) -> str | None:
        if r.get("deleted"):
            return "deleted"
        if str(r.get("state", "active")) != "active":
            return f"state {r.get('state')}"
        if str(r.get("rule_type", "client_side")) != "client_side":
            return f"rule_type {r.get('rule_type')} (server-side rules are not evaluated)"
        if self.module and r.get("module_type") not in (None, self.module):
            return f"module {r.get('module_type')}"
        if self.item_id is not None and r.get("item_id") not in (None, self.item_id):
            return f"item {r.get('item_id')}"
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

    def _unsupported(self, msg: str) -> None:
        if self.strict:
            raise UnsupportedRuleError(msg)
        self.unsupported.append(msg)
        self.warnings.append(msg)

    def _parse_rule(self, r: Mapping[str, Any], index: int) -> Rule | None:
        name = str(r.get("name") or r.get("id"))
        try:
            cond = self._parse_group(r.get("condition_data") or {}, name)
            actions = self._parse_actions((r.get("action_data") or {}).get("action") or [], name)
        except _Skip as e:
            self._unsupported(f"rule {name!r} skipped: {e}")
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
        )

    def _parse_group(self, data: Mapping[str, Any], rule: str) -> ConditionGroup:
        if not isinstance(data, Mapping) or not data:
            return ConditionGroup("all", ())
        modes = [k for k in ("all", "any") if k in data]
        if len(modes) != 1:
            raise _Skip(f"condition must have exactly one of all/any, got {sorted(data)}")
        mode = modes[0]
        items: list[Condition | ConditionGroup] = []
        for c in data.get(mode) or []:  # "all": null means "always"
            if isinstance(c, Mapping) and ("all" in c or "any" in c) and "operator" not in c:
                items.append(self._parse_group(c, rule))
                continue
            op = str(c.get("operator") or "")
            if op not in SUPPORTED_OPERATORS:
                raise _Skip(f"unsupported operator {op!r}")
            raw_name = str(c.get("name") or "")
            api = self.spec.resolve_name(raw_name)
            if api is None:
                self.warnings.append(f"rule {rule!r}: condition on unknown field {raw_name!r}")
            val = c.get("value")
            vals = tuple(str(v) for v in val) if isinstance(val, (list, tuple)) else (() if val is None else (str(val),))
            items.append(Condition(raw_name, api, op, vals))
        return ConditionGroup(mode, tuple(items))

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
            targets = []
            for n in names:
                api = self.spec.resolve_name(str(n))
                if api is None:
                    self.warnings.append(f"rule {rule!r}: {prop} on unknown field {n!r} ignored")
                else:
                    targets.append(api)
            out.append(Action(prop, tuple(targets), vals))
        return tuple(out)

    # -- evaluation -------------------------------------------------------
    def rules_for(self, mode: str = "new") -> list[Rule]:
        return [r for r in self.rules if r.applies_to(mode, self.user_type)]

    def evaluate(self, values: Mapping[str, Any], mode: str = "new") -> EvaluatedForm:
        """Evaluate rules for ``values`` (user-entered, API names)."""
        user = {k: v for k, v in values.items() if not is_empty(v)}
        rules = self.rules_for(mode)
        populated: dict[str, Any] = {}
        result: _Pass | None = None
        for _ in range(MAX_PASSES):
            eff = dict(user)
            for k, v in populated.items():
                eff.setdefault(k, v)
            result = self._pass(rules, eff, user)
            if result.pop_values == populated:
                break
            populated = result.pop_values
        else:  # pragma: no cover - defensive
            self.warnings.append("populate rules did not settle; result may be unstable")
        assert result is not None
        return self._finish(result, user)

    def _pass(self, rules: Sequence[Rule], eff: Mapping[str, Any], user: Mapping[str, Any]) -> "_Pass":
        vis = {n: f.visible_base for n, f in self.spec.by_name.items()}
        req = {n: f.required_base for n, f in self.spec.by_name.items()}
        dis: set[str] = set()
        msgs: list[Message] = []
        pop_values: dict[str, Any] = {}
        pop_rule: dict[str, Any] = {}
        matched: list[Any] = []
        for rule in rules:
            ok = self._match_group(rule.condition, eff)
            if ok:
                matched.append(rule.id)
            for a in rule.actions:
                prop = a.property if ok else (_REVERSE.get(a.property) if rule.reversible else None)
                if prop is None:
                    continue
                if prop == "message":
                    msgs.extend(Message(t, rule.id, rule.name, rule.is_validation) for t in a.targets)
                    continue
                for t in a.targets:
                    if prop == "show":
                        vis[t] = True
                    elif prop == "hide":
                        vis[t] = False
                    elif prop == "mandate":
                        req[t] = True
                    elif prop == "unmandate":
                        req[t] = False
                    elif prop == "disable":
                        dis.add(t)
                    elif prop == "enable":
                        dis.discard(t)
                    elif prop == "populate":
                        if t in user:
                            continue  # never overwrite user input
                        v = self._populate_value(self.spec.by_name[t], a.values, rule)
                        if v is not None:
                            pop_values[t] = v
                            pop_rule[t] = rule.id
        return _Pass(vis, req, dis, msgs, pop_values, pop_rule, matched, dict(eff))

    def _finish(self, p: "_Pass", user: Mapping[str, Any]) -> EvaluatedForm:
        visible: list[str] = []
        shown: set[str] = set()
        values = p.eff
        for f in self.spec.ordered():
            if not p.vis.get(f.api_name, False):
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
        required = {n for n in visible if p.req.get(n) and self.spec.by_name[n].kind != "notice"}
        # values of fields that are not visible stay in .values (conditions
        # may still read them) but payload builders drop them.
        return EvaluatedForm(
            visible=visible,
            required=required,
            disabled=p.dis,
            values=dict(values),
            populated=dict(p.pop_rule),
            messages=p.msgs,
            matched=p.matched,
        )

    # -- matching ---------------------------------------------------------
    def _match_group(self, g: ConditionGroup, eff: Mapping[str, Any]) -> bool:
        if not g.items:
            return True
        results = (self._match_group(c, eff) if isinstance(c, ConditionGroup) else self._match(c, eff) for c in g.items)
        return all(results) if g.mode == "all" else any(results)

    def _match(self, c: Condition, eff: Mapping[str, Any]) -> bool:
        f = self.spec.by_name.get(c.field) if c.field else None
        v = eff.get(c.field) if c.field else None
        if c.operator in ("selected", "not_selected"):
            if f is not None and f.kind == "checkbox":
                sel = v is True
            else:
                sel = not is_empty(v) and v is not False
            return sel if c.operator == "selected" else not sel
        wanted = {norm(x) for x in c.values}
        hit = any(keys & wanted for keys in self._value_keys(f, v, eff))
        if c.operator in ("is", "includes"):
            return hit
        return not hit  # is_not, does_not_include

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


def _choice_by_any(f: Field, raw: Any) -> Choice | None:
    key = norm(raw)
    for c in f.choices:
        if c.display_id is not None and norm(c.display_id) == key:
            return c
    for c in f.choices:
        if c.id is not None and norm(c.id) == key:
            return c
    return f.choice(raw)


class _Skip(Exception):
    pass


@dataclass
class _Pass:
    vis: dict[str, bool]
    req: dict[str, bool]
    dis: set[str]
    msgs: list[Message]
    pop_values: dict[str, Any]
    pop_rule: dict[str, Any]
    matched: list[Any]
    eff: dict[str, Any]
