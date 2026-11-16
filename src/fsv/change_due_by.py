"""Reviewed change resolution due date (``changes update --due-by``).

One ``PATCH /api/_/changes/<id>`` of only ``{"due_by": ISO}`` on the shared
reviewed contract: the timestamp must be ISO-8601 with an explicit UTC
offset (checked before any request is planned; a naive time is refused with
the tenant time zone as a hint), the plan hash binds tenant, change, payload,
a stable snapshot of the change and a record-state token, ``--yes`` needs
the full ``--expect-plan``, the send is journaled once, and readback compares
the stored due date with the sent one as instants.
"""
from __future__ import annotations

import json
from datetime import datetime, timezone
from typing import Any

from fsv import config, journal
from fsv.attachments import owner_row
from fsv.readback import same_instant
from fsv.reviewed_snapshot import stable_snapshot
from fsv.reviewed_write import ReviewedWriteError, StatefulWritePlan, execute, prior_guards, stateful_context
from fsv.watchers import positive_id

ACTION = "change_due_by"


class NothingToChange(Exception):
    pass


def _tenant_zone(client: Any) -> str | None:
    try:
        me = client.me()
    except Exception:
        return None
    name = me.get("time_zone") if isinstance(me, dict) else None
    return name if isinstance(name, str) and name.strip() else None


def parse_due_by(value: str, client: Any = None) -> datetime:
    """ISO-8601 date-time with an explicit offset (``Z`` or ``+07:00``)."""
    text = (value or "").strip()
    try:
        parsed = datetime.fromisoformat(text.replace("Z", "+00:00") if text.endswith("Z") else text)
    except ValueError:
        raise ValueError(f"--due-by {value!r} is not an ISO-8601 date-time "
                         "(e.g. 2030-06-09T18:00:00+07:00); nothing was sent") from None
    if "T" not in text and " " not in text:
        raise ValueError(f"--due-by {value!r} has no time of day; give a date-time with an offset; nothing was sent")
    if parsed.tzinfo is None or parsed.utcoffset() is None:
        zone = _tenant_zone(client) if client is not None else None
        hint = f" (the agent profile time zone is {zone})" if zone else ""
        raise ValueError(f"--due-by {value!r} has no UTC offset{hint}; add one, e.g. "
                         f"{parsed.strftime('%Y-%m-%dT%H:%M:%S')}+07:00 or ...Z; nothing was sent")
    return parsed


def build_plan(client: Any, target: int, due_by: str) -> StatefulWritePlan:
    parse_due_by(due_by, client)
    cid = positive_id(target)
    row = owner_row(client, "changes", cid)
    if row.get("deleted") is True:
        raise ReviewedWriteError(f"change {cid} is deleted; nothing was sent")
    if "due_by" in row and same_instant(row.get("due_by"), due_by):
        raise NothingToChange(f"change {cid} is already due by {row.get('due_by')}; nothing to change")
    token = journal.repeat_token((ACTION,), cid, row)
    context, state, prior = stateful_context({"change": stable_snapshot(row)}, token)
    return StatefulWritePlan(ACTION, cid, config.DOMAIN, "PATCH", f"changes/{cid}",
                             json.dumps({"due_by": due_by}, sort_keys=True), context, state, prior)


def review(plan: StatefulWritePlan, client: Any) -> dict[str, Any]:
    out = plan.review()
    out["due_by"] = {"sent": plan.payload["due_by"],
                     "utc": parse_due_by(plan.payload["due_by"]).astimezone(timezone.utc).isoformat()}
    return out


def send(plan: StatefulWritePlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    due_by = plan.payload["due_by"]

    def verify(_response: Any) -> dict[str, Any]:
        row = owner_row(client, "changes", plan.target)
        stored = row.get("due_by")
        matched = "due_by" in row and same_instant(stored, due_by)
        out: dict[str, Any] = {"target": plan.target, "fields": {"due_by": "matched" if matched else
                               "unavailable" if "due_by" not in row else "mismatched"},
                               "due_by": stored, "verified": matched}
        if "manual_dueby" in row:
            out["manual_dueby"] = row["manual_dueby"]
        return out

    return execute(plan, client, reviewed_hash=reviewed_hash,
                   rebuild=lambda: build_plan(client, plan.target, due_by), verify=verify,
                   guards=prior_guards(plan))
