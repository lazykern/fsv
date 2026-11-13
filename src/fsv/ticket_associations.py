"""Reviewed ticket-side associations: problems, changes (both directions) and
child tickets, using the wires the ticket page sends (captured from the UI).

* problem link/unlink: ``PUT tickets/{id}/problems/{pid}`` ``{"type": "link"|"unlink"}``;
  readback ``GET tickets/{id}/problems``.
* change link: ``PUT tickets/{id}/changes/associate``
  ``{"item_ids": [cid], "change_type": "change_cause"|"change", "type": "link"}``;
  unlink ``PUT tickets/{id}/changes/{cid}`` ``{"type": "unlink", "change_type": ...}``;
  readback ``GET tickets/{id}/changes?change_type=...`` (the type is mandatory).
* child link: ``POST tickets/{id}/associate-child-tickets`` ``{"ticket_ids": [child]}``;
  unlink ``DELETE tickets/{id}/remove_child?ticket_id=child`` (204);
  readback ``GET tickets/{id}/child-tickets?type=active`` plus the
  ``related_tickets`` tab ``hierarchy`` (2 parent, 1 child, 0 none): the
  child list answers 400 on a ticket that is not a parent.
"""
from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any
from urllib.parse import urlencode

from fsv import config, journal
from fsv.errors import APIError
from fsv.resources import CHANGES, PROBLEMS, TICKETS, parse_id
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute
from fsv.ticket_properties import _record
from fsv.views import collect_pages
from fsv.watchers import positive_id

CHANGE_TYPES = {"initiating": "change_cause", "change_cause": "change_cause", "cause": "change_cause",
                "initiated-by": "change", "initiated_by": "change", "change": "change"}
CHANGE_TYPE_LABELS = {"change_cause": "change initiating this ticket", "change": "change initiated by this ticket"}
PARENT, CHILD, NONE = 2, 1, 0
HIERARCHY = {PARENT: "parent", CHILD: "child", NONE: "none"}


@dataclass(frozen=True)
class LinkPlan(WritePlan):
    kind: str
    item: int
    change_type: str | None = None

    def review(self) -> dict[str, Any]:
        out = {**super().review(), "ticket_id": self.target, "kind": self.kind, "item_id": self.item}
        if self.change_type:
            out["change_type"] = self.change_type
            out["meaning"] = CHANGE_TYPE_LABELS[self.change_type]
        return out


def change_type(value: str) -> str:
    try:
        return CHANGE_TYPES[str(value).strip().casefold()]
    except KeyError:
        raise ValueError("--kind must be initiating (change_cause) or initiated-by (change)") from None


def _read(client: Any, path: str, what: str) -> Any:
    try:
        return client.int_get(path)
    except APIError as exc:
        if exc.status == 404:
            raise ReviewedWriteError(f"{what} not found (HTTP 404); nothing was sent") from None
        raise


def _exact(client: Any, module: str, ident: int) -> dict[str, Any]:
    if module == "tickets":
        try:
            return _record(client, ident)
        except APIError as exc:
            if exc.status == 404:
                raise ReviewedWriteError(f"ticket {ident} not found (HTTP 404); nothing was sent") from None
            raise
        except Exception as exc:  # PropertyError: exact identity
            raise ReviewedWriteError(str(exc)) from None
    raw = _read(client, f"{module}/{ident}", f"{module[:-1]} {ident}")
    row = raw.get(module[:-1], raw) if isinstance(raw, dict) else None
    if not isinstance(row, dict) or str(row.get("id")) != str(ident):
        raise ReviewedWriteError(f"{module[:-1]} read did not identify the exact target; nothing was sent")
    return row


def _complete(client: Any, path: str, key: str, params: dict[str, Any] | None, what: str) -> list[dict[str, Any]]:
    rows, meta, missing = collect_pages(client, path, key, params, what=what)
    if missing is not None or meta.get("complete") is not True:
        raise ReviewedWriteError(f"complete {what} list unavailable; nothing was sent")
    ids = [str(r.get("id")) for r in rows]
    if any(i in ("None", "") for i in ids) or len(set(ids)) != len(ids):
        raise ReviewedWriteError(f"{what} list has missing or duplicate IDs")
    return rows


def problems(client: Any, ticket: int) -> list[dict[str, Any]]:
    return _complete(client, f"tickets/{ticket}/problems", "problems", None, "associated problems")


def changes(client: Any, ticket: int, ctype: str) -> list[dict[str, Any]]:
    return _complete(client, f"tickets/{ticket}/changes", "changes", {"change_type": ctype}, "associated changes")


