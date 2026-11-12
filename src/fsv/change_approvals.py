"""Reviewed manual approval REQUEST, verified by new per-member approval IDs.

Four captured rules are supported. Never trust the POST response as proof of creation
or replay a POST to recover an identifier. Chains can additionally verify group names.
"""
from __future__ import annotations

import html
import json
import re
from dataclasses import dataclass
from types import SimpleNamespace
from typing import Any, Protocol, Sequence

import typer

from fsv import config, journal, views
from fsv.client import get_client
from fsv.errors import APIError
from fsv.resources import CHANGES, parse_id

RULES = {"everyone": (1, "Everyone"), "anyone": (2, "Anyone"),
         "majority": (3, "Majority"), "first-responder": (4, "First responder")}
_ACTION = "change_approval_request"
_UNKNOWN = "approval request outcome unknown; inspect the change and journal before any retry; do not resubmit"
_WARNING = ("Approval emails are irreversible; cancel/revoke cannot recall them. "
            "This requests approval; it does not approve the change. Tenant workflows may also notify people.")
_EMAIL = re.compile(r"[^\s@<>,;]+@[^\s@<>,;]+\.[^\s@<>,;]+\Z")


class ApprovalError(RuntimeError):
    """Safe user-facing failure without raw session or response details."""


def _id(value: Any) -> int | None:
    if isinstance(value, bool) or not isinstance(value, (int, str)):
        return None
    if not re.fullmatch(r"[0-9]+", str(value)) or int(value) <= 0:
        return None
    return int(value)


def _recipient(value: str) -> None:
    if not isinstance(value, str) or any(ord(c) < 32 or ord(c) == 127 for c in value):
        raise ValueError("approver must be a positive agent ID or exact email")
    if _id(value) is None and not _EMAIL.fullmatch(value):
        raise ValueError("approver must be a positive agent ID or exact email (no implicit self)")


def resolve_approvers(values: Sequence[str], client: Any = None) -> tuple[int, ...]:
    """Explicit numeric agent IDs or unique active exact-email autocomplete hits.

    Autocomplete id is the agent ID; user_id belongs to the distinct user
    namespace and must not be substituted for the captured notified_to IDs.
    """
    if not isinstance(values, (list, tuple)) or not values:
        raise ValueError("at least one explicit approver is required")
    for value in values:
        _recipient(value)
    resolved = []
    for value in values:
        aid = _id(value)
        if aid is None:
            try:
                c = client if client is not None else get_client()
                rows = c.autocomplete("agents", value)
                if not isinstance(rows, list):
                    raise ValueError("unreadable lookup")
                matches = [r for r in rows if isinstance(r, dict)
                           and str(r.get("email", "")).casefold() == value.casefold()
                           and not r.get("deleted") and not r.get("deactivated")
                           and "(deactivated)" not in str(r.get("value", "")).casefold()]
                if len(matches) != 1 or _id(matches[0].get("id")) is None:
                    raise ValueError("ambiguous agent")
                aid = _id(matches[0]["id"])
            except Exception:
                raise ApprovalError("approver email must resolve to exactly one active agent ID; no POST attempted; use an explicit agent ID if lookup is unavailable") from None
        if aid in resolved:
            raise ValueError("duplicate approver; specify each agent exactly once")
        resolved.append(aid)
    return tuple(resolved)


@dataclass(frozen=True)
class ApprovalPlan:
    change_id: int
    name: str
    message: str
    approver_ids: tuple[int, ...]
    tenant: str
    rule: str = "everyone"

    @property
    def path(self) -> str:
        return f"changes/{self.change_id}/approval-groups"

    @property
    def payload(self) -> dict[str, Any]:
        content = html.escape(self.message.replace("\r\n", "\n").replace("\r", "\n")).replace("\n", "<br>")
        return {"approval_groups": [{"name": self.name, "approval_type": RULES[self.rule][0],
                "email_content": f'<p dir="ltr">{content}</p>',
                "notified_to": list(self.approver_ids)}]}

    @property
    def hash(self) -> str:
        return journal.plan_hash({"tenant": self.tenant, "path": self.path, "payload": self.payload}) or ""

    def review(self) -> dict[str, Any]:
        return {"action": _ACTION, "tenant": self.tenant, "change_id": self.change_id,
                "method": "POST", "path": f"/api/_/{self.path}", "payload": self.payload,
                "recipients": list(self.approver_ids), "approval_rule": RULES[self.rule][1],
                "message_text": self.message, "plan_hash": self.hash, "warning": _WARNING,
                "verification": "Complete before/after approval record IDs, exact new recipients, requested status and exact selected rule ID. When complete chains identify exactly one new matching group, verify its exact name and ID too.",
                "contract": "Per-member approvals and named approval-chain groups observed; POST response not trusted."}


