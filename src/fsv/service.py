"""Reusable data-fetching layer for both CLI and TUI."""
from __future__ import annotations

import threading
from typing import Any

from fsv import config, schema as schema_mod
from fsv.client import Client, get_client
from fsv.render import strip_html
from fsv.resources import CHANGES, PROBLEMS, TICKETS, Resource, format_id


def list_items(
    resource: Resource,
    client: Client | None = None,
    page: int = 1,
    per_page: int = 30,
    filter_name: str | None = None,
    order_by: str | None = None,
    order_type: str = "desc",
    query_hash: str | None = None,
) -> tuple[list[dict[str, Any]], int]:
    c = client or get_client()
    params: dict[str, Any] = {
        "per_page": per_page,
        "page": page,
        "include": resource.list_include,
        "cache": "true",
    }
    if filter_name:
        params["filter"] = filter_name
    if order_by:
        params["order_by"] = order_by
        params["order_type"] = order_type
    if query_hash:
        params["query_hash"] = query_hash
        if resource.name in ("changes", "tickets"):
            params["advanced_query_hash"] = ""
    data = c.int_get(resource.api_path, params=params)
    items = data.get(resource.list_key, [])
    total = (data.get("meta") or {}).get("total_count") or data.get("total") or len(items)
    for item in items:
        item["_resource"] = resource
    return items, total


_SEARCH_ENTITY_MAP = {
    "tickets": TICKETS,
    "problems": PROBLEMS,
    "changes": CHANGES,
}


def _normalize_search_result(row: dict, entity: str) -> dict[str, Any] | None:
    import re
    resource = _SEARCH_ENTITY_MAP.get(entity)
    if resource is None:
        return None
    if row.get("ticket_path"):
        prefix = "ticket"
        owner_field = "responder_name"
    else:
        prefix = "itil_module"
        owner_field = "owner_name"
    path = row.get(f"{prefix}_path") or ""
    m = re.search(r"/(\d+)$", path)
    if not m:
        return None
    item_id = int(m.group(1))
    display_id = row.get(f"{prefix}_display_id") or ""
    group_key = "ticket_group" if prefix == "ticket" else "itil_module_group"
    return {
        "id": item_id,
        "display_id": item_id,
        "human_display_id": display_id,
        "subject": strip_html(row.get("subject") or ""),
        "status": row.get(f"{prefix}_status") or "",
        "priority_label": row.get(f"{prefix}_priority") or "",
        "requester": {"name": ""},
        "_agent": row.get(owner_field) or "",
        "_group": row.get(group_key) or "",
        "created_at": row.get("created_at") or "",
        "_resource": resource,
        "_search_result": True,
    }


def search_items(
    term: str,
    entity: str | None = None,
    sort: str | None = None,
    page: int = 1,
    client: Client | None = None,
) -> tuple[list[dict[str, Any]], dict[str, int]]:
    c = client or get_client()
    if entity and entity in _SEARCH_ENTITY_MAP:
        data = c.fulltext_search(entity, term, page=page, sort=sort)
        items = []
        for row in data.get("results", []):
            norm = _normalize_search_result(row, entity)
            if norm:
                items.append(norm)
        totals = {entity: data.get("total_entries", len(items))}
        return items, totals

    entities = ["tickets", "problems", "changes"]
    results: list[list[dict[str, Any]]] = [[] for _ in entities]
    totals: dict[str, int] = {}

    def _fetch(ent: str, idx: int) -> None:
        try:
            data = c.fulltext_search(ent, term, page=page, sort=sort)
            for row in data.get("results", []):
                norm = _normalize_search_result(row, ent)
                if norm:
                    results[idx].append(norm)
            totals[ent] = data.get("total_entries", len(results[idx]))
        except Exception:
            pass

    threads = [
        threading.Thread(target=_fetch, args=(ent, idx), daemon=True)
        for idx, ent in enumerate(entities)
    ]
    for t in threads:
        t.start()
    for t in threads:
        t.join()

    all_items = [item for bucket in results for item in bucket]
    return all_items, totals


def list_work_items(
    client: Client | None = None,
    per_page: int = 30,
    page: int = 1,
) -> tuple[list[dict[str, Any]], int]:
    c = client or get_client()
    results: list[list[dict[str, Any]]] = [[] for _ in range(3)]

    def _fetch(res: Resource, idx: int) -> None:
        try:
            items, _ = list_items(res, client=c, page=page, per_page=per_page)
            results[idx] = items
        except Exception:
            pass

    threads = [
        threading.Thread(target=_fetch, args=(res, idx), daemon=True)
        for idx, res in enumerate((TICKETS, CHANGES, PROBLEMS))
    ]
    for t in threads:
        t.start()
    for t in threads:
        t.join()

    all_items = [item for bucket in results for item in bucket]
    all_items.sort(key=lambda x: x.get("updated_at") or "", reverse=True)
    return all_items, len(all_items)


