"""Reviewed ticket property changes from a blocked UI request."""
from __future__ import annotations

import json
from dataclasses import dataclass, replace
from typing import Any

from fsv import config, journal
from fsv.readback import same_instant
from fsv.change_tasks import _date
from fsv.errors import APIError
from fsv.formmeta import ClientResolver, RULES_UNAVAILABLE_KEY, existing_from_record, is_status, load_meta
from fsv.forms import FormState, build_br_validation_excludes
from fsv.forms.payload import TICKET_KEYS, encode_value
from fsv.resources import TICKETS, parse_id

_ACTION = "ticket_properties"


class PropertyError(RuntimeError):
    pass


def _record(c: Any, ticket_id: int) -> dict[str, Any]:
    raw = c.int_get(f"tickets/{ticket_id}")
    if not isinstance(raw, dict):
        raise PropertyError("ticket read returned an invalid record; no update attempted")
    row = raw.get("ticket", raw)
    if not isinstance(row, dict) or str(row.get("id")) != str(ticket_id):
        raise PropertyError("ticket read did not identify the exact target; no update attempted")
    return row


@dataclass(frozen=True)
class PropertyPlan:
    ticket_id: int
    tenant: str
    body_json: str
    before_json: str
    record_hash: str
    answers_json: str = "{}"
    metadata_hash: str = ""

    @property
    def payload(self) -> dict[str, Any]:
        return json.loads(self.body_json)

    @property
    def before(self) -> dict[str, Any]:
        return json.loads(self.before_json)

    @property
    def hash(self) -> str:
        intent = {"tenant": self.tenant, "ticket_id": self.ticket_id,
                  "payload": self.payload, "before": self.before, "record_hash": self.record_hash}
        if self.metadata_hash:
            intent["metadata_hash"] = self.metadata_hash
        return journal.plan_hash(intent) or ""

    def review(self) -> dict[str, Any]:
        after = {k: v for k, v in self.payload.items() if k != "br_validation_excludes"}
        return {"action": _ACTION, "method": "PUT", "path": f"/api/_/tickets/{self.ticket_id}",
                "before": self.before, "after": after, "payload": self.payload,
                "plan_hash": self.hash, "warning": "Tags replace the full existing list. Updates may trigger tenant workflows."}


def build_plan(c: Any, ticket_id: str | int, *, tags: list[str] | None = None,
               planned_start: str | None = None, planned_end: str | None = None,
               answers: dict[str, Any] | None = None) -> PropertyPlan:
    cid = parse_id(str(ticket_id), TICKETS)
    if cid <= 0:
        raise ValueError("ticket ID must be positive")
    if tags is not None and (not isinstance(tags, list) or not tags or
                             any(not isinstance(t, str) or not t.strip() or t != t.strip() or
                                 any(ord(ch) < 32 or ord(ch) == 127 for ch in t) for t in tags)):
        raise ValueError("tags must be nonempty text without surrounding whitespace or control characters")
    body: dict[str, Any] = {}
    if tags is not None:
        body["tags"] = list(tags)
    for key, raw in (("planned_start_date", planned_start), ("planned_end_date", planned_end)):
        if raw is not None:
            parsed = _date(raw)
            if parsed is None:
                raise ValueError("planned dates cannot be empty")
            body[key] = parsed
    if not body and not answers:
        raise ValueError("specify --set, --tag, --planned-start or --planned-end")
    row = _record(c, cid)
    missing = [k for k in body if k not in row]
    if missing:
        raise PropertyError("ticket read did not include: " + ", ".join(missing))
    before = {k: row[k] for k in body}
    try:
        meta = load_meta(c, "ticket", "edit", fresh=True, module_id=cid)
        if meta.rules.get(RULES_UNAVAILABLE_KEY):
            raise PropertyError("ticket business rules are unavailable; use the web form")
        engine = meta.engine()
        if any(meta.spec.field(name).api_name == "requested_for" for name in answers or {}):
            # Cached rule attributes describe the old beneficiary, not the
            # newly selected person. Missing exact context must block rules.
            engine.context = replace(engine.context, requested_for=None)
        state = FormState(meta.spec, engine, mode="edit", existing=existing_from_record(meta.spec, row),
                          resolver=ClientResolver(c))
        answered = set()
        for name, value in (answers or {}).items():
            field = meta.spec.field(name)
            if field.api_name in answered:
                raise PropertyError(f"{field.label}: duplicate property answer")
            answered.add(field.api_name)
            beneficiary = field.api_name == "requested_for" and field.field_type == "default_requested_for"
            if (not field.sent and not beneficiary) or field.synthetic or field.kind == "notice":
                raise PropertyError(f"{field.label}: field is not writable")
            state.set(field.api_name, value)
        allowed = {"subject", "description", "requester_id", "requested_for_id", "group_id", "responder_id",
                   "status", "priority", "urgency", "impact", "source", "category", "type"}
        for name in answers or {}:
            field = meta.spec.field(name)
            value = state.get(field.api_name)
            if field.api_name not in state.evaluated.visible:
                raise PropertyError(f"{field.label}: answered field is hidden")
            # Clearing an optional routing lookup is a captured explicit null,
            # not a lookup ID to resolve. Validation below still enforces rules.
            if value is None and field.kind in {"lookup_agent", "lookup_group"}:
                encoded = None
            else:
                encoded = encode_value(meta.spec, field, value, state.evaluated.values)
            if is_status(field):
                choice = field.choice(value)
                if choice is None or choice.label.casefold() in {"resolved", "closed"}:
                    raise PropertyError("combined resolution/closure requirements are not proven; use the web form")
            if field.is_custom:
                body.setdefault("custom_fields", {})[field.api_name] = encoded
            else:
                key = field.api_name + "_id" if field.api_name in {"requester", "requested_for"} else TICKET_KEYS.get(field.api_name, field.api_name)
                if key not in allowed:
                    raise PropertyError(f"{field.label}: update wire is not proven")
                body[key] = encoded
        missing = [k for k in body if k not in row]
        if missing:
            raise PropertyError("ticket read did not include: " + ", ".join(missing))
        before = {k: row[k] for k in body}
        for key, value in body.items():
            field = meta.spec.find(key)
            if field is not None:
                state.set(field.api_name, value)
        if state.rules.unavailable_dependencies("edit") or any("unknown field" in w or "none was given" in w for w in state.rules.warnings):
            raise PropertyError("ticket business rules need unavailable fields or context; use the web form")
        problems = state.errors()
        if problems:
            raise PropertyError("ticket form validation failed: " + "; ".join(problems))
        body["br_validation_excludes"] = build_br_validation_excludes(state.evaluated, "ticket")
    except PropertyError:
        raise
    except Exception:
        raise PropertyError("could not evaluate ticket edit form and rules; no update attempted") from None
    return PropertyPlan(cid, config.DOMAIN, json.dumps(body, sort_keys=True, ensure_ascii=False),
                        json.dumps(before, sort_keys=True, ensure_ascii=False), journal.plan_hash(row) or "",
                        json.dumps(answers or {}, sort_keys=True, ensure_ascii=False),
                        meta.fingerprint.hash if answers else "")


