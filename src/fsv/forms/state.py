"""Form state: the user's answers, validated, with rules re-evaluated after
every change.

The state keeps the *load* values (defaults on a new form, the record on an
edit form) apart from the user's answers. Every change re-runs the engine
from scratch: in the event model the answers are replayed as change events
(web field order by default, or the order they were given with
``event_order="input"``), so editing an earlier answer replays the whole
sequence with the new value.
"""
from __future__ import annotations

import datetime as dt
import os
import re
from dataclasses import dataclass, field as dc_field
from typing import Any, Mapping, Protocol, Sequence
from fsv.forms.errors import AmbiguousLookupError, FieldDisabledError, ValueParseError
from fsv.forms.rules import Conflict, Decision, EvaluatedForm, Explanation, Message, RulesEngine, is_empty
from fsv.forms.spec import Choice, Field, FormSpec, norm, plain_text
from fsv.forms.tz import resolve_timezone

# position: answers replayed in web field order (YAML / --set contract)
# input:    answers replayed in the order they were first given
# history:  every set() call replayed as a change event, repeats included
#           (what a person clicking through the web form does)
EVENT_ORDERS = ("position", "input", "history")
_TRUE = {"yes", "true", "y"}
_FALSE = {"no", "false", "n"}
_INT = re.compile(r"^[+-]?\d+$")


@dataclass(frozen=True)
class LookupValue:
    """A resolved person/group/agent (or other lookup record)."""

    id: Any
    display: str = ""
    email: str | None = None

    def __str__(self) -> str:
        if self.display and self.email:
            return f"{self.display} <{self.email}>"
        return self.display or self.email or str(self.id)


class LookupResolver(Protocol):
    """Injected by the caller (CLI/TUI); the forms package does no HTTP.

    ``entity`` is ``user``, ``group``, ``agent`` or the field's
    ``lookup_entity`` (e.g. ``change_window``). Return every candidate; the
    engine picks a single or exact match, or raises AmbiguousLookupError.
    """

    def lookup(self, entity: str, query: str, field: Field) -> Sequence[LookupValue]: ...


@dataclass
class Diff:
    """What changed in the form after a ``set``/``unset``."""

    field: str
    appeared: list[str] = dc_field(default_factory=list)
    disappeared: list[str] = dc_field(default_factory=list)
    became_required: list[str] = dc_field(default_factory=list)
    no_longer_required: list[str] = dc_field(default_factory=list)
    messages_added: list[Message] = dc_field(default_factory=list)
    messages_removed: list[Message] = dc_field(default_factory=list)
    populated: dict[str, Any] = dc_field(default_factory=dict)
    cleared: list[str] = dc_field(default_factory=list)

    def __bool__(self) -> bool:
        return any(
            (
                self.appeared,
                self.disappeared,
                self.became_required,
                self.no_longer_required,
                self.messages_added,
                self.messages_removed,
                self.populated,
                self.cleared,
            )
        )

    def describe(self, spec: FormSpec) -> list[str]:
        """Human-readable lines (labels, not API names)."""

        def labels(names: Sequence[str]) -> str:
            return ", ".join(spec.by_name[n].label for n in names)

        out = []
        if self.appeared:
            req = [n for n in self.appeared if n in self.became_required]
            out.append(f"added: {labels(self.appeared)}" + (f" (required: {labels(req)})" if req else ""))
        if self.disappeared:
            out.append(f"removed: {labels(self.disappeared)}")
        now_req = [n for n in self.became_required if n not in self.appeared]
        if now_req:
            out.append(f"now required: {labels(now_req)}")
        if self.no_longer_required:
            out.append(f"no longer required: {labels(self.no_longer_required)}")
        for name, value in self.populated.items():
            out.append(f"filled in: {spec.by_name[name].label} = {value}")
        if self.cleared:
            out.append(f"cleared: {labels(self.cleared)}")
        out.extend(f"message: {m.text}" for m in self.messages_added)
        return out