def build_plan(change_id: int | str, name: str, message: str, *, approver_ids: Sequence[int], rule: str = "everyone") -> ApprovalPlan:
    if isinstance(change_id, bool) or not isinstance(change_id, (int, str)):
        raise ValueError("change ID must be a positive display ID")
    cid = parse_id(str(change_id), CHANGES)
    if cid <= 0:
        raise ValueError("change ID must be positive")
    if not isinstance(name, str) or not name.strip() or any(ord(c) < 32 or ord(c) == 127 for c in name):
        raise ValueError("approval group name must be nonempty text without control characters")
    if not isinstance(message, str) or not message.strip() or any(
            (ord(c) < 32 and c not in "\r\n\t") or ord(c) == 127 for c in message):
        raise ValueError("message must be nonempty plain text without unsafe control characters")
    if not isinstance(approver_ids, (list, tuple)) or not approver_ids or any(
            type(a) is not int or a <= 0 for a in approver_ids):
        raise ValueError("at least one explicit positive agent ID is required")
    if len(set(approver_ids)) != len(approver_ids):
        raise ValueError("duplicate approver; specify each agent exactly once")
    if rule not in RULES:
        raise ValueError("rule must be everyone, anyone, majority or first-responder")
    return ApprovalPlan(cid, name, message, tuple(approver_ids), config.DOMAIN, rule)


class _InventoryPlan(Protocol):
    @property
    def change_id(self) -> int: ...


def _inventory(client: Any, plan: _InventoryPlan) -> list[dict[str, Any]]:
    seen: set[int] = set()
    totals: set[int] = set()

    def strict_get(path: str, params: Any = None) -> Any:
        # The generic collector filters non-object rows and deduplicates IDs.
        # For write verification, inspect raw pages before that information loss.
        data = client.int_get(path, params=params)
        # The observed endpoint returns a bare list; retain envelope pagination
        # support without allowing the collector to discard malformed rows.
        if isinstance(data, list):
            data = {"approvals": data}
        if not isinstance(data, dict) or not isinstance(data.get("approvals"), list):
            raise ApprovalError("approval inventory is unreadable")
        for source in (data, data.get("meta", {})):
            if not isinstance(source, dict) or source.get("partial") or source.get("complete") is False:
                raise ApprovalError("approval inventory is partial")
            for key in ("total_count", "count"):
                if key in source:
                    total = source[key]
                    if isinstance(total, bool) or not re.fullmatch(r"[0-9]+", str(total)):
                        raise ApprovalError("approval inventory has an invalid total")
                    totals.add(int(str(total)))
        for row in data["approvals"]:
            aid = _id(row.get("id")) if isinstance(row, dict) else None
            if aid is None or aid in seen:
                raise ApprovalError("approval inventory contains invalid or duplicate IDs")
            seen.add(aid)
        return data

    tab = views.load_tab(SimpleNamespace(int_get=strict_get), CHANGES, {"display_id": plan.change_id}, "approvals")
    if (tab.missing or tab.meta.get("complete") is not True or tab.meta.get("partial")
            or tab.meta.get("has_more") or not isinstance(tab.data, list)):
        raise ApprovalError("approval inventory is incomplete")
    ids = [_id(r.get("id")) if isinstance(r, dict) else None for r in tab.data]
    if None in ids or len(ids) != len(set(ids)):
        raise ApprovalError("approval inventory contains invalid or duplicate IDs")
    if totals and totals != {len(ids)}:
        raise ApprovalError("approval inventory does not match the declared total")
    return tab.data


