"""Typed dates and date-times for the form, with a live preview.

Accepted (case-insensitive; the time zone is the form's, see
``FormState.tz``, unless an offset is typed):

* ``2026-11-26 19:00``, ``2026-11-26T19:00+07:00``, ``2026/11/26 7pm``
* ``26 Nov 2026 19:00``, ``Nov 26 2026 7:30pm``, ``Thu 26 Nov 2026 19:00``
* ``today 18:00``, ``tomorrow 9am``, ``+2d 10:00``, ``+1w 09:00``, ``fri 22:00``
* a time alone (``19:00``) means today

Month and weekday names are matched in English *and* by number, so any
locale can type ``2026-11-26``. The preview never depends on the
machine's locale.
"""
from __future__ import annotations

import calendar
import datetime as dt
import re
from dataclasses import dataclass
from typing import Any

from fsv.forms.tz import localize

MONTHS = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"]
WEEKDAYS = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"]
MONTH_NAMES = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
DAY_NAMES = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
TIME_SUGGESTIONS = ["09:00", "10:00", "13:00", "14:00", "18:00", "19:00", "20:00", "22:00"]

_TIME = re.compile(
    r"(?:^|[\sT]+)(?:at\s+)?(?P<h>\d{1,2})(?::(?P<m>\d{2}))?(?::(?P<s>\d{2}))?\s*(?P<ap>am|pm)?"
    r"\s*(?P<off>z|[+-]\d{2}:?\d{2})?\s*$",
    re.I,
)
_ISO = re.compile(r"^(\d{4})[-/.](\d{1,2})[-/.](\d{1,2})$")
_DMY = re.compile(r"^(\d{1,2})\s*([a-z]+)\.?,?\s*(\d{4})?$", re.I)
_MDY = re.compile(r"^([a-z]+)\.?\s*(\d{1,2}),?\s*(\d{4})?$", re.I)
_REL = re.compile(r"^\+\s*(\d+)\s*([dw])$", re.I)


@dataclass
class Parsed:
    value: Any = None  # dt.date / aware dt.datetime / None (empty)
    preview: str = ""
    error: str = ""

    @property
    def ok(self) -> bool:
        return not self.error


def _month(word: str) -> int | None:
    w = word.casefold()[:3]
    return MONTHS.index(w) + 1 if w in MONTHS else None


def _parse_date(text: str, today: dt.date) -> dt.date | None:
    t = " ".join(text.strip().casefold().replace(",", " ").split())
    if not t:
        return None
    parts = t.split(" ", 1)
    if len(parts) == 2 and parts[0][:3] in WEEKDAYS and not parts[0].isdigit():
        rest = parts[1]
        if re.search(r"\d", rest):
            t = rest  # "thu 26 nov 2026": the weekday is decoration
    if t in ("today", "now"):
        return today
    if t == "tomorrow":
        return today + dt.timedelta(days=1)
    if t == "yesterday":
        return today - dt.timedelta(days=1)
    m = _REL.match(t)
    if m:
        n = int(m.group(1))
        return today + dt.timedelta(days=n * (7 if m.group(2).lower() == "w" else 1))
    if t[:3] in WEEKDAYS and t.isalpha():
        target = WEEKDAYS.index(t[:3])
        ahead = (target - today.weekday()) % 7 or 7
        return today + dt.timedelta(days=ahead)
    m = _ISO.match(t)
    if m:
        return dt.date(int(m.group(1)), int(m.group(2)), int(m.group(3)))
    for rx, order in ((_DMY, "dmy"), (_MDY, "mdy")):
        m = rx.match(t)
        if not m:
            continue
        if order == "dmy":
            day, mon, year = m.group(1), _month(m.group(2)), m.group(3)
        else:
            mon, day, year = _month(m.group(1)), m.group(2), m.group(3)
        if mon is None:
            continue
        if year:
            return dt.date(int(year), mon, int(day))
        cand = dt.date(today.year, mon, int(day))
        return cand if cand >= today else dt.date(today.year + 1, mon, int(day))
    raise ValueError(f"cannot read {text.strip()!r} as a date")


