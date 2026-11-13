"""Private saved-view lifecycle (tickets and changes), gated on exact ownership.

The web UI's "Save as" dialog offers three visibility radios with fixed codes
(captured on tickets and changes alike): Myself = 3, All agents = 1, Agents in
groups = 2. Exact view reads carry only the number, never a label. A view is
treated as private when it is owned by the signed-in agent, is not a default
or system view, has an empty ``group_ids`` list and either carries a private
label or the Myself code. Anything else stays read-only here.
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

# Save-as dialog radio codes (platform UI constants, not tenant configuration).
VISIBILITY_MYSELF = 3
VISIBILITY_LABELS = {1: "All agents", 2: "Agents in groups", 3: "Myself"}
PRIVATE_LABELS = {"me", "only me", "private", "myself"}


@dataclass(frozen=True)
class ViewKind:
    module: str
    path: str
    envelope: str
    actions: frozenset[str]
    prefix: str


TICKETS = ViewKind("tickets", "ticket_filters", "ticket_filter", frozenset({"create", "edit", "delete"}),
                   "private_view_")
# Only the change-view create wire is captured; rename/delete stay refused.
CHANGES = ViewKind("changes", "change-filters", "change_filter", frozenset({"create"}), "change_private_view_")
KINDS = {"tickets": TICKETS, "changes": CHANGES}


@dataclass(frozen=True)
class ViewPlan(WritePlan):
    source_id: int


def _kind(kind: "ViewKind | str") -> ViewKind:
    return KINDS[kind] if isinstance(kind, str) else kind


def _detail(client: Any, view_id: int, kind: ViewKind) -> dict[str, Any]:
    raw = client.int_get(f"{kind.path}/{positive_id(view_id)}")
    row = raw.get(kind.envelope, raw) if isinstance(raw, dict) else None
    if not isinstance(row, dict) or positive_id(row.get("id")) != view_id:
        raise ReviewedWriteError("exact saved-view identity is unavailable")
    if row is not raw and isinstance(raw.get("meta"), dict) and "meta" not in row:
        row = {**row, "meta": raw["meta"]}
    return row


def detail(client: Any, view_id: int, kind: "ViewKind | str" = TICKETS) -> dict[str, Any]:
    return _detail(client, view_id, _kind(kind))


def private_visibility(row: dict[str, Any], user: int, kind: "ViewKind | str" = TICKETS) -> int:
    kind = _kind(kind)
    if (positive_id(row.get("user_id")) != user or row.get("default") is not False or
            row.get("system", False) is not False):
        raise ReviewedWriteError("only self-owned, explicitly non-default private views are supported")
    groups = row.get("group_ids", [])
    if groups != [] or row.get("group_id") is not None:
        raise ReviewedWriteError("the view is shared with groups; only views visible to you alone are supported")
    value = row.get("visibility")
    if isinstance(value, dict):
        label, value = value.get("label"), value.get("id")
    else:
        label = row.get("visibility_name")
    code = value if isinstance(value, int) and not isinstance(value, bool) else None
    if isinstance(label, str):
        if label.strip().casefold() not in PRIVATE_LABELS:
            raise ReviewedWriteError(f"view visibility is {label!r}, not private; nothing was sent")
        return positive_id(value)
    if code != VISIBILITY_MYSELF or "group_ids" not in row:
        shown = VISIBILITY_LABELS.get(code, "unknown") if code is not None else "unreadable"
        raise ReviewedWriteError(
            f"GET /api/_/{kind.path}/{row.get('id')} returns visibility {code if code is not None else 'unreadable'} "
            f"({shown}){'' if 'group_ids' in row else ' without group_ids'}; only Myself "
            f"({VISIBILITY_MYSELF}) with an empty group_ids list is treated as private (1 = All agents, "
            "2 = Agents in groups). Nothing was sent")
    return VISIBILITY_MYSELF


def _workspace_values(value: Any) -> list[Any]:
    if isinstance(value, list):
        return [v.get("id") if isinstance(v, dict) else v for v in value]
    return [value.get("id")] if isinstance(value, dict) else [value]


def _conditions(row: dict[str, Any], kind: ViewKind) -> list[Any]:
    if kind is CHANGES:
        advanced = row.get("advanced_query_hash")
        if row.get("query_hash") is not None or not isinstance(advanced, dict) or set(advanced) != {"all"} \
                or not isinstance(advanced.get("all"), list):
            raise ReviewedWriteError("only the captured change-view shape (advanced_query_hash.all) is supported")
        return advanced["all"]
    if row.get("advanced_query_hash") is not None or not isinstance(row.get("query_hash"), list):
        raise ReviewedWriteError("only the captured ordinary query-hash subset is supported")
    return row["query_hash"]


def _is_workspace(cond: Any) -> bool:
    return isinstance(cond, dict) and cond.get("condition") == "workspace_id"


def workspace(row: dict[str, Any], kind: "ViewKind | str" = TICKETS) -> int:
    """One workspace from the read: top-level id, ``meta.workspace_id`` and the
    workspace condition must agree (exact reads omit the top-level id)."""
    kind = _kind(kind)
    found: list[Any] = []
    if row.get("workspace_id") is not None:
        found.append(row["workspace_id"])
    meta = row.get("meta") if isinstance(row.get("meta"), dict) else {}
    if meta.get("workspace_id") is not None:
        found.extend(_workspace_values(meta["workspace_id"]))
    try:
        conditions = _conditions(row, kind)
    except ReviewedWriteError:
        conditions = []
    for cond in conditions:
        if _is_workspace(cond):
            found.extend(_workspace_values(cond.get("value")))
    try:
        ids = {positive_id(v) for v in found}
    except ReviewedWriteError:
        raise ReviewedWriteError("the view's workspace is unreadable; nothing was sent") from None
    if len(ids) != 1:
        raise ReviewedWriteError("the view's workspace is missing or ambiguous in its read; nothing was sent")
    return ids.pop()


def _write_conditions(conditions: list[Any], ws: int, kind: ViewKind) -> list[Any]:
    """Clone the runtime conditions unchanged except the workspace condition,
    whose read form (e.g. ``"2"``) differs from the write form the UI sends."""
    out = []
    for cond in conditions:
        if not isinstance(cond, dict):
            raise ReviewedWriteError("unreadable view condition")
        if _is_workspace(cond):
            if cond.get("operator") not in {"is", "is_in"}:
                raise ReviewedWriteError("unsupported workspace condition operator")
            cond = {**cond, "value": ws if kind is CHANGES else [{"id": ws}]}
        out.append(cond)
    return out


def _canonical(conditions: list[Any], ws: int) -> list[str]:
    rows = []
    for cond in conditions:
        if _is_workspace(cond):
            try:
                values = {positive_id(v) for v in _workspace_values(cond.get("value"))}
            except ReviewedWriteError:
                values = set()
            cond = {"condition": "workspace_id", "workspace": sorted(values) if values != {ws} else ws}
        rows.append(json.dumps(cond, sort_keys=True, ensure_ascii=False))
    return sorted(rows)


def build_plan(client: Any, source_id: int, *, action: str, name: str | None = None,
               kind: "ViewKind | str" = TICKETS) -> ViewPlan:
    kind = _kind(kind)
    action = "edit" if action == "rename" else action
    if action not in {"create", "edit", "delete"}:
        raise ValueError("unsupported saved-view action (create | rename | delete)")
    if action not in kind.actions:
        raise ReviewedWriteError(
            f"{kind.module} private-view {'rename' if action == 'edit' else action} is not captured "
            f"(PUT/DELETE /api/_/{kind.path}/<id> has no accepted wire); nothing was sent; use the web UI")
    source_id = positive_id(source_id)
    user = self_id(client)
    row = _detail(client, source_id, kind)
    visibility = private_visibility(row, user, kind)
    if action == "delete":
        if name is not None:
            raise ValueError("delete does not accept a name")
        payload, method, path = None, "DELETE", f"{kind.path}/{source_id}"
    else:
        if not isinstance(name, str) or not name.strip():
            raise ValueError("create/rename requires a nonempty --name")
        if row.get("order_by") != "created_at" or row.get("order_type") != "desc":
            raise ReviewedWriteError("only captured created-at descending views are supported")
        ws = workspace(row, kind)
        conditions = _write_conditions(_conditions(row, kind), ws, kind)
        payload = {"name": name, "workspace_id": ws, "visibility": visibility,
                   "order_by": row["order_by"], "order_type": row["order_type"]}
        if kind is CHANGES:
            payload.update(query_hash=None, advanced_query_hash={"all": conditions})
        else:
            payload.update(query_hash=conditions, advanced_query_hash=None)
        method, path = ("POST", kind.path) if action == "create" else ("PUT", f"{kind.path}/{source_id}")
    context = journal.snapshot_hash({"self": user, "source": row, "source_id": source_id}) or ""
    return ViewPlan(kind.prefix + action, source_id, config.DOMAIN, method, path,
                    json.dumps(payload, sort_keys=True, ensure_ascii=False), context, source_id)


def assess(client: Any, view_id: int, kind: "ViewKind | str" = TICKETS) -> dict[str, Any]:
    """Read-only: the view plus whether fsv would treat it as private."""
    kind = _kind(kind)
    row = _detail(client, view_id, kind)
    out: dict[str, Any] = {"view": row}
    try:
        out["visibility"] = VISIBILITY_LABELS.get(private_visibility(row, self_id(client), kind), "private")
        out["private"] = True
    except ReviewedWriteError as exc:
        out.update(private=False, reason=str(exc))
    try:
        out["workspace_id"] = workspace(row, kind)
    except ReviewedWriteError as exc:
        out["workspace_id"] = None
        out.setdefault("reason", str(exc))
    return out


def own_views(client: Any, kind: "ViewKind | str" = TICKETS) -> dict[str, Any]:
    """Self-owned non-default rows from the list; privacy is decided per exact read."""
    kind = _kind(kind)
    raw = client.int_get(kind.path)
    rows = None
    if isinstance(raw, dict):
        lists = [v for k, v in raw.items() if isinstance(v, list) and (k.endswith("filters") or k.endswith("views"))]
        rows = lists[0] if len(lists) == 1 else None
    elif isinstance(raw, list):
        rows = raw
    if not isinstance(rows, list):
        raise ReviewedWriteError("saved-view list shape is unreadable")
    user = self_id(client)
    meta = raw.get("meta") if isinstance(raw, dict) and isinstance(raw.get("meta"), dict) else {}
    editable = {str(v) for v in meta.get("editable_filter_ids") or []}
    out = [{"id": r.get("id"), "name": r.get("name"), "editable": str(r.get("id")) in editable if editable else None}
           for r in rows if isinstance(r, dict) and str(r.get("user_id")) == str(user) and r.get("default") is False]
    return {"views": out, "note": "the list carries no visibility; run show ID to see whether a view is private"}


def send(plan: ViewPlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    kind = CHANGES if plan.action.startswith(CHANGES.prefix) else TICKETS
    action = plan.action.removeprefix(kind.prefix)

    def verify(response: Any) -> dict[str, Any]:
        view_id = plan.source_id
        if action == "create":
            row = response.get(kind.envelope, response) if isinstance(response, dict) else None
            view_id = positive_id(row.get("id") if isinstance(row, dict) else None)
            if view_id == plan.source_id:
                raise ReviewedWriteError("create returned the source identity")
        if action == "delete":
            try:
                _detail(client, view_id, kind)
            except APIError as exc:
                if exc.status != 404:
                    raise
            else:
                return {"view_id": view_id, "verified": False}
            rows, meta, _ = collect_pages(client, kind.path, kind.path, {"page": 1, "per_page": 30},
                                          what="saved views")
            return {"view_id": view_id, "verified": bool(meta.get("complete")) and
                    not any(str(row.get("id")) == str(view_id) for row in rows)}
        row = _detail(client, view_id, kind)
        payload = plan.payload
        ws = payload["workspace_id"]
        fields: dict[str, str] = {}
        try:
            visibility = private_visibility(row, self_id(client), kind)
            fields["visibility"] = "matched" if visibility == payload["visibility"] else "mismatched"
        except ReviewedWriteError:
            fields["visibility"] = "mismatched"
        try:
            fields["workspace_id"] = "matched" if workspace(row, kind) == ws else "mismatched"
        except ReviewedWriteError:
            fields["workspace_id"] = "unavailable"
        for key in ("name", "order_by", "order_type"):
            fields[key] = "unavailable" if key not in row else "matched" if row[key] == payload[key] else "mismatched"
        sent = payload["advanced_query_hash"]["all"] if kind is CHANGES else payload["query_hash"]
        try:
            fields["conditions"] = ("matched" if _canonical(_conditions(row, kind), ws) == _canonical(sent, ws)
                                    else "mismatched")
        except ReviewedWriteError:
            fields["conditions"] = "unavailable"
        return {"view_id": view_id, "fields": fields, "verified": all(v == "matched" for v in fields.values())}

    return execute(plan, client, reviewed_hash=reviewed_hash,
                   rebuild=lambda: build_plan(client, plan.source_id, action=action,
                                              name=plan.payload["name"] if plan.payload else None, kind=kind),
                   verify=verify)