def get_change_approval_groups(change_id: int, c: Any = None, *, module: str = "changes") -> list[dict[str, Any]]:
    """Read complete approval chains, returning raw groups with their record identities.

    Never turn permission, malformed response or pagination failures into an empty
    inventory. Unknown pagination contracts fail closed instead of guessing URLs.
    """
    if module not in {"changes", "tickets"}:
        raise ValueError("unsupported approval module")
    if _id(change_id) is None:
        raise ValueError("record ID must be positive")
    c = c if c is not None else get_client()
    groups: list[dict[str, Any]] = []
    seen_chains: set[int] = set()
    seen_groups: set[int] = set()
    seen_records: set[int] = set()
    totals: set[int] = set()
    for page in range(1, 101):
        data = c.int_get(f"{module}/{change_id}/approval-chains",
                         params={"page": page, "per_page": 100})
        if not isinstance(data, dict) or not isinstance(data.get("approval_chains"), list):
            raise ApprovalError("approval chains inventory is unreadable")
        more = False
        for source in (data, data.get("meta", {})):
            if not isinstance(source, dict) or source.get("partial") or source.get("complete") is False:
                raise ApprovalError("approval chains inventory is partial")
            for key in ("total_count", "count"):
                if key in source:
                    value = source[key]
                    if type(value) is not int or value < 0:
                        raise ApprovalError("approval chains inventory has an invalid total")
                    totals.add(value)
            for key in ("has_next", "has_more"):
                if key in source:
                    if type(source[key]) is not bool:
                        raise ApprovalError("approval chains pagination is unreadable")
                    more = more or source[key]
            if "next_page" in source and source["next_page"] is not None:
                if type(source["next_page"]) is not int or source["next_page"] != page + 1:
                    raise ApprovalError("approval chains pagination is unreadable")
                more = True
            if "total_pages" in source:
                value = source["total_pages"]
                if type(value) is not int or value < page:
                    raise ApprovalError("approval chains pagination is unreadable")
                more = more or page < value
            if source.get("next") or source.get("links"):
                raise ApprovalError("unsupported approval chains pagination")
        chains = data["approval_chains"]
        if more and not chains:
            raise ApprovalError("approval chains inventory is incomplete")
        def require_complete(node: dict[str, Any], meta_key: str, count: int) -> None:
            for source in (node, node.get(meta_key, {})):
                if (not isinstance(source, dict) or source.get("partial") or source.get("complete") is False
                        or source.get("has_more") or source.get("has_next") or source.get("next_page")
                        or source.get("next") or source.get("links")):
                    raise ApprovalError("nested approval chains inventory is incomplete")
                for key in ("count", "total_count"):
                    if key in source and (type(source[key]) is not int or source[key] != count):
                        raise ApprovalError("nested approval chains inventory total mismatch")

        for chain in chains:
            cid = _id(chain.get("id")) if isinstance(chain, dict) else None
            if cid is None or cid in seen_chains or not isinstance(chain.get("approval_groups"), list):
                raise ApprovalError("approval chains contain invalid or duplicate chain identities")
            seen_chains.add(cid)
            require_complete(chain, "approval_chain_meta", len(chain["approval_groups"]))
            for group in chain["approval_groups"]:
                gid = _id(group.get("id")) if isinstance(group, dict) else None
                if gid is None or gid in seen_groups or not isinstance(group.get("approvals"), list):
                    raise ApprovalError("approval chains contain invalid or duplicate group identities")
                seen_groups.add(gid)
                require_complete(group, "approval_group_meta", len(group["approvals"]))
                if type(group.get("approval_status")) is not int or group["approval_status"] < 0:
                    raise ApprovalError("approval group status unreadable")
                for row in group["approvals"]:
                    rid = _id(row.get("id")) if isinstance(row, dict) else None
                    mid = _id(row.get("member_id")) if isinstance(row, dict) else None
                    if rid is None or rid in seen_records or mid is None:
                        raise ApprovalError("approval group record identity unreadable or duplicate")
                    seen_records.add(rid)
                    if "member" in row and (not isinstance(row["member"], dict) or _id(row["member"].get("id")) != mid):
                        raise ApprovalError("approval group member identity conflicting")
                    if "change_id" in row and _id(row["change_id"]) != int(change_id):
                        raise ApprovalError("approval group change identity conflicting")
                    status = row.get("status")
                    if not (type(status) is int and status >= 0 or isinstance(status, dict)
                            and type(status.get("id")) is int and status["id"] >= 0
                            and ("name" not in status or isinstance(status["name"], str) and status["name"].strip())):
                        raise ApprovalError("approval group record status unreadable")
                groups.append(group)
        if not more:
            if totals and totals != {len(seen_chains)}:
                raise ApprovalError("approval chains inventory does not match the declared total")
            return groups
    raise ApprovalError("approval chains inventory page limit exceeded")