def _split_time(text: str) -> tuple[str, dt.time | None, dt.tzinfo | None]:
    """(date text, time, typed offset). A time needs ``:`` or am/pm."""
    m = _TIME.search(text)
    if not m or (m.group("m") is None and m.group("ap") is None):
        return text, None, None
    h, mi, s = int(m.group("h")), int(m.group("m") or 0), int(m.group("s") or 0)
    ap = (m.group("ap") or "").lower()
    if ap:
        if not 1 <= h <= 12:
            raise ValueError(f"{h}{ap}: the hour must be 1-12 with am/pm")
        h = h % 12 + (12 if ap == "pm" else 0)
    if h > 23 or mi > 59 or s > 59:
        raise ValueError(f"{m.group(0).strip()!r} is not a time of day")
    off = None
    if m.group("off"):
        o = m.group("off").upper()
        if o == "Z":
            off = dt.timezone.utc
        else:
            sign = -1 if o[0] == "-" else 1
            digits = o[1:].replace(":", "")
            off = dt.timezone(sign * dt.timedelta(hours=int(digits[:2]), minutes=int(digits[2:])))
    return text[: m.start()].strip(), dt.time(h, mi, s), off


def offset_text(v: dt.datetime) -> str:
    off = v.utcoffset() or dt.timedelta(0)
    total = int(off.total_seconds() // 60)
    sign = "+" if total >= 0 else "-"
    total = abs(total)
    return f"{sign}{total // 60:02d}:{total % 60:02d}"


def tz_phrase(tz_source: str) -> str:
    s = tz_source or ""
    if s.startswith("Freshservice user time zone"):
        return "from your Freshservice profile tz"
    if s == "FSV_TZ":
        return "from FSV_TZ"
    if s == "tz argument":
        return "from the form's tz"
    return "from this computer's tz (no Freshservice profile tz)"


def date_text(d: dt.date) -> str:
    return f"{DAY_NAMES[d.weekday()]} {d.day} {MONTH_NAMES[d.month - 1]} {d.year}"


def preview(value: Any, tz_source: str = "", typed_offset: bool = False) -> str:
    if isinstance(value, dt.datetime):
        where = "as typed" if typed_offset else tz_phrase(tz_source)
        return f"{date_text(value.date())} {value:%H:%M} ({offset_text(value)} {where})"
    if isinstance(value, dt.date):
        return date_text(value)
    return ""


def parse_when(text: str, *, tz: dt.tzinfo, tz_source: str = "", date_only: bool = False,
               today: dt.date | None = None) -> Parsed:
    """Parse what the person typed; never raises."""
    raw = (text or "").strip()
    if not raw:
        return Parsed()
    today = today or dt.datetime.now(tz).date()
    try:
        date_part, time, off = _split_time(raw)
        if date_only:
            if time is not None:
                return Parsed(error="this field takes a date only (no time)")
            d = _parse_date(date_part, today)
            return Parsed(d, preview(d))
        d = _parse_date(date_part, today) if date_part else (today if time is not None else None)
        if d is None:
            return Parsed(error="type a date and a time, e.g. 2026-11-26 19:00")
        if time is None:
            return Parsed(error=f"{date_text(d)}: add a time, e.g. {d.isoformat()} 19:00")
        if off is not None:
            value = dt.datetime.combine(d, time, tzinfo=off)
        else:  # a daylight-saving gap or overlap is refused with the offsets to pick
            value = localize(dt.datetime.combine(d, time), tz)
        return Parsed(value, preview(value, tz_source, off is not None))
    except ValueError as e:
        msg = str(e)
        if "day is out of range" in msg or "month must be" in msg:
            msg = f"{raw!r} is not a real date"
        return Parsed(error=msg)


def to_text(value: Any, tz: dt.tzinfo | None = None) -> str:
    """The input text for a stored value (round-trips through parse_when to
    the same instant): seconds are kept when set, and the offset is written
    out when it is not what the form zone ``tz`` gives that wall time (or
    when no zone is known)."""
    if isinstance(value, dt.datetime):
        text = f"{value:%Y-%m-%d %H:%M}" + (f":{value.second:02d}" if value.second else "")
        if value.tzinfo is not None:
            local = value.replace(tzinfo=None)
            same = tz is not None and local.replace(tzinfo=tz, fold=value.fold).utcoffset() == value.utcoffset() \
                and _no_dst_doubt(local, tz)
            if not same:
                text += " " + offset_text(value)
        return text
    if isinstance(value, dt.date):
        return value.isoformat()
    return "" if value is None else str(value)


def _no_dst_doubt(local: dt.datetime, tz: dt.tzinfo) -> bool:
    try:
        localize(local, tz)
        return True
    except ValueError:
        return False


def month_weeks(year: int, month: int, firstweekday: int = 0) -> list[list[dt.date]]:
    return calendar.Calendar(firstweekday).monthdatescalendar(year, month)


def add_months(d: dt.date, n: int) -> dt.date:
    m = d.month - 1 + n
    y, m = d.year + m // 12, m % 12 + 1
    return dt.date(y, m, min(d.day, calendar.monthrange(y, m)[1]))
