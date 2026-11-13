"""Reviewed problem status transitions and captured closure preconditions."""
from __future__ import annotations

import datetime as dt
import json
import re
from dataclasses import dataclass
from typing import Any

from fsv import config, journal, planning_text, task_status
from fsv.formmeta import ClientResolver, existing_from_record, load_meta, rules_gate, rules_unavailable_warning
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
    warnings_json: str = "[]"
    # Runtime status labels by ID (not hashed: the metadata is bound through
    # context_hash). Used to check a workflow rewrite against the readback.
    status_labels_json: str = "{}"

    def review(self) -> dict[str, Any]:
        out = {**super().review(), "to": self.label, "before": json.loads(self.before_json),
               "side_effects": "Server may auto-fill empty planned_start_date and planned_end_date on status transitions; readback reports these values."}
        warnings = json.loads(self.warnings_json)
        if warnings:
            out["warnings"] = warnings
        return out


SOLUTION_RULE = "check_workaround_or_solution_added"
SOLUTION_EVIDENCE = ("observed on one tenant: only 2 of 62 closed problems had a linked workaround / solution, "
                     "so this closure rule is tenant configuration, not a platform rule")


def _rules_path() -> Any:
    return config.CACHE_DIR / "forms" / f"{config.cache_domain_key()}--problem-closure-rules.json"


def proven_closure_rules() -> set[str]:
    """Closure-rule codes this tenant has proven by rejecting a closure
    (``violated_closure_rules``); the rules themselves are not readable."""
    try:
        data = json.loads(_rules_path().read_text())
    except (OSError, ValueError):
        return set()
    return {str(x) for x in data.get("codes", [])} if isinstance(data, dict) else set()


def remember_closure_rules(codes: list[str]) -> None:
    if not codes:
        return
    try:
        path = _rules_path()
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({"codes": sorted(proven_closure_rules() | set(codes))}))
    except OSError:
        pass


def closure_context(client: Any, target: int) -> dict[str, Any]:
    """Closure preflight. Missing Root Cause / Impact / Symptoms blocks. A
    missing linked workaround / solution blocks only when this tenant has
    proven that rule (an earlier ``check_workaround_or_solution_added``
    rejection); otherwise it is a review warning and the server decides."""
    rows = planning_text.inventory(client, "problems", target)
    solutions = inventory(client, target, "solutions")
    missing = []
    warnings: list[str] = []
    for label, key in planning_text.PROBLEM_SECTIONS.items():
        matches = [r for r in rows if r.get("name") == key]
        content = planning_text.section_content(matches[0]) if len(matches) == 1 else None
        if not isinstance(content, str) or not strip_html(content).strip():
            missing.append(label.title())
    if not any(not row.get("deleted") and type(row.get("article_type")) is int
               and row["article_type"] in (1, 2) for row in solutions):
        if SOLUTION_RULE in proven_closure_rules():
            missing.append("linked workaround / solution")
        else:
            warnings.append("no linked workaround / solution: some tenants reject closure without one "
                            f"({SOLUTION_RULE}); not blocked because this tenant has not proven that rule "
                            f"({SOLUTION_EVIDENCE}); the server decides")
    if missing:
        raise ReviewedWriteError("violated_closure_rules: missing " + ", ".join(missing))
    return {"analysis": rows, "solutions": solutions, "warnings": warnings}


def build_plan(client: Any, target: str | int, to: str) -> StatusPlan:
    target = positive_id(parse_id(str(target), PROBLEMS))
    before = record(client, target)
    if "status" not in before:
        raise ReviewedWriteError("current problem status unavailable")
    if before.get("deleted") is True:
        raise ReviewedWriteError(f"problem {target} is deleted (in trash); restore it before changing status")
    meta = load_meta(client, "problem", "edit", fresh=True, module_id=target)
    blockers = rules_gate(meta.rules, meta.spec, ["status"])
    if blockers:
        raise ReviewedWriteError("problem rules are unavailable and this write depends on them: " + "; ".join(blockers))
    warnings = [w for w in [rules_unavailable_warning(meta.rules)] if w]
    status_id, label = task_status.resolve(to, meta.raw_fields, kind="problem")
    state = FormState(meta.spec, meta.engine(), mode="edit", existing=existing_from_record(meta.spec, before), resolver=ClientResolver(client))
    field = meta.spec.field("status")
    state.set(field.api_name, status_id)
    gate = state.rule_gate()
    if gate.blockers or state.evaluated.messages:
        raise ReviewedWriteError("problem rules need unavailable context or acknowledgement; use web form"
                                 + ("".join("; " + b for b in gate.blockers)))
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
        warnings += closure.get("warnings", [])
    context = {"before": before, "fields": meta.form_fields, "rules": meta.rules, "closure": closure}
    return StatusPlan("problem_status", target, config.DOMAIN, "PUT", f"problems/{target}",
                      json.dumps({"status": status_id}), journal.snapshot_hash(context) or "", label,
                      json.dumps(before, sort_keys=True), json.dumps(warnings),
                      json.dumps({str(i): name for i, name in task_status.active_choices(meta.raw_fields, "problem")},
                                 sort_keys=True))


def status_readback(before: Any, after: Any, sent: Any) -> tuple[bool, str | None]:
    """(verified, note). A problem workflow may rewrite the status right after
    the write (for example to a monitoring status): a readback status that
    differs from both the sent and the pre-write status is that rewrite, so
    the write is verified with a note. An unchanged status is not verified."""
    if after is None or isinstance(after, bool):
        return False, None
    if submitted_value_matches(after, sent):
        return True, None
    if before is not None and str(after) != str(before):
        return True, f"status rewritten server-side to {after} after the write (workflow)"
    return False, None


