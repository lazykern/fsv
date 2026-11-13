"""Change maintenance windows (captured agent UI wires).

* list: ``GET /api/_/maintenance_windows`` -> ``{"maintenance_windows": [...]}``.
* create: the UI first posts the overlap check
  ``POST /api/_/change_windows/overlapping_windows`` with
  ``{type: "blackout_events", start_date, start_time, end_date, end_time,
  criteria, workspace_id}`` and then
  ``POST /api/_/maintenance_windows`` with ``{"change_window": {name,
  description, start_date "DD-MM-YYYY", start_time "h:mmam", end_date,
  end_time, change_event: {additional_info: {}, criteria: [...]},
  workspace_id, alert_suppression}}`` -> 201 ``{"maintenance_window": {id,...}}``.
  The UI requires at least one "Applicable to Changes" condition; the only
  captured condition is ``{"name": "change_type", "operator": "is",
  "value": "<choice id>"}``.
* delete: ``DELETE /api/_/maintenance_windows/<id>`` -> 200
  ``{"success": true}``; readback is absence from the list.

Associating a window with a change (matching windows) is not implemented.
"""
from __future__ import annotations

import datetime as dt
import json
from dataclasses import dataclass
from typing import Any

from fsv import config, journal
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute
from fsv.watchers import positive_id

OVERLAP_PATH = "change_windows/overlapping_windows"


def list_windows(client: Any) -> list[dict[str, Any]]:
    raw = client.int_get("maintenance_windows")
    rows = raw.get("maintenance_windows") if isinstance(raw, dict) else None
    if not isinstance(rows, list) or not all(isinstance(r, dict) for r in rows):
        raise ReviewedWriteError("maintenance window list unavailable")
    ids = [positive_id(r.get("id")) for r in rows]
    if len(set(ids)) != len(ids):
        raise ReviewedWriteError("maintenance window list has duplicate ids")
    if isinstance(raw.get("meta"), dict) and raw["meta"].get("total_count") not in (None, len(rows)):
        raise ReviewedWriteError("maintenance window list is incomplete; nothing selected")
    return rows


def _when(text: str) -> tuple[str, str]:
    try:
        value = dt.datetime.strptime(text.strip(), "%Y-%m-%d %H:%M")
    except ValueError:
        raise ReviewedWriteError(f"window time {text!r} must be 'YYYY-MM-DD HH:MM' (tenant time zone)") from None
    hour = value.hour % 12 or 12
    return value.strftime("%d-%m-%Y"), f"{hour}:{value.minute:02d}{'am' if value.hour < 12 else 'pm'}"


def _change_type_criterion(sch: dict[str, Any], expr: str) -> dict[str, str]:
    name, sep, raw = expr.partition("=")
    if not sep or name.strip().casefold() not in {"change_type", "change type", "type"}:
        raise ReviewedWriteError("only the captured condition 'change_type=<label or id>' is supported")
    field = next((f for f in sch.get("fields") or [] if f.get("name") == "change_type"), None)
    if not field:
        raise ReviewedWriteError("change_type field unavailable in change form metadata")
    wanted = raw.strip()
    found = [c for c in field.get("choices") or [] if isinstance(c, dict) and
             (str(c.get("id")) == wanted or str(c.get("value") or c.get("name") or "").casefold() == wanted.casefold())]
    ids = {positive_id(c.get("id")) for c in found}
    if len(ids) != 1:
        shown = ", ".join(str(c.get("value") or c.get("id")) for c in field.get("choices") or [] if isinstance(c, dict))
        raise ReviewedWriteError(f"no unique change type {wanted!r}; options: {shown}")
    return {"name": "change_type", "operator": "is", "value": str(ids.pop())}


@dataclass(frozen=True)
class WindowPlan:
    write: WritePlan
    overlap_json: str

    @property
    def hash(self) -> str:
        return self.write.hash

    def review(self) -> dict[str, Any]:
        return {**self.write.review(), "overlap_check": {"method": "POST", "path": "/api/_/" + OVERLAP_PATH,
                                                         "payload": json.loads(self.overlap_json)},
                "warning": "The overlap check is sent first; any overlapping window refuses the create unless "
                           "--allow-overlap. Single attempt; never resend an unknown outcome."}


