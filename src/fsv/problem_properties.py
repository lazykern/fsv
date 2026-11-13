"""Reviewed problem planned dates and effort (captured: ``PUT /api/_/problems/<id>``
with only the dirty keys, e.g. ``{"planned_start_date": ..., "planned_end_date":
...}``; effort text ``"1h 2m"``).

Allowed only when the tenant's problem form metadata has the field; the
ticket-only guard of ``update --planned-*`` does not apply to problems.
"""
from __future__ import annotations

import json
from dataclasses import dataclass
from datetime import datetime
from typing import Any

from fsv import config, journal
from fsv.change_tasks import _date, _effort, same_effort
from fsv.formmeta import (ClientResolver, existing_from_record, load_meta, rules_gate,
                          rules_unavailable_warning)
from fsv.forms import FormState
from fsv.problem_workflows import record
from fsv.readback import same_instant
from fsv.resources import PROBLEMS, parse_id
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute
from fsv.watchers import positive_id

KEYS = ("planned_start_date", "planned_end_date", "planned_effort")


@dataclass(frozen=True)
class PlanningPlan(WritePlan):
    before_json: str
    warnings_json: str = "[]"

    def review(self) -> dict[str, Any]:
        out = {**super().review(), "before": json.loads(self.before_json), "after": self.payload,
               "side_effects": "Tenant workflows may react to planning changes."}
        warnings = json.loads(self.warnings_json)
        if warnings:
            out["warnings"] = warnings
        return out


def _instant(value: str) -> datetime | None:
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        return None
    return parsed if parsed.utcoffset() is not None else None


def build_plan(client: Any, target: str | int, *, planned_start: str | None = None,
               planned_end: str | None = None, planned_effort: str | None = None) -> PlanningPlan:
    target = positive_id(parse_id(str(target), PROBLEMS))
    payload: dict[str, Any] = {}
    for key, raw, parse in (("planned_start_date", planned_start, _date), ("planned_end_date", planned_end, _date),
                            ("planned_effort", planned_effort, _effort)):
        if raw is not None:
            value = parse(raw)
            if value is None:
                raise ValueError(f"{key} cannot be empty")
            payload[key] = value
    if not payload:
        raise ValueError("specify --planned-start, --planned-end or --planned-effort")
    before_row = record(client, target)
    if before_row.get("deleted") is True:
        raise ReviewedWriteError(f"problem {target} is deleted (in trash); restore it before updating")
    start = payload.get("planned_start_date", before_row.get("planned_start_date"))
    end = payload.get("planned_end_date", before_row.get("planned_end_date"))
    if isinstance(start, str) and isinstance(end, str) and _instant(end) and _instant(start) \
            and _instant(end) < _instant(start):
        raise ValueError("planned end must not precede planned start")
    meta = load_meta(client, "problem", "edit", fresh=True, module_id=target, record=before_row)
    for key in payload:
        if meta.spec.find(key) is None:
            raise ReviewedWriteError(f"problem form metadata has no {key} field; nothing sent")
    blockers = rules_gate(meta.rules, meta.spec, list(payload))
    if blockers:
        raise ReviewedWriteError("problem rules are unavailable and this write depends on them: " + "; ".join(blockers))
    warnings = [w for w in [rules_unavailable_warning(meta.rules)] if w] + list(meta.form_notes)
    state = FormState(meta.spec, meta.engine(), mode="edit", existing=existing_from_record(meta.spec, before_row),
                      resolver=ClientResolver(client))
    for key, value in payload.items():
        field = meta.spec.find(key)
        state.set(field.api_name, value)
        if field.api_name not in state.evaluated.visible:
            raise ReviewedWriteError(f"{field.label}: hidden on the problem form")
        if field.api_name in state.evaluated.disabled:
            raise ReviewedWriteError(f"{field.label}: locked by a problem form rule")
    gate = state.rule_gate()
    if gate.blockers or state.evaluated.messages:
        raise ReviewedWriteError("problem rules need unavailable context or acknowledgement; use the web form"
                                 + ("".join("; " + b for b in gate.blockers)))
    before = {k: before_row.get(k) for k in payload}
    context = {"before": before, "fields": meta.form_fields, "rules": meta.rules}
    return PlanningPlan("problem_planning", target, config.DOMAIN, "PUT", f"problems/{target}",
                        json.dumps(payload, sort_keys=True), journal.snapshot_hash(context) or "",
                        json.dumps(before, sort_keys=True), json.dumps(warnings))


def send(plan: PlanningPlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    inputs = plan.payload

    def verify(_response: Any) -> dict[str, Any]:
        row = record(client, plan.target)
        fields = {}
        for key, expected in inputs.items():
            if key not in row:
                fields[key] = "unavailable"
            elif key == "planned_effort":
                fields[key] = "matched" if same_effort(row[key], expected) else "mismatched"
            else:
                fields[key] = "matched" if same_instant(row[key], expected) else "mismatched"
        return {"problem_id": plan.target, "fields": fields,
                "verified": all(v == "matched" for v in fields.values())}

    return execute(plan, client, reviewed_hash=reviewed_hash, verify=verify,
                   rebuild=lambda: build_plan(client, plan.target, planned_start=inputs.get("planned_start_date"),
                                              planned_end=inputs.get("planned_end_date"),
                                              planned_effort=inputs.get("planned_effort")))