def hierarchy(client: Any, ticket: int) -> int:
    raw = _read(client, f"tickets/{ticket}/tabs", f"ticket {ticket} tabs")
    tabs = raw if isinstance(raw, list) else raw.get("tabs") if isinstance(raw, dict) else None
    found = [t for t in tabs or [] if isinstance(t, dict) and t.get("name") == "related_tickets"]
    if not found and isinstance(raw, dict) and isinstance(raw.get("related_tickets"), dict):
        found = [raw["related_tickets"]]
    values = {t.get("hierarchy") for t in found}
    if len(values) != 1:
        raise ReviewedWriteError("ticket hierarchy (related_tickets tab) unavailable; nothing was sent")
    value = values.pop()
    if type(value) is not int or value not in HIERARCHY:
        raise ReviewedWriteError("ticket hierarchy value is unreadable")
    return value


def children(client: Any, ticket: int) -> list[dict[str, Any]]:
    """Active children of a parent; a ticket that is not a parent has none
    (its child list answers 400, so the tab decides)."""
    if hierarchy(client, ticket) != PARENT:
        return []
    return _complete(client, f"tickets/{ticket}/child-tickets", "child_tickets",
                     {"page": 1, "per_page": 30, "type": "active"}, "child tickets")


def _ids(rows: list[dict[str, Any]]) -> set[int]:
    return {positive_id(r.get("id")) for r in rows}


def listing(client: Any, ticket: str | int, kind: str, ctype: str | None = None) -> dict[str, Any]:
    tid = positive_id(parse_id(str(ticket), TICKETS))
    if kind == "problem":
        return {"ticket_id": tid, "problems": problems(client, tid), "complete": True}
    if kind == "change":
        ct = change_type(ctype or "")
        return {"ticket_id": tid, "change_type": ct, "meaning": CHANGE_TYPE_LABELS[ct],
                "changes": changes(client, tid, ct), "complete": True}
    level = hierarchy(client, tid)
    return {"ticket_id": tid, "hierarchy": HIERARCHY[level], "child_tickets": children(client, tid),
            "complete": True, "note": "active children only (type=active)"}


def _rows_of(raw: Any, keys: tuple[str, ...]) -> list[dict[str, Any]]:
    if isinstance(raw, list):
        rows = raw
    elif isinstance(raw, dict):
        rows = next((raw[k] for k in keys if isinstance(raw.get(k), list)), None)
        if rows is None:
            lists = [v for v in raw.values() if isinstance(v, list) and all(isinstance(r, dict) for r in v)]
            rows = lists[0] if len(lists) == 1 else None
    else:
        rows = None
    if not isinstance(rows, list) or any(not isinstance(r, dict) for r in rows):
        raise ReviewedWriteError("candidate list is unreadable")
    return rows


def _brief(row: dict[str, Any]) -> dict[str, Any]:
    return {k: row.get(k) for k in ("id", "human_display_id", "subject", "status_name", "status") if k in row}


def candidates(client: Any, ticket: str | int, kind: str, search: str | None = None,
               ctype: str | None = None) -> dict[str, Any]:
    """Read-only picker lists (single page; never a complete inventory)."""
    tid = positive_id(parse_id(str(ticket), TICKETS))
    if kind == "problem":
        raw = (client.int_get("problems/search", {"q": search}) if search else
               client.int_get(f"tickets/{tid}/problems", {"filter": "list", "page": 1, "per_page": 30}))
        rows = _rows_of(raw, ("problems",))
    elif kind == "change":
        ct = change_type(ctype or "")
        raw = (client.int_get("changes/search", {"q": search}) if search else
               client.int_get(f"tickets/{tid}/changes", {"filter": "list", "change_type": ct, "page": 1,
                                                          "per_page": 30, "ticket_id": tid}))
        rows = _rows_of(raw, ("changes",))
    else:
        raw = client.int_get(f"tickets/{tid}/suggest_child_tickets", {"search_term": search} if search else None)
        rows = _rows_of(raw, ("tickets", "child_tickets", "suggested_tickets", "suggest_child_tickets"))
        if search:
            # The server list is not filtered by the term; filter locally.
            needle = search.casefold()
            rows = [r for r in rows if needle in " ".join(str(r.get(k) or "") for k in
                                                           ("subject", "human_display_id", "id")).casefold()]
    out = {"ticket_id": tid, "kind": kind, "search": search, "candidates": [_brief(r) for r in rows],
           "scope": "one picker page or search result; not a complete inventory", "mutation": "none"}
    if kind == "problem" and search or kind == "change" and search:
        out["note"] = "search is token based; records created in the last minute may not be indexed yet"
    return out


