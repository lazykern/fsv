"""Strict semantic comparison of server-normalized readback values."""
from __future__ import annotations

from datetime import datetime, timezone
from decimal import Decimal, InvalidOperation
import re
import time
from typing import Any, Callable


def same_text(actual: Any, expected: Any) -> bool:
    """Compare rich-text wrappers/entities and whitespace, not missing values."""
    from fsv.views import one_line

    return isinstance(actual, str) and isinstance(expected, str) and one_line(actual) == one_line(expected)


def resolution_text(record: dict[str, Any]) -> str | None:
    """Internal details wrap notes_html; v2 ticket details return plain text."""
    value = record.get("resolution_notes")
    if isinstance(value, dict):
        value = value.get("notes_html")
    if isinstance(value, str):
        return value
    alias = record.get("resolution_notes_html")
    return alias if isinstance(alias, str) else None


def submitted_value_matches(actual: Any, expected: Any) -> bool:
    """Compare submitted keys only, allowing canonical scalar wire normalization.

    Missing keys, nulls, booleans and ordered lists remain distinct. In particular,
    do not turn arbitrary numeric-looking text (e.g. leading-zero codes) into IDs.
    """
    if isinstance(expected, dict):
        return isinstance(actual, dict) and all(
            key in actual and submitted_value_matches(actual[key], value)
            for key, value in expected.items())
    if isinstance(expected, list):
        return isinstance(actual, list) and len(actual) == len(expected) and all(
            submitted_value_matches(a, e) for a, e in zip(actual, expected))
    if type(actual) is type(expected):
        return actual == expected
    if type(expected) is bool:
        return isinstance(actual, str) and actual.casefold() == str(expected).lower()
    if type(actual) is bool or actual is None or expected is None:
        return False
    if type(expected) in (int, float) and isinstance(actual, str):
        try:
            return Decimal(actual).is_finite() and Decimal(actual) == Decimal(str(expected))
        except InvalidOperation:
            return False
    if isinstance(expected, str) and type(actual) in (int, float):
        return str(actual) == expected
    return False


def custom_fields_match(actual: Any, expected: Any) -> bool:
    """Submitted custom keys only, including native lookup identity wrappers.

    A lookup display value is not its identity. Never match a different ID merely
    because its label happens to equal the submitted ID or value.
    """
    if not isinstance(actual, dict) or not isinstance(expected, dict):
        return False
    for key, value in expected.items():
        if key not in actual:
            return False
        observed = actual[key]
        if isinstance(observed, dict) and set(observed) == {"id", "value"} and not isinstance(value, (dict, list, bool)) and value is not None:
            observed = observed["id"]
        if not submitted_value_matches(observed, value):
            return False
    return True


def same_instant(actual: Any, expected: Any) -> bool:
    """Compare explicit-offset timestamps without assuming a local timezone."""
    def parse(value: Any) -> datetime:
        if not isinstance(value, str):
            raise ValueError("timestamp must be text")
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
        if parsed.utcoffset() is None:
            raise ValueError("timestamp needs an offset")
        return parsed.astimezone(timezone.utc)

    try:
        return parse(actual) == parse(expected)
    except (ValueError, TypeError, OverflowError):
        return False


READBACK_DELAYS = (0.0, 0.5, 1.5, 3.0)
_sleep = time.sleep


def bounded_readback(verify: Callable[[], dict[str, Any]]) -> dict[str, Any]:
    """Re-run only an independent read-only verifier, never the mutation sender.

    Bulk endpoints can acknowledge before every exact record read reflects the
    change. Take at most four observations over five seconds; the final
    mismatches/unavailable fields are preserved and the caller's durable
    journal still forbids any resend.
    """
    result: dict[str, Any] = {"verified": False}
    for attempt, delay in enumerate(READBACK_DELAYS, 1):
        if delay:
            _sleep(delay)
        try:
            result = dict(verify())
        except Exception:
            result = {"verified": False, "warning": "Independent readback unavailable; do not resend."}
        result["readback_attempts"] = attempt
        if result.get("verified") is True:
            break
    return result


def duration_seconds(value: Any) -> int:
    """Parse unbounded hours, minutes and optional seconds; never wrap at 24h."""
    if not isinstance(value, str) or not re.fullmatch(r"\d{2,}:[0-5]\d(?::[0-5]\d)?", value):
        raise ValueError("duration must be HH:MM or HH:MM:SS")
    parts = [int(part) for part in value.split(":")]
    return parts[0] * 3600 + parts[1] * 60 + (parts[2] if len(parts) == 3 else 0)


def same_duration(actual: Any, expected: Any) -> bool:
    try:
        return duration_seconds(actual) == duration_seconds(expected)
    except ValueError:
        return False
