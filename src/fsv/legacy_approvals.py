"""Legacy flat-approval driver (tenants whose ``approval-chains`` answers
400/404, see :mod:`fsv.approval_model`).

CAPTURE-BASED, NOT YET LIVE-PROVEN ON ANY TENANT. The wires come from
blocked (aborted) web submits; the server responses were never observed:

* change request: ``POST changes/<id>/approvals``
  ``{"approval_type": <rule id>, "notified_to": [agent ids]}`` (a CAB is
  expanded client-side through ``GET cabs`` + ``GET changes/<id>/cab-members``;
  no CAB id/name/message on the wire);
* ticket/SR request: ``POST tickets/<id>/approvals``
  ``{"approval_type": 1, "email_content": <GET approvals/approval_template>,
  "notified_to": [requester user ids]}``;
* remind: bodyless ``PUT <module>/<id>/approvals/<approval id>/resend``.

The POST/PUT response is never trusted: every send is single-attempt,
journaled, and verified only by an independent read of the flat
``<module>/<id>/approvals`` list (rows ``id, member_id, level_id,
status{id,name}, type{id,name}, last_reminded_at``). Cancel and approve/reject
have no captured legacy wire and are refused.
"""
from __future__ import annotations

import json
from dataclasses import dataclass
from datetime import datetime
from typing import Any

from fsv import config, journal
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute
from fsv.watchers import positive_id

PROVENANCE = "legacy flat approvals driver: capture-based, not yet live-proven on any tenant"
# Rule ids observed on the legacy model; other rules are unverified there.
RULES = {"everyone": 1, "first-responder": 4}
MODULES = ("changes", "tickets")


def refuse(action: str) -> ReviewedWriteError:
    return ReviewedWriteError(f"{action} refused: this tenant uses legacy flat approvals and no {action} wire was "
                              "captured for that model; use the web UI")


def inventory(client: Any, module: str, target: int) -> list[dict[str, Any]]:
    """The complete flat approvals list; malformed, partial or duplicate rows
    fail closed (never an empty inventory)."""
    if module not in MODULES:
        raise ValueError("unsupported approval module")
    raw = client.int_get(f"{module}/{target}/approvals")
    if isinstance(raw, list):
        raw = {"approvals": raw}
    if not isinstance(raw, dict) or not isinstance(raw.get("approvals"), list):
        raise ReviewedWriteError("flat approvals list unreadable")
    meta = raw.get("meta") if isinstance(raw.get("meta"), dict) else {}
    if any(meta.get(k) for k in ("partial", "has_more", "has_next", "next_page")) or meta.get("complete") is False:
        raise ReviewedWriteError("flat approvals list incomplete")
    ids = []
    for row in raw["approvals"]:
        if not isinstance(row, dict):
            raise ReviewedWriteError("flat approval row unreadable")
        ids.append(positive_id(row.get("id")))
        positive_id(row.get("member_id"))
    if len(set(ids)) != len(ids):
        raise ReviewedWriteError("duplicate flat approval identities")
    return raw["approvals"]


def _status_id(row: dict[str, Any], key: str = "status") -> int | None:
    value = row.get(key)
    ident = value.get("id") if isinstance(value, dict) else None
    return ident if type(ident) is int else None


@dataclass(frozen=True)
class LegacyPlan(WritePlan):
    module: str
    inputs_json: str
    before_json: str  # the baseline flat inventory (verification compares against it)

    def review(self) -> dict[str, Any]:
        inputs = json.loads(self.inputs_json)
        out = {**super().review(), "driver": "legacy", "provenance": PROVENANCE, **inputs,
               "verification": "Independent read of the flat approvals list only; the write response is not trusted.",
               "warning": "Approval emails are irreversible and cannot be recalled (no legacy cancel). "
                          "Single-attempt write; an unknown outcome must be inspected, never resent."}
        return out


def _template(client: Any) -> str:
    raw = client.int_get("approvals/approval_template")
    if isinstance(raw, dict):
        raw = raw.get("approval_template", raw.get("template"))
    if not isinstance(raw, str) or not raw.strip():
        raise ReviewedWriteError("approval email template unavailable; nothing sent")
    return raw