def build_plan(client: Any, ticket: str | int, kind: str, item: str | int, *, unlink: bool = False,
               ctype: str | None = None) -> LinkPlan:
    tid = positive_id(parse_id(str(ticket), TICKETS))
    if kind not in {"problem", "change", "child"} or type(unlink) is not bool:
        raise ValueError("unsupported ticket association")
    resource = {"problem": PROBLEMS, "change": CHANGES, "child": TICKETS}[kind]
    iid = positive_id(parse_id(str(item), resource))
    record = _exact(client, "tickets", tid)
    context: dict[str, Any] = {"ticket": record}
    ct = None
    if kind == "problem":
        from fsv.ticket_kind import INCIDENT, kind_evidence
        evidence = kind_evidence(record)
        if not evidence or set(evidence.values()) != {INCIDENT}:
            raise ReviewedWriteError("problems associate with incidents only; this ticket is not a verified incident")
        context["item"] = _exact(client, "problems", iid)
        rows = problems(client, tid)
        context["inventory"] = rows
        present = iid in _ids(rows)
        method, path = "PUT", f"tickets/{tid}/problems/{iid}"
        payload: Any = {"type": "unlink" if unlink else "link"}
    elif kind == "change":
        ct = change_type(ctype or "")
        context["item"] = _exact(client, "changes", iid)
        rows = changes(client, tid, ct)
        context["inventory"] = rows
        present = iid in _ids(rows)
        if unlink:
            method, path, payload = "PUT", f"tickets/{tid}/changes/{iid}", {"type": "unlink", "change_type": ct}
        else:
            method, path = "PUT", f"tickets/{tid}/changes/associate"
            payload = {"item_ids": [iid], "change_type": ct, "type": "link"}
    else:
        if iid == tid:
            raise ValueError("a ticket cannot be its own child")
        context["item"] = _exact(client, "tickets", iid)
        parent_level, child_level = hierarchy(client, tid), hierarchy(client, iid)
        rows = children(client, tid)
        context.update(inventory=rows, levels=[parent_level, child_level])
        present = iid in _ids(rows)
        if not unlink:
            if parent_level == CHILD:
                raise ReviewedWriteError(f"ticket {tid} is itself a child ticket; it cannot take children")
            if child_level != NONE and not present:
                raise ReviewedWriteError(f"ticket {iid} is already a {HIERARCHY[child_level]} ticket; nothing was sent")
            method, path, payload = "POST", f"tickets/{tid}/associate-child-tickets", {"ticket_ids": [iid]}
        else:
            method, path, payload = "DELETE", f"tickets/{tid}/remove_child?" + urlencode({"ticket_id": iid}), None
    if present != unlink:
        raise ReviewedWriteError(f"{kind} {iid} is {'already' if present else 'not'} associated with ticket {tid}"
                                 + (f" as {CHANGE_TYPE_LABELS[ct]}" if ct else "") + "; nothing was sent")
    action = f"ticket_{'unlink' if unlink else 'link'}_{kind}"
    return LinkPlan(action, tid, config.DOMAIN, method, path, json.dumps(payload, sort_keys=True),
                    journal.snapshot_hash(context) or "", kind, iid, ct)


def send(plan: LinkPlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    unlink = plan.action.startswith("ticket_unlink_")

    def verify(_response: Any) -> dict[str, Any]:
        out: dict[str, Any] = {"ticket_id": plan.target, "item_id": plan.item, "kind": plan.kind}
        if plan.kind == "problem":
            linked = plan.item in _ids(problems(client, plan.target))
        elif plan.kind == "change":
            linked = plan.item in _ids(changes(client, plan.target, plan.change_type or ""))
            out["change_type"] = plan.change_type
        else:
            in_list = plan.item in _ids(children(client, plan.target))
            level = hierarchy(client, plan.item)
            out["child_hierarchy"] = HIERARCHY[level]
            linked = in_list and level == CHILD if not unlink else in_list or level != NONE
        out.update(linked=linked, verified=linked != unlink)
        return out

    return execute(plan, client, reviewed_hash=reviewed_hash, verify=verify,
                   rebuild=lambda: build_plan(client, plan.target, plan.kind, plan.item, unlink=unlink,
                                              ctype=plan.change_type))
