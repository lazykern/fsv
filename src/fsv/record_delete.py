"""Reviewed problem and knowledge-article deletion through captured wires.

Captured (agent UI, JSON content type, no body):

* ``DELETE /api/_/problems/<n>`` -> soft delete; exact GET then shows
  ``deleted: true`` (listed in the deleted filter).
* ``PUT /api/_/problems/<n>/restore`` -> restored (``deleted: false``).
* ``DELETE /api/_/problems/delete-forever?ids=<n>`` (trash view) -> gone; the
  exact GET answers 404.
* ``DELETE /api/_/solutions/articles/<id>`` -> moved to the solutions trash.
* ``DELETE /api/_/solutions/articles/<id>/delete_forever`` -> exact GET 404.

Every write is hash-bound to the exact record snapshot, single-attempt and
journaled; readback is the deleted flag or a 404. Project deletion lives on a
different product host with its own session and is not implemented.
"""
from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any

from fsv import config, journal
from fsv.errors import APIError
from fsv.resources import PROBLEMS, parse_id
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute
from fsv.watchers import positive_id

PROBLEM_ACTIONS = {"delete", "restore", "delete-forever"}
ARTICLE_ACTIONS = {"delete", "delete-forever"}
DESTRUCTIVE = {("problem", "delete-forever"), ("article", "delete"), ("article", "delete-forever")}


def _get(client: Any, path: str, key: str, ident: int) -> dict[str, Any] | None:
    """Exact row, or None on an HTTP 404 (absent). Other failures raise."""
    try:
        raw = client.int_get(path)
    except APIError as exc:
        if getattr(exc, "status", None) == 404:
            return None
        raise ReviewedWriteError(f"exact {key} unavailable (HTTP {exc.status})") from None
    row = raw.get(key, raw) if isinstance(raw, dict) else None
    if not isinstance(row, dict) or positive_id(row.get("id")) != ident:
        raise ReviewedWriteError(f"exact {key} unavailable")
    return row


def _problem(client: Any, pid: int) -> dict[str, Any] | None:
    return _get(client, f"problems/{pid}", "problem", pid)


def _article(client: Any, aid: int) -> dict[str, Any] | None:
    return _get(client, f"solutions/articles/{aid}", "article", aid)


def problem_plan(client: Any, target: str | int, action: str) -> WritePlan:
    if action not in PROBLEM_ACTIONS:
        raise ReviewedWriteError(f"unknown problem action {action!r}")
    pid = positive_id(parse_id(str(target), PROBLEMS))
    row = _problem(client, pid)
    if row is None:
        raise ReviewedWriteError(f"problem {pid} not found; nothing to {action}")
    deleted = row.get("deleted")
    if not isinstance(deleted, bool):
        raise ReviewedWriteError("exact problem deleted flag unavailable")
    if action == "delete" and deleted:
        raise ReviewedWriteError("problem is already in the trash; use restore or delete-forever")
    if action in {"restore", "delete-forever"} and not deleted:
        raise ReviewedWriteError(f"{action} applies only to a problem in the trash (delete it first)")
    method, path = {"delete": ("DELETE", f"problems/{pid}"), "restore": ("PUT", f"problems/{pid}/restore"),
                    "delete-forever": ("DELETE", f"problems/delete-forever?ids={pid}")}[action]
    state = {"problem": row}
    if action == "delete":  # a delete after a restore is a new plan, not a replay
        state["state"] = journal.delete_state_tokens([row], [pid], ("problem_restore",))[0]
    return WritePlan("problem_" + action.replace("-", "_"), pid, config.DOMAIN, method, path, "null",
                     journal.snapshot_hash(state) or "")


def article_plan(client: Any, ident: int | str, action: str) -> WritePlan:
    if action not in ARTICLE_ACTIONS:
        raise ReviewedWriteError(f"unknown article action {action!r}")
    aid = positive_id(int(ident) if str(ident).isdigit() else ident)
    row = _article(client, aid)
    if row is None:
        raise ReviewedWriteError(f"article {aid} not found; nothing to delete")
    if action == "delete" and row.get("deleted"):
        raise ReviewedWriteError("article is already in the trash; use delete-forever")
    path = f"solutions/articles/{aid}" + ("/delete_forever" if action == "delete-forever" else "")
    return WritePlan("solution_" + action.replace("-", "_"), aid, config.DOMAIN, "DELETE", path, "null",
                     journal.snapshot_hash({"article": row}) or "")


MAX_FOREVER = 50


@dataclass(frozen=True)
class ForeverBatch:
    """Explicit IDs to delete permanently, one captured request per record, in order."""
    kind: str
    plans: tuple[WritePlan, ...]
    rows_json: str
    max_records: int | None

    @property
    def ids(self) -> list[int]:
        return [p.target for p in self.plans]

    @property
    def action(self) -> str:
        return self.plans[0].action

    @property
    def hash(self) -> str:
        return journal.plan_hash({"action": self.action, "plans": [p.hash for p in self.plans],
                                  "ids": self.ids, "max_records": self.max_records}) or ""

    @property
    def rows(self) -> list[dict[str, Any]]:
        return json.loads(self.rows_json)

    def review(self) -> dict[str, Any]:
        return {"action": self.action, "method": "DELETE", "requests": ["/api/_/" + p.path for p in self.plans],
                f"{self.kind}_ids": self.ids, "selected": self.rows, "max_records": self.max_records,
                "effect": f"PERMANENT: removes the listed {self.kind}(s); they cannot be restored.",
                "requires": "--confirm-destructive", "plan_hash": self.hash,
                "warning": f"One request per {self.kind}, in order; stops at the first failed or unverified one. "
                           "Never replay an uncertain outcome."}