def _verify_new(rows: list[dict[str, Any]], plan: ApprovalPlan) -> None:
    """One fresh, requested selected-rule record per explicit recipient; no extras.

    Group name/ID are absent from the observed list. Neither level_id nor user_id
    is a substitute. Existing approval records are only baseline ID evidence.
    """
    members: list[int] = []
    for row in rows:
        aid = _id(row.get("member_id"))
        member = row.get("member")
        if aid is None or (member is not None and (
                not isinstance(member, dict) or _id(member.get("id")) != aid)):
            raise ApprovalError("approval member identity unavailable or conflicting")
        for key, expected_id, expected_name in (("status", 0, "requested"), ("type", RULES[plan.rule][0], "everyone" if plan.rule == "everyone" else None)):
            value = row.get(key)
            if (not isinstance(value, dict) or type(value.get("id")) is not int
                    or value["id"] != expected_id or (expected_name is not None and value.get("name") != expected_name)):
                raise ApprovalError("approval status or rule does not match the request")
        members.append(aid)
    if len(members) != len(plan.approver_ids) or set(members) != set(plan.approver_ids):
        raise ApprovalError("new approval records do not exactly match the recipients")


def send(plan: ApprovalPlan, client: Any = None, *, reviewed_hash: str | None,
         read_only: bool = False, dry_run: bool = False) -> dict[str, Any]:
    if build_plan(plan.change_id, plan.name, plan.message, approver_ids=plan.approver_ids, rule=plan.rule) != plan:
        raise ApprovalError("approval plan or tenant changed; review a new plan")
    if dry_run:
        return {"status": "dry_run", **plan.review()}
    if read_only:
        raise ApprovalError("read-only mode: no approval request was sent")
    if reviewed_hash is None:
        return {"status": "cancelled", "change_id": plan.change_id}
    if reviewed_hash != plan.hash:
        raise ApprovalError("approval review hash does not match; review the exact request again")
    if not plan.tenant:
        raise ApprovalError("configure a tenant before requesting approval")
    def events() -> list[dict[str, Any]]:
        return [r for r in journal.read() if r.get("kind") == f"plan:{_ACTION}" and r.get("plan") == plan.hash]
    prior = events()
    if prior and prior[-1].get("event") in ("started", "done"):
        raise ApprovalError("this approval request was sent or its outcome is unknown; inspect the change and journal; do not resubmit")
    try:
        c = client if client is not None else get_client()
        before_rows = _inventory(c, plan)
        before_ids = {_id(r["id"]) for r in before_rows}
        csrf = c._get_csrf()
    except Exception:
        raise ApprovalError("could not prepare a complete approval inventory/session; no approval POST was attempted") from None
    # Optional enrichment: unsupported/unreadable chains must not disable the
    # established complete per-record verification path.
    try:
        before_groups = get_change_approval_groups(plan.change_id, c)
    except Exception:
        before_groups = None
    journal.record_plan("started", plan.hash, action=_ACTION, target=plan.change_id)
    recorded = events()
    if not recorded or recorded[-1].get("event") != "started":
        raise ApprovalError("could not persist approval journal; no approval request was sent")
    try:
        # int_post silently replays on 403. Exactly one lower-level request.
        c._request("POST", f"{config.API_INT}/{plan.path}",
                   content=json.dumps(plan.payload, ensure_ascii=False).encode("utf-8"),
                   headers={"Content-Type": "application/json", "X-CSRF-Token": csrf})
    except APIError as exc:
        if 400 <= exc.status < 500 and exc.status not in (403, 408, 429):
            journal.record_plan("failed", plan.hash, action=_ACTION, target=plan.change_id)
            raise ApprovalError(f"approval request rejected (HTTP {exc.status}); no automatic retry") from None
        raise ApprovalError(_UNKNOWN) from None
    except Exception:
        raise ApprovalError(_UNKNOWN) from None
    try:
        after_rows = _inventory(c, plan)
        after_ids = {_id(r["id"]) for r in after_rows}
        if not before_ids <= after_ids:
            raise ApprovalError(_UNKNOWN)
        # Inspect ALL newly created record IDs, including unrelated concurrent
        # requests. Do not select a matching subset or infer a group identity.
        exact = [r for r in after_rows if _id(r["id"]) not in before_ids]
        _verify_new(exact, plan)
    except Exception:
        raise ApprovalError(_UNKNOWN) from None
    result = {"status": "submitted", "change_id": plan.change_id,
              "approval_ids": [_id(r["id"]) for r in exact], "group_name_verified": False,
              "verified": True, "readback": exact, "plan_hash": plan.hash}
    if before_groups is not None:
        try:
            after_groups = get_change_approval_groups(plan.change_id, c)
            baseline = {_id(g["id"]) for g in before_groups}
            current = {_id(g["id"]) for g in after_groups}
            fresh = [g for g in after_groups if _id(g["id"]) not in baseline]
            if baseline <= current and len(fresh) == 1:
                group = fresh[0]
                members = [_id(r["member_id"]) for r in group["approvals"]]
                records = {_id(r["id"]): r for r in group["approvals"]}
                if (group.get("name") == plan.name
                        and len(members) == len(plan.approver_ids)
                        and set(members) == set(plan.approver_ids)
                        and records.keys() == {_id(r["id"]) for r in exact}
                        and all(_id(records[_id(r["id"])]["member_id"]) == _id(r["member_id"])
                                and records[_id(r["id"])]["status"] == r["status"] for r in exact)):
                    result.update(group_id=_id(group["id"]), group_name_verified=True)
        except Exception:
            pass  # Complete record-ID verification above remains authoritative.
    journal.record_plan("done", plan.hash, action=_ACTION, target=plan.change_id)
    return result


