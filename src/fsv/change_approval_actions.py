"""Reviewed approval decisions and cancellation using complete chain identity evidence.

Approval and cancellation were accepted live; rejection was captured aborted.
Read-back reports raw status evidence, never mappings inferred from write values.
"""
from __future__ import annotations

from fsv.tenant import tenant_json

import json
from dataclasses import dataclass
from typing import Any

import typer

from fsv import config, journal
from fsv.change_approvals import (ApprovalError, LegacyChainsError, _id, _inventory, _require_chains,
                                  get_change_approval_groups)
from fsv.client import get_client
from fsv.errors import APIError
from fsv.resources import CHANGES, parse_id

_WARNING = ("Approval actions notify people; approval/rejection may move the change workflow. "
            "Cancellation is irreversible. Review the exact tenant and identifiers.")
_UNKNOWN = "approval action outcome unknown; inspect the change and journal before any retry; do not resubmit"
# The decision/cancel wires were proven on the chains model only.
_COMMAND = {"approve": "approval-respond --approve", "reject": "approval-respond --reject", "cancel": "approval-cancel"}


@dataclass(frozen=True)
class ActionPlan:
    change_id: int
    target_id: int
    operation: str
    remark: str | None
    tenant: str

    @property
    def action(self) -> str:
        return "change_approval_cancel" if self.operation == "cancel" else "change_approval_respond"

    @property
    def path(self) -> str:
        suffix = (f"approval-groups/{self.target_id}/cancel" if self.operation == "cancel"
                  else f"approvals/{self.target_id}")
        return f"changes/{self.change_id}/{suffix}"

    @property
    def payload(self) -> dict[str, Any] | None:
        if self.operation == "cancel":
            return None  # NO body, not JSON null or {}.
        body: dict[str, Any] = {"approval_status": 1 if self.operation == "approve" else 2}
        if self.remark is not None:
            body["remark"] = self.remark
        return body

    @property
    def hash(self) -> str:
        return journal.plan_hash({"tenant": self.tenant, "method": "PUT", "path": self.path,
                                  "payload": self.payload}) or ""

    def review(self) -> dict[str, Any]:
        result = {"action": self.action, "operation": self.operation, "tenant": self.tenant,
                  "change_id": self.change_id, "method": "PUT", "path": f"/api/_/{self.path}",
                  "payload": self.payload, "body_present": self.payload is not None,
                  "plan_hash": self.hash, "warning": _WARNING,
                  "contract": "Approve accepted live (requested=0, approved=1); reject captured aborted (read-back mapping unobserved).",
                  "verification": "Complete approvals list; exact record ID and changed raw status. Named decision checked if exposed; no numeric mapping inferred."}
        result["group_id" if self.operation == "cancel" else "approval_id"] = self.target_id
        if self.operation == "cancel":
            result.update(send_enabled=True,
                          contract="Bodyless JSON-media-type cancel accepted live; chains exposed cancelled status {id:3,name:cancelled}.",
                          verification="Complete approval chains: exact group disappears or all original approvals change away from requested; report raw statuses.")
        return result


def build_plan(change_id: int | str, target_id: int | str, operation: str,
               remark: str | None = None) -> ActionPlan:
    if isinstance(change_id, bool) or not isinstance(change_id, (str, int)):
        raise ValueError("change ID must be a positive display ID")
    cid = parse_id(str(change_id), CHANGES)
    if cid <= 0:
        raise ValueError("change ID must be positive")
    tid = _id(target_id)
    if tid is None:
        raise ValueError("approval record/group ID must be a positive integer")
    if operation not in ("approve", "reject", "cancel"):
        raise ValueError("choose exactly one of approve, reject or cancel")
    if remark is not None and (not isinstance(remark, str) or any(
            (ord(c) < 32 and c not in "\r\n\t") or ord(c) == 127 for c in remark)):
        raise ValueError("remark must be plain text without unsafe control characters")
    if operation == "reject" and (remark is None or not remark.strip()):
        raise ValueError("reject requires a nonempty --remark")
    if operation == "cancel" and remark is not None:
        raise ValueError("cancel sends no body and does not accept a remark")
    return ActionPlan(cid, tid, operation, remark, config.DOMAIN)


