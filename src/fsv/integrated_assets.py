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
    if (page > 1 and isinstance(raw, dict) and isinstance(raw.get("error"), str) and raw.get("data") == []
            and type(raw.get("total_rows")) is int and 0 <= raw["total_rows"] <= (page - 1) * 50):
        # Native out-of-range answer: {"error": "Requested page is out of range.", "data": [], "page": 1, ...}
        total = raw["total_rows"]
        raise ValueError(f"no results on page {page}: the {kind} inventory has {total} row(s) "
                         f"({max(1, -(-total // 50))} page(s) of 50)")
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


INTEGRATED = "integrated"
LEGACY = "legacy"
# Legacy (non-integrated ITAM) association reads, from read-only captures:
# changes answer {"associated_assets": [...]} on <id>/assets, tickets on
# <id>/associated_assets. Problems have no captured legacy read.
LEGACY_READS = {"changes": "changes/{id}/assets", "tickets": "tickets/{id}/associated_assets"}


def _present(client: Any, path: str, params: dict[str, Any] | None = None) -> bool:
    from fsv.errors import APIError
    try:
        client.int_get(path, params=params)
    except APIError as exc:
        if exc.status == 404:
            return False
        raise
    return True


def driver(client: Any, module: str, target: str | int, kind: str = "devices") -> str:
    """Which association endpoint this tenant exposes for the record, probed
    by read-only GETs (404 = absent): the integrated-ITAM ``associated-cis``
    (kept when present) or the legacy asset list. Neither: fail closed."""
    module, target = target_id(module, target)
    if _present(client, f"{module}/{target}/associated-cis",
                {"page": 1, "association_type": "services" if kind == "services" else "cis"}):
        return INTEGRATED
    legacy = LEGACY_READS.get(module)
    if legacy and _present(client, legacy.format(id=target)):
        return LEGACY
    raise ReviewedWriteError(f"no asset association endpoint found for {module} {target} "
                             "(associated-cis and the legacy asset list both answered 404); nothing read or sent")


def legacy_inventory(client: Any, module: str, target: str | int) -> list[dict[str, Any]]:
    """The legacy associated-assets list (one unpaginated response). Rows must
    carry a positive asset ``display_id`` (the id the legacy writes use)."""
    module, target = target_id(module, target)
    raw = client.int_get(LEGACY_READS[module].format(id=target))
    if not isinstance(raw, dict) or not isinstance(raw.get("associated_assets"), list):
        raise ReviewedWriteError("legacy asset association list unreadable")
    meta = raw.get("meta")
    if isinstance(meta, dict) and (meta.get("partial") or meta.get("has_more") or meta.get("next_page")):
        raise ReviewedWriteError("legacy asset association list incomplete")
    rows = []
    for row in raw["associated_assets"]:
        item = row.get("config_item") if isinstance(row, dict) else None
        if not isinstance(item, dict):
            raise ReviewedWriteError("legacy asset association row unreadable")
        positive_id(item.get("display_id"))
        rows.append(row)
    ids = [positive_id(r["config_item"]["display_id"]) for r in rows]
    if len(set(ids)) != len(ids):
        raise ReviewedWriteError("duplicate legacy asset associations")
    return sorted(rows, key=lambda r: positive_id(r["config_item"]["display_id"]))


# Legacy association writes (blocked-submit captures; responses unobserved):
# item ids are asset DISPLAY ids. Removal was not captured.
LEGACY_WRITES = {"changes": "changes/{id}/assets/associate", "tickets": "tickets/{id}/associate_assets"}
LEGACY_ENTITY = {"changes": "change", "tickets": "ticket"}