class FormState:
    """User answers + the evaluated form.

    ``values`` holds only what the user (or a default) set; rule-populated
    values live in ``evaluated.values`` with provenance in
    ``evaluated.populated``.
    """

    def __init__(
        self,
        spec: FormSpec,
        rules: RulesEngine | None = None,
        mode: str = "new",
        values: Mapping[str, Any] | None = None,
        *,
        resolver: LookupResolver | None = None,
        tz: str | None = None,
        apply_defaults: bool = True,
        existing: Mapping[str, Any] | None = None,
        event_order: str = "position",
    ) -> None:
        if mode not in ("new", "edit"):
            raise ValueError("mode must be 'new' or 'edit'")
        if event_order not in EVENT_ORDERS:
            raise ValueError(f"event_order must be one of {EVENT_ORDERS}")
        self.spec = spec
        self.rules = rules if rules is not None else RulesEngine(spec, [])
        self.mode = mode
        self.resolver = resolver
        self.event_order = event_order
        self.tz, self.tz_source = resolve_timezone(tz, self.rules.user)
        self.values: dict[str, Any] = {}
        self.sources: dict[str, str] = {}  # "default" | "existing" | "user"
        self._seq: list[str] = []  # user answers in the order given
        self._cleared: set[str] = set()  # load values the user cleared
        self._events: list[tuple[str, Any]] = []  # every user change, for event_order="history"
        self.acknowledged: set[tuple[str, str]] = set()
        if apply_defaults and mode == "new":
            for f in spec.fields:
                if f.default is not None:
                    self.values[f.api_name] = f.default
                    self.sources[f.api_name] = "default"
        for k, v in (existing or {}).items():
            f = spec.field(k)
            if not is_empty(v):
                self.values[f.api_name] = v
                self.sources[f.api_name] = "existing"
        # load values (defaults / the record being edited): the state the
        # rules see on load, before any change event
        self.base: dict[str, Any] = dict(self.values)
        self.evaluated: EvaluatedForm = self._evaluate()
        # parse initial answers in form order so nested/section children
        # are validated after their parents
        given = {spec.field(k).api_name: v for k, v in (values or {}).items()}
        for name in self.rules.position_order(given):
            f = spec.by_name[name]
            parsed = self.parse(f, given[name])
            self._store(f, parsed, "user")
            self.evaluated = self._evaluate()
        self._check_refused()

    # -- editing ----------------------------------------------------------
    def _evaluate(self) -> EvaluatedForm:
        user: dict[str, Any] = {k: v for k, v in self.values.items() if self.sources.get(k) == "user"}
        user.update({k: None for k in self._cleared})
        order = self._seq if self.event_order == "input" else None
        events = self._events if self.event_order == "history" else None
        return self.rules.evaluate(user, self.mode, base=self.base, order=order, events=events)

    def _check_refused(self) -> None:
        if self.evaluated.refused:
            name, d = self.evaluated.refused[0]
            label = self.spec.by_name[name].label
            why = f" ({d.describe()})" if d else ""
            raise FieldDisabledError(f"{label} is disabled by a business rule{why}; it cannot be set")

    def set(self, name: str, raw: Any) -> Diff:
        """Parse/validate ``raw`` for the field and re-evaluate the rules."""
        f = self.spec.field(name)
        if f.api_name in self.evaluated.disabled:
            d = self.evaluated.decision(f.api_name, "disabled")
            why = f" ({d.describe()})" if d else ""
            raise FieldDisabledError(f"{f.label} is disabled by a business rule{why}")
        parsed = self.parse(f, raw)
        before = self.evaluated
        snapshot = (dict(self.values), dict(self.sources), list(self._seq), set(self._cleared), list(self._events))
        self._store(f, parsed, "user")
        cleared = self._clear_lower_levels(f, before.values.get(f.api_name))
        self.evaluated = self._evaluate()
        if self.evaluated.refused:
            self.values, self.sources, self._seq, self._cleared, self._events = snapshot
            refused = self.evaluated
            self.evaluated = before
            name0, d = refused.refused[0]
            why = f" ({d.describe()})" if d else ""
            raise FieldDisabledError(
                f"setting {f.label} disables {self.spec.by_name[name0].label}, which you also set{why}"
            )
        diff = _diff(f.api_name, before, self.evaluated)
        diff.cleared = cleared
        return diff

    def unset(self, name: str) -> Diff:
        return self.set(name, None)

    def _store(self, f: Field, value: Any, source: str) -> None:
        name = f.api_name
        if source == "user":
            self._events.append((name, None if is_empty(value) else value))
        if is_empty(value):
            self.values.pop(name, None)
            self.sources.pop(name, None)
            if source == "user" and name in getattr(self, "base", {}):
                self._cleared.add(name)  # clearing a load value is a change event
                if name not in self._seq:
                    self._seq.append(name)
            elif name in self._seq:
                self._seq.remove(name)
        else:
            self.values[name] = value
            self.sources[name] = source
            self._cleared.discard(name)
            if source == "user" and name not in self._seq:
                self._seq.append(name)

    def _clear_lower_levels(self, f: Field, old: Any) -> list[str]:
        """A nested parent changed value: clear every lower level (web
        behaviour), even when a same-named child would still fit."""
        if f.kind != "nested" or _same_value(old, self.values.get(f.api_name)):
            return []
        cleared: list[str] = []
        cur: Field | None = f
        while cur is not None:
            child = next((c for c in self.spec.fields if c.parent == cur.api_name and c.is_nested_level), None)
            if child is None:
                break
            if child.api_name in self.values:
                self._store(child, None, "user")
                cleared.append(child.api_name)
            cur = child
        return cleared

    # -- acknowledgements -------------------------------------------------
    def unacknowledged(self) -> list[Message]:
        """Blocking (``is_validation``) messages not acknowledged yet."""
        return [m for m in self.evaluated.blocking_messages if m.key not in self.acknowledged]

    def acknowledge(self, *messages: Message | str) -> None:
        """Acknowledge blocking messages (by Message, rule id, or message text);
        with no arguments, acknowledge every current one."""
        current = self.evaluated.blocking_messages
        if not messages:
            self.acknowledged.update(m.key for m in current)
            return
        for want in messages:
            hits = [m for m in current if want == m or str(want) in (str(m.rule_id), m.text)]
            if not hits:
                raise ValueError(f"no blocking message matches {want!r}")
            self.acknowledged.update(m.key for m in hits)

    # -- parsing ----------------------------------------------------------
    def parse(self, f: Field | str, raw: Any) -> Any:
        """Validate ``raw`` for a field and return the stored value."""
        if isinstance(f, str):
            f = self.spec.field(f)
        if raw is None or (isinstance(raw, str) and raw.strip() == "") or (isinstance(raw, (list, tuple)) and not raw):
            return None
        k = f.kind
        if k == "notice":
            raise ValueParseError(f.label, raw, "is an information block and cannot be set")
        if k in ("text", "textarea", "richtext"):
            return str(raw)
        if k == "number":
            return _parse_number(f, raw)
        if k == "checkbox":
            return _parse_bool(f, raw)
        if k in ("dropdown", "department", "nested"):
            return self._parse_choice(f, raw).value
        if k == "multi_select":
            return tuple(c.value for c in self._parse_multi(f, raw))
        if k == "date":
            return _parse_date(f, raw)
        if k == "datetime":
            return _parse_datetime(f, raw, self.tz)
        if k in ("lookup_user", "lookup_group", "lookup_agent", "lookup"):
            return self._parse_lookup(f, raw)
        if k == "attachment":
            return _parse_attachments(f, raw)
        raise ValueParseError(f.label, raw, f"unsupported field kind {k!r}")  # pragma: no cover

    def _choices(self, f: Field) -> tuple[Choice, ...]:
        pool = self.spec.choices_for(f, {**self.evaluated.values, **self.values})
        if f.is_nested_level and not pool:
            parent = self.spec.by_name.get(f.parent or "")
            raise ValueParseError(f.label, None, f"choose {parent.label if parent else 'the parent level'} first")
        return pool

    def _parse_choice(self, f: Field, raw: Any) -> Choice:
        pool = self._choices(f)
        if isinstance(raw, Choice):
            raw = raw.value
        ch = f.choice(raw, pool)
        if ch is None or not ch.visible:
            opts = [c.label for c in pool if c.visible]
            shown = ", ".join(opts[:15]) + (f", … ({len(opts)} total)" if len(opts) > 15 else "")
            raise ValueParseError(f.label, raw, f"{raw!r} is not a valid choice; choose one of: {shown}", opts)
        return ch

    def _parse_multi(self, f: Field, raw: Any) -> list[Choice]:
        pool = self._choices(f)
        if isinstance(raw, str):
            whole = f.choice(raw, pool)
            parts: Sequence[Any] = [raw] if whole is not None else [p.strip() for p in raw.split(",") if p.strip()]
        else:
            parts = list(raw)
        out: list[Choice] = []
        for p in parts:
            ch = self._parse_choice(f, p)
            if ch not in out:
                out.append(ch)
        return out

    def _parse_lookup(self, f: Field, raw: Any) -> Any:
        if f.multi:
            items = [p.strip() for p in raw.split(",") if p.strip()] if isinstance(raw, str) else list(raw)
            out: list[LookupValue] = []
            for it in items:
                lv = self._lookup_one(f, it)
                if lv not in out:
                    out.append(lv)
            return tuple(out)
        if isinstance(raw, (list, tuple)):
            if len(raw) != 1:
                raise ValueParseError(f.label, raw, "takes a single value")
            raw = raw[0]
        return self._lookup_one(f, raw)

    def _lookup_one(self, f: Field, raw: Any) -> LookupValue:
        if isinstance(raw, LookupValue):
            return raw
        if isinstance(raw, bool):
            raise ValueParseError(f.label, raw, "expects a name, email or id")
        if isinstance(raw, int) or (isinstance(raw, str) and raw.strip().isdigit()):
            return LookupValue(id=int(raw))
        query = str(raw).strip()
        if self.resolver is None:
            raise ValueParseError(f.label, raw, "needs a lookup resolver (or pass a numeric id)")
        entity = {"lookup_user": "user", "lookup_group": "group", "lookup_agent": "agent"}.get(
            f.kind, f.lookup_entity or "lookup"
        )
        found = list(self.resolver.lookup(entity, query, f))
        if len(found) == 1:
            return found[0]
        q = norm(query)
        exact = [c for c in found if q in {norm(c.display), norm(c.email or ""), norm(str(c))}]
        if len(exact) == 1:
            return exact[0]
        if not found:
            raise ValueParseError(f.label, raw, f"no match for {query!r}")
        raise AmbiguousLookupError(f.label, query, exact or found)

    # -- queries ----------------------------------------------------------
    def get(self, name: str) -> Any:
        """Effective value (user, default or populated)."""
        f = self.spec.field(name)
        return self.evaluated.values.get(f.api_name)

    def is_populated(self, name: str) -> bool:
        return self.spec.field(name).api_name in self.evaluated.populated

    def visible_fields(self) -> list[Field]:
        return [self.spec.by_name[n] for n in self.evaluated.visible]

    @property
    def messages(self) -> list[Message]:
        return list(self.evaluated.messages)

    def missing_required(self) -> list[Field]:
        out = []
        for f in self.visible_fields():
            if f.api_name not in self.evaluated.required:
                continue
            v = self.evaluated.values.get(f.api_name)
            if f.kind == "checkbox":
                if v is not True:
                    out.append(f)
            elif is_empty(v):
                out.append(f)
        return out

    def errors(self) -> list[str]:
        """Everything that would stop the web form from submitting
        (blocking messages count until acknowledged)."""
        out = [f"{f.label} is required" for f in self.missing_required()]
        out.extend(f"blocked by rule (acknowledge to continue): {m.text}" for m in self.unacknowledged())
        return out

    def dropped(self) -> list[tuple[Field, Any, Decision | None]]:
        """Fields holding a value that will NOT be sent because they are
        hidden: (field, value, the hide decision if a rule hid it)."""
        out = []
        vis = set(self.evaluated.visible)
        for f in self.spec.ordered():
            if f.api_name in vis or f.kind == "notice":
                continue
            v = self.evaluated.values.get(f.api_name)
            if not is_empty(v):
                out.append((f, v, self.evaluated.decision(f.api_name, "visible")))
        return out

    def explain(self, names: Sequence[str] | None = None) -> list[Explanation]:
        return self.evaluated.explain(names)

    @property
    def conflicts(self) -> list[Conflict]:
        return list(self.evaluated.conflicts)

    def payload_values(self) -> dict[str, Any]:
        """Effective values of visible input fields, in form order.
        Hidden fields' values are dropped."""
        out: dict[str, Any] = {}
        for f in self.visible_fields():
            if f.kind == "notice":
                continue
            v = self.evaluated.values.get(f.api_name)
            if not is_empty(v):
                out[f.api_name] = v
        return out

    def display(self, name: str, value: Any = None) -> str:
        f = self.spec.field(name)
        v = self.evaluated.values.get(f.api_name) if value is None else value
        return _display(self.spec, f, v, self.evaluated.values)

    def to_review(self) -> list[tuple[str, str]]:
        """Ordered (label, display value) rows for visible filled fields,
        followed by ("Message"/"Blocked", text) rows."""
        rows: list[tuple[str, str]] = []
        for name, v in self.payload_values().items():
            f = self.spec.by_name[name]
            text = _display(self.spec, f, v, self.evaluated.values)
            if name in self.evaluated.populated:
                text += " (auto)"
            rows.append((f.label, text))
        for name, (old, rid) in self.evaluated.overridden.items():
            f = self.spec.by_name[name]
            rows.append(("Replaced", f"{f.label}: your value {_display(self.spec, f, old, self.evaluated.values)!r} "
                                     f"was replaced by rule {rid} (populate)"))
        for f, v, d in self.dropped():
            why = f"hidden by rule {d.rule_name!r}" if d and d.action == "hide" else "hidden"
            rows.append(("Dropped", f"{f.label} = {_display(self.spec, f, v, self.evaluated.values)} ({why}; not sent)"))
        acked = self.acknowledged
        for m in self.evaluated.messages:
            if m.blocking:
                rows.append(("Acknowledged" if m.key in acked else "Blocked", m.text))
            else:
                rows.append(("Message", m.text))
        return rows


