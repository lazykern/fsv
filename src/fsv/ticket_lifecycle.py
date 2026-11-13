"""Reviewed ticket transitions; resolution text persists before the status write."""
from __future__ import annotations

import json
from dataclasses import dataclass, field as dc_field
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
            *, resolution: str | None = None, row: dict[str, Any] | None = None,
            warnings_out: list[str] | None = None, ack: bool = False) -> tuple[dict[str, Any], str, str]:
    row = _record(client, cid) if row is None else row
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
    if field.api_name not in state.evaluated.visible:
        raise ReviewedWriteError("status is hidden on the ticket form; no write attempted")
    gate = state.rule_gate()
    if gate.blockers:
        raise ReviewedWriteError("ticket rules block this transition; no write attempted: " + "; ".join(gate.blockers))
    if warnings_out is not None:
        warnings_out.extend(gate.warnings)
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
    if ack:
        # Same explicit acknowledgement concept as the create flows' --ack:
        # the reviewer read the blocking (is_validation) rule messages.
        state.acknowledge()
    acknowledged = sorted(m.text for m in state.evaluated.blocking_messages if m.key in state.acknowledged)
    errors = state.errors()
    if errors:
        hint = ("; pass --ack to acknowledge the blocking rule message(s) after reading them"
                if state.unacknowledged() else "")
        raise ReviewedWriteError("ticket form validation failed: " + "; ".join(errors) + hint)
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
    bound = {"record": journal.snapshot_hash(row), "metadata": meta.fingerprint.hash}
    if acknowledged:
        bound["acknowledged"] = acknowledged  # only when used: unacknowledged plan hashes stay stable
    return payload, json.dumps(bound, sort_keys=True), choice.label


@dataclass(frozen=True)
class LifecyclePlan:
    target: int
    tenant: str
    status: str
    answers_json: str
    payload_json: str
    context_hash: str
    note_html: str | None
    # display only: not part of the plan identity (keeps journal hashes stable)
    rule_warnings: tuple[str, ...] = dc_field(default=(), compare=False)

    @property
    def payload(self) -> dict[str, Any]:
        return json.loads(self.payload_json)

    @property
    def hash(self) -> str:
        return journal.plan_hash({k: v for k, v in self.__dict__.items() if k != "rule_warnings"}) or ""

    def review(self) -> dict[str, Any]:
        steps = []
        if self.note_html is not None:
            steps.append({"method": "PUT", "path": f"/api/_/tickets/{self.target}",
                          "payload": {"resolution_notes": self.note_html,
                                      "br_validation_excludes": self.payload["br_validation_excludes"]}})
        steps.append({"method": "PUT", "path": f"/api/_/tickets/{self.target}", "payload": self.payload})
        out = {"action": "ticket_transition", "status_label": self.status, "steps": steps,
                "plan_hash": self.hash, "warning": "Resolution save and transition are separate writes, not atomic. Unknown or partial outcomes must not be resent. Tenant workflows may notify people."}
        if self.rule_warnings:
            out["rule_warnings"] = list(self.rule_warnings)
        acknowledged = self.acknowledged
        if acknowledged:
            out["acknowledged_rule_messages"] = acknowledged
            out["warning"] += (" Acknowledged blocking rule messages are tenant policy: the web form blocks submit on them,"
                               " and the server or a workflow may still reject or revert the transition.")
        return out

    @property
    def acknowledged(self) -> list[str]:
        return list(json.loads(self.context_hash).get("acknowledged") or [])


def build_plan(client: Any, target: str | int, status: str, *, note: str | None = None,
               answers: dict[str, Any] | None = None, ack: bool = False) -> LifecyclePlan:
    cid = parse_id(str(target), TICKETS)
    if cid <= 0:
        raise ValueError("ticket ID must be positive")
    if note is not None and status.casefold() != "resolved":
        raise ValueError("--note is supported by resolve only")
    html = note_plan(cid, note).body_html if note is not None else None
    warnings: list[str] = []
    payload, fingerprint, label = context(client, cid, status, answers, resolution=html, warnings_out=warnings, ack=ack)
    return LifecyclePlan(cid, config.DOMAIN, label, json.dumps(answers or {}, sort_keys=True),
                         json.dumps(payload, sort_keys=True), fingerprint, html, tuple(warnings))


