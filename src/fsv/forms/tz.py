"""Time-zone resolution for date-time answers without an offset.

Order: explicit ``tz=`` > ``FSV_TZ`` > the Freshservice user's ``time_zone``
(from the business_rules response ``user`` object) > the system time zone.
Freshservice stores Rails/ActiveSupport zone names ("Bangkok", "Eastern Time
(US & Canada)"); they are mapped to IANA names here. Nothing is hardcoded to
one region.
"""
from __future__ import annotations

import datetime as dt
import os
from typing import Any, Mapping
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError

# ActiveSupport::TimeZone::MAPPING (Rails zone name -> IANA).
RAILS_ZONES: dict[str, str] = {
    "International Date Line West": "Etc/GMT+12",
    "Midway Island": "Pacific/Midway",
    "American Samoa": "Pacific/Pago_Pago",
    "Hawaii": "Pacific/Honolulu",
    "Alaska": "America/Juneau",
    "Pacific Time (US & Canada)": "America/Los_Angeles",
    "Tijuana": "America/Tijuana",
    "Mountain Time (US & Canada)": "America/Denver",
    "Arizona": "America/Phoenix",
    "Chihuahua": "America/Chihuahua",
    "Mazatlan": "America/Mazatlan",
    "Central Time (US & Canada)": "America/Chicago",
    "Saskatchewan": "America/Regina",
    "Guadalajara": "America/Mexico_City",
    "Mexico City": "America/Mexico_City",
    "Monterrey": "America/Monterrey",
    "Central America": "America/Guatemala",
    "Eastern Time (US & Canada)": "America/New_York",
    "Indiana (East)": "America/Indiana/Indianapolis",
    "Bogota": "America/Bogota",
    "Lima": "America/Lima",
    "Quito": "America/Lima",
    "Atlantic Time (Canada)": "America/Halifax",
    "Caracas": "America/Caracas",
    "La Paz": "America/La_Paz",
    "Santiago": "America/Santiago",
    "Newfoundland": "America/St_Johns",
    "Brasilia": "America/Sao_Paulo",
    "Buenos Aires": "America/Argentina/Buenos_Aires",
    "Montevideo": "America/Montevideo",
    "Georgetown": "America/Guyana",
    "Puerto Rico": "America/Puerto_Rico",
    "Greenland": "America/Godthab",
    "Mid-Atlantic": "Atlantic/South_Georgia",
    "Azores": "Atlantic/Azores",
    "Cape Verde Is.": "Atlantic/Cape_Verde",
    "Dublin": "Europe/Dublin",
    "Edinburgh": "Europe/London",
    "Lisbon": "Europe/Lisbon",
    "London": "Europe/London",
    "Casablanca": "Africa/Casablanca",
    "Monrovia": "Africa/Monrovia",
    "UTC": "Etc/UTC",
    "Belgrade": "Europe/Belgrade",
    "Bratislava": "Europe/Bratislava",
    "Budapest": "Europe/Budapest",
    "Ljubljana": "Europe/Ljubljana",
    "Prague": "Europe/Prague",
    "Sarajevo": "Europe/Sarajevo",
    "Skopje": "Europe/Skopje",
    "Warsaw": "Europe/Warsaw",
    "Zagreb": "Europe/Zagreb",
    "Brussels": "Europe/Brussels",
    "Copenhagen": "Europe/Copenhagen",
    "Madrid": "Europe/Madrid",
    "Paris": "Europe/Paris",
    "Amsterdam": "Europe/Amsterdam",
    "Berlin": "Europe/Berlin",
    "Bern": "Europe/Zurich",
    "Zurich": "Europe/Zurich",
    "Rome": "Europe/Rome",
    "Stockholm": "Europe/Stockholm",
    "Vienna": "Europe/Vienna",
    "West Central Africa": "Africa/Algiers",
    "Bucharest": "Europe/Bucharest",
    "Cairo": "Africa/Cairo",
    "Helsinki": "Europe/Helsinki",
    "Kyiv": "Europe/Kiev",
    "Kyev": "Europe/Kiev",
    "Riga": "Europe/Riga",
    "Sofia": "Europe/Sofia",
    "Tallinn": "Europe/Tallinn",
    "Vilnius": "Europe/Vilnius",
    "Athens": "Europe/Athens",
    "Istanbul": "Europe/Istanbul",
    "Minsk": "Europe/Minsk",
    "Jerusalem": "Asia/Jerusalem",
    "Harare": "Africa/Harare",
    "Pretoria": "Africa/Johannesburg",
    "Kaliningrad": "Europe/Kaliningrad",
    "Moscow": "Europe/Moscow",
    "St. Petersburg": "Europe/Moscow",
    "Volgograd": "Europe/Volgograd",
    "Samara": "Europe/Samara",
    "Kuwait": "Asia/Kuwait",
    "Riyadh": "Asia/Riyadh",
    "Nairobi": "Africa/Nairobi",
    "Baghdad": "Asia/Baghdad",
    "Tehran": "Asia/Tehran",
    "Abu Dhabi": "Asia/Muscat",
    "Muscat": "Asia/Muscat",
    "Baku": "Asia/Baku",
    "Tbilisi": "Asia/Tbilisi",
    "Yerevan": "Asia/Yerevan",
    "Kabul": "Asia/Kabul",
    "Ekaterinburg": "Asia/Yekaterinburg",
    "Islamabad": "Asia/Karachi",
    "Karachi": "Asia/Karachi",
    "Tashkent": "Asia/Tashkent",
    "Chennai": "Asia/Kolkata",
    "Kolkata": "Asia/Kolkata",
    "Mumbai": "Asia/Kolkata",
    "New Delhi": "Asia/Kolkata",
    "Kathmandu": "Asia/Kathmandu",
    "Astana": "Asia/Dhaka",
    "Dhaka": "Asia/Dhaka",
    "Sri Jayawardenepura": "Asia/Colombo",
    "Almaty": "Asia/Almaty",
    "Novosibirsk": "Asia/Novosibirsk",
    "Rangoon": "Asia/Rangoon",
    "Bangkok": "Asia/Bangkok",
    "Hanoi": "Asia/Bangkok",
    "Jakarta": "Asia/Jakarta",
    "Krasnoyarsk": "Asia/Krasnoyarsk",
    "Beijing": "Asia/Shanghai",
    "Chongqing": "Asia/Chongqing",
    "Hong Kong": "Asia/Hong_Kong",
    "Urumqi": "Asia/Urumqi",
    "Kuala Lumpur": "Asia/Kuala_Lumpur",
    "Singapore": "Asia/Singapore",
    "Taipei": "Asia/Taipei",
    "Perth": "Australia/Perth",
    "Irkutsk": "Asia/Irkutsk",
    "Ulaanbaatar": "Asia/Ulaanbaatar",
    "Seoul": "Asia/Seoul",
    "Osaka": "Asia/Tokyo",
    "Sapporo": "Asia/Tokyo",
    "Tokyo": "Asia/Tokyo",
    "Yakutsk": "Asia/Yakutsk",
    "Darwin": "Australia/Darwin",
    "Adelaide": "Australia/Adelaide",
    "Canberra": "Australia/Melbourne",
    "Melbourne": "Australia/Melbourne",
    "Sydney": "Australia/Sydney",
    "Brisbane": "Australia/Brisbane",
    "Hobart": "Australia/Hobart",
    "Vladivostok": "Asia/Vladivostok",
    "Guam": "Pacific/Guam",
    "Port Moresby": "Pacific/Port_Moresby",
    "Magadan": "Asia/Magadan",
    "Srednekolymsk": "Asia/Srednekolymsk",
    "Solomon Is.": "Pacific/Guadalcanal",
    "New Caledonia": "Pacific/Noumea",
    "Fiji": "Pacific/Fiji",
    "Kamchatka": "Asia/Kamchatka",
    "Marshall Is.": "Pacific/Majuro",
    "Auckland": "Pacific/Auckland",
    "Wellington": "Pacific/Auckland",
    "Nuku'alofa": "Pacific/Tongatapu",
    "Tokelau Is.": "Pacific/Fakaofo",
    "Chatham Is.": "Pacific/Chatham",
    "Samoa": "Pacific/Apia",
}