def create_plan(client: Any, name: str, description: str, start: str, end: str, criteria: list[str], *,
                alert_suppression: bool = True) -> WindowPlan:
    from fsv import schema as schema_mod
    from fsv.query import runtime_workspace
    from fsv.resources import CHANGES
    if not name.strip():
        raise ReviewedWriteError("window name is required")
    if not criteria:
        raise ReviewedWriteError("at least one --criteria change_type=<type> row is required "
                                 "(the window's 'Applicable to Changes' condition)")
    sd, st = _when(start)
    ed, et = _when(end)
    if dt.datetime.strptime(end, "%Y-%m-%d %H:%M") <= dt.datetime.strptime(start, "%Y-%m-%d %H:%M"):
        raise ReviewedWriteError("window end must be after its start")
    sch = schema_mod.load(CHANGES, client)
    rows = [_change_type_criterion(sch, c) for c in criteria]
    ws = positive_id(runtime_workspace(sch)) if runtime_workspace(sch) is not None else None
    if ws is None:
        raise ReviewedWriteError("workspace unavailable; set forms.workspace_id")
    payload = {"change_window": {"name": name, "description": description, "start_date": sd, "start_time": st,
                                 "end_date": ed, "end_time": et,
                                 "change_event": {"additional_info": {}, "criteria": rows},
                                 "workspace_id": ws, "alert_suppression": alert_suppression}}
    overlap = {"type": "blackout_events", "start_date": sd, "start_time": st, "end_date": ed, "end_time": et,
               "criteria": rows, "workspace_id": ws}
    context = journal.snapshot_hash({"windows": list_windows(client), "change_type": [
        f for f in sch.get("fields") or [] if f.get("name") == "change_type"]}) or ""
    write = WritePlan("change_window_create", 0, config.DOMAIN, "POST", "maintenance_windows",
                      json.dumps(payload, sort_keys=True), context)
    return WindowPlan(write, json.dumps(overlap, sort_keys=True))


_FLAG_WORDS = ("overlap", "conflict")
# Exact boolean keys whose names do not contain a flag word. ``match_found``
# is the tenant's real answer to overlapping_windows (VERIFIED LIVE: with no
# windows at all it answered {"match_found": false}).
_FLAG_KEYS = frozenset({"match_found"})
_COUNT_KEYS = frozenset({"count", "total", "total_count", "overlap_count", "overlapping_count", "conflict_count"})
_UNSPECIFIED = {"overlap": "reported without details"}


def _shape(value: Any, depth: int = 0) -> str:
    """Type-only outline of an answer (never values) for refusal messages."""
    if isinstance(value, dict) and depth < 2:
        inner = ", ".join(f"{k}: {_shape(v, depth + 1)}" for k, v in list(value.items())[:8])
        return "{" + inner + ("" if len(value) <= 8 else ", ...") + "}"
    return {dict: "object", list: "list", bool: "bool", str: "text", type(None): "null"}.get(type(value), "number")