# ---------------------------------------------------------------------------
# helpers


def _diff(name: str, before: EvaluatedForm, after: EvaluatedForm) -> Diff:
    bv, av = set(before.visible), set(after.visible)
    order = {n: i for i, n in enumerate(after.visible)}
    border = {n: i for i, n in enumerate(before.visible)}
    d = Diff(field=name)
    d.appeared = sorted(av - bv, key=lambda n: order[n])
    d.disappeared = sorted(bv - av, key=lambda n: border[n])
    d.became_required = sorted(after.required - before.required, key=lambda n: order.get(n, 1 << 30))
    d.no_longer_required = sorted(before.required - after.required, key=lambda n: border.get(n, 1 << 30))
    bm = {(m.rule_id, m.text) for m in before.messages}
    am = {(m.rule_id, m.text) for m in after.messages}
    d.messages_added = [m for m in after.messages if (m.rule_id, m.text) not in bm]
    d.messages_removed = [m for m in before.messages if (m.rule_id, m.text) not in am]
    for n, rid in after.populated.items():
        if before.populated.get(n) != rid or before.values.get(n) != after.values.get(n):
            d.populated[n] = after.values.get(n)
    return d


def _parse_number(f: Field, raw: Any) -> int | float:
    if isinstance(raw, bool):
        raise ValueParseError(f.label, raw, "expects a number")
    if isinstance(raw, (int, float)):
        return raw
    s = str(raw).strip().replace(",", "")
    if _INT.match(s):
        return int(s)
    try:
        return float(s)
    except ValueError:
        raise ValueParseError(f.label, raw, f"{raw!r} is not a number") from None


