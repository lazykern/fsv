"""Status meaning from runtime metadata, never from English labels.

Freshservice's built-in statuses keep fixed internal ids whatever a tenant
renames or translates them to (tickets: 2 Open, 3 Pending, 4 Resolved,
5 Closed; problems: 1 Open, 3 Closed). Custom statuses get other ids and
never resolve or close a record. A role ("open", "pending", "resolved",
"closed") is identified only when the record's own runtime status choices
contain its internal id exactly once; anything else is ambiguous and callers
refuse rather than guess from a label.

The words "Open", "Pending", "Resolved" and "Closed" appear only as CLI
vocabulary (``fsv tickets resolve``, ``--set-status Pending``): they name a
role, which is then looked up by internal id in the tenant's choices.
"""
from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Iterable

from fsv.choice_ids import runtime_choice_id

BUILTIN_IDS: dict[str, dict[str, int]] = {
    "ticket": {"open": 2, "pending": 3, "resolved": 4, "closed": 5},
    "problem": {"open": 1, "closed": 3},
}
TERMINAL_ROLES = frozenset({"resolved", "closed"})


class StatusSemanticsError(ValueError):
    """A status role cannot be identified from runtime metadata."""


@dataclass(frozen=True)
class Role:
    """A status meaning requested by a command (not a tenant label)."""

    name: str

    def __str__(self) -> str:
        return self.name.title()


OPEN, PENDING, RESOLVED, CLOSED = Role("open"), Role("pending"), Role("resolved"), Role("closed")


def _module(module: str) -> str:
    m = (module or "").strip().lower().rstrip("s")
    if m not in BUILTIN_IDS:
        raise StatusSemanticsError(f"no status semantics for {module!r}")
    return m


def _ident(choice: Any) -> int | None:
    raw = choice.get("id") if isinstance(choice, dict) else getattr(choice, "id", None)
    return runtime_choice_id(raw)


def _visible(choice: Any) -> bool:
    if isinstance(choice, dict):
        return choice.get("visible", True) is not False
    return getattr(choice, "visible", True) is not False


def _label(choice: Any) -> str:
    if isinstance(choice, dict):
        return str(choice.get("value") or choice.get("label") or choice.get("name") or choice.get("id"))
    return str(getattr(choice, "label", "") or getattr(choice, "value", "") or getattr(choice, "id", ""))


def find(choices: Iterable[Any], module: str, role: Role | str) -> Any:
    """The runtime choice for ``role`` (by internal id); refuse when absent
    or duplicated."""
    name = role.name if isinstance(role, Role) else str(role).strip().lower()
    ids = BUILTIN_IDS[_module(module)]
    if name not in ids:
        raise StatusSemanticsError(f"{module} has no built-in {name!r} status")
    hits = [c for c in choices if _ident(c) == ids[name]]
    if len(hits) != 1:
        raise StatusSemanticsError(
            f"the {name} status is not identifiable from the runtime status metadata (the built-in internal id is "
            f"{'missing' if not hits else 'duplicated'}); pass the tenant's own status label explicitly")
    return hits[0]


def role_of(choices: Iterable[Any], module: str, value: Any) -> str | None:
    """The built-in role of a status id, only when the runtime choices carry
    that id; None for custom or unknown statuses."""
    ident = runtime_choice_id(value)
    if ident is None:
        return None
    pool = [c for c in choices if _ident(c) == ident]
    if len(pool) != 1:
        return None
    for name, builtin in BUILTIN_IDS[_module(module)].items():
        if builtin == ident:
            return name
    return None


def closes(choices: Iterable[Any], module: str, value: Any) -> bool | None:
    """True: the status resolves/closes the record; False: it does not;
    None: ambiguous (the runtime choices do not carry every built-in
    terminal id, so a custom-looking status cannot be ruled out)."""
    pool = list(choices)
    role = role_of(pool, module, value)
    if role is not None:
        return role in TERMINAL_ROLES
    ident = runtime_choice_id(value)
    if ident is None or sum(1 for c in pool if _ident(c) == ident) != 1:
        return None
    ids = BUILTIN_IDS[_module(module)]
    terminal = [ids[r] for r in TERMINAL_ROLES if r in ids]
    return False if all(sum(1 for c in pool if _ident(c) == t) == 1 for t in terminal) else None


def select(choices: Iterable[Any], module: str, wanted: Role | str) -> Any:
    """A visible runtime choice for a :class:`Role` (by internal id) or a
    tenant label (exact, case-insensitive). An English role word that is not
    a tenant label is read as that role. Refuses ambiguity."""
    pool = [c for c in choices if _visible(c)]
    if isinstance(wanted, Role):
        return find(pool, module, wanted)
    text = str(wanted or "").strip()
    folded = text.casefold()
    hits = [c for c in pool if folded and folded in {
        str(getattr(c, "label", "") if not isinstance(c, dict) else c.get("value") or "").casefold(),
        str(getattr(c, "value", "") if not isinstance(c, dict) else c.get("label") or "").casefold()}]
    if len(hits) == 1:
        return hits[0]
    if len(hits) > 1:
        raise StatusSemanticsError(f"status {text!r} matches several runtime statuses; use a unique label")
    if folded in BUILTIN_IDS[_module(module)]:
        return find(pool, module, Role(folded))
    shown = ", ".join(_label(c) for c in pool[:12])
    raise StatusSemanticsError(f"status {text!r} is not a runtime status label" + (f"; choices: {shown}" if shown else ""))