def readback(before: dict[str, Any] | None, row: dict[str, Any], payload: dict[str, Any],
             note_html: str | None) -> tuple[bool, list[str]]:
    """(verified, notes) for a transition readback.

    The status and the resolution note decide the outcome. Server-side
    workflows may rewrite routing, custom fields or even the status right
    after the write: a change away from the pre-write status is accepted as
    verified with a note naming each rewritten key. An unchanged status (or
    a lost note) is a mismatch.
    """
    notes: list[str] = []
    if note_html is not None and not same_text(resolution_text(row), note_html):
        return False, ["resolution note not found on readback"]
    want, got = payload.get("status"), row.get("status")
    if got is None or isinstance(got, bool):
        return False, ["status missing on readback"]
    if str(got) != str(want):
        prior = (before or {}).get("status")
        if prior is None or str(got) == str(prior):
            return False, [f"status is still {got}"]
        notes.append(f"status rewritten server-side to {got} after the write (workflow)")
    for key, value in payload.items():
        if key in ("status", "br_validation_excludes"):
            continue
        actual = row.get(key)
        same = submitted_value_matches(actual, value) if key == "custom_fields" else (
            key in row and type(actual) is not bool and actual == value)
        if not same:
            notes.append(f"{key} differs on readback (server-side workflow or normalization)")
    return True, notes


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
    refusal = journal.replay_refusal(events, "transition was sent or outcome unknown; inspect ticket and journal before retry")
    if refusal:
        raise ReviewedWriteError(refusal)
    answers = json.loads(plan.answers_json)
    before = _record(client, plan.target)
    ack = bool(plan.acknowledged)
    payload, fingerprint, _ = context(client, plan.target, plan.status, answers, resolution=plan.note_html,
                                      row=before, ack=ack)
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

    # Phases for an explicit partial report: the note and the status are
    # separate writes, and tenant workflows/rules can act between them.
    phase = "start"
    try:
        if plan.note_html is not None:
            baseline = _record(client, plan.target)
            if journal.snapshot_hash(baseline) != json.loads(plan.context_hash)["record"]:
                raise ReviewedWriteError("ticket changed before note save")
            phase = "note_sent"
            put({"resolution_notes": plan.note_html, "br_validation_excludes": payload["br_validation_excludes"]})
            after = _record(client, plan.target)
            if not same_text(resolution_text(after), plan.note_html):
                raise ReviewedWriteError("resolution save was not verified; transition not attempted")
            phase = "note_saved"
            # A saved note may update timestamps/derived resolution fields, but
            # must not silently change routing, status, or existing answers.
            changed = [key for key in ("status", "group_id", "responder_id", "custom_fields")
                       if key not in after or after[key] != baseline.get(key)]
            if changed:
                raise ReviewedWriteError("the ticket changed during the note save (" + ", ".join(changed) +
                                         "), typically a tenant workflow or rule")
            fresh_payload, fresh_context, _ = context(client, plan.target, plan.status, answers, ack=ack)
            if (fresh_payload != payload or json.loads(fresh_context)["metadata"] !=
                    json.loads(plan.context_hash)["metadata"]):
                raise ReviewedWriteError("transition requirements changed after the note save")
        phase = "status_sent"
        put(payload)
        after = _record(client, plan.target)
        verified, notes = readback(before, after, payload, plan.note_html)
    except Exception as exc:
        raise ReviewedWriteError(_partial_report(client, plan, phase, exc)) from None
    if not verified:
        if plan.note_html is not None and notes and notes[0].startswith("status is still"):
            raise ReviewedWriteError(
                f"resolution note saved; status unchanged ({_status_text(after)}): the status write was accepted but"
                f" the ticket stayed put, held by a tenant workflow or rule. {_next_step(plan)}")
        raise ReviewedWriteError("transition readback mismatch (" + "; ".join(notes) +
                                 "); outcome unknown; do not resend")
    journal.record_plan("done", plan.hash, action=action, target=plan.target)
    out = {"status": "submitted", "ticket_id": plan.target, "status_label": plan.status,
           "plan_hash": plan.hash, "verified": True}
    if notes:
        out["verified_with_note"] = notes
    return out


def _status_text(row: Any) -> str:
    if not isinstance(row, dict) or row.get("status") is None:
        return "status unavailable on readback"
    name = row.get("status_name")
    return f"readback status {row['status']}" + (f" {name}" if isinstance(name, str) and name else "")


def _next_step(plan: LifecyclePlan) -> str:
    return (f"Next: inspect `fsv tickets get {plan.target}` and `fsv tickets activity {plan.target}` for the workflow/"
            f"approval/rule entry; once it no longer holds the ticket, review the status alone with "
            f"`fsv tickets status {plan.target} {plan.status} --dry-run` (the note is already saved). Do not resend this plan.")


def _partial_report(client: Any, plan: LifecyclePlan, phase: str, exc: Exception) -> str:
    """Explicit outcome for a failure between/after the two transition writes."""
    from fsv.errors import APIError, server_message
    try:
        row = _record(client, plan.target)
    except Exception:  # noqa: BLE001 - the report must not hide the original failure
        row = None
    status = _status_text(row)
    reason = str(exc) if isinstance(exc, ReviewedWriteError) else ""
    if isinstance(exc, APIError) and 400 <= exc.status < 500:
        reason = f"HTTP {exc.status}" + (f": {server_message(exc.body)}" if server_message(exc.body) else "")
    if phase == "note_saved":
        return (f"resolution note saved and verified; status unchanged ({status}): transition not attempted because "
                f"{reason or 'the ticket changed after the note save'}. {_next_step(plan)}")
    if phase == "status_sent" and plan.note_html is not None:
        what = f"the status write was rejected ({reason})" if reason else "the status write outcome is unknown"
        return f"resolution note saved; {what}; {status}. {_next_step(plan)}"
    if phase == "note_sent":
        return (f"resolution note save outcome unknown{' (' + reason + ')' if reason else ''}; transition not attempted;"
                f" {status}. Inspect the ticket and journal; do not resend this plan.")
    return ("transition outcome unknown or partial" + (f" ({reason})" if reason else "") +
            f"; {status}; inspect ticket and journal; do not resend")