def _status(row: dict[str, Any]) -> Any:
    """Validate raw status without inventing a decision ID-to-name mapping."""
    value = row.get("status")
    if type(value) is int and value >= 0:
        return value
    if (isinstance(value, dict) and type(value.get("id")) is int and value["id"] >= 0
            and ("name" not in value or isinstance(value["name"], str) and value["name"].strip())):
        return value
    raise ApprovalError("approval status unreadable")


def _requested(status: Any) -> bool:
    return status == 0 or isinstance(status, dict) and (status["id"] == 0 or status.get("name") == "requested")


def _cancel_group(client: Any, plan: ActionPlan) -> dict[str, Any]:
    try:
        groups = get_change_approval_groups(plan.change_id, client)
    except LegacyChainsError as exc:
        raise exc.refusal("approval-cancel") from None
    except Exception:
        raise ApprovalError("could not read complete approval chains; no cancel PUT attempted") from None
    group = next((g for g in groups if _id(g["id"]) == plan.target_id), None)
    if group is None:
        raise ApprovalError("approval group missing from complete chains inventory; no cancel PUT attempted")
    if not group["approvals"] or not any(_requested(_status(r)) for r in group["approvals"]):
        raise ApprovalError("approval group has no requested approval records; no cancel PUT attempted")
    return group


def _verify_cancel(client: Any, plan: ActionPlan, before: dict[str, Any]) -> dict[str, Any]:
    groups = get_change_approval_groups(plan.change_id, client)
    after = next((g for g in groups if _id(g["id"]) == plan.target_id), None)
    statuses = None
    if after is not None:
        baseline = {_id(r["id"]): r for r in before["approvals"]}
        current = {_id(r["id"]): r for r in after["approvals"]}
        if baseline.keys() != current.keys():
            raise ApprovalError(_UNKNOWN)
        for rid, row in current.items():
            if _id(row["member_id"]) != _id(baseline[rid]["member_id"]) or _requested(_status(row)):
                raise ApprovalError(_UNKNOWN)
        if not any(_status(current[rid]) != _status(row) for rid, row in baseline.items()):
            raise ApprovalError(_UNKNOWN)
        statuses = [{"approval_id": r["id"], "member_id": r["member_id"], "status": _status(r)}
                    for r in after["approvals"]]
    return {"status": "submitted", "operation": "cancel", "change_id": plan.change_id,
            "group_id": plan.target_id, "verified": True,
            "verification_basis": "group_gone" if after is None else "changed_raw_statuses_only",
            "before_status": [{"approval_id": r["id"], "member_id": r["member_id"], "status": _status(r)}
                              for r in before["approvals"]],
            "observed_status": statuses, "readback": after, "plan_hash": plan.hash,
            "warning": "Observed group disappearance/status transition only; cancellation status names and notification delivery are not inferred."}


