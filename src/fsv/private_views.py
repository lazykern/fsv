"""Private ticket-view lifecycle, gated on runtime ownership/visibility metadata.

A numeric visibility code alone cannot prove a view is private. Accounts that
omit a named visibility in their view metadata remain read-only here.
"""
from __future__ import annotations

from dataclasses import dataclass
import json
from typing import Any

from fsv import config, journal
from fsv.errors import APIError
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute
from fsv.views import collect_pages
from fsv.watchers import positive_id, self_id


@dataclass(frozen=True)
class ViewPlan(WritePlan):
    source_id: int


def detail(client: Any, view_id: int) -> dict[str, Any]:
    raw = client.int_get(f"ticket_filters/{positive_id(view_id)}")
    row = raw.get("ticket_filter", raw) if isinstance(raw, dict) else None
    if not isinstance(row, dict) or positive_id(row.get("id")) != view_id:
        raise ReviewedWriteError("exact saved-view identity is unavailable")
    return row


def private_visibility(row: dict[str, Any], user: int) -> int:
    if (positive_id(row.get("user_id")) != user or row.get("default") is not False or
            row.get("system", False) is not False):
        raise ReviewedWriteError("only self-owned, explicitly non-default private views are supported")
    value = row.get("visibility")
    if isinstance(value, dict):
        label, value = value.get("label"), value.get("id")
    else:
        label = row.get("visibility_name")
    if not isinstance(label, str) or label.strip().casefold() not in {"me", "only me", "private"}:
        raise ReviewedWriteError("named private visibility metadata is unavailable; numeric visibility alone is not proof")
    return positive_id(value)


def build_plan(client: Any, source_id: int, *, action: str, name: str | None = None) -> ViewPlan:
    if action not in {"create", "edit", "delete"}:
        raise ValueError("unsupported saved-view action")
    source_id = positive_id(source_id)
    user = self_id(client)
    row = detail(client, source_id)
    visibility = private_visibility(row, user)
    if action == "delete":
        if name is not None:
            raise ValueError("delete does not accept a name")
        payload, method, path = None, "DELETE", f"ticket_filters/{source_id}"
    else:
        if not isinstance(name, str) or not name.strip():
            raise ValueError("create/edit requires a nonempty --name")
        if row.get("advanced_query_hash") is not None or not isinstance(row.get("query_hash"), list):
            raise ReviewedWriteError("only the captured ordinary query-hash subset is supported")
        if row.get("order_by") != "created_at" or row.get("order_type") != "desc":
            raise ReviewedWriteError("only captured created-at descending views are supported")
        # Clone the runtime query unchanged; no invented filter or enum IDs.
        payload = {"name": name, "workspace_id": positive_id(row.get("workspace_id")),
                   "visibility": visibility, "query_hash": row["query_hash"], "advanced_query_hash": None,
                   "order_by": row["order_by"], "order_type": row["order_type"]}
        method, path = ("POST", "ticket_filters") if action == "create" else ("PUT", f"ticket_filters/{source_id}")
    context = journal.snapshot_hash({"self": user, "source": row, "source_id": source_id}) or ""
    return ViewPlan("private_view_" + action, source_id, config.DOMAIN, method, path,
                    json.dumps(payload, sort_keys=True, ensure_ascii=False), context, source_id)


def send(plan: ViewPlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    action = plan.action.removeprefix("private_view_")

    def verify(response: Any) -> dict[str, Any]:
        view_id = plan.source_id
        if action == "create":
            row = response.get("ticket_filter", response) if isinstance(response, dict) else {}
            view_id = positive_id(row.get("id"))
            if view_id == plan.source_id:
                raise ReviewedWriteError("create returned the source identity")
        if action == "delete":
            try:
                detail(client, view_id)
            except APIError as exc:
                if exc.status != 404:
                    raise
            else:
                return {"view_id": view_id, "verified": False}
            rows, meta, _ = collect_pages(client, "ticket_filters", "ticket_filters",
                                          {"page": 1, "per_page": 30}, what="saved views")
            return {"view_id": view_id, "verified": bool(meta.get("complete")) and
                    not any(str(row.get("id")) == str(view_id) for row in rows)}
        row = detail(client, view_id)
        # Derive the enum from the same named metadata, not a tenant literal.
        visibility = private_visibility(row, self_id(client))
        fields = {key: "unavailable" if key not in row else
                  "matched" if (visibility if key == "visibility" else row[key]) == value else "mismatched"
                  for key, value in plan.payload.items()}
        return {"view_id": view_id, "fields": fields, "verified": all(v == "matched" for v in fields.values())}

    return execute(plan, client, reviewed_hash=reviewed_hash,
                   rebuild=lambda: build_plan(client, plan.source_id, action=action,
                                               name=plan.payload["name"] if plan.payload else None), verify=verify)
