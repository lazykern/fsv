"""Reviewed problem updates (captured: ``PUT /api/_/problems/<id>`` with only
the dirty keys, custom fields nested under ``custom_fields``).

Planned dates and effort (``problem_planning``: e.g. ``{"planned_start_date":
...}``; effort text ``"1h 2m"``) and the other form fields (``problem_update``:
priority, subject, description, group, agent, category and custom fields)
share one contract: form metadata and rules evaluated against the record, a
plan hash over tenant, target, the exact changed-key body and a record-state
token, one journaled send, and an independent readback of every changed key.

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
from fsv.readback import custom_fields_match, same_instant, same_text, submitted_value_matches
from fsv.resources import PROBLEMS, parse_id
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute, prior_guards, stateful_context
from fsv.watchers import positive_id

KEYS = ("planned_start_date", "planned_end_date", "planned_effort")


ACTION = "problem_planning"
UPDATE_ACTION = "problem_update"
# Rich text compared semantically on readback only (the payload is verbatim).
_TEXT_KEYS = frozenset({"description", "description_html"})


class NothingToChange(ReviewedWriteError):
    """Every requested value already matches the record: no plan exists."""


@dataclass(frozen=True)
class PlanningPlan(WritePlan):
    before_json: str
    warnings_json: str = "[]"
    # Record-state token (journal.repeat_token) bound in context_hash, and the
    # context without it (journals written before the token was bound).
    state_json: str = ""
    prior_context: str = ""

    def review(self) -> dict[str, Any]:
        out = {**super().review(), "before": json.loads(self.before_json), "after": self.payload,
               "side_effects": "Tenant workflows may react to planning changes." if self.action == ACTION
               else "Tenant workflows and notifications may react to these changes."}
        if self.state_json:
            out["record_state"] = json.loads(self.state_json)
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
    # Planned dates A -> B -> A read the same values again: bind the server
    # updated_at and the verified planning sends on this problem.
    bound, state, prior = stateful_context(context, journal.repeat_token((ACTION,), target, before_row))
    return PlanningPlan(ACTION, target, config.DOMAIN, "PUT", f"problems/{target}",
                        json.dumps(payload, sort_keys=True), bound,
                        json.dumps(before, sort_keys=True), json.dumps(warnings), state, prior)


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

    return execute(plan, client, reviewed_hash=reviewed_hash, verify=verify, guards=prior_guards(plan),
                   rebuild=lambda: build_plan(client, plan.target, planned_start=inputs.get("planned_start_date"),
                                              planned_end=inputs.get("planned_end_date"),
                                              planned_effort=inputs.get("planned_effort")))


def _same(key: str, actual: Any, expected: Any) -> bool:
    if key in _TEXT_KEYS:
        return same_text(actual, expected)
    return submitted_value_matches(actual, expected)


def _changed_keys(row: dict[str, Any], body: dict[str, Any]) -> tuple[dict[str, Any], dict[str, Any]]:
    """(changed-key body, the record's current values of those keys). A key
    the record does not expose counts as changed (it cannot be proven equal)."""
    changed: dict[str, Any] = {}
    before: dict[str, Any] = {}
    for key, value in body.items():
        if key == "custom_fields":
            current = row.get("custom_fields") if isinstance(row.get("custom_fields"), dict) else {}
            for name, wanted in value.items():
                if name in current and custom_fields_match(current, {name: wanted}):
                    continue
                changed.setdefault("custom_fields", {})[name] = wanted
                before.setdefault("custom_fields", {})[name] = current.get(name)
        elif key in row and _same(key, row[key], value):
            continue
        else:
            changed[key] = value
            before[key] = row.get(key)
    return changed, before


def build_update_plan(client: Any, target: str | int, body: dict[str, Any]) -> PlanningPlan:
    """Reviewed problem field update from an already-resolved wire body
    (top-level keys plus nested ``custom_fields``). Unchanged keys are
    dropped; :class:`NothingToChange` when none remain."""
    target = positive_id(parse_id(str(target), PROBLEMS))
    if not isinstance(body, dict) or not body or (set(body) == {"custom_fields"} and not body["custom_fields"]):
        raise ValueError("nothing to update; pass at least one field")
    if "status" in body:
        raise ReviewedWriteError("problem status changes go through `fsv problems status ID --to STATUS` "
                                 "(a reviewed transition with closure checks); nothing sent")
    if set(body) & set(KEYS):
        raise ReviewedWriteError("problem planned dates/effort are reviewed on their own; send other fields separately")
    before_row = record(client, target)
    if before_row.get("deleted") is True:
        raise ReviewedWriteError(f"problem {target} is deleted (in trash); restore it before updating")
    payload, before = _changed_keys(before_row, body)
    if not payload:
        raise NothingToChange(f"nothing to change: problem {target} already has these values; nothing sent")
    meta = load_meta(client, "problem", "edit", fresh=True, module_id=target, record=before_row)
    wanted = [(k, v) for k, v in payload.items() if k != "custom_fields"] + \
        list((payload.get("custom_fields") or {}).items())
    fields = []
    for key, value in wanted:
        field = meta.spec.find(key)
        if field is None:
            raise ReviewedWriteError(f"problem form metadata has no {key} field; nothing sent")
        fields.append((field, value))
    blockers = rules_gate(meta.rules, meta.spec, [f.api_name for f, _ in fields])
    if blockers:
        raise ReviewedWriteError("problem rules are unavailable and this write depends on them: " + "; ".join(blockers))
    warnings = [w for w in [rules_unavailable_warning(meta.rules)] if w] + list(meta.form_notes)
    state = FormState(meta.spec, meta.engine(), mode="edit", existing=existing_from_record(meta.spec, before_row),
                      resolver=ClientResolver(client))
    for field, value in fields:
        state.set(field.api_name, value)
    for field, _value in fields:
        if field.api_name not in state.evaluated.visible:
            raise ReviewedWriteError(f"{field.label}: hidden on the problem form")
        if field.api_name in state.evaluated.disabled:
            raise ReviewedWriteError(f"{field.label}: locked by a problem form rule")
    gate = state.rule_gate()
    if gate.blockers or state.evaluated.messages:
        raise ReviewedWriteError("problem rules need unavailable context or acknowledgement; use the web form"
                                 + ("".join("; " + b for b in gate.blockers)))
    context = {"before": before, "fields": meta.form_fields, "rules": meta.rules}
    # Priority A -> B -> A reads the same record again: bind the server
    # updated_at and the verified update sends on this problem.
    bound, token, prior = stateful_context(context, journal.repeat_token((UPDATE_ACTION,), target, before_row))
    return PlanningPlan(UPDATE_ACTION, target, config.DOMAIN, "PUT", f"problems/{target}",
                        json.dumps(payload, sort_keys=True, ensure_ascii=False), bound,
                        json.dumps(before, sort_keys=True, ensure_ascii=False), json.dumps(warnings), token, prior)


def update_fields(row: dict[str, Any], payload: dict[str, Any]) -> dict[str, str]:
    """Per changed key: matched, mismatched or unavailable (custom fields as
    ``custom_fields.<name>``)."""
    fields: dict[str, str] = {}
    for key, expected in payload.items():
        if key == "custom_fields":
            current = row.get("custom_fields")
            for name, value in expected.items():
                label = "custom_fields." + name
                if not isinstance(current, dict) or name not in current:
                    fields[label] = "unavailable"
                else:
                    fields[label] = "matched" if custom_fields_match(current, {name: value}) else "mismatched"
        elif key not in row:
            fields[key] = "unavailable"
        else:
            fields[key] = "matched" if _same(key, row[key], expected) else "mismatched"
    return fields


def send_update(plan: PlanningPlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    payload = plan.payload

    def verify(_response: Any) -> dict[str, Any]:
        fields = update_fields(record(client, plan.target), payload)
        return {"problem_id": plan.target, "fields": fields,
                "verified": bool(fields) and all(v == "matched" for v in fields.values())}

    return execute(plan, client, reviewed_hash=reviewed_hash, verify=verify, guards=prior_guards(plan),
                   rebuild=lambda: build_update_plan(client, plan.target, payload))
