"""Strict semantic comparison of server-normalized readback values."""
from __future__ import annotations

from datetime import datetime, timezone
import re
from typing import Any


def same_text(actual: Any, expected: Any) -> bool:
    """Compare rich-text wrappers/entities and whitespace, not missing values."""
    from fsv.views import one_line

    return isinstance(actual, str) and isinstance(expected, str) and one_line(actual) == one_line(expected)


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
