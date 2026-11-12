"""Captured native list-column preferences and CSV export-request subset."""
from __future__ import annotations

from dataclasses import dataclass
import json
from typing import Any

from fsv import config, journal
from fsv.resources import REGISTRY
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute
from fsv.views import find_view, load_views


@dataclass(frozen=True)
class ListPlan(WritePlan):
    inputs_json: str


def headers(client: Any, module: str) -> list[dict[str, Any]]:
    if module not in REGISTRY:
        raise ValueError("unsupported list module")
    raw = client.int_get(f"{module}/headers")
    rows = raw.get("headers") if isinstance(raw, dict) else None
    if not isinstance(rows, list) or not rows or any(not isinstance(row, dict) or not row.get("name") for row in rows):
        raise ReviewedWriteError("runtime list-header metadata is unavailable")
    if len({row["name"] for row in rows}) != len(rows):
        raise ReviewedWriteError("ambiguous runtime list-header keys")
    return rows


def columns_plan(client: Any, module: str, columns: list[str], frozen: list[str]) -> ListPlan:
    rows = headers(client, module)
    names = {row["name"] for row in rows}
    if not columns or len(set(columns)) != len(columns) or set(columns) - names or "display_id" in columns:
        raise ValueError("choose unique exact runtime column names; display_id is anchored automatically")
    if "ticket_subject" not in columns:
        raise ValueError("captured preference contract requires ticket_subject")
    if len(set(frozen)) != len(frozen) or set(frozen) - set(columns):
        raise ValueError("frozen columns must be unique selected columns")
    if frozen and (module != "changes" or frozen != ["requester"]):
        raise ValueError("only change requester freezing plus the automatic display_id anchor is captured")
    payload = {"columns": {key: i + 1 for i, key in enumerate(columns)}, "frozen_columns": [*frozen, "display_id"]}
    payload["columns"]["display_id"] = payload["columns"]["ticket_subject"]
    return ListPlan("list_columns", 0, config.DOMAIN, "POST", f"{module}/headers",
                    json.dumps(payload, sort_keys=True), journal.snapshot_hash(rows) or "",
                    json.dumps({"module": module, "columns": columns, "frozen": frozen}))


_EXPORT_KEYS = {"subject": "ticket_subject", "priority_name": "priority", "status_name": "status",
                "group_name": "group", "responder_name": "assigned_to", "owner": "assigned_to",
                "display_id": "display_id", "requester_name": "requester", "requester_info": "requester",
                "department_name": "department_name"}


def export_source(client: Any, module: str, view: str) -> tuple[Any, dict[str, Any]]:
    views, missing = load_views(client, REGISTRY[module])
    if missing:
        raise ReviewedWriteError("saved-view inventory is unavailable")
    selected = find_view(views, view)
    path = REGISTRY[module].filters_path
    raw = client.int_get(f"{path}/{selected.id}")
    row = raw.get(REGISTRY[module].item_key + "_filter", raw) if isinstance(raw, dict) else None
    if not isinstance(row, dict) or str(row.get("id")) != selected.id:
        raise ReviewedWriteError("exact saved-view settings are unavailable")
    if not isinstance(row.get("query_hash"), list) or row.get("advanced_query_hash") is not None:
        raise ReviewedWriteError("only ordinary saved-view queries have a captured export contract")
    return selected, row


def export_plan(client: Any, module: str, view: str, fields: list[str]) -> ListPlan:
    rows = headers(client, module)
    labels = {row["name"]: row.get("label") for row in rows}
    allowed = set(_EXPORT_KEYS) - ({"owner", "department_name"} if module == "tickets" else {"responder_name", "requester_name"})
    if not fields or len(set(fields)) != len(fields) or set(fields) - allowed:
        raise ValueError("select unique captured export field keys")
    export_fields = {}
    for key in fields:
        label = labels.get(_EXPORT_KEYS[key])
        if not isinstance(label, str) or not label.strip():
            raise ReviewedWriteError("runtime export-field label is unavailable for " + key)
        export_fields[key] = label
    selected, row = export_source(client, module, view)
    # CSV / 30 days / created_at are the only captured export-dialog variant,
    # not guessed enum IDs or options inferred from another endpoint.
    payload = {"format": "csv", "date_filter": 30, "export_fields": export_fields,
               "custom_fields": {}, "data_hash": row["query_hash"]}
    if module == "tickets":
        payload.update(ticket_state_filter="created_at", advanced_data_hash=None,
                       selected_filter_view={"id": selected.id, "name": selected.name,
                                             "is_additional_filters_applied": False})
    else:
        payload["state_filter"] = "created_at"
        if module == "changes":
            payload["advanced_data_hash"] = None
    context = journal.snapshot_hash({"headers": rows, "view": row, "selection": selected.to_json()}) or ""
    return ListPlan("list_export", 0, config.DOMAIN, "POST", f"{module}/export",
                    json.dumps(payload, sort_keys=True), context,
                    json.dumps({"module": module, "view": selected.id, "fields": fields}))


def send(plan: ListPlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    inputs = json.loads(plan.inputs_json)
    columns = plan.action == "list_columns"

    def verify(response: Any) -> dict[str, Any]:
        if not columns:
            # No captured job/download lookup: do not invent one or claim a file.
            _, row = export_source(client, inputs["module"], inputs["view"])
            return {"verified": False, "source_readback": str(row.get("id")) == inputs["view"],
                    "job": "unavailable", "download": "unavailable",
                    "warning": "Export requested once; delivery/job/download effects are unverified. Do not resend."}
        rows = headers(client, inputs["module"])
        by_name = {row["name"]: row for row in rows}
        fields = {}
        for key, position in plan.payload["columns"].items():
            row = by_name.get(key, {})
            fields[key] = ("unavailable" if not all(k in row for k in ("selected", "position", "frozen")) else
                           "matched" if row["selected"] is True and row["position"] == position and
                           row["frozen"] is (key in plan.payload["frozen_columns"]) else "mismatched")
        omitted = [row["name"] for row in rows if row["name"] not in plan.payload["columns"] and row.get("selected") is not False]
        return {"fields": fields, "unverified_omissions": omitted,
                "verified": not omitted and all(value == "matched" for value in fields.values())}

    return execute(plan, client, reviewed_hash=reviewed_hash,
                   rebuild=lambda: columns_plan(client, **inputs) if columns else export_plan(client, **inputs), verify=verify)