def build_forever(client: Any, kind: str, idents: list[str], *, max_records: int | None) -> ForeverBatch:
    """Permanent delete of explicit IDs; more than one ID needs --max-records (1 to 50)."""
    if kind not in {"problem", "article"}:
        raise ValueError("unsupported delete-forever kind")
    if not idents:
        raise ValueError(f"give one or more explicit {kind} IDs")
    if max_records is not None and (type(max_records) is not int or not 1 <= max_records <= MAX_FOREVER):
        raise ValueError(f"--max-records must be 1 to {MAX_FOREVER}")
    if len(idents) > 1 and max_records is None:
        raise ValueError(f"deleting more than one {kind} permanently requires --max-records N (1 to {MAX_FOREVER})")
    if len(idents) > (max_records or 1):
        raise ValueError("more IDs than --max-records; refusing the whole plan; nothing was sent")
    build = (lambda i: problem_plan(client, i, "delete-forever")) if kind == "problem" else \
        (lambda i: article_plan(client, i, "delete-forever"))
    plans = [build(i) for i in idents]
    ids = [p.target for p in plans]
    if len(ids) != len(set(ids)):
        raise ValueError(f"give distinct {kind} IDs")
    getter = _problem if kind == "problem" else _article
    rows = []
    for pid in ids:
        row = getter(client, pid) or {}
        rows.append({"id": pid, "title": row.get("subject") if kind == "problem" else row.get("title")})
    return ForeverBatch(kind, tuple(plans), json.dumps(rows, sort_keys=True, ensure_ascii=False), max_records)


def send_forever(batch: ForeverBatch, client: Any, *, reviewed_hash: str | None,
                 confirm_destructive: bool = False) -> dict[str, Any]:
    if reviewed_hash is None:
        return {"status": "cancelled", "verified": False}
    if reviewed_hash != batch.hash:
        raise ReviewedWriteError("full review hash changed; review again")
    if confirm_destructive is not True:
        raise ReviewedWriteError(f"{batch.kind} delete-forever requires --confirm-destructive after inspecting the printed ID list")
    key = f"{batch.kind}_ids"
    results: dict[str, Any] = {}
    for index, sub in enumerate(batch.plans):
        try:
            result = send(sub, client, reviewed_hash=sub.hash, confirm_destructive=True)
        except ReviewedWriteError as exc:
            result = {"verified": False, "error": str(exc)}
        results[str(sub.target)] = {k: result.get(k) for k in ("verified", "error", "warning") if k in result}
        if not result.get("verified"):
            return {"status": "partial" if index else "failed", "verified": False, "plan_hash": batch.hash,
                    key: batch.ids, "results": results, "stopped_at": sub.target, "not_attempted": batch.ids[index + 1:],
                    "warning": f"Stopped at the first failed or unverified {batch.kind}; inspect it and the journal; never replay."}
    return {"status": "submitted", "verified": True, "plan_hash": batch.hash, key: batch.ids, "results": results}


def _kind(plan: WritePlan) -> tuple[str, str]:
    kind, _, act = plan.action.partition("_")
    return ("problem" if kind == "problem" else "article"), act.replace("_", "-")


def review(plan: WritePlan, client: Any) -> dict[str, Any]:
    kind, action = _kind(plan)
    row = (_problem if kind == "problem" else _article)(client, plan.target) or {}
    title = row.get("subject") if kind == "problem" else row.get("title")
    notes = {
        ("problem", "delete"): "Moves the problem to the trash; `fsv problems restore` brings it back.",
        ("problem", "restore"): "Restores the problem from the trash.",
        ("problem", "delete-forever"): "PERMANENT: removes the trashed problem; it cannot be restored.",
        ("article", "delete"): "Moves the article to the solutions trash; fsv has no captured article restore.",
        ("article", "delete-forever"): "PERMANENT: removes the article; it cannot be restored.",
    }
    return {**plan.review(), f"{kind}_id": plan.target, "title": title, "deleted": row.get("deleted"),
            "effect": notes[(kind, action)],
            "requires": "--confirm-destructive" if (kind, action) in DESTRUCTIVE else None,
            "warning": "Single attempt; an unknown outcome must be inspected, never automatically resent."}


def send(plan: WritePlan, client: Any, *, reviewed_hash: str | None, confirm_destructive: bool = False) -> dict[str, Any]:
    kind, action = _kind(plan)
    if reviewed_hash is not None and (kind, action) in DESTRUCTIVE and confirm_destructive is not True:
        raise ReviewedWriteError(f"{kind} {action} requires --confirm-destructive after inspecting the review")
    getter = _problem if kind == "problem" else _article
    rebuild = (lambda: problem_plan(client, plan.target, action)) if kind == "problem" else \
        (lambda: article_plan(client, plan.target, action))

    def verify(_response: Any) -> dict[str, Any]:
        row = getter(client, plan.target)
        if action == "delete-forever":
            state = "matched" if row is None else "mismatched"
            return {f"{kind}_id": plan.target, "fields": {"absent": state}, "verified": state == "matched"}
        want = action == "delete"
        if row is None:
            state = "matched" if kind == "article" and want else "mismatched"
        elif not isinstance(row.get("deleted"), bool):
            state = "unavailable"
        else:
            state = "matched" if row["deleted"] is want else "mismatched"
        return {f"{kind}_id": plan.target, "fields": {"deleted": state}, "verified": state == "matched"}

    from fsv.readback import bounded_readback
    return execute(plan, client, reviewed_hash=reviewed_hash, rebuild=rebuild,
                   verify=lambda response: bounded_readback(lambda: verify(response)))