def register(app: typer.Typer) -> None:
    @app.command("approval-request", help="Review and REQUEST manual change approval (selected rule; sends irreversible emails).")
    def approval_request(
        id_: str = typer.Argument(..., metavar="CHANGE_ID"),
        name: str = typer.Option(..., "--name", help="exact approval group name"),
        message: str = typer.Option(..., "--message", help="plain text; always HTML-escaped"),
        approver: list[str] = typer.Option(..., "--approver", help="explicit agent ID or exact email (repeatable; no implicit recipients)"),
        rule: str = typer.Option("everyone", "--rule", help="everyone, anyone, majority or first-responder"),
        dry_run: bool = typer.Option(False, "--dry-run", help="review only; emails may require read-only lookup"),
        read_only: bool = typer.Option(False, "--read-only", help="refuse approval requests"),
        yes: bool = typer.Option(False, "--yes", "-y", help="confirm the full previously reviewed --expect-plan"),
        expected: str | None = typer.Option(None, "--expect-plan", help="full hash from the exact prior review"),
        no_input: bool = typer.Option(False, "--no-input", help="never prompt"),
        json_out: bool = typer.Option(False, "--json", help="emit structured result"),
    ) -> None:
        try:
            if read_only and not dry_run:
                raise ApprovalError("read-only mode: no approval request was sent")
            # Validate everything local before any email lookup.
            build_plan(id_, name, message, approver_ids=[1], rule=rule)
            plan = build_plan(id_, name, message, approver_ids=resolve_approvers(approver), rule=rule)
            if dry_run:
                typer.echo(json.dumps(send(plan, reviewed_hash=None, dry_run=True), indent=2))
                return
            typer.echo(json.dumps(plan.review(), indent=2), err=True)
            if expected is not None and expected != plan.hash:
                raise ApprovalError("approval review hash does not match; review the exact request again")
            if yes:
                if expected != plan.hash:
                    raise ApprovalError("--yes requires the full --expect-plan from a prior approval review")
            elif no_input:
                raise ApprovalError("sending requires --yes and --expect-plan in no-input mode")
            elif not typer.confirm("Send this exact approval request and irreversible emails (no verified revoke)?", default=False):
                typer.echo("Cancelled; no approval request was sent.")
                return
            result = send(plan, reviewed_hash=plan.hash)
            if json_out:
                typer.echo(json.dumps(result, indent=2))
            else:
                typer.echo(f"Approval requested on change {plan.change_id}; approval record IDs: {result['approval_ids']}; complete inventory verification: {result['verified']}; group name verified: {result['group_name_verified']}")
        except (ValueError, ApprovalError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None