def _parse_bool(f: Field, raw: Any) -> bool:
    if isinstance(raw, bool):
        return raw
    s = str(raw).strip().casefold()
    if s in _TRUE:
        return True
    if s in _FALSE:
        return False
    raise ValueParseError(f.label, raw, f"{raw!r} is not yes/no/true/false", ["yes", "no"])


def _parse_date(f: Field, raw: Any) -> dt.date:
    if isinstance(raw, dt.datetime):
        return raw.date()
    if isinstance(raw, dt.date):
        return raw
    try:
        return dt.date.fromisoformat(str(raw).strip())
    except ValueError:
        raise ValueParseError(f.label, raw, f"{raw!r} is not a date (YYYY-MM-DD)") from None


def _same_value(a: Any, b: Any) -> bool:
    if is_empty(a) and is_empty(b):
        return True
    return a is not None and b is not None and norm(a) == norm(b)


def _parse_datetime(f: Field, raw: Any, tz: dt.tzinfo) -> dt.datetime:
    if isinstance(raw, dt.datetime):
        value = raw
    elif isinstance(raw, dt.date):
        raise ValueParseError(f.label, raw, "needs a time as well as a date")
    else:
        s = str(raw).strip()
        if re.fullmatch(r"\d{4}-\d{2}-\d{2}", s):
            raise ValueParseError(f.label, raw, "needs a time as well as a date (YYYY-MM-DD HH:MM)")
        try:
            value = dt.datetime.fromisoformat(s)
        except ValueError:
            raise ValueParseError(
                f.label, raw, f"{raw!r} is not a date-time (YYYY-MM-DD HH:MM, optional +HH:MM offset)"
            ) from None
    if value.tzinfo is None:
        value = value.replace(tzinfo=tz)
    return value