def send(plan: ActionPlan, client: Any = None, *, reviewed_hash: str | None,
         read_only: bool = False, dry_run: bool = False) -> dict[str, Any]:
    if build_plan(plan.change_id, plan.target_id, plan.operation, plan.remark) != plan:
        raise ApprovalError("approval action plan or tenant changed; review a new plan")
    if dry_run:
        review = plan.review()
        if plan.operation == "cancel":
            review["group"] = _cancel_group(client if client is not None else get_client(), plan)
        return {"status": "dry_run", **review}
    if read_only:
        raise ApprovalError("read-only mode: no approval action was sent")
    if reviewed_hash is None:
        return {"status": "cancelled", "change_id": plan.change_id}
    if reviewed_hash != plan.hash:
        raise ApprovalError("approval action review hash does not match; review the exact action again")
    if not plan.tenant:
        raise ApprovalError("configure a tenant before an approval action")
    def events() -> list[dict[str, Any]]:
        return [r for r in journal.read() if r.get("kind") == f"plan:{plan.action}" and r.get("plan") == plan.hash]

    prior = events()
    if prior and prior[-1].get("event") in ("started", "done"):
        raise ApprovalError("this approval action was sent or its outcome is unknown; inspect the change and journal; do not resubmit")
    c = client if client is not None else get_client()
    if plan.operation != "cancel":  # cancel reads the chains itself (_cancel_group)
        _require_chains(c, plan.change_id, _COMMAND[plan.operation])
    try:
        if plan.operation == "cancel":
            before = _cancel_group(c, plan)
            before_rows = before["approvals"]
        else:
            before_rows = _inventory(c, plan)
        if plan.operation != "cancel":
            before = next(r for r in before_rows if _id(r["id"]) == plan.target_id)
            before_status = _status(before)
            if before_status != {"id": 0, "name": "requested"}:
                raise ApprovalError("target approval is not requested")
            member_id = _id(before.get("member_id"))
            if member_id is None:
                raise ApprovalError("approval member identity unavailable")
            member = before.get("member")
            if member is not None and (not isinstance(member, dict) or _id(member.get("id")) != member_id):
                raise ApprovalError("approval member identity conflicting")
            if "change_id" in before and _id(before["change_id"]) != plan.change_id:
                raise ApprovalError("approval change identity conflicting")
        csrf = c._get_csrf()
    except ApprovalError:
        if plan.operation == "cancel":
            raise
        raise ApprovalError("could not prepare complete approvals inventory/session with the exact requested record; no approval PUT attempted") from None
    except Exception:
        raise ApprovalError("could not prepare complete approvals inventory/session with the exact requested record; no approval PUT attempted") from None
    journal.record_plan("started", plan.hash, action=plan.action, target=plan.change_id)
    recorded = events()
    if not recorded or recorded[-1].get("event") != "started":
        raise ApprovalError("could not persist approval action journal; no approval PUT attempted")
    try:
        # Never int_put: it can replay on 403. The response body is not evidence.
        c._request("PUT", f"{config.API_INT}/{plan.path}",
                   **({"content": json.dumps(plan.payload, ensure_ascii=False).encode("utf-8"),
                       "headers": {"Content-Type": "application/json", "X-CSRF-Token": csrf}}
                      if plan.payload is not None else
                      {"content": b"", "headers": {"Content-Type": "application/json",
                                                   "X-CSRF-Token": csrf}}))
    except APIError as exc:
        if 400 <= exc.status < 500 and exc.status not in (403, 408, 429):
            journal.record_plan("failed", plan.hash, action=plan.action, target=plan.change_id)
            raise ApprovalError(f"approval action rejected (HTTP {exc.status}); no automatic retry") from None
        raise ApprovalError(_UNKNOWN) from None
    except Exception:
        raise ApprovalError(_UNKNOWN) from None
    if plan.operation == "cancel":
        try:
            result = _verify_cancel(c, plan, before)
        except Exception:
            raise ApprovalError(_UNKNOWN) from None
        journal.record_plan("done", plan.hash, action=plan.action, target=plan.change_id)
        return result
    try:
        after_rows = _inventory(c, plan)
        exact = next(r for r in after_rows if _id(r["id"]) == plan.target_id)
        # Preserve baseline identities; disappearance is not proof of a decision.
        if not {_id(r["id"]) for r in before_rows} <= {_id(r["id"]) for r in after_rows}:
            raise ApprovalError(_UNKNOWN)
        if _id(exact.get("member_id")) != _id(before.get("member_id")):
            raise ApprovalError(_UNKNOWN)
        member = exact.get("member")
        if member is not None and (not isinstance(member, dict) or _id(member.get("id")) != member_id):
            raise ApprovalError(_UNKNOWN)
        if "change_id" in exact and _id(exact["change_id"]) != plan.change_id:
            raise ApprovalError(_UNKNOWN)
        raw = _status(exact)
        raw_id = raw.get("id") if isinstance(raw, dict) else raw
        name = raw.get("name") if isinstance(raw, dict) else None
        expected = "approved" if plan.operation == "approve" else "rejected"
        # Observed requested=0, approved=1, cancelled=3. Verify names rather
        # than inferring any mapping (especially rejection) from write values.
        if raw == before_status or raw_id == 0 or name == "requested":
            raise ApprovalError(_UNKNOWN)
        if name in ("approved", "rejected", "cancelled", "canceled") and name != expected:
            raise ApprovalError(_UNKNOWN)
    except Exception:
        raise ApprovalError(_UNKNOWN) from None
    journal.record_plan("done", plan.hash, action=plan.action, target=plan.change_id)
    result = {"status": "submitted", "change_id": plan.change_id, "approval_id": plan.target_id,
              "operation": plan.operation, "verified": True, "decision_verified": name == expected,
              "verification_basis": "named_decision" if name == expected else "changed_raw_status_only",
              "before_status": before_status, "observed_status": raw, "readback": exact,
              "plan_hash": plan.hash}
    if name != expected:
        result["warning"] = "Raw status changed; decision name/mapping unobserved. Only the status transition is verified; do not resend."
    return result


