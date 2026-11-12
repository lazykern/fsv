"""Captured problem creation, body-only notes and self-owned basic tasks."""
from __future__ import annotations

from dataclasses import dataclass
import json
from typing import Any

from fsv import config, journal
from fsv.formmeta import (ClientResolver, RULES_UNAVAILABLE_KEY, apply_answers,
                         find_field, is_requester, load_meta, resolve_requester)
from fsv.forms import FormState
from fsv.forms.payload import _check, _module_body
from fsv.readback import same_text, submitted_value_matches
from fsv.resources import PROBLEMS, parse_id
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute
from fsv.views import collect_pages
from fsv.watchers import positive_id, self_id


@dataclass(frozen=True)
class ProblemPlan(WritePlan):
    inputs_json: str


def record(client: Any, target: int) -> dict[str, Any]:
    raw = client.int_get(f"problems/{target}")
    row = raw.get("problem", raw) if isinstance(raw, dict) else None
    if not isinstance(row, dict) or str(row.get("id")) != str(target):
        raise ReviewedWriteError("exact problem identity is unavailable")
    return row


def inventory(client: Any, target: int, kind: str) -> list[dict[str, Any]]:
    rows, meta, _ = collect_pages(client, f"problems/{target}/{kind}", kind,
                                  {"page": 1, "per_page": 30}, what="problem " + kind)
    if not meta.get("complete"):
        raise ReviewedWriteError("complete problem " + kind + " inventory is unavailable")
    ids = [positive_id(row.get("id")) for row in rows]
    if len(set(ids)) != len(ids):
        raise ReviewedWriteError("duplicate problem child identities")
    return sorted(rows, key=lambda row: positive_id(row["id"]))


def create_plan(client: Any, answers: dict[str, Any], requester: str) -> ProblemPlan:
    meta = load_meta(client, "problem", "new", fresh=True)
    if meta.rules.get(RULES_UNAVAILABLE_KEY):
        raise ReviewedWriteError("problem rules are unavailable; no static-form write")
    person = resolve_requester(client, requester)
    if not person.email:
        raise ValueError("resolved requester needs an email")
    state = FormState(meta.spec, meta.engine(), mode="new", resolver=ClientResolver(client))
    fields = [f for f in meta.spec.fields if is_requester(f)]
    if len(fields) != 1:
        raise ReviewedWriteError("unique problem requester field is unavailable")
    if any(is_requester(find_field(meta.spec, key)) for key in answers):
        raise ReviewedWriteError("requester answers are not allowed; use --requester")
    state.set(fields[0].api_name, person)
    errors = apply_answers(state, answers)
    if errors:
        raise ReviewedWriteError("; ".join(errors))
    if state.rules.unavailable_dependencies("new"):
        raise ReviewedWriteError("problem rules reference unavailable fields or context")
    if state.evaluated.messages:
        raise ReviewedWriteError("problem form has rule messages; use the web form")
    _check(state, False, skip=(fields[0].api_name,))
    top, custom = _module_body(state, {}, ("subject", "description", "status", "priority", "impact"),
                               state.evaluated.values.get("status"))
    top.pop("requester", None)
    if set(top) - {"subject", "description", "status", "priority", "impact"}:
        raise ReviewedWriteError("problem form needs uncaptured top-level create fields")
    if not all(key in top for key in ("subject", "description", "status", "priority", "impact")):
        raise ReviewedWriteError("subject, description, status, priority and impact must be selected from the form")
    payload = {**top, "custom_fields": custom, "email": person.email}
    context = journal.plan_hash({"fields": meta.form_fields, "rules": meta.rules, "requester": str(person)}) or ""
    return ProblemPlan("problem_create", 0, config.DOMAIN, "POST", "problems",
                       json.dumps(payload, sort_keys=True, ensure_ascii=False), context,
                       json.dumps({"answers": answers, "requester": requester}, sort_keys=True))