def send(plan: PropertyPlan, c: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    if reviewed_hash is None:
        return {"status": "cancelled"}
    if not plan.tenant or plan.tenant != config.DOMAIN or reviewed_hash != plan.hash:
        raise PropertyError("tenant or review hash changed; review again")
    rows = [r for r in journal.read() if r.get("kind") == f"plan:{_ACTION}" and r.get("plan") == plan.hash]
    if rows and rows[-1].get("event") in ("started", "done"):
        raise PropertyError("update was sent or its outcome is unknown; inspect the ticket and journal before retry")
    payload = plan.payload
    fresh = build_plan(c, plan.ticket_id, tags=payload.get("tags"),
                       planned_start=payload.get("planned_start_date"), planned_end=payload.get("planned_end_date"),
                       answers=json.loads(plan.answers_json))
    if fresh.hash != plan.hash:
        raise PropertyError("ticket or form changed since review; review again")
    try:
        csrf = c._get_csrf()
    except Exception:
        raise PropertyError("could not prepare session; no update attempted") from None
    journal.record_plan("started", plan.hash, action=_ACTION, target=plan.ticket_id)
    events = [r for r in journal.read() if r.get("kind") == f"plan:{_ACTION}" and r.get("plan") == plan.hash]
    if not events or events[-1].get("event") != "started":
        raise PropertyError("could not persist journal; no update attempted")
    try:
        c._request("PUT", f"{config.API_INT}/tickets/{plan.ticket_id}",
                   content=plan.body_json.encode("utf-8"),
                   headers={"Content-Type": "application/json", "X-CSRF-Token": csrf})
    except APIError as exc:
        if 400 <= exc.status < 500 and exc.status not in (403, 408, 429):
            journal.record_plan("failed", plan.hash, action=_ACTION, target=plan.ticket_id)
            raise PropertyError(f"update rejected (HTTP {exc.status}); no retry") from None
        raise PropertyError("update outcome unknown; inspect ticket and journal before retry") from None
    except Exception:
        raise PropertyError("update outcome unknown; inspect ticket and journal before retry") from None
    journal.record_plan("done", plan.hash, action=_ACTION, target=plan.ticket_id)
    result: dict[str, Any] = {"status": "submitted", "ticket_id": plan.ticket_id,
                              "plan_hash": plan.hash, "verified": False}
    try:
        row = _record(c, plan.ticket_id)
        fields = {}
        for key, expected in payload.items():
            if key == "br_validation_excludes":
                continue
            actual = row.get(key)
            if key not in row:
                fields[key] = "unavailable"
            elif key in ("planned_start_date", "planned_end_date"):
                fields[key] = "matched" if same_instant(actual, expected) else "mismatched"
            elif key == "custom_fields" and isinstance(actual, dict):
                fields[key] = "matched" if all(k in actual and actual[k] == v for k, v in expected.items()) else "mismatched"
            else:
                fields[key] = "matched" if actual == expected else "mismatched"
        result["fields"] = fields
        result["verified"] = all(v == "matched" for v in fields.values())
    except Exception:
        result["warning"] = "Read-back failed; do not resend."
    if not result["verified"]:
        result["warning"] = "Ticket fields not fully verified on read-back; do not resend."
    return result
