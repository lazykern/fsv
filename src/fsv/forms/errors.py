"""Exceptions raised by the form engine."""
from __future__ import annotations

from typing import Any, Sequence


class FormError(Exception):
    """Base class for every form-engine error."""


class UnknownFieldError(FormError):
    """No field matches the given API name or label."""

    def __init__(self, name: str, suggestions: Sequence[str] = ()) -> None:
        self.name = name
        self.suggestions = list(suggestions)
        msg = f"unknown field: {name!r}"
        if self.suggestions:
            msg += f" (did you mean: {', '.join(self.suggestions)})"
        super().__init__(msg)


class ValueParseError(FormError):
    """A raw value could not be parsed/validated for a field."""

    def __init__(self, field: str, raw: Any, message: str, options: Sequence[str] = ()) -> None:
        self.field = field
        self.raw = raw
        self.options = list(options)
        super().__init__(f"{field}: {message}")


class AmbiguousLookupError(FormError):
    """A lookup query matched more than one record; the caller must choose."""

    def __init__(self, field: str, query: str, candidates: Sequence[Any]) -> None:
        self.field = field
        self.query = query
        self.candidates = list(candidates)
        shown = ", ".join(getattr(c, "display", str(c)) or str(getattr(c, "id", c)) for c in self.candidates[:10])
        super().__init__(f"{field}: {query!r} matches {len(self.candidates)} records: {shown}")


class FieldDisabledError(FormError):
    """The field is disabled by a business rule and cannot be edited."""


class FormRuleError(FormError):
    """The form has a business rule the engine cannot evaluate faithfully.

    Raised by default (strict mode) so that fsv never silently computes the
    wrong visible/required set; ``RulesEngine(..., strict=False)`` skips such
    rules instead and lists them in ``engine.unsupported``.
    """

    def __init__(self, message: str, rule_id: Any = None, rule_name: str | None = None) -> None:
        self.rule_id = rule_id
        self.rule_name = rule_name
        super().__init__(message)


class UnsupportedRuleError(FormRuleError):
    """A business rule uses an operator/action/evaluate_on this engine does not implement."""


class PayloadError(FormError):
    """The form is not complete/valid enough to build a create payload."""

    def __init__(self, problems: Sequence[str]) -> None:
        self.problems = list(problems)
        super().__init__("; ".join(self.problems) or "invalid payload")