def legacy_candidates(client: Any, module: str, target: str | int) -> list[dict[str, Any]]:
    """Assets the web offers to associate (the requester's assets):
    ``GET assets-to-associate?entity=&user_id=<requester>&entity_id=``, all
    pages; incomplete pagination fails closed."""
    module, target = target_id(module, target)
    if module not in LEGACY_ENTITY:
        raise ReviewedWriteError("no legacy asset association for this module")
    raw = client.int_get(f"{module}/{target}")
    row = raw.get(module[:-1], raw) if isinstance(raw, dict) else None
    if not isinstance(row, dict) or str(row.get("id")) != str(target):
        raise ReviewedWriteError("exact record identity unavailable")
    requester = positive_id(row.get("requester_id"))
    rows: list[dict[str, Any]] = []
    for page in range(1, 51):
        data = client.int_get("assets-to-associate", params={
            "entity": LEGACY_ENTITY[module], "per_page": 30, "page": page, "user_id": requester,
            "entity_id": target, "include_meta": "true"})
        if not isinstance(data, dict) or not isinstance(data.get("assets"), list):
            raise ReviewedWriteError("asset candidates unreadable")
        total = (data.get("meta") or {}).get("total_count")
        if type(total) is not int or total < 0:
            raise ReviewedWriteError("asset candidate total unavailable")
        rows.extend(data["assets"])
        if len(rows) >= total:
            break
        if not data["assets"]:
            raise ReviewedWriteError("asset candidates incomplete")
    else:
        raise ReviewedWriteError("asset candidate page limit exceeded")
    ids = [positive_id(r.get("display_id")) if isinstance(r, dict) else positive_id(None) for r in rows]
    if len(set(ids)) != len(ids) or len(rows) != total:
        raise ReviewedWriteError("asset candidates inconsistent")
    return rows


def legacy_plan(client: Any, module: str, target: int, ids: list[int], *, kind: str = "devices",
                remove: bool = False) -> WritePlan:
    if remove:
        raise ReviewedWriteError("removing a legacy asset association has no captured wire; use the web UI; nothing sent")
    if kind != "devices" or module not in LEGACY_WRITES:
        raise ReviewedWriteError("legacy association writes cover assets on changes and tickets only; nothing sent")
    before = legacy_inventory(client, module, target)
    current = {positive_id(r["config_item"]["display_id"]) for r in before}
    if set(ids) & current:
        raise ReviewedWriteError("requested asset is already associated; omit it")
    offered = {positive_id(r["display_id"]) for r in legacy_candidates(client, module, target)}
    if not set(ids) <= offered:
        raise ReviewedWriteError("asset display ID is not among the assets the web offers for this record "
                                 "(assets-to-associate); nothing sent")
    return WritePlan("legacy_asset_add", target, config.DOMAIN, "PUT", LEGACY_WRITES[module].format(id=target),
                     json.dumps({"item_ids": ids}), journal.snapshot_hash(before) or "")


def listing(client: Any, module: str, target: str | int, kind: str = "devices") -> dict[str, Any]:
    """Read-only association list on whichever driver the tenant exposes."""
    if driver(client, module, target, kind) == INTEGRATED:
        return {"data": inventory(client, module, target, kind), "complete": True, "driver": INTEGRATED}
    if kind != "devices":
        raise ReviewedWriteError("the legacy asset association list has no business services; use the web UI")
    return {"data": legacy_inventory(client, module, target), "complete": True, "driver": LEGACY}


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
    if driver(client, module, target, kind) != INTEGRATED:
        return legacy_plan(client, module, target, ids, kind=kind, remove=remove)
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
    if plan.action == "legacy_asset_add":
        def verify_legacy(_response: Any) -> dict[str, Any]:
            # the PUT response shape is unobserved: only this independent read counts
            rows = legacy_inventory(client, module, plan.target)
            actual = {positive_id(r["config_item"]["display_id"]) for r in rows}
            return {"verified": set(ids) <= actual, "readback": rows, "driver": LEGACY,
                    "provenance": "legacy asset association: capture-based, not yet live-proven on any tenant"}
        return execute(plan, client, reviewed_hash=reviewed_hash, verify=verify_legacy,
                       rebuild=lambda: build_plan(client, module, plan.target, ids, kind=kind, remove=remove))

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