def build_request(client: Any, module: str, target: int, recipients: list[int], *, rule: str = "everyone",
                  recipient_context_json: str = "") -> LegacyPlan:
    """A reviewed legacy approval request. ``recipients`` are agent ids for
    changes (CABs already expanded) and requester user ids for tickets."""
    if module not in MODULES:
        raise ValueError("unsupported approval module")
    target = positive_id(target)
    if rule not in RULES:
        raise ValueError("legacy approvals support only the observed rules: " + ", ".join(RULES))
    ids = sorted(positive_id(i) for i in recipients)
    if not ids or len(set(ids)) != len(ids):
        raise ValueError("at least one distinct explicit recipient is required")
    before = inventory(client, module, target)
    payload: dict[str, Any] = {"approval_type": RULES[rule]}
    if module == "tickets":
        payload["email_content"] = _template(client)  # placeholders are rendered by the server
    payload["notified_to"] = ids
    inputs = {"operation": "request", "rule": rule, "recipients": ids,
              "recipient_sources": json.loads(recipient_context_json or "[]")}
    context = {"before": before, "recipient_sources": inputs["recipient_sources"]}
    return LegacyPlan(f"legacy_approval_request_{module}", target, config.DOMAIN, "POST", f"{module}/{target}/approvals",
                      json.dumps(payload, sort_keys=True, ensure_ascii=False), journal.snapshot_hash(context) or "",
                      module, json.dumps(inputs, sort_keys=True), json.dumps(before, sort_keys=True))


def build_remind(client: Any, module: str, target: int, approval_id: int) -> LegacyPlan:
    if module not in MODULES:
        raise ValueError("unsupported approval module")
    target, approval_id = positive_id(target), positive_id(approval_id)
    before = inventory(client, module, target)
    matches = [r for r in before if positive_id(r["id"]) == approval_id]
    if len(matches) != 1:
        raise ReviewedWriteError("exact approval record unavailable in the flat approvals list")
    if _status_id(matches[0]) != 0:
        raise ReviewedWriteError("only a requested approval (raw status 0) can be reminded")
    inputs = {"operation": "remind", "approval_id": approval_id}
    return LegacyPlan(f"legacy_approval_remind_{module}", target, config.DOMAIN, "PUT",
                      f"{module}/{target}/approvals/{approval_id}/resend", "null",
                      journal.snapshot_hash({"before": before}) or "", module, json.dumps(inputs, sort_keys=True),
                      json.dumps(before, sort_keys=True))


def _instant(value: Any) -> datetime | None:
    if not isinstance(value, str):
        return None
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        return None
    return parsed if parsed.utcoffset() is not None else None


def verify(client: Any, plan: LegacyPlan) -> dict[str, Any]:
    inputs = json.loads(plan.inputs_json)
    before = json.loads(plan.before_json)
    after = inventory(client, plan.module, plan.target)
    old = {positive_id(r["id"]) for r in before}
    if not old <= {positive_id(r["id"]) for r in after}:
        return {"verified": False, "unverified": ["baseline approval records disappeared"], "readback": after}
    if inputs["operation"] == "remind":
        exact = next(r for r in after if positive_id(r["id"]) == inputs["approval_id"])
        prev = next(r for r in before if positive_id(r["id"]) == inputs["approval_id"])
        now, then = _instant(exact.get("last_reminded_at")), _instant(prev.get("last_reminded_at"))
        ok = _status_id(exact) == _status_id(prev) and now is not None and (then is None or now > then)
        return {"verified": ok, "readback": exact, **({} if ok else {"unverified": ["last_reminded_at did not advance"]})}
    fresh = [r for r in after if positive_id(r["id"]) not in old]
    members = sorted(positive_id(r["member_id"]) for r in fresh)
    if members != inputs["recipients"]:
        # never select a matching subset: concurrent or missing records are unknown
        return {"verified": False, "unverified": ["new approval records do not exactly match the recipients"],
                "readback": fresh}
    notes = []
    for r in fresh:
        if _status_id(r) != 0:
            notes.append(f"approval {r['id']}: raw status {_status_id(r)} is not requested (0)")
        if _status_id(r, "type") != RULES[inputs["rule"]]:
            notes.append(f"approval {r['id']}: raw type {_status_id(r, 'type')} does not match the rule")
    return {"verified": not notes, "approval_ids": sorted(positive_id(r["id"]) for r in fresh), "readback": fresh,
            **({"unverified": notes} if notes else {})}


def send(plan: LegacyPlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    inputs = json.loads(plan.inputs_json)

    def rebuild() -> LegacyPlan:
        if inputs["operation"] == "remind":
            return build_remind(client, plan.module, plan.target, inputs["approval_id"])
        return build_request(client, plan.module, plan.target, inputs["recipients"], rule=inputs["rule"],
                             recipient_context_json=json.dumps(inputs["recipient_sources"]) if inputs["recipient_sources"] else "")

    result = execute(plan, client, reviewed_hash=reviewed_hash, rebuild=rebuild, verify=lambda _r: verify(client, plan))
    result["provenance"] = PROVENANCE
    return result
