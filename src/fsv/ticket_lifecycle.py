"""Reviewed ticket transitions; resolution text persists before the status write."""
from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any

from fsv import config, journal
from fsv.formmeta import ClientResolver, RULES_UNAVAILABLE_KEY, existing_from_record, is_status, load_meta
from fsv.forms import FormState, build_br_validation_excludes
from fsv.forms.payload import encode_value
from fsv.forms.state import is_empty
from fsv.readback import resolution_text, same_text, submitted_value_matches
from fsv.resources import TICKETS, parse_id
from fsv.reviewed_write import ReviewedWriteError
from fsv.ticket_notes import build_plan as note_plan
from fsv.ticket_properties import _record


def context(client: Any, cid: int, status: str, answers: dict[str, Any] | None = None,
            *, resolution: str | None = None) -> tuple[dict[str, Any], str, str]:
    row = _record(client, cid)
    meta = load_meta(client, "ticket", "edit", fresh=True, module_id=cid)
    if meta.rules.get(RULES_UNAVAILABLE_KEY):
        raise ReviewedWriteError("ticket rules unavailable; no write attempted")
    state = FormState(meta.spec, meta.engine(), mode="edit", existing=existing_from_record(meta.spec, row),
                      resolver=ClientResolver(client))
    fields = [f for f in meta.spec.fields if is_status(f)]
    if len(fields) != 1:
        raise ReviewedWriteError("status metadata is not unique")
    field = fields[0]
    choices = [c for c in field.choices if c.visible and status.casefold() in {c.label.casefold(), c.value.casefold()}]
    from fsv.choice_ids import runtime_choice_id
    status_id = runtime_choice_id(choices[0].id) if len(choices) == 1 else None
    if status_id is None:
        raise ReviewedWriteError("status must identify one visible runtime status label")
    choice = choices[0]
    closing = choice.label.casefold() in {"resolved", "closed"}
    state.set(field.api_name, choice.value)
    custom = {}
    seen = set()
    for name, value in (answers or {}).items():
        f = meta.spec.field(name)
        if f.api_name in seen or not f.is_custom or not f.sent or f.synthetic or f.kind == "notice":
            raise ReviewedWriteError(f"{f.label}: --set requires unique writable custom fields")
        seen.add(f.api_name)
        state.set(f.api_name, value)
    if field.api_name not in state.evaluated.visible or state.rules.unavailable_dependencies("edit") or any(
            "unknown field" in w or "none was given" in w for w in state.rules.warnings):
        raise ReviewedWriteError("ticket rules need unavailable fields or context; no write attempted")
    # Closure flags are separate from ordinary edit requiredness. Hidden fields
    # are not requested by the native closure form; conditional rules still run.
    required = set(state.evaluated.required)
    if closing:
        for raw in meta.raw_fields:
            if raw.get("required_for_closure") is True:
                f = meta.spec.find(raw.get("name", ""))
                if f is None:
                    raise ReviewedWriteError("unknown required fields: " + str(raw.get("name") or raw.get("label")))
                required.add(f.api_name)
    missing = [f"{f.label} ({f.api_name})" for f in state.visible_fields()
               if f.api_name in required and (is_empty(state.get(f.api_name)) or
                                             (f.kind == "checkbox" and state.get(f.api_name) is not True))]
    if missing:
        raise ReviewedWriteError("required fields: " + ", ".join(missing) + "; supply --set FIELD=VALUE")
    errors = state.errors()
    if errors:
        raise ReviewedWriteError("ticket form validation failed: " + "; ".join(errors))
    for name in seen:
        f = meta.spec.field(name)
        if name not in state.evaluated.visible:
            raise ReviewedWriteError(f"{f.label}: answered field is hidden")
        custom[name] = encode_value(meta.spec, f, state.get(name), state.evaluated.values)
    if closing and not str(resolution if resolution is not None else resolution_text(row) or "").strip():
        raise ReviewedWriteError("a persisted resolution note is required; use tickets resolve ID --note TEXT first")
    payload = {"status": status_id, "br_validation_excludes": build_br_validation_excludes(state.evaluated, "ticket")}
    # Native transitions preserve routing, including explicit nulls. Never pick
    # an agent/group or replace the rest of the custom-field inventory.
    for key in ("group_id", "responder_id"):
        if key not in row:
            raise ReviewedWriteError(f"ticket routing field unavailable: {key}")
        payload[key] = row[key]
    if custom:
        payload["custom_fields"] = custom
    return payload, json.dumps({"record": journal.snapshot_hash(row), "metadata": meta.fingerprint.hash}, sort_keys=True), choice.label