def _parse_attachments(f: Field, raw: Any) -> tuple[Any, ...]:
    items = [raw] if isinstance(raw, (str, int, os.PathLike)) else list(raw)
    items = [os.fspath(i) if isinstance(i, os.PathLike) else i for i in items]
    if f.max_files is not None and len(items) > f.max_files:
        raise ValueParseError(f.label, raw, f"accepts at most {f.max_files} file(s)")
    if f.allowed_file_types:
        allowed = {t.lower() if t.startswith(".") else "." + t.lower() for t in f.allowed_file_types}
        for i in items:
            if isinstance(i, str) and os.path.splitext(i)[1].lower() not in allowed:
                raise ValueParseError(f.label, raw, f"{i!r}: allowed file types are {', '.join(sorted(allowed))}")
    return tuple(items)


def _display(spec: FormSpec, f: Field, v: Any, values: Mapping[str, Any]) -> str:
    if is_empty(v):
        return ""
    if f.kind in ("dropdown", "department", "nested"):
        ch = f.choice(v, spec.choices_for(f, values))
        return ch.label if ch else str(v)
    if f.kind == "multi_select":
        labels = []
        for x in v:
            ch = f.choice(x)
            labels.append(ch.label if ch else str(x))
        return ", ".join(labels)
    if f.kind == "checkbox":
        return "Yes" if v else "No"
    if f.kind == "date" and isinstance(v, dt.date):
        return v.strftime("%d %b %Y")
    if f.kind == "datetime" and isinstance(v, dt.datetime):
        return v.strftime("%d %b %Y %H:%M") + f" ({_offset(v)})"
    if isinstance(v, (list, tuple)):
        return ", ".join(os.path.basename(x) if isinstance(x, str) else str(x) for x in v)
    if f.kind in ("textarea", "richtext"):
        text = " ".join(plain_text(str(v)).split())
        return text if len(text) <= 120 else text[:119] + "…"
    return str(v)


def _offset(v: dt.datetime) -> str:
    off = v.utcoffset() or dt.timedelta(0)
    total = int(off.total_seconds() // 60)
    sign = "+" if total >= 0 else "-"
    total = abs(total)
    return f"{sign}{total // 60:02d}:{total % 60:02d}"