_SET_STATUS = re.compile(r"set status as\s*(.+?)\s*(?:,|$)")


def _activity_rows(rows: Any) -> list[tuple[dt.datetime, str, str, str]]:
    """(created_at, content text, sub_contents text, actor name) for parseable rows."""
    out = []
    for row in rows if isinstance(rows, list) else []:
        if not isinstance(row, dict):
            continue
        try:
            at = dt.datetime.fromisoformat(str(row.get("created_at")).replace("Z", "+00:00"))
        except (TypeError, ValueError):
            continue
        if at.tzinfo is None:
            continue
        subs = row.get("sub_contents")
        sub = " , ".join(strip_html(str(s)) for s in subs if isinstance(s, str)) if isinstance(subs, list) else ""
        actor = row.get("actor") if isinstance(row.get("actor"), dict) else {}
        out.append((at, strip_html(str(row.get("content") or "")), sub, str(actor.get("name") or "")))
    return out


def _set_status(text: str) -> str | None:
    found = _SET_STATUS.search(" ".join(text.split()))
    return found.group(1).strip() if found else None


# A workflow triggered by the write can be stamped a moment before the
# write's own activity row (a closure also fills planned dates in the same
# save). Rows older than this before the write's entry are not its evidence.
WORKFLOW_SKEW = dt.timedelta(seconds=5)


def workflow_activity_note(client: Any, target: int, since: dt.datetime, requested: str | None = None) -> str | None:
    found = workflow_activity(client, target, since, requested)
    return found[0] if found else None


def workflow_activity(client: Any, target: int, since: dt.datetime,
                      requested: str | None = None) -> tuple[str, str] | None:
    """(note, label) when the first activity page shows a workflow that set
    the status after the write (for example back to the pre-write status).

    Native rows: the agent write is ``set status as <b>LABEL</b>`` (a closure
    adds ``, set planned end/start date as ...`` to the same row); the
    workflow run is ``executed <a>NAME</a> workflow from ... event`` with the
    status it set in ``sub_contents``. With ``requested``, the activity must
    also show this write's own status entry, and the workflow entry must not
    be older than it (beyond ``WORKFLOW_SKEW``); otherwise the readback stays
    unverified.
    """
    from fsv import service
    from fsv.resources import PROBLEMS
    try:
        rows = _activity_rows(service.get_activities(PROBLEMS, target, client=client))
    except Exception:
        return None
    rows = [r for r in rows if r[0] >= since]
    written = None
    if requested is not None:
        want = " ".join(requested.split()).casefold()
        mine = [at for at, text, _sub, _actor in rows
                if "workflow" not in text.casefold() and (_set_status(text) or "").casefold() == want]
        if not mine:
            return None
        written = max(mine)
    for at, text, sub, actor in rows:
        if "workflow" not in text.casefold() or (written is not None and at < written - WORKFLOW_SKEW):
            continue
        label = _set_status(sub) or _set_status(text)
        if label is None:
            continue
        if requested is None:
            return "status rewritten server-side after the write (activity shows a workflow set it)", label
        return (f"status set to {requested} by this write, then rewritten server-side to {label} "
                f"by a workflow ({actor or 'workflow'}) per the activity log"), label
    return None


def rejection(exc: Any) -> str:
    errors = exc.body.get("errors", []) if isinstance(exc.body, dict) else []
    for error in errors if isinstance(errors, list) else []:
        if isinstance(error, dict) and error.get("code") == "violated_closure_rules":
            desc = error.get("description", {})
            names = {"check_analysis_completed": "Root Cause, Impact and Symptoms must be filled",
                     "check_workaround_or_solution_added": "a workaround / solution must be linked"}
            details = [text for key, text in names.items() if isinstance(desc, dict) and key in desc]
            # the server proved these rules for this tenant: later preflights block on them
            remember_closure_rules([key for key in names if isinstance(desc, dict) and key in desc])
            return "violated_closure_rules: " + ("; ".join(details) or "closure prerequisites rejected; inspect analysis and solutions")
    if exc.status == 405:
        return "write rejected (HTTP 405): the problem is not editable (deleted or in trash?); no retry"
    return f"write rejected (HTTP {exc.status}); no retry"


def send(plan: StatusPlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    def verify(_response: Any) -> dict[str, Any]:
        row = record(client, plan.target)
        verified, note = status_readback(json.loads(plan.before_json).get("status"), row.get("status"),
                                         plan.payload["status"])
        conflict = None
        if not verified and row.get("status") is not None:
            found = workflow_activity(client, plan.target, since, plan.label)
            note = found[0] if found else None
            verified = note is not None
            # The workflow's status must be the server's resulting status
            # (compared by runtime label when the plan carries the labels).
            labels = json.loads(plan.status_labels_json)
            observed = labels.get(str(row.get("status")))
            if found and observed is not None and observed.casefold() != found[1].casefold():
                verified, note = False, None
                conflict = (f"activity shows a workflow setting {found[1]}, but the readback status is {observed}; "
                            "not verified")
        out = {"problem_id": plan.target, "verified": verified,
               "readback": {k: row[k] for k in ("status", "planned_start_date", "planned_end_date") if k in row},
               "unavailable": [k for k in ("planned_start_date", "planned_end_date") if k not in row]}
        if note:
            out["verified_with_note"] = note
        if conflict:
            out["readback_conflict"] = conflict
        return out
    since = dt.datetime.now(dt.timezone.utc) - dt.timedelta(minutes=2)  # tolerate clock skew
    return execute(plan, client, reviewed_hash=reviewed_hash,
                   rebuild=lambda: build_plan(client, plan.target, plan.label), verify=verify, rejection=rejection)