def get_item(
    resource: Resource,
    item_id: int,
    client: Client | None = None,
) -> dict[str, Any]:
    c = client or get_client()
    include = None
    if resource.name == "tickets":
        include = "requester,stats,phone,feedback,ticket_status"
    elif resource.name in ("changes", "problems"):
        include = "requester,stats"
    params: dict[str, Any] = {}
    if include:
        params["include"] = include
    data = c.int_get(f"{resource.api_path}/{item_id}", params=params or None)
    item = data.get(resource.item_key, data)
    item["_resource"] = resource
    return item


def get_activities(
    resource: Resource, item_id: int, client: Client | None = None, page: int = 1,
) -> list[dict[str, Any]]:
    c = client or get_client()
    data = c.int_get(f"{resource.api_path}/{item_id}/activities", params={"page": page})
    return data.get("activities", [])


def get_notes(
    resource: Resource, item_id: int, client: Client | None = None, page: int = 1,
) -> list[dict[str, Any]]:
    c = client or get_client()
    if resource.name == "tickets":
        data = c.int_get(
            f"tickets/{item_id}/conversations",
            params={"page": page, "include": "user,phone,feedback"},
        )
        return data.get("conversations", [])
    data = c.int_get(
        f"{resource.api_path}/{item_id}/notes",
        params={"page": page, "include": "user"},
    )
    return data.get("notes", [])


def get_tasks(
    resource: Resource, item_id: int, client: Client | None = None, page: int = 1,
) -> list[dict[str, Any]]:
    c = client or get_client()
    data = c.int_get(f"{resource.api_path}/{item_id}/tasks", params={"page": page})
    return data.get("tasks", [])


def get_approvals(
    resource: Resource, item_id: int, client: Client | None = None,
) -> list[dict[str, Any]]:
    c = client or get_client()
    data = c.int_get(f"{resource.api_path}/{item_id}/approvals")
    return data.get("approvals", [])


def get_assets(
    resource: Resource, item_id: int, client: Client | None = None,
) -> list[dict[str, Any]]:
    c = client or get_client()
    if resource.name == "changes":
        from fsv.create import get_change_assets
        return get_change_assets(item_id, c)
    if resource.name == "tickets":
        data = c.int_get(f"{resource.api_path}/{item_id}/associated_assets")
        return data.get("associated_assets", [])
    data = c.int_get(f"{resource.api_path}/{item_id}/assets")
    return data.get("assets", [])


def get_associations(
    resource: Resource, item_id: int, client: Client | None = None,
) -> dict[str, Any]:
    c = client or get_client()
    if resource.name == "tickets":
        result: dict[str, Any] = {}
        try:
            result["changes"] = c.int_get(
                f"tickets/{item_id}/changes", {"change_type": "change"},
            ).get("changes", [])
        except Exception:
            result["changes"] = []
        try:
            result["problems"] = c.int_get(f"tickets/{item_id}/problems").get("problems", [])
        except Exception:
            result["problems"] = []
        return result
    if resource.name == "changes":
        from fsv.create import get_change_associations
        try:
            return get_change_associations(item_id, c)
        except Exception:
            return {}
    if resource.name == "problems":
        result = {}
        try:
            result["changes"] = c.int_get(
                f"problems/{item_id}/changes",
            ).get("changes", [])
        except Exception:
            result["changes"] = []
        try:
            result["incidents"] = c.int_get(
                f"problems/{item_id}/tickets",
            ).get("tickets", [])
        except Exception:
            result["incidents"] = []
        return result
    return {}


def get_requested_items(
    item_id: int, client: Client | None = None,
) -> list[dict[str, Any]]:
    c = client or get_client()
    data = c.int_get(f"tickets/{item_id}/requested_items")
    items = data.get("requested_items", [])
    result = []
    for item in items:
        rid = item.get("id")
        if rid:
            try:
                detail = c.int_get(
                    f"tickets/{item_id}/requested_items/{rid}",
                    params={"view": "more_info"},
                )
                result.append({**item, **detail.get("requested_item", detail)})
            except Exception:
                result.append(item)
        else:
            result.append(item)
    return result


def resolve_status(item: dict, resource: Resource, schema: dict) -> str:
    sid = item.get("status")
    return (
        item.get("status_name")
        or (item.get("change_status") or {}).get("name")
        or (item.get("ticket_status") or {}).get("name")
        or (item.get("problem_status") or {}).get("name")
        or schema_mod.choice_label("status", sid, schema)
        or str(sid)
    )


def resolve_priority(item: dict) -> str:
    return schema_mod.PRIORITY.get(item.get("priority") or 0, "-")


def item_url(resource: Resource, item_id: int) -> str:
    return f"https://{config.DOMAIN}/a/{resource.api_path}/{item_id}"