@dataclass(frozen=True)
class LifecyclePlan:
    target: int
    tenant: str
    status: str
    answers_json: str
    payload_json: str
    context_hash: str
    note_html: str | None

    @property
    def payload(self) -> dict[str, Any]:
        return json.loads(self.payload_json)

    @property
    def hash(self) -> str:
        return journal.plan_hash(self.__dict__) or ""

    def review(self) -> dict[str, Any]:
        steps = []
        if self.note_html is not None:
            steps.append({"method": "PUT", "path": f"/api/_/tickets/{self.target}",
                          "payload": {"resolution_notes": self.note_html,
                                      "br_validation_excludes": self.payload["br_validation_excludes"]}})
        steps.append({"method": "PUT", "path": f"/api/_/tickets/{self.target}", "payload": self.payload})
        return {"action": "ticket_transition", "status_label": self.status, "steps": steps,
                "plan_hash": self.hash, "warning": "Resolution save and transition are separate writes, not atomic. Unknown or partial outcomes must not be resent. Tenant workflows may notify people."}


def build_plan(client: Any, target: str | int, status: str, *, note: str | None = None,
               answers: dict[str, Any] | None = None) -> LifecyclePlan:
    cid = parse_id(str(target), TICKETS)
    if cid <= 0:
        raise ValueError("ticket ID must be positive")
    if note is not None and status.casefold() != "resolved":
        raise ValueError("--note is supported by resolve only")
    html = note_plan(cid, note).body_html if note is not None else None
    payload, fingerprint, label = context(client, cid, status, answers, resolution=html)
    return LifecyclePlan(cid, config.DOMAIN, label, json.dumps(answers or {}, sort_keys=True),
                         json.dumps(payload, sort_keys=True), fingerprint, html)


def matches(row: dict[str, Any], payload: dict[str, Any]) -> bool:
    for key, value in payload.items():
        if key == "br_validation_excludes":
            continue
        if key == "custom_fields":
            actual = row.get(key)
            if not submitted_value_matches(actual, value):
                return False
        elif key not in row or type(row[key]) is bool or row[key] != value:
            return False
    return True


def send(plan: LifecyclePlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    if reviewed_hash is None:
        return {"status": "cancelled", "verified": False}
    if not plan.tenant or config.DOMAIN != plan.tenant or reviewed_hash != plan.hash:
        raise ReviewedWriteError("tenant or full review hash changed; review again")
    action = "ticket_transition"
    events = [r for r in journal.read() if r.get("kind") == "plan:" + action and r.get("plan") == plan.hash]
    if events and events[-1].get("event") in {"started", "done"}:
        raise ReviewedWriteError("transition was sent or outcome unknown; inspect ticket and journal before retry")
    answers = json.loads(plan.answers_json)
    payload, fingerprint, _ = context(client, plan.target, plan.status, answers, resolution=plan.note_html)
    if fingerprint != plan.context_hash or payload != plan.payload:
        raise ReviewedWriteError("ticket or metadata changed; review again")
    csrf = client._get_csrf()
    journal.record_plan("started", plan.hash, action=action, target=plan.target)
    events = [r for r in journal.read() if r.get("kind") == "plan:" + action and r.get("plan") == plan.hash]
    if not events or events[-1].get("event") != "started":
        raise ReviewedWriteError("could not persist ambiguity journal; no write attempted")

    def put(body: dict[str, Any]) -> None:
        client._request("PUT", f"{config.API_INT}/tickets/{plan.target}",
                        content=json.dumps(body, sort_keys=True).encode(),
                        headers={"Content-Type": "application/json", "X-CSRF-Token": csrf})

    try:
        if plan.note_html is not None:
            baseline = _record(client, plan.target)
            if journal.snapshot_hash(baseline) != json.loads(plan.context_hash)["record"]:
                raise ReviewedWriteError("ticket changed before note save")
            put({"resolution_notes": plan.note_html, "br_validation_excludes": payload["br_validation_excludes"]})
            after = _record(client, plan.target)
            if not same_text(resolution_text(after), plan.note_html):
                raise ReviewedWriteError("resolution save was not verified; transition not attempted")
            # A saved note may update timestamps/derived resolution fields, but
            # must not silently change routing, status, or existing answers.
            for key in ("status", "group_id", "responder_id", "custom_fields"):
                if key not in after or after[key] != baseline.get(key):
                    raise ReviewedWriteError("ticket changed during note save; transition not attempted")
            fresh_payload, fresh_context, _ = context(client, plan.target, plan.status, answers)
            if (fresh_payload != payload or json.loads(fresh_context)["metadata"] !=
                    json.loads(plan.context_hash)["metadata"]):
                raise ReviewedWriteError("transition requirements changed after note save")
        put(payload)
        after = _record(client, plan.target)
        verified = matches(after, payload) and (plan.note_html is None or same_text(resolution_text(after), plan.note_html))
    except Exception:
        raise ReviewedWriteError("transition outcome unknown or partial; inspect ticket and journal; do not resend") from None
    if not verified:
        raise ReviewedWriteError("transition readback mismatch; outcome unknown; do not resend")
    journal.record_plan("done", plan.hash, action=action, target=plan.target)
    return {"status": "submitted", "ticket_id": plan.target, "status_label": plan.status,
            "plan_hash": plan.hash, "verified": True}