def zone(name: str | None) -> dt.tzinfo | None:
    """IANA or Rails zone name -> tzinfo (None if unknown/empty)."""
    if not name:
        return None
    name = str(name).strip()
    for cand in (name, RAILS_ZONES.get(name)):
        if not cand:
            continue
        try:
            return ZoneInfo(cand)
        except (ZoneInfoNotFoundError, ValueError):
            continue
    return None


def system_timezone() -> dt.tzinfo:
    """The machine's zone: ``TZ``, then /etc/localtime, then the current offset."""
    tz = zone(os.environ.get("TZ", "").lstrip(":"))
    if tz is not None:
        return tz
    try:
        target = os.path.realpath("/etc/localtime")
        if "zoneinfo/" in target:
            tz = zone(target.split("zoneinfo/", 1)[1])
            if tz is not None:
                return tz
    except OSError:  # pragma: no cover
        pass
    return dt.datetime.now().astimezone().tzinfo or dt.timezone.utc


def resolve_timezone(explicit: str | None = None, user: Mapping[str, Any] | None = None) -> tuple[dt.tzinfo, str]:
    """(tzinfo, where it came from) for date-times entered without an offset."""
    for value, source in ((explicit, "tz argument"), (os.environ.get("FSV_TZ"), "FSV_TZ")):
        if value:
            tz = zone(value)
            if tz is None:
                raise ValueError(f"unknown time zone {value!r} ({source})")
            return tz, source
    if user:
        tz = zone(user.get("time_zone"))
        if tz is not None:
            return tz, f"Freshservice user time zone ({user.get('time_zone')})"
    return system_timezone(), "system time zone"


def _off(v: dt.datetime) -> str:
    off = v.utcoffset() or dt.timedelta(0)
    total = int(off.total_seconds() // 60)
    sign = "+" if total >= 0 else "-"
    total = abs(total)
    return f"{sign}{total // 60:02d}:{total % 60:02d}"


def tz_name(tz: dt.tzinfo) -> str:
    return str(getattr(tz, "key", None) or tz)


def localize(naive: dt.datetime, tz: dt.tzinfo) -> dt.datetime:
    """Attach ``tz`` to a wall-clock time, refusing times a daylight-saving
    change makes ambiguous: a time that does not exist (the clocks skip it)
    or that happens twice (the clocks go back) raises ``ValueError`` naming
    the offsets to choose from. Fixed-offset zones never raise."""
    first = naive.replace(tzinfo=tz, fold=0)
    second = naive.replace(tzinfo=tz, fold=1)
    back = first.astimezone(dt.timezone.utc).astimezone(tz).replace(tzinfo=None)
    when = f"{naive:%Y-%m-%d %H:%M}"
    if back != naive.replace(tzinfo=None):
        raise ValueError(f"{when} does not exist in {tz_name(tz)} (the clocks skip ahead then); "
                         f"type another time, or add an offset ({_off(first)} or {_off(second)})")
    if first.utcoffset() != second.utcoffset():
        raise ValueError(f"{when} happens twice in {tz_name(tz)} (the clocks go back then); "
                         f"add the offset: {_off(first)} for the first, {_off(second)} for the second")
    return first
