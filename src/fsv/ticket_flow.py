"""Reviewed, metadata-driven incident creation; no automatic write retry."""
from __future__ import annotations

import json
from typing import Any, Callable

from fsv import change_flow as flow, config, journal
from fsv.drafts import Draft
from fsv.errors import APIError
from fsv.formmeta import RULES_UNAVAILABLE_KEY, apply_answers, is_requester, is_status, rules_warnings
from fsv.forms import FormError, FormState, LookupValue
from fsv.forms.errors import PayloadError
from fsv.forms.payload import build_ticket_payload
from fsv.review import Review


def build_create_plan(meta: Any, draft: Draft, *, resolver: Any, requester: LookupValue | None,
                      ack_all: bool = False, cc_emails: tuple[str, ...] = (), devices: tuple[int, ...] = ()) -> flow.Plan:
    spec = meta.spec
    state = FormState(spec, meta.engine(), mode="new", resolver=resolver)
    answers, status_raw, req_raw, errors = flow._split_special(meta, draft)
    if req_raw not in (None, ""):
        errors.append("requester must be specified with --requester or the draft requester key, not answers")
    if draft.attachments or draft.planning:
        errors.append("ticket attachments and planning fields are not supported")
    req_field = next((f for f in spec.fields if is_requester(f)), None)
    if requester is not None and req_field is not None:
        try:
            state.set(req_field.api_name, requester)
        except FormError as e:
            errors.append(str(e))
    failed: set[str] = set()
    errors.extend(apply_answers(state, answers, failed))
    warnings = list(spec.warnings) + rules_warnings(meta.rules) + flow._acknowledge(state, draft.ack, ack_all)
    sf = next((f for f in spec.fields if is_status(f)), None)
    status = sf.default if sf and sf.api_name in state.evaluated.visible else None
    risks: list[str] = []
    if status_raw not in (None, ""):
        if sf is None or sf.api_name not in state.evaluated.visible:
            errors.append("status is not visible on this form")
        else:
            try:
                status = state.parse(sf, status_raw)
                if status != sf.default:
                    risks.append("Non-default status may trigger workflows or notifications")
            except FormError as e:
                errors.append(str(e))
    skip = {f"{req_field.label} is required"} if req_field else set()
    skip |= {f"{spec.by_name[n].label} is required" for n in failed}
    errors.extend(p for p in state.errors() if p not in skip)
    # Unknown action targets or absent rule context can change which fields
    # are visible/required. Static checks are not a safe substitute.
    if meta.rules.get(RULES_UNAVAILABLE_KEY):
        errors.append("business rules are unavailable; use the web form")
    if (state.rules.unavailable_dependencies("new")
            or any("unknown field" in w or "none was given" in w for w in state.rules.warnings)):
        errors.append("business rules reference unavailable fields or context; use the web form")
    for raw in meta.raw_fields:
        if any(raw.get(k) is True for k in ("deleted", "is_deleted", "deprecated", "is_deprecated")):
            field = spec.find(str(raw.get("name") or ""))
            if field and field.api_name in state.payload_values():
                errors.append(f"{field.label}: deprecated or deleted field cannot be sent")
    if requester is None or not requester.email:
        errors.insert(0, "requester email is required; use --requester or the draft requester key")
    from fsv.ticket_notes import _EMAIL, _valid_recipient
    if not isinstance(cc_emails, (list, tuple)) or any(not isinstance(v, str) or not _valid_recipient(v) or not _EMAIL.fullmatch(v) for v in cc_emails):
        errors.append("CC recipients must be plain email addresses")
    payload = None
    if requester is not None and requester.email:
        try:
            payload = build_ticket_payload(state, requester.email, requester.display, status=status,
                                           allow_incomplete=bool(errors))
            if cc_emails:
                payload["cc_emails"] = list(cc_emails)
        except (FormError, PayloadError) as e:
            errors.extend(getattr(e, "problems", [str(e)]))
    if devices and payload is not None:
        from fsv.watchers import positive_id
        payload["device_ids"] = [str(positive_id(i)) for i in devices]
    review = Review(title="New Incident")
    review.rows.append(("Requester", str(requester) if requester else "(not set)"))
    if cc_emails and not any(not isinstance(v, str) for v in cc_emails):
        review.rows.append(("CC", ", ".join(cc_emails)))
    for name in state.payload_values():
        f = spec.by_name[name]
        if not is_requester(f) and not is_status(f):
            review.rows.append((f.label, state.display(name) + flow._auto_suffix(state, name) + flow._unsent_suffix(f)))
    if sf is not None and status is not None:
        choice = sf.choice(status)
        review.rows.append((f"{sf.label} on create", str(choice.label if choice else status)))
    review.notify = ["Creating an incident may notify its requester and assignees and start workflows."]
    review.messages = flow._messages(state)
    review.notes, review.dropped = flow._notes(state), flow._dropped(state)
    review.risks, review.errors, review.warnings, review.metadata = risks, errors, warnings, meta.as_of()
    plan = flow.Plan("create", meta, state, draft, payload, {"attachments": []}, [], review, errors,
                     requester=requester, risks=risks)
    if payload is not None and not errors:
        plan.hash = flow._hash(payload, plan.files, [])
        review.plan_hash = plan.hash
    return plan


