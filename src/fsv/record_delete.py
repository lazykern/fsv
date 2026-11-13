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
    return WritePlan("problem_" + action.replace("-", "_"), pid, config.DOMAIN, method, path, "null",
                     journal.snapshot_hash({"problem": row}) or "")


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
