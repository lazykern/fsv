"""Reviewed initiating-ticket relationships; no write retries or inferred success."""
from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any

from fsv import config, journal, views
from fsv.client import get_client
from fsv.errors import APIError
from fsv.resources import CHANGES, parse_id

_ACTION = "change_ticket_association"
_UNKNOWN = "association outcome unknown/unverified; inspect the change and journal; do not resubmit"
KINDS = ("tickets", "problems", "releases")


class AssociationError(ValueError):
    """Safe user-facing failure, without response/session details."""


def _id(value: Any) -> int | None:
    if isinstance(value, bool) or not isinstance(value, (int, str)):
        return None
    if not str(value).isascii() or not str(value).isdecimal() or int(value) <= 0:
        return None
    return int(value)


class AssociationReader:
    """Validate rows before the paginator can discard malformed entries."""

    def __init__(self, client: Any):
        self.client = client

    def int_get(self, path: str, params: Any = None) -> Any:
        try:
            data = self.client.int_get(path, params)
        except APIError:
            raise
        except Exception:
            raise AssociationError("association page unavailable") from None
        key = path.rsplit("/", 1)[-1]
        if not isinstance(data, dict) or not isinstance(data.get(key), list) or any(
                not isinstance(row, dict) or _id(row.get("id")) is None for row in data[key]):
            raise AssociationError("unreadable association page")
        return data


def inventory(change_id: int, client: Any = None, *, kinds: tuple[str, ...] = KINDS) -> dict[str, Any]:
    """Retain partial rows with explicit completeness; unavailable is never empty."""
    c = client if client is not None else get_client()
    result: dict[str, Any] = {"meta": {}, "warnings": []}

    for kind in kinds:
        rows, meta, missing = views.collect_pages(
            AssociationReader(c), f"changes/{change_id}/{kind}", kind, {"per_page": 50, "page": 1}, what=kind)
        result[kind] = rows
        result["meta"][kind] = meta
        if missing or not meta.get("complete"):
            result["warnings"].append(meta.get("partial") or f"{kind}: unavailable")
    result["complete"] = all(m.get("complete") is True for m in result["meta"].values())
    return result


def ticket_ids(change_id: int, client: Any) -> set[int]:
    data = inventory(change_id, client, kinds=("tickets",))
    if not data["complete"]:
        raise AssociationError("initiating-ticket inventory unavailable/incomplete; not an empty list")
    return {int(row["id"]) for row in data["tickets"]}


@dataclass(frozen=True)
class AssociationPlan:
    change_id: int
    ticket_ids: tuple[int, ...]
    unlink: bool
    tenant: str

    @property
    def method(self) -> str:
        return "DELETE" if self.unlink else "PUT"

    @property
    def path(self) -> str:
        tail = str(self.ticket_ids[0]) if self.unlink else "associate"
        return f"changes/{self.change_id}/tickets/{tail}"

    @property
    def payload(self) -> dict[str, Any]:
        return {"type": "unlink"} if self.unlink else {"item_ids": list(self.ticket_ids), "module_type": "ticket"}

    @property
    def params(self) -> dict[str, Any] | None:
        return {"module_type": "ticket"} if self.unlink else None

    @property
    def hash(self) -> str:
        return journal.plan_hash({"tenant": self.tenant, "method": self.method, "path": self.path,
                                  "payload": self.payload, "params": self.params}) or ""

    def review(self) -> dict[str, Any]:
        return {"action": _ACTION, "tenant": self.tenant, "change_id": self.change_id,
                "ticket_ids": list(self.ticket_ids), "method": self.method,
                "path": f"/api/_/{self.path}", "payload": self.payload, "params": self.params,
                "plan_hash": self.hash,
                "warning": "Initiating-ticket relationship only. Tenant workflows may notify people. One attempt; never automatically resubmit."}


def build_plan(change_id: int | str, ticket_ids: list[int] | tuple[int, ...], *, unlink: bool = False) -> AssociationPlan:
    if isinstance(change_id, bool) or not isinstance(change_id, (int, str)):
        raise AssociationError("change ID must be a positive display ID")
    cid = parse_id(str(change_id), CHANGES)
    if cid <= 0 or type(unlink) is not bool:
        raise AssociationError("invalid change ID or association action")
    if not isinstance(ticket_ids, (list, tuple)) or not ticket_ids or any(_id(v) is None for v in ticket_ids):
        raise AssociationError("ticket IDs must be positive internal ticket IDs")
    ids = tuple(sorted({int(v) for v in ticket_ids}))
    if unlink and len(ids) != 1:
        raise AssociationError("unlink accepts one ticket per reviewed request; review each removal separately")
    return AssociationPlan(cid, ids, unlink, config.DOMAIN)


def send(plan: AssociationPlan, client: Any = None, *, reviewed_hash: str | None,
         read_only: bool = False, dry_run: bool = False) -> dict[str, Any]:
    if build_plan(plan.change_id, plan.ticket_ids, unlink=plan.unlink) != plan:
        raise AssociationError("association plan or tenant changed; review again")
    if dry_run:
        return {"status": "dry_run", **plan.review()}
    if read_only:
        raise AssociationError("read-only mode: no association request was sent")
    if reviewed_hash is None:
        return {"status": "cancelled", "verified": False}
    if reviewed_hash != plan.hash or not plan.tenant:
        raise AssociationError("association review hash/tenant does not match; review again")
    events = [r for r in journal.read() if r.get("kind") == f"plan:{_ACTION}" and r.get("plan") == plan.hash]
    refusal = journal.replay_refusal(events, _UNKNOWN)
    if refusal:
        raise AssociationError(refusal)
    try:
        c = client if client is not None else get_client()
        before = ticket_ids(plan.change_id, c)
        csrf = c._get_csrf()
    except Exception:
        raise AssociationError("could not prepare complete initiating-ticket inventory/session; no write attempted") from None
    desired = set(plan.ticket_ids)
    if (not plan.unlink and desired <= before) or (plan.unlink and not desired & before):
        return {"status": "already_in_requested_state", "verified": True, **plan.review()}
    journal.record_plan("started", plan.hash, action=_ACTION, target=plan.change_id)
    events = [r for r in journal.read() if r.get("kind") == f"plan:{_ACTION}" and r.get("plan") == plan.hash]
    if not events or events[-1].get("event") != "started":
        raise AssociationError("could not persist association journal; no write attempted")
    try:
        # Submit the exact reviewed bytes once.
        c._request(plan.method, f"{config.API_INT}/{plan.path}",
                   content=json.dumps(plan.payload).encode("utf-8"), params=plan.params,
                   headers={"Content-Type": "application/json", "X-CSRF-Token": csrf})
    except APIError as exc:
        if 400 <= exc.status < 500 and exc.status not in (403, 408, 429):
            journal.record_plan("failed", plan.hash, action=_ACTION, target=plan.change_id)
            raise AssociationError(f"association rejected (HTTP {exc.status}); no automatic retry") from None
        raise AssociationError(_UNKNOWN) from None
    except Exception:
        raise AssociationError(_UNKNOWN) from None
    # Success envelopes are not an observed contract. Only a complete readback
    # can establish state, including when the write response is malformed/empty.
    try:
        after = ticket_ids(plan.change_id, c)
    except Exception:
        raise AssociationError(_UNKNOWN) from None
    verified = not (desired & after) if plan.unlink else desired <= after
    if not verified:
        raise AssociationError(_UNKNOWN)
    journal.record_plan("done", plan.hash, action=_ACTION, target=plan.change_id)
    return {"status": "submitted", "verified": True, **plan.review()}