def check_duplicate(plan: flow.Plan, allow_duplicate: bool = False) -> None:
    # An unresolved started event is never overridden by --allow-duplicate.
    rows = [r for r in journal.read() if r.get("kind") == "plan:ticket_create" and r.get("plan") == plan.hash]
    if rows and rows[-1].get("event") == "started":
        raise flow.FlowExit(4, "previous incident create outcome is unknown; check the server before any retry")
    if not allow_duplicate and journal.recent_plan(plan.hash, action="ticket_create"):
        raise flow.FlowExit(4, "this incident plan was already sent; check the server before --allow-duplicate")


def execute(plan: flow.Plan, client: Any, *, allow_duplicate: bool = False) -> dict[str, Any]:
    if not plan.ok or not plan.hash or plan.payload is None:
        raise flow.FlowExit(2, "cannot create an incomplete incident plan")
    check_duplicate(plan, allow_duplicate)
    # Use the lower-level request to submit the exact reviewed bytes once.
    # Resolve CSRF before marking the operation started, as it performs only reads.
    csrf = client._get_csrf()
    before = len(journal.read())
    journal.record_plan("started", plan.hash, action="ticket_create")
    entries = journal.read()
    if len(entries) <= before or entries[-1].get("plan") != plan.hash or entries[-1].get("event") != "started":
        raise flow.FlowExit(5, "could not persist the incident journal; nothing was sent")
    try:
        # content= bypasses the client's verbose JSON body preview, while
        # preserving the exact JSON wire body; plan journal stores its hash.
        result = client._request("POST", f"{config.API_INT}/tickets",
                                 content=json.dumps(plan.payload, ensure_ascii=False, default=str).encode("utf-8"),
                                 headers={"Content-Type": "application/json", "X-CSRF-Token": csrf})
    except APIError as e:
        if 400 <= e.status < 500 and e.status not in (403, 408, 429):
            journal.record_plan("failed", plan.hash, action="ticket_create")
            raise flow.FlowExit(5, f"incident create rejected (HTTP {e.status}); no automatic retry") from None
        raise flow.FlowExit(5, "incident create outcome unknown; check the server before any retry") from None
    except Exception:
        raise flow.FlowExit(5, "incident create outcome unknown; check the server before any retry") from None
    ticket = result.get("ticket", result) if isinstance(result, dict) else None
    cid = ticket.get("id") if isinstance(ticket, dict) else None
    if cid is None:
        raise flow.FlowExit(5, "incident create response has no ID; outcome unknown, check the server before retry")
    journal.record_plan("done", plan.hash, action="ticket_create", created_id=cid)
    result_out: dict[str, Any] = {"id": cid, "result": ticket, "plan_hash": plan.hash,
                                  "steps": [{"step": "create", "ok": True}]}
    if "cc_emails" in plan.payload:
        verification: dict[str, Any] = {"verified": False}
        try:
            raw = client.int_get(f"tickets/{cid}")
            row = raw.get("ticket", raw) if isinstance(raw, dict) else None
            if not isinstance(row, dict) or str(row.get("id")) != str(cid):
                raise ValueError("identity mismatch")
            verification["field"] = ("unavailable" if "cc_emails" not in row else
                                     "matched" if row["cc_emails"] == plan.payload["cc_emails"] else "mismatched")
            verification["verified"] = verification["field"] == "matched"
        except Exception:
            verification["field"] = "unavailable"
        if not verification["verified"]:
            verification["warning"] = "Incident CC was not fully verified on exact-ticket read-back; do not resend."
        result_out["verification"] = verification
    if plan.payload.get("device_ids"):
        from fsv.integrated_assets import verify_devices
        result_out["device_verification"] = verify_devices(client, "tickets", cid, plan.payload["device_ids"])
    return result_out


def send(plan: flow.Plan, client: Any, rebuild: Callable[[Any], flow.Plan], *,
         expected: str | None = None, allow_duplicate: bool = False) -> dict[str, Any]:
    check_duplicate(plan, allow_duplicate)
    new, diff = flow.freshen(plan, client, rebuild)
    if diff is not None:
        raise flow.FlowExit(4, "form metadata changed; review again with --dry-run")
    flow.expect(new, expected)
    return execute(new, client, allow_duplicate=allow_duplicate)
