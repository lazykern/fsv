"""Integrated inventory search and reviewed CI/service association contracts."""
from __future__ import annotations

import json
from typing import Any

from fsv import config, journal
from fsv.resources import TICKETS, CHANGES, PROBLEMS, parse_id
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute
from fsv.watchers import positive_id

FAMILIES = {"devices": "device", "assets": "asset", "services": "businessservice"}
RESOURCES = {"tickets": TICKETS, "sr": TICKETS, "changes": CHANGES, "problems": PROBLEMS}


def target_id(module: str, target: str | int) -> tuple[str, int]:
    if module not in RESOURCES:
        raise ValueError("unsupported association module")
    return ("tickets" if module == "sr" else module, positive_id(parse_id(str(target), RESOURCES[module])))


def search(client: Any, query: str = "", *, kind: str = "devices", page: int = 1) -> dict[str, Any]:
    if kind not in FAMILIES or type(page) is not int or page < 1:
        raise ValueError("choose devices, assets or services and a positive page")
    raw = client._request("GET", f"https://{config.DOMAIN}/itam/core/{FAMILIES[kind]}/api/",
                          params={"order": "", "page": page, "page_size": 50, "q": query})
    if not isinstance(raw, dict) or raw.get("error") or not isinstance(raw.get("data"), list):
        raise ReviewedWriteError("integrated inventory unavailable")
    total = raw.get("total_rows")
    if type(total) is not int or total < 0 or raw.get("page") != page:
        raise ReviewedWriteError("integrated inventory pagination unavailable")
    ids = []
    for row in raw["data"]:
        if not isinstance(row, dict) or not isinstance(row.get("name"), dict):
            raise ReviewedWriteError("integrated inventory identity unavailable")
        ident = positive_id(row["name"].get("id"))
        pk = row.get(FAMILIES[kind] + "_pk")
        if pk is not None and positive_id(pk) != ident:
            raise ReviewedWriteError("conflicting inventory identities")
        ids.append(ident)
    if len(set(ids)) != len(ids) or total < len(ids):
        raise ReviewedWriteError("invalid inventory totals or duplicate identities")
    return {**raw, "kind": kind, "complete": page == 1 and len(ids) == total}


def inventory(client: Any, module: str, target: str | int, kind: str = "devices") -> list[dict[str, Any]]:
    module, target = target_id(module, target)
    if kind not in {"devices", "services"}:
        raise ValueError("association kind must be devices or services; physical asset writes are unproven")
    rows: list[dict[str, Any]] = []
    totals = set()
    pages = set()
    for page in range(1, 101):
        raw = client.int_get(f"{module}/{target}/associated-cis", params={"page": page, "association_type": "services" if kind == "services" else "cis"})
        if not isinstance(raw, dict) or not isinstance(raw.get("data"), list) or not isinstance(raw.get("meta"), dict):
            raise ReviewedWriteError("association inventory unavailable")
        meta = raw["meta"]
        if any(type(meta.get(k)) is not int or meta[k] < 0 for k in ("total_count", "total_pages")):
            raise ReviewedWriteError("association pagination unavailable")
        if meta.get("partial") or meta.get("complete") is False:
            raise ReviewedWriteError("association inventory incomplete")
        totals.add(meta["total_count"])
        pages.add(meta["total_pages"])
        for row in raw["data"]:
            if not isinstance(row, dict):
                raise ReviewedWriteError("invalid association row")
            positive_id(row.get("item_id"))
            positive_id(row.get("itam_cmdb_request_id"))
            if row.get("category") != ("Business service" if kind == "services" else "Device"):
                raise ReviewedWriteError("unproven association category")
            rows.append(row)
        if page >= meta["total_pages"]:
            break
        if not raw["data"]:
            raise ReviewedWriteError("association page missing")
    else:
        raise ReviewedWriteError("association page limit exceeded")
    for key in ("item_id", "itam_cmdb_request_id"):
        ids = [positive_id(r[key]) for r in rows]
        if len(set(ids)) != len(ids):
            raise ReviewedWriteError("duplicate association identities")
    if totals != {len(rows)} or len(pages) != 1:
        raise ReviewedWriteError("association inventory total mismatch")
    return sorted(rows, key=lambda row: positive_id(row["item_id"]))


def build_plan(client: Any, module: str, target: str | int, ids: list[int], *, kind: str = "devices", remove: bool = False) -> WritePlan:
    module, target = target_id(module, target)
    ids = sorted(positive_id(i) for i in ids)
    if not ids or len(ids) != len(set(ids)) or (remove and len(ids) != 1):
        raise ValueError("provide distinct inventory IDs; removal accepts exactly one ID")
    before = inventory(client, module, target, kind)
    current = {positive_id(r["item_id"]): r for r in before}
    if remove:
        if ids[0] not in current:
            raise ReviewedWriteError("inventory ID is not associated with this exact record")
        payload = {"itam_cmdb_request_id": positive_id(current[ids[0]]["itam_cmdb_request_id"])}
    else:
        if set(ids) & current.keys():
            raise ReviewedWriteError("requested inventory ID is already associated; omit it")
        payload = {"businessapp_ids" if kind == "services" else "device_ids": ids}
    return WritePlan("integrated_asset_remove" if remove else "integrated_asset_add", target, config.DOMAIN, "PUT",
                     f"{module}/{target}/{'disassociate-ci' if remove else 'associate-ci'}", json.dumps(payload, sort_keys=True),
                     journal.snapshot_hash(before) or "")


def send(plan: WritePlan, client: Any, module: str, ids: list[int], *, kind: str = "devices", remove: bool = False, reviewed_hash: str | None) -> dict[str, Any]:
    def verify(_response: Any) -> dict[str, Any]:
        rows = inventory(client, module, plan.target, kind)
        actual = {positive_id(r["item_id"]) for r in rows}
        return {"verified": not (set(ids) & actual) if remove else set(ids) <= actual, "readback": rows}
    return execute(plan, client, reviewed_hash=reviewed_hash,
                   rebuild=lambda: build_plan(client, module, plan.target, ids, kind=kind, remove=remove), verify=verify)

def verify_devices(client: Any, module: str, target: int, devices: list[str]) -> dict[str, Any]:
    try:
        rows = inventory(client, module, target)
        verified = {positive_id(i) for i in devices} <= {positive_id(r["item_id"]) for r in rows}
    except Exception:
        rows, verified = None, False
    return {"verified": verified, "readback": rows,
            **({} if verified else {"warning": "Create-time devices not independently verified; do not resend."})}