def _choice(fields: list[dict[str, Any]], name: str, label: str) -> int:
    matches = [f for f in fields if f.get("name") == name]
    if len(matches) != 1:
        raise ReviewedWriteError("unique runtime " + name + " metadata is unavailable")
    choices = [c for c in matches[0].get("choices", []) if isinstance(c, dict) and
               not any(c.get(k) for k in ("deleted", "is_deleted", "deactivated", "deleted_at")) and
               str(c.get("value") or c.get("label") or c.get("name") or "").casefold() == label.casefold()]
    if len(choices) != 1 or type(choices[0].get("id")) is not int or choices[0]["id"] < 0:
        raise ReviewedWriteError("active runtime " + name + " choice is unavailable or ambiguous")
    return choices[0]["id"]


def child_plan(client: Any, target: str | int, *, kind: str, text: str) -> ProblemPlan:
    if kind not in {"note", "task"}:
        raise ValueError("unsupported problem child kind")
    if not isinstance(text, str) or not text.strip():
        raise ValueError("nonempty text is required")
    target = positive_id(parse_id(str(target), PROBLEMS))
    parent = record(client, target)
    rows = inventory(client, target, "notes" if kind == "note" else "tasks")
    context: dict[str, Any] = {"parent": parent, "before": rows}
    if kind == "note":
        # Captured body-only note: do not infer privacy or add notification keys.
        payload = {"body": text}
    else:
        user = self_id(client)
        raw = client.int_get("problem_task_fields")
        fields = next((raw[k] for k in ("problem_task_fields", "task_fields", "fields")
                       if isinstance(raw, dict) and isinstance(raw.get(k), list)), None)
        if fields is None:
            raise ReviewedWriteError("problem task metadata is unavailable")
        payload = {"title": text, "owner_id": user, "status": _choice(fields, "status", "Open"),
                   "notify_before": _choice(fields, "notify_before", "Never"), "planned_effort": "1m"}
        for field in fields:
            if any(field.get(k) is True for k in ("required", "required_for_agents", "required_on_create")) and field.get("name") not in payload:
                raise ReviewedWriteError("problem task has an uncaptured required field")
        context.update(fields=fields, self=user)
    return ProblemPlan("problem_" + kind, target, config.DOMAIN, "POST",
                       f"problems/{target}/" + ("notes" if kind == "note" else "tasks"),
                       json.dumps(payload, sort_keys=True, ensure_ascii=False), journal.snapshot_hash(context) or "",
                       json.dumps({"text": text, "kind": kind}))


def send(plan: ProblemPlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    inputs = json.loads(plan.inputs_json)
    create = plan.action == "problem_create"

    def rebuild() -> ProblemPlan:
        return create_plan(client, **inputs) if create else child_plan(client, plan.target, **inputs)

    def verify(response: Any) -> dict[str, Any]:
        key = "problem" if create else ("note" if inputs["kind"] == "note" else "task")
        raw = response.get(key, response) if isinstance(response, dict) else {}
        exact_id = positive_id(raw.get("id"))
        if create:
            row = record(client, exact_id)
        else:
            rows = inventory(client, plan.target, "notes" if key == "note" else "tasks")
            row = next((r for r in rows if str(r.get("id")) == str(exact_id)), None)
        def matches(key: str, actual: Any, expected: Any) -> bool:
            if create and key == "custom_fields":
                return submitted_value_matches(actual, expected)
            if create and key == "description":
                return same_text(actual, expected)
            return type(actual) is type(expected) and actual == expected

        fields = {k: "unavailable" if row is None or k not in row else
                  "matched" if matches(k, row[k], value) else "mismatched"
                  for k, value in plan.payload.items()}
        return {key + "_id": exact_id, "fields": fields,
                "verified": row is not None and all(v == "matched" for v in fields.values())}

    return execute(plan, client, reviewed_hash=reviewed_hash, rebuild=rebuild, verify=verify)
