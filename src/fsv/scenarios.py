"""Single-target scenario execution: wire known, side effects not provable."""
from __future__ import annotations

import json
from typing import Any

from fsv import canned, config, journal
from fsv.resources import TICKETS, parse_id
from fsv.reviewed_write import WritePlan, execute
from fsv.ticket_properties import _record
from fsv.watchers import positive_id


def context(client: Any, target: int, scenario_id: int) -> dict[str, Any]:
    return {"scenario": canned.scenario(client, scenario_id), "ticket": _record(client, target)}


def build_plan(client: Any, target: str | int, scenario_id: int) -> WritePlan:
    target = positive_id(parse_id(str(target), TICKETS))
    scenario_id = positive_id(scenario_id)
    payload = {"scenario_id": scenario_id, "ids": [target]}
    return WritePlan("ticket_scenario", target, config.DOMAIN, "PUT", "tickets/execute_scenario",
                     json.dumps(payload, sort_keys=True), journal.snapshot_hash(context(client, target, scenario_id)) or "")


def send(plan: WritePlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    def verify(response: Any) -> dict[str, Any]:
        row = _record(client, plan.target)
        return {"ticket_id": plan.target, "identity_verified": str(row.get("id")) == str(plan.target),
                "verified": False, "warning": "Exact ticket was read back; scenario side effects (including mail/workflows) are unverified. Do not resend."}

    return execute(plan, client, reviewed_hash=reviewed_hash,
                   rebuild=lambda: build_plan(client, plan.target, plan.payload["scenario_id"]), verify=verify)
