"""Captured change relationship contracts, including the project proxy boundary."""
from __future__ import annotations

import json
import re
from datetime import date
from typing import Any
from urllib.parse import urlencode

from fsv import config, journal
from fsv.attachments import owner_row
from fsv.errors import APIError
from fsv.resources import CHANGES, parse_id
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute
from fsv.reviewed_snapshot import stable_snapshot
from fsv.ticket_notes import _EMAIL
from fsv.reviewed_inventory import complete_inventory, single_page_inventory
from fsv.watchers import positive_id

KINDS = {"caused-incident": ("tickets", "ticket_cause"), "problem": ("problems", "problem"),
         "release": ("releases", None), "project": ("projects", None), "project-task": ("issues", None)}


def proxy_body(raw: Any) -> dict:
    if not isinstance(raw, dict) or type(raw.get("status")) is not int or raw["status"] != 200 or not isinstance(raw.get("body"), dict):
        raise ReviewedWriteError("project proxy did not return an accepted nested status/body")
    return raw["body"]


def inventory(client: Any, target: int, kind: str) -> list[dict]:
    if kind not in KINDS:
        raise ValueError("unsupported change relationship kind")
    key, _ = KINDS[kind]
    if kind in {"project", "project-task"}:
        route = "fetch_linked_fr_projects" if kind == "project" else "linked_issues"
        raw = client.int_get("freshrelease/settings/" + route,
                             {"placeholder[resourceId]": str(target), "placeholder[resourceType]": "Change"})
        body = proxy_body(raw)
        rows, meta = body.get(key), body.get("meta")
        if not isinstance(rows, list) or not isinstance(meta, dict) or meta.get("page") != 1 or meta.get("total_pages") not in (0, 1) or type(meta.get("total")) is not int or meta["total"] != len(rows):
            raise ReviewedWriteError("complete single-page linked project inventory unavailable")
        rows = single_page_inventory(body, key)
    else:
        params = {"per_page": 50, "page": 1}
        if kind == "caused-incident":
            params["filter"] = "associated_tickets_caused"
        rows = complete_inventory(client, f"changes/{target}/{key}", key, params)
    identities = [positive_id(r.get("id")) for r in rows if isinstance(r, dict)]
    if len(identities) != len(rows) or len(identities) != len(set(identities)):
        raise ReviewedWriteError("relationship inventory has invalid or duplicate IDs")
    return sorted(rows, key=lambda r: positive_id(r["id"]))


def _refusal(exc: APIError, what: str) -> ReviewedWriteError:
    """One-line refusal for a failed read while building a relationship plan."""
    if exc.status == 404:
        return ReviewedWriteError(f"{what} not found (HTTP 404); nothing was sent")
    status = f"HTTP {exc.status}" if exc.status else "local error"
    return ReviewedWriteError(f"could not read {what} ({status}); nothing was sent")


def build_plan(client: Any, target: str | int, *, kind: str, action: str,
               item: str | None = None, project: dict | None = None,
               project_key: str | None = None, name: str | None = None) -> WritePlan:
    cid = positive_id(parse_id(str(target), CHANGES))
    if kind not in KINDS or action not in {"add", "remove"}:
        raise ValueError("choose a captured relationship kind and add/remove")
    try:
        return _build_plan(client, cid, kind=kind, action=action, item=item, project=project,
                           project_key=project_key, name=name)
    except APIError as exc:
        # Every read here (parent change, relationship inventory, candidate
        # record) is a precondition: a server error is a clean refusal.
        raise _refusal(exc, f"change {cid} or its {kind} relationships") from None


