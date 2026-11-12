"""Reviewed problem status transitions and captured closure preconditions."""
from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any

from fsv import config, journal, planning_text, task_status
from fsv.formmeta import ClientResolver, RULES_UNAVAILABLE_KEY, existing_from_record, load_meta
from fsv.forms import FormState
from fsv.problem_workflows import inventory, record
from fsv.readback import submitted_value_matches
from fsv.render import strip_html
from fsv.resources import PROBLEMS, parse_id
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute
from fsv.watchers import positive_id


@dataclass(frozen=True)
class StatusPlan(WritePlan):
    label: str
    before_json: str

    def review(self) -> dict[str, Any]:
        return {**super().review(), "to": self.label, "before": json.loads(self.before_json),
                "side_effects": "Server may auto-fill empty planned_start_date and planned_end_date on status transitions; readback reports these values."}


def closure_context(client: Any, target: int) -> dict[str, Any]:
    rows = planning_text.inventory(client, "problems", target)
    solutions = inventory(client, target, "solutions")
    missing = []
    for label, key in planning_text.PROBLEM_SECTIONS.items():
        matches = [r for r in rows if r.get("name") == key]
        content = planning_text.section_content(matches[0]) if len(matches) == 1 else None
        if not isinstance(content, str) or not strip_html(content).strip():
            missing.append(label.title())
    if not any(not row.get("deleted") and type(row.get("article_type")) is int
               and row["article_type"] in (1, 2) for row in solutions):
        missing.append("linked workaround / solution")
    if missing:
        raise ReviewedWriteError("violated_closure_rules: missing " + ", ".join(missing))
    return {"analysis": rows, "solutions": solutions}


def build_plan(client: Any, target: str | int, to: str) -> StatusPlan:
    target = positive_id(parse_id(str(target), PROBLEMS))
    before = record(client, target)
    if "status" not in before:
        raise ReviewedWriteError("current problem status unavailable")
    meta = load_meta(client, "problem", "edit", fresh=True, module_id=target)
    if meta.rules.get(RULES_UNAVAILABLE_KEY):
        raise ReviewedWriteError("problem rules unavailable")
    status_id, label = task_status.resolve(to, meta.raw_fields)
    state = FormState(meta.spec, meta.engine(), mode="edit", existing=existing_from_record(meta.spec, before), resolver=ClientResolver(client))
    field = meta.spec.field("status")
    state.set(field.api_name, status_id)
    if (state.rules.unavailable_dependencies("edit") or state.evaluated.messages or
            any("unknown field" in w or "none was given" in w for w in state.rules.warnings)):
        raise ReviewedWriteError("problem rules need unavailable context or acknowledgement; use web form")
    if field.api_name not in state.evaluated.visible or state.errors():
        raise ReviewedWriteError("problem form validation failed: " + "; ".join(state.errors()))
    closure = {}
    if label.casefold() == "closed":
        closure = closure_context(client, target)
        missing = []
        for raw in meta.raw_fields:
            if raw.get("required_for_closure") is True:
                name = raw.get("name")
                value = before.get(name, before.get("custom_fields", {}).get(name))
                if value is None or value == "" or value == []:
                    missing.append(str(raw.get("label") or name))
        if missing:
            raise ReviewedWriteError("violated_closure_rules: missing required closure fields: " + ", ".join(missing))
    context = {"before": before, "fields": meta.form_fields, "rules": meta.rules, "closure": closure}
    return StatusPlan("problem_status", target, config.DOMAIN, "PUT", f"problems/{target}",
                      json.dumps({"status": status_id}), journal.snapshot_hash(context) or "", label,
                      json.dumps(before, sort_keys=True))


def rejection(exc: Any) -> str:
    errors = exc.body.get("errors", []) if isinstance(exc.body, dict) else []
    for error in errors if isinstance(errors, list) else []:
        if isinstance(error, dict) and error.get("code") == "violated_closure_rules":
            desc = error.get("description", {})
            names = {"check_analysis_completed": "Root Cause, Impact and Symptoms must be filled",
                     "check_workaround_or_solution_added": "a workaround / solution must be linked"}
            details = [text for key, text in names.items() if isinstance(desc, dict) and key in desc]
            return "violated_closure_rules: " + ("; ".join(details) or "closure prerequisites rejected; inspect analysis and solutions")
    return f"write rejected (HTTP {exc.status}); no retry"


def send(plan: StatusPlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    def verify(_response: Any) -> dict[str, Any]:
        row = record(client, plan.target)
        return {"problem_id": plan.target, "verified": "status" in row and submitted_value_matches(row["status"], plan.payload["status"]),
                "readback": {k: row[k] for k in ("status", "planned_start_date", "planned_end_date") if k in row},
                "unavailable": [k for k in ("planned_start_date", "planned_end_date") if k not in row]}
    return execute(plan, client, reviewed_hash=reviewed_hash,
                   rebuild=lambda: build_plan(client, plan.target, plan.label), verify=verify, rejection=rejection)