def _overlaps(response: Any) -> list[Any]:
    """Conflicting windows from the overlap-check answer.

    Accepted shapes: the native ``{"match_found": bool}`` answer; a list of
    windows; ``true``/``false`` (JSON or text); an
    object whose values are window lists (one or several, e.g. per window
    kind, optionally one level nested), overlap/conflict boolean flags and
    numeric counts; an empty object (nothing reported). A flag or count that
    reports an overlap without rows yields one placeholder conflict. Anything
    else, or flags/counts contradicting the lists, raises.
    """
    if isinstance(response, str) and response.strip().casefold() in ("true", "false"):
        response = response.strip().casefold() == "true"
    if isinstance(response, bool):
        return [_UNSPECIFIED] if response else []
    if isinstance(response, list):
        return response
    rows: list[Any] = []
    flags: list[bool] = []

    def walk(obj: dict[str, Any], depth: int) -> bool:
        for key, value in obj.items():
            word = str(key).casefold()
            if isinstance(value, list):
                rows.extend(value)
            elif isinstance(value, bool) and (word in _FLAG_KEYS or any(w in word for w in _FLAG_WORDS)):
                flags.append(value)
            elif (isinstance(value, int) and not isinstance(value, bool) and value >= 0
                  and (word in _COUNT_KEYS or any(w in word for w in _FLAG_WORDS))):
                flags.append(value > 0)
            elif isinstance(value, dict) and depth == 0:
                if not walk(value, 1):
                    return False
            else:
                return False
        return True

    if isinstance(response, dict) and walk(response, 0):
        if any(flags) and not rows:
            return [_UNSPECIFIED]
        if rows and flags and not any(flags):
            raise ReviewedWriteError("overlap check answer is contradictory (no-overlap flag with windows listed); "
                                     "nothing was created")
        return rows
    raise ReviewedWriteError(f"overlap check answer not understood (shape {_shape(response)}); nothing was created. "
                             "Check `fsv changes windows list` yourself; --allow-overlap skips this check")


def send_create(plan: WindowPlan, client: Any, *, reviewed_hash: str | None, allow_overlap: bool = False,
                rebuild: Any = None) -> dict[str, Any]:
    if reviewed_hash is None:
        return {"status": "cancelled", "verified": False}
    events = [e for e in journal.read() if e.get("kind") == "plan:" + plan.write.action and e.get("plan") == plan.hash]
    refusal = journal.replay_refusal(events, "window create was sent or outcome unknown; inspect the window list, do not replay")
    if refusal:
        raise ReviewedWriteError(refusal)
    if reviewed_hash != plan.hash or (rebuild is not None and rebuild().hash != plan.hash):
        raise ReviewedWriteError("windows or metadata changed since review; review again (nothing was sent)")
    csrf = client._get_csrf()
    answer = client._request("POST", f"{config.API_INT}/{OVERLAP_PATH}", content=plan.overlap_json.encode(),
                             headers={"Content-Type": "application/json", "X-CSRF-Token": csrf})
    try:
        conflicts: list[Any] | None = _overlaps(answer)
    except ReviewedWriteError:
        if not allow_overlap:
            raise
        conflicts = None  # explicitly accepted: overlaps unknown
    if conflicts and not allow_overlap:
        raise ReviewedWriteError(f"{len(conflicts)} overlapping window(s); nothing was created "
                                 "(review them, then pass --allow-overlap)")
    name = plan.write.payload["change_window"]["name"]

    def verify(response: Any) -> dict[str, Any]:
        row = response.get("maintenance_window") if isinstance(response, dict) else None
        wid = positive_id(row.get("id") if isinstance(row, dict) else None)
        listed = [r for r in list_windows(client) if positive_id(r.get("id")) == wid]
        state = "matched" if len(listed) == 1 and listed[0].get("name") == name else "mismatched"
        return {"window_id": wid, "fields": {"name": state}, "overlaps": "unknown" if conflicts is None else len(conflicts), "verified": state == "matched"}

    return execute(plan.write, client, reviewed_hash=reviewed_hash, rebuild=lambda: plan.write, verify=verify)


def delete_plan(client: Any, ident: int | str) -> WritePlan:
    wid = positive_id(int(ident) if str(ident).isdigit() else ident)
    rows = [r for r in list_windows(client) if positive_id(r.get("id")) == wid]
    if len(rows) != 1:
        raise ReviewedWriteError(f"maintenance window {wid} not found")
    return WritePlan("change_window_delete", wid, config.DOMAIN, "DELETE", f"maintenance_windows/{wid}", "null",
                     journal.snapshot_hash({"window": rows[0]}) or "")


def send_delete(plan: WritePlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    def verify(_response: Any) -> dict[str, Any]:
        gone = all(positive_id(r.get("id")) != plan.target for r in list_windows(client))
        return {"window_id": plan.target, "fields": {"absent": "matched" if gone else "mismatched"}, "verified": gone}
    return execute(plan, client, reviewed_hash=reviewed_hash, rebuild=lambda: delete_plan(client, plan.target),
                   verify=verify)