def _build_plan(client: Any, cid: int, *, kind: str, action: str, item: str | None, project: dict | None,
                project_key: str | None, name: str | None) -> WritePlan:
    try:
        parent = owner_row(client, "changes", cid)
    except APIError as exc:
        raise _refusal(exc, f"change {cid}") from None
    try:
        rows = inventory(client, cid, kind)
    except APIError as exc:
        raise _refusal(exc, f"the {kind} relationship inventory of change {cid}") from None
    key, module = KINDS[kind]
    if kind in {"project", "project-task"}:
        matches = [r for r in rows if str(r.get("id")) == item or r.get("key") == item] if item else []
        if action == "remove" and len(matches) != 1:
            raise ReviewedWriteError("exact linked project/task identity unavailable")
        if action == "add" and matches:
            raise ReviewedWriteError("relationship already present")
        if kind == "project" and action == "add":
            if item or not isinstance(project, dict):
                raise ValueError("project add is create-and-associate, not an existing-project link; pass project details")
            required = {"name", "description", "start_date", "end_date", "owner_email", "project_category"}
            if set(project) != required or any(not isinstance(v, str) or not v.strip() for v in project.values()):
                raise ValueError("project requires name, description, start_date, end_date, owner_email and project_category text")
            if not _EMAIL.fullmatch(project["owner_email"]) or not re.fullmatch(r"[1-9][0-9]*", project["project_category"]):
                raise ValueError("invalid project owner/category")
            start, end = date.fromisoformat(project["start_date"]), date.fromisoformat(project["end_date"])
            if end < start:
                raise ValueError("project end precedes start")
            body = {"project": {**project, "public_workspace": "0", "user_emails": []},
                    "meta": {"subject": project["name"]}, "remote_resource_id": str(cid), "remote_resource_type": "Change"}
            route = "create_fr_project"
        elif kind == "project":
            row = matches[0]
            body = {"activity_data": {"key": row["key"], "name": row["name"]}, "project_key": row["key"],
                    "project_id": str(positive_id(row["id"])), "remote_resource_id": str(cid), "remote_resource_type": "Change"}
            route = "unlink_fr_project"
        else:
            if not isinstance(project_key, str) or not re.fullmatch(r"[A-Z][A-Z0-9_]*", project_key):
                raise ValueError("project-task requires an exact uppercase --project-key")
            if not isinstance(item, str) or not re.fullmatch(re.escape(project_key) + r"-[1-9][0-9]*", item):
                raise ValueError("project-task item must be an exact project-key-number")
            try:
                projects = inventory(client, cid, "project")
            except APIError as exc:
                raise _refusal(exc, f"the linked projects of change {cid}") from None
            selected = [p for p in projects if p.get("key") == project_key]
            if len(selected) != 1:
                raise ReviewedWriteError("project task requires an exact linked project")
            if action == "remove":
                name = matches[0].get("name") or matches[0].get("title")
                if str(matches[0].get("project_id")) != str(selected[0]["id"]):
                    raise ReviewedWriteError("task and project identities conflict")
            if not name or not isinstance(name, str):
                raise ValueError("project task add needs its exact --name")
            placeholder = {"projectId": project_key, "resourceId": str(cid), "resourceType": "Change"}
            body = {"activity_data": {"key": item, "name": name}, "placeholder": placeholder}
            if action == "add":
                body.update(meta={"subject": parent["subject"]}, remote_resource_id=str(cid),
                            remote_resource_type="Change", issue={"id": item})
                route = "create_issue"
            else:
                body["issue_id"] = item
                route = "unlink_issue"
        method, path = "POST", "freshrelease/settings/" + route
    else:
        if project is not None or project_key is not None or name is not None:
            raise ValueError("project options do not apply to this relationship")
        ident = positive_id(item)
        present = any(positive_id(r["id"]) == ident for r in rows)
        if present != (action == "remove"):
            raise ReviewedWriteError("relationship already present or removal target absent")
        if kind == "caused-incident" and action == "add":
            try:
                record = owner_row(client, "tickets", ident)
            except APIError as exc:
                raise _refusal(exc, f"incident {ident}") from None
            from fsv.ticket_kind import exact_ticket_type
            if exact_ticket_type(record) != "incident":
                raise ReviewedWriteError("caused relationship requires a proven incident")
        elif action == "add":
            try:
                raw = client.int_get(f"{key}/{ident}")
            except APIError as exc:
                raise _refusal(exc, f"{key[:-1]} {ident}") from None
            row = raw.get(key[:-1], raw) if isinstance(raw, dict) else {}
            if positive_id(row.get("id")) != ident:
                raise ReviewedWriteError("exact relationship candidate unavailable")
        if kind == "release":
            method, path, body = "PUT", f"changes/{cid}/releases/{ident}", {"type": "link" if action == "add" else "unlink"}
        elif action == "add":
            method, path, body = "PUT", f"changes/{cid}/{key}/associate", {"item_ids": [ident], "module_type": module}
        else:
            method, path, body = "DELETE", f"changes/{cid}/{key}/{ident}", {"type": "unlink"}
            if kind == "caused-incident":
                path += "?" + urlencode({"module_type": "ticket_cause"})
    intent = dict(target=cid, kind=kind, action=action, item=item, project=project, project_key=project_key,
                  name=name if action == "add" else None)
    context = {"intent": intent, "parent": parent, "rows": rows}
    return WritePlan("change_relationship_" + kind + "_" + action, cid, config.DOMAIN, method, path,
                     json.dumps(body, sort_keys=True), json.dumps(stable_snapshot(context), sort_keys=True))


def send(plan: WritePlan, client: Any, *, reviewed_hash: str | None) -> dict:
    context = json.loads(plan.context_hash)
    intent = context["intent"]
    def verify(response):
        rows = inventory(client, plan.target, intent["kind"])
        before = {positive_id(r["id"]) for r in context["rows"]}
        after = {positive_id(r["id"]) for r in rows}
        if intent["kind"] in {"project", "project-task"}:
            body = proxy_body(response)
            if intent["action"] == "remove" and body.get("success") is not True:
                raise ReviewedWriteError("nested unlink success unavailable")
            if intent["kind"] == "project" and intent["action"] == "add":
                created = body.get("project", {})
                ident = positive_id(created.get("id"))
                exact = [r for r in rows if positive_id(r["id"]) == ident]
                verified = (after == before | {ident} and ident not in before and len(exact) == 1 and
                            exact[0].get("name") == intent["project"]["name"] and exact[0].get("key") == created.get("key"))
                return {"item_id": ident, "verified": verified}
            if intent["action"] == "add":
                returned = body.get("issue", {})
                ident = positive_id(returned.get("id"))
                return {"item_id": ident, "verified": returned.get("key") == intent["item"] and ident not in before and
                        after == before | {ident} and any(r.get("key") == intent["item"] and positive_id(r["id"]) == ident for r in rows)}
            exact = [r for r in context["rows"] if str(r.get("id")) == intent["item"] or r.get("key") == intent["item"]]
            ident = positive_id(exact[0]["id"])
        else:
            ident = positive_id(intent["item"])
        wanted = before | {ident} if intent["action"] == "add" else before - {ident}
        return {"item_id": ident, "verified": after == wanted}
    return execute(plan, client, reviewed_hash=reviewed_hash,
                   rebuild=lambda: build_plan(client, **intent), verify=verify)
