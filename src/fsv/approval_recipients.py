"""Resolve agent-group/CAB selections to explicit eligible approval-user IDs."""
from __future__ import annotations
import json
from typing import Any
from fsv.reviewed_write import ReviewedWriteError
from fsv.watchers import positive_id


def _rows(raw: Any, key: str) -> list[dict[str, Any]]:
    if not isinstance(raw, dict) or not isinstance(raw.get(key), list) or any(not isinstance(r, dict) for r in raw[key]):
        raise ReviewedWriteError("recipient metadata unavailable")
    rows = raw[key]
    meta = raw.get("meta", {})
    if not isinstance(meta, dict) or any(meta.get(k) for k in ("has_more", "has_next", "next_page", "partial")) or meta.get("complete") is False:
        raise ReviewedWriteError("recipient metadata incomplete")
    if "count" in meta and (type(meta["count"]) is not int or meta["count"] != len(rows)):
        raise ReviewedWriteError("recipient metadata count mismatch")
    ids = [positive_id(r.get("id")) for r in rows]
    if len(set(ids)) != len(ids):
        raise ReviewedWriteError("duplicate recipient metadata identities")
    return rows


def members(client: Any, target: int, kind: str, ident: int) -> list[int]:
    if kind == "groups":
        # Count is a total in the captured group-member response. Each page is
        # validated before aggregation; no empty-page success on partial data.
        rows = []
        total = None
        for page in range(1, 101):
            raw = client.int_get(f"changes/{target}/groups/{ident}/members", params={"page": page, "per_page": 30})
            if not isinstance(raw, dict) or not isinstance(raw.get("approval_users"), list) or not isinstance(raw.get("meta"), dict):
                raise ReviewedWriteError("group members unavailable")
            count = raw["meta"].get("count")
            if type(count) is not int or count < 0 or total is not None and count != total or raw["meta"].get("partial"):
                raise ReviewedWriteError("group members total unavailable/inconsistent")
            total = count
            chunk = raw["approval_users"]
            rows.extend(chunk)
            if len(rows) == total:
                break
            if not chunk or len(rows) > total:
                raise ReviewedWriteError("group members incomplete")
        else:
            raise ReviewedWriteError("group members page limit exceeded")
        rows = _rows({"approval_users": rows}, "approval_users")
    elif kind == "cabs":
        raw = client.int_get(f"changes/{target}/cab-members", params={"cab_id": ident})
        # Native CAB lookups use approval_agents; agent-group lookups use
        # approval_users. Reject conflicting dual envelopes rather than guessing.
        key = "approval_agents" if isinstance(raw, dict) and "approval_agents" in raw else "approval_users"
        rows = _rows(raw, key)
        if isinstance(raw, dict) and key == "approval_agents" and "approval_users" in raw:
            if sorted(positive_id(r["id"]) for r in rows) != sorted(positive_id(r["id"]) for r in _rows(raw, "approval_users")):
                raise ReviewedWriteError("conflicting CAB recipient metadata")
    else:
        raise ValueError("unsupported recipient source")
    if not rows or any(r.get("deleted") or r.get("deactivated") for r in rows):
        raise ReviewedWriteError("empty or inactive group/CAB recipients; no request attempted")
    return sorted(positive_id(r["id"]) for r in rows)


def resolve(client: Any, target: int, groups: list[str], cabs: list[str]) -> tuple[list[int], str]:
    context = []
    for kind, values in (("groups", groups), ("cabs", cabs)):
        if not values:
            continue
        raw = client.int_get("bootstrap/agents_groups" if kind == "groups" else "cabs")
        choices = _rows(raw, kind)
        seen = set()
        for value in values:
            matches = [r for r in choices if str(r["id"]) == value or str(r.get("name", "")).casefold() == value.casefold()]
            if len(matches) != 1 or matches[0].get("deleted") or matches[0].get("deactivated"):
                raise ReviewedWriteError("group/CAB must resolve to one active runtime ID or exact name")
            ident = positive_id(matches[0]["id"])
            if ident in seen:
                raise ValueError("duplicate recipient source")
            seen.add(ident)
            context.append({"kind": kind, "id": ident, "members": members(client, target, kind, ident)})
    ids = sorted({i for source in context for i in source["members"]})
    return ids, json.dumps(context, sort_keys=True) if context else ""


def validate(client: Any, target: int, context_json: str) -> None:
    for source in json.loads(context_json or "[]"):
        if members(client, target, source["kind"], source["id"]) != source["members"]:
            raise ReviewedWriteError("group/CAB membership changed since review; review again")