def _run(plan: ActionPlan, *, dry_run: bool, read_only: bool, yes: bool,
         expected: str | None, no_input: bool, json_out: bool) -> None:
    if read_only and not dry_run:
        raise ApprovalError("read-only mode: no approval action was sent")
    if dry_run:
        typer.echo(tenant_json(send(plan, reviewed_hash=None, dry_run=True), indent=2))
        return
    if expected is not None and expected != plan.hash:
        raise ApprovalError("approval action review hash does not match; review the exact action again")
    if yes:
        if expected != plan.hash:
            raise ApprovalError("--yes requires the full --expect-plan from a prior approval action review")
    elif no_input:
        raise ApprovalError("sending requires --yes and --expect-plan in no-input mode")
    review = plan.review()
    if plan.operation == "cancel":
        review["group"] = _cancel_group(get_client(), plan)
    typer.echo(tenant_json(review, indent=2), err=True)
    if not yes and not typer.confirm("Send this exact approval action (notifies people; may move workflow; cancel irreversible)?", default=False):
        typer.echo("Cancelled; no approval action was sent.")
        return
    result = send(plan, reviewed_hash=plan.hash, read_only=read_only)
    if json_out:
        typer.echo(tenant_json(result, indent=2))
    else:
        typer.echo(f"Approval {'group' if plan.operation == 'cancel' else 'record'} {plan.target_id} submitted on change {plan.change_id}; "
                   f"observed raw status: {json.dumps(result['observed_status'])}; "
                   f"verification: {result['verification_basis']}")
        if result.get("warning"):
            typer.echo(result["warning"], err=True)


def register(app: typer.Typer) -> None:
    @app.command("approval-respond", help="Review and approve/reject an exact approval record (notifies people; may move workflow).")
    def approval_respond(
        change: str = typer.Argument(..., metavar="CHANGE_ID"),
        approval_id: str = typer.Argument(..., metavar="APPROVAL_ID"),
        approve: bool = typer.Option(False, "--approve"),
        reject: bool = typer.Option(False, "--reject"),
        remark: str | None = typer.Option(None, "--remark", help="required nonempty for reject; optional for approve"),
        dry_run: bool = typer.Option(False, "--dry-run"),
        read_only: bool = typer.Option(False, "--read-only"),
        yes: bool = typer.Option(False, "--yes", "-y"),
        expected: str | None = typer.Option(None, "--expect-plan"),
        no_input: bool = typer.Option(False, "--no-input"),
        json_out: bool = typer.Option(False, "--json"),
    ) -> None:
        try:
            if approve == reject:
                raise ValueError("choose exactly one of --approve or --reject")
            plan = build_plan(change, approval_id, "approve" if approve else "reject", remark)
            _run(plan, dry_run=dry_run, read_only=read_only, yes=yes, expected=expected,
                 no_input=no_input, json_out=json_out)
        except (ValueError, ApprovalError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None

    @app.command("approval-cancel", help="Review and cancel an exact approval group (irreversible; notifies people).")
    def approval_cancel(
        change: str = typer.Argument(..., metavar="CHANGE_ID"),
        group_id: str = typer.Argument(..., metavar="GROUP_ID"),
        dry_run: bool = typer.Option(False, "--dry-run"),
        read_only: bool = typer.Option(False, "--read-only"),
        yes: bool = typer.Option(False, "--yes", "-y"),
        expected: str | None = typer.Option(None, "--expect-plan"),
        no_input: bool = typer.Option(False, "--no-input"),
        json_out: bool = typer.Option(False, "--json"),
    ) -> None:
        try:
            plan = build_plan(change, group_id, "cancel")
            _run(plan, dry_run=dry_run, read_only=read_only, yes=yes, expected=expected,
                 no_input=no_input, json_out=json_out)
        except (ValueError, ApprovalError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None
