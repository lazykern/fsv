"""SR approvals: captured group requests, decisions, cancellation and reminders."""
from __future__ import annotations

import html
import json
from dataclasses import dataclass
from typing import Any

from fsv import approval_model, config, journal
from fsv.change_approvals import RULES, ApprovalError, get_change_approval_groups
from fsv.resources import TICKETS, parse_id
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute
from fsv.watchers import positive_id


@dataclass(frozen=True)
class ApprovalPlan(WritePlan):
    inputs_json: str


def inventory(client: Any, target: int) -> list[dict[str, Any]]:
    try:
        return get_change_approval_groups(target, client, module="tickets")
    except ApprovalError as exc:
        raise ReviewedWriteError(str(exc)) from None


def listing(client: Any, target: int) -> dict[str, Any]:
    """Read-only list on either approval model: chain groups when the tenant
    has them, else the flat per-member approvals list (``tickets approvals``)."""
    model, status = approval_model.detect(client, "tickets", target)
    if model != approval_model.LEGACY:
        return {"approval_groups": inventory(client, target)}
    from fsv import service
    from fsv.resources import TICKETS as _T
    return {"model": "legacy", "note": approval_model.describe(status),
            "approvals": service.get_ticket_approvals(_T, target, client=client)}


def _status_id(row: Any) -> int | None:
    """Raw status id of an approval record; names are localisable labels and
    are never compared."""
    status = row.get("status") if isinstance(row, dict) else None
    ident = status.get("id") if isinstance(status, dict) else None
    return ident if type(ident) is int else None


def recipients(client: Any, values: list[str]) -> list[int]:
    ids = []
    for value in values:
        if value.isascii() and value.isdigit():
            ident = positive_id(value)
        else:
            from fsv.change_approvals import _EMAIL
            if not _EMAIL.fullmatch(value):
                raise ValueError("approver must be a positive user ID or exact email")
            rows = client.autocomplete("requesters", value)
            if not isinstance(rows, list):
                raise ReviewedWriteError("requester lookup unavailable")
            matches = [r for r in rows if isinstance(r, dict) and str(r.get("email", "")).casefold() == value.casefold()
                       and not r.get("deleted") and not r.get("deactivated")]
            if len(matches) != 1:
                raise ReviewedWriteError("approver must resolve to one active requester user ID")
            ident = positive_id(matches[0].get("id"))
        if ident in ids:
            raise ValueError("duplicate approver")
        ids.append(ident)
    if not ids:
        raise ValueError("at least one explicit approver required")
    return ids


def build_plan(client: Any, target: str | int, operation: str, *, ident: int | None = None,
               name: str | None = None, message: str | None = None, approvers: list[str] | None = None,
               rule: str = "everyone", remark: str = "") -> ApprovalPlan:
    target = positive_id(parse_id(str(target), TICKETS))
    if operation not in {"request", "approve", "reject", "cancel", "remind"}:
        raise ValueError("unsupported approval action; delegation is unproven")
    if operation == "reject" and not remark.strip():
        raise ValueError("reject requires a nonempty --remark")
    approval_model.require_chains(client, "tickets", target, f"sr approvals {operation}")
    before = inventory(client, target)
    path = f"tickets/{target}"
    if operation == "request":
        if not name or not name.strip() or len(name) > 100 or any(ord(c) < 32 for c in name):
            raise ValueError("nonempty approval name (max 100) required")
        if not message or not message.strip() or rule not in RULES:
            raise ValueError("nonempty message and captured approval rule required")
        content = "<p>" + html.escape(message).replace("\n", "<br>") + "</p>"
        if len(content) > 65530:
            raise ValueError("approval email content exceeds 65530 characters")
        payload = {"approval_groups": [{"name": name, "approval_type": RULES[rule][0], "email_content": content,
                                        "notified_to": recipients(client, approvers or [])}]}
        path += "/approval-groups"
    else:
        ident = positive_id(ident)
        rows = before if operation == "cancel" else [a for g in before for a in g["approvals"]]
        matches = [r for r in rows if positive_id(r["id"]) == ident]
        if len(matches) != 1:
            raise ReviewedWriteError("exact approval/group ID unavailable")
        row = matches[0]
        requested = row.get("approval_status") == 0 if operation == "cancel" else _status_id(row) == 0
        if not requested:
            raise ReviewedWriteError("target is not a requested approval/group; immutable history cannot be changed")
        if operation in {"approve", "reject"}:
            payload = {"approval_status": 1 if operation == "approve" else 2, "remark": remark}
            path += f"/approvals/{ident}"
        else:
            payload = None
            path += f"/approval-groups/{ident}/cancel" if operation == "cancel" else f"/approvals/{ident}/resend"
    inputs = dict(operation=operation, ident=ident, name=name, message=message, approvers=approvers, rule=rule, remark=remark)
    return ApprovalPlan("sr_approval_" + operation, target, config.DOMAIN, "POST" if operation == "request" else "PUT",
                        path, json.dumps(payload, sort_keys=True), journal.snapshot_hash(before) or "", json.dumps(inputs, sort_keys=True))


def send(plan: ApprovalPlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    inputs = json.loads(plan.inputs_json)
    before = inventory(client, plan.target)
    def verify(_response: Any) -> dict[str, Any]:
        after = inventory(client, plan.target)
        operation, ident = inputs["operation"], inputs["ident"]
        if operation == "request":
            old = {positive_id(g["id"]) for g in before}
            fresh = [g for g in after if positive_id(g["id"]) not in old]
            requested = plan.payload["approval_groups"][0]
            matched = len(fresh) == 1 and fresh[0].get("name") == requested["name"]
            if matched:
                group = fresh[0]
                rows = group["approvals"]
                matched = (group.get("approval_type") == requested["approval_type"] and
                           len(rows) == len(requested["notified_to"]) and
                           {positive_id(r["member_id"]) for r in rows} == set(requested["notified_to"]) and
                           all(_status_id(r) == 0 for r in rows))
            return {"verified": bool(matched), "readback": fresh}
        rows = after if operation == "cancel" else [a for g in after for a in g["approvals"]]
        exact = next((r for r in rows if positive_id(r["id"]) == ident), {})
        if operation == "cancel":
            old_group = next(g for g in before if positive_id(g["id"]) == ident)
            matched = (exact.get("approval_status") == 3 and bool(exact.get("approvals")) and
                       {positive_id(r["id"]) for r in exact["approvals"]} == {positive_id(r["id"]) for r in old_group["approvals"]} and
                       all(_status_id(r) == 3 for r in exact["approvals"]))
        elif operation == "remind":
            old = next(r for g in before for r in g["approvals"] if positive_id(r["id"]) == ident)
            from datetime import datetime
            def instant(value):
                if not isinstance(value, str):
                    return None
                dt = datetime.fromisoformat(value.replace("Z", "+00:00"))
                return dt if dt.utcoffset() is not None else None
            now, previous = instant(exact.get("last_reminded_at")), instant(old.get("last_reminded_at"))
            matched = exact.get("status") == old.get("status") and now is not None and (previous is None or now > previous)
        else:
            status_id = plan.payload["approval_status"]
            matched = (_status_id(exact) == status_id and
                       any(isinstance(r, dict) and r.get("status") == status_id and r.get("data") == inputs["remark"] for r in exact.get("remark", [])))
        return {"verified": bool(matched), "readback": exact}
    return execute(plan, client, reviewed_hash=reviewed_hash,
                   rebuild=lambda: build_plan(client, plan.target, **inputs), verify=verify)
