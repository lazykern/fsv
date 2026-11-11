"""Reviewed change-task CREATE from a blocked UI request, not a live response.

The response contract is unobserved. Never replay a POST to recover an ID.
"""
from __future__ import annotations

import html
import json
import re
from dataclasses import dataclass
from datetime import datetime
from typing import Any

import typer

from fsv import config, journal, views, task_status
from fsv.client import get_client
from fsv.errors import APIError
from fsv.resources import CHANGES, parse_id

STATUS_LABELS = ("Open", "In Progress", "Completed", "Blocked")
NOTIFY_BEFORE = {"Never": 0, "15 Minutes": 900, "30 Minutes": 1800,
                 "45 Minutes": 2700, "1 Hour": 3600, "2 Hours": 7200}
_ACTION = "change_task_create"
_UNKNOWN = "task outcome unknown; inspect the change and journal before any retry; do not resubmit"


class TaskError(RuntimeError):
    """Safe user-facing error, without raw response/session details."""


def _id(value: Any) -> int | None:
    if isinstance(value, bool) or not isinstance(value, (str, int)):
        return None
    if not re.fullmatch(r"[0-9]+", str(value)) or int(value) <= 0:
        return None
    return int(value)


def resolve_owner(value: str | None, client: Any = None) -> int | None:
    """No implicit self assignment; numeric user ID or exact active agent email."""
    if value is None or value == "":
        return None
    if _id(value) is not None:
        return _id(value)
    if not re.fullmatch(r"[^\s@<>,;]+@[^\s@<>,;]+\.[^\s@<>,;]+", value):
        raise ValueError("owner must be a positive user ID or exact email (not 'me')")
    try:
        c = client if client is not None else get_client()
        rows = c.autocomplete("agents", value)
        if not isinstance(rows, list):
            raise ValueError("unreadable lookup")
    except Exception:
        raise TaskError("could not resolve owner email; no task POST was attempted; use an explicit user ID if unavailable") from None
    matches = [r for r in rows if isinstance(r, dict) and str(r.get("email", "")).casefold() == value.casefold()
               and not r.get("deleted") and not r.get("deactivated")
               and "(Deactivated)" not in str(r.get("value", ""))]
    if len(matches) != 1 or _id(matches[0].get("user_id")) is None:
        raise TaskError("owner email must match exactly one active agent with a user ID; use an explicit user ID if unavailable")
    return _id(matches[0]["user_id"])


def resolve_group(value: str | None, client: Any = None) -> int | None:
    if value is None or not value.strip():
        return None
    if _id(value) is not None:
        return _id(value)
    try:
        c = client if client is not None else get_client()
        rows = c.int_get("bootstrap/agents_groups")["groups"]
        matches = [r for r in rows if isinstance(r, dict)
                   and str(r.get("name", "")).casefold() == value.casefold()]
        if len(matches) != 1 or _id(matches[0].get("id")) is None:
            raise ValueError()
        return _id(matches[0]["id"])
    except Exception:
        raise TaskError("group must resolve to exactly one exact name or be a positive ID; no POST attempted") from None


def status_label(value: str) -> str:
    if not isinstance(value, str) or not value.strip() or any(ord(c) < 32 for c in value):
        raise ValueError("status must be a nonempty runtime label or ID")
    return value.strip()


def resolve_status(value: str, client: Any = None) -> int:
    label = status_label(value)
    try:
        c = client if client is not None else get_client()
        return task_status.resolve(label, task_status.fields_from(c))[0]
    except Exception:
        raise TaskError(f"{label} requires an exact runtime task-status metadata ID; no ID inferred and no POST attempted") from None


def _date(value: str | None) -> str | None:
    if value is None or not value.strip():
        return None
    try:
        if "T" not in value:
            raise ValueError()
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
        offset = parsed.utcoffset()
        if offset is None or parsed.microsecond or offset.total_seconds() % 60:
            raise ValueError()
        return parsed.isoformat(timespec="seconds")
    except (ValueError, AttributeError):
        raise ValueError("planned dates require ISO-8601 date/time with explicit offset, whole seconds (no host timezone inferred)") from None


def _effort(value: str | None) -> str | None:
    if value is None or not value.strip():
        return None
    raw = value.strip()
    clock = re.fullmatch(r"([0-9]+):([0-5][0-9])", raw)
    units = re.fullmatch(r"(?:(\d+)h)?\s*(?:(\d+)m)?", raw)
    if clock:
        hours, minutes = map(int, clock.groups())
    elif units and any(units.groups()):
        hours, minutes = (int(v or 0) for v in units.groups())
    else:
        raise ValueError("planned effort must be duration text (1h30m) or HH:MM (01:30)")
    hours, minutes = divmod(hours * 60 + minutes, 60)
    return f"{hours}h {minutes}m"


def _note(value: str | None) -> str | None:
    if value is None or not value.strip():
        return None
    if any((ord(c) < 32 and c not in "\r\n\t") or ord(c) == 127 for c in value):
        raise ValueError("note must be plain text without unsafe control characters")
    return value


@dataclass(frozen=True)
class TaskPlan:
    change_id: int
    title: str
    owner_id: int | None
    tenant: str
    group_id: int | None = None
    status_label: str = "Open"
    status_id: int = 1
    note: str | None = None
    notify_before: str = "Never"
    planned_start: str | None = None
    planned_end: str | None = None
    planned_effort: str | None = None

    @property
    def path(self) -> str:
        return f"changes/{self.change_id}/tasks"

    @property
    def payload(self) -> dict[str, Any]:
        body = {"title": self.title, "status": self.status_id, "notify_before": NOTIFY_BEFORE[self.notify_before]}
        if self.note is not None:
            text = html.escape(self.note.replace("\r\n", "\n").replace("\r", "\n")).replace("\n", "<br>")
            body["description"] = f"<div><p>{text}</p></div>"
        for key, value in (("group_id", self.group_id), ("planned_start_date", self.planned_start),
                           ("planned_end_date", self.planned_end), ("planned_effort", self.planned_effort)):
            if value is not None:
                body[key] = value
        if self.owner_id is not None:
            body["owner_id"] = self.owner_id
        return body

    @property
    def hash(self) -> str:
        # Preserve existing default-plan hashes so old started/done guards survive.
        body = {"tenant": self.tenant, "path": self.path, "payload": self.payload}
        if self.status_label not in ("Open", "In Progress"):
            body["status_label"] = self.status_label
        return journal.plan_hash(body) or ""

    def review(self) -> dict[str, Any]:
        return {"action": _ACTION, "tenant": self.tenant, "change_id": self.change_id,
                "method": "POST", "path": f"/api/_/{self.path}", "payload": self.payload,
                "owner": self.owner_id, "status_label": self.status_label, "reminder": self.notify_before,
                "note_text": self.note,
                "plan_hash": self.hash,
                "warning": "Task creation/assignment may notify people or trigger tenant workflows. Never is a reminder setting, not a no-notification guarantee.",
                "contract": "Blocked request captured; accepted response unobserved. Blank optional fields omitted."}


def build_plan(change_id: int | str, title: str, *, owner_id: int | None = None,
               group_id: int | None = None, status: str = "Open", status_id: int | None = None,
               note: str | None = None, notify_before: str = "Never",
               planned_start: str | None = None, planned_end: str | None = None,
               planned_effort: str | None = None) -> TaskPlan:
    if isinstance(change_id, bool) or not isinstance(change_id, (int, str)):
        raise ValueError("change ID must be a positive display ID")
    cid = parse_id(str(change_id), CHANGES)
    if cid <= 0:
        raise ValueError("change ID must be positive")
    if not isinstance(title, str) or not title.strip() or any(ord(c) < 32 or ord(c) == 127 for c in title):
        raise ValueError("task title must be nonempty text without control characters")
    if owner_id is not None and (type(owner_id) is not int or owner_id <= 0):
        raise ValueError("owner ID must be a positive integer or none")
    if group_id is not None and (type(group_id) is not int or group_id <= 0):
        raise ValueError("group ID must be a positive integer or none")
    label = status_label(status)
    if type(status_id) is not int or status_id <= 0:
        raise ValueError("status ID must first be resolved from runtime metadata")
    reminder = next((k for k in NOTIFY_BEFORE if isinstance(notify_before, str)
                     and k.casefold() == notify_before.casefold()), None)
    if reminder is None:
        raise ValueError("notify before must be " + ", ".join(NOTIFY_BEFORE))
    start, end = _date(planned_start), _date(planned_end)
    if start and end and datetime.fromisoformat(end) < datetime.fromisoformat(start):
        raise ValueError("planned end must not precede planned start")
    return TaskPlan(cid, title, owner_id, config.DOMAIN, group_id, label, status_id,
                    _note(note), reminder, start, end, _effort(planned_effort))


def _inventory(client: Any, plan: TaskPlan) -> list[dict[str, Any]]:
    tab = views.load_tab(client, CHANGES, {"display_id": plan.change_id}, "tasks")
    if tab.missing or not tab.meta.get("complete") or not isinstance(tab.data, list):
        raise TaskError("task inventory is incomplete")
    if any(not isinstance(row, dict) or _id(row.get("id")) is None for row in tab.data):
        raise TaskError("task inventory contains an invalid task ID")
    return tab.data


def _matches(row: dict[str, Any], plan: TaskPlan) -> bool:
    owner = row.get("owner_id")
    return row.get("title") == plan.title and "owner_id" in row and (
        owner is None if plan.owner_id is None else _id(owner) == plan.owner_id)


def _compare_fields(exact: dict[str, Any] | None, plan: TaskPlan) -> tuple[list[str], list[str], list[str]]:
    verified_fields, unverified_fields, mismatched_fields = [], [], []
    for key, expected in plan.payload.items():
        if exact is None or key not in exact:
            unverified_fields.append(key)
            continue
        observed = exact[key]
        if key in ("group_id", "owner_id"):
            matched = _id(observed) == expected
        elif key in ("planned_start_date", "planned_end_date"):
            try:
                matched = datetime.fromisoformat(observed.replace("Z", "+00:00")) == datetime.fromisoformat(expected)
            except (ValueError, AttributeError, TypeError):
                matched = False
        else:
            matched = type(observed) is type(expected) and observed == expected
        (verified_fields if matched else mismatched_fields).append(key)
    return verified_fields, unverified_fields, mismatched_fields


def _returned_id(response: Any) -> int | None:
    if not isinstance(response, dict):
        return None
    task = response.get("task")
    if isinstance(task, dict):
        return _id(task.get("id"))
    return _id(response.get("id"))


def send(plan: TaskPlan, client: Any = None, *, reviewed_hash: str | None,
         read_only: bool = False, dry_run: bool = False) -> dict[str, Any]:
    """One reviewed POST; missing ID uses a complete before/after task inventory.

    An ambiguous result leaves the journal started, blocking identical plans.
    Returned IDs are never replaced with a title match when read-back fails.
    """
    if build_plan(plan.change_id, plan.title, owner_id=plan.owner_id, group_id=plan.group_id,
                  status=plan.status_label, status_id=plan.status_id, note=plan.note,
                  notify_before=plan.notify_before, planned_start=plan.planned_start,
                  planned_end=plan.planned_end, planned_effort=plan.planned_effort) != plan:
        raise TaskError("task plan or tenant changed; review a new plan")
    if dry_run:
        return {"status": "dry_run", **plan.review()}
    if read_only:
        raise TaskError("read-only mode: no task was sent")
    if reviewed_hash is None:
        return {"status": "cancelled", "change_id": plan.change_id}
    if reviewed_hash != plan.hash:
        raise TaskError("task review hash does not match; review the exact task again")
    if not plan.tenant:
        raise TaskError("configure a tenant before creating a task")
    events = [r for r in journal.read() if r.get("kind") == f"plan:{_ACTION}" and r.get("plan") == plan.hash]
    if events and events[-1].get("event") in ("started", "done"):
        raise TaskError("this task was sent or its outcome is unknown; inspect the change and journal; do not resubmit")
    try:
        c = client if client is not None else get_client()
        before = {_id(row["id"]) for row in _inventory(c, plan)}
        csrf = c._get_csrf()
    except Exception:
        raise TaskError("could not prepare a complete task inventory/session; no task POST was attempted") from None
    journal.record_plan("started", plan.hash, action=_ACTION, target=plan.change_id)
    events = [r for r in journal.read() if r.get("kind") == f"plan:{_ACTION}" and r.get("plan") == plan.hash]
    if not events or events[-1].get("event") != "started":
        raise TaskError("could not persist task journal; no task was sent")
    try:
        # int_post repeats writes on 403; the lower-level method does not.
        response = c._request("POST", f"{config.API_INT}/{plan.path}",
                              content=json.dumps(plan.payload, ensure_ascii=False).encode("utf-8"),
                              headers={"Content-Type": "application/json", "X-CSRF-Token": csrf})
    except APIError as exc:
        if 400 <= exc.status < 500 and exc.status not in (403, 408, 429):
            journal.record_plan("failed", plan.hash, action=_ACTION, target=plan.change_id)
            raise TaskError(f"task rejected (HTTP {exc.status}); no automatic retry") from None
        raise TaskError(_UNKNOWN) from None
    except Exception:
        raise TaskError(_UNKNOWN) from None
    tid = _returned_id(response)
    exact = None
    try:
        after = _inventory(c, plan)
        if tid is None:
            candidates = [r for r in after if _id(r["id"]) not in before and _matches(r, plan)
                          and not _compare_fields(r, plan)[2]]
            if len(candidates) == 1:
                exact = candidates[0]
                tid = _id(exact["id"])
        else:
            exact = next((r for r in after if _id(r["id"]) == tid), None)
    except Exception:
        pass
    if tid is None:
        raise TaskError(_UNKNOWN)
    journal.record_plan("done", plan.hash, action=_ACTION, target=plan.change_id, created_id=tid)
    identity_verified = exact is not None and tid not in before and _matches(exact, plan)
    verified_fields, unverified_fields, mismatched_fields = _compare_fields(exact, plan)
    verified = identity_verified and not mismatched_fields

    result = {"status": "submitted", "change_id": plan.change_id, "task_id": tid,
              "verified": verified, "identity_verified": identity_verified,
              "fields_verified": verified and not unverified_fields,
              "verified_fields": verified_fields, "unverified_fields": unverified_fields,
              "mismatched_fields": mismatched_fields, "readback": exact, "plan_hash": plan.hash}
    if not verified:
        result["warning"] = "Returned task ID/state not verified in task read-back; do not resend."
    elif unverified_fields:
        result["warning"] = "Task identity verified, but fields absent from GET remain unverified: " + ", ".join(unverified_fields) + "; do not resend."
    return result


def register(app: typer.Typer) -> None:
    @app.command("tasks-add", help="Review and create a change task (no implicit owner).")
    def tasks_add(
        id_: str = typer.Argument(..., metavar="CHANGE_ID"),
        title: str = typer.Option(..., "--title", help="exact task title to review"),
        owner: str | None = typer.Option(None, "--owner", help="explicit user ID or exact agent email; default none, no implicit self"),
        group: str | None = typer.Option(None, "--group", help="positive group ID or exact group name"),
        status: str = typer.Option("Open", "--status", help="exact active runtime status label or ID (including the default Open)"),
        note: str | None = typer.Option(None, "--note", help="plain text, HTML-escaped"),
        notify_before: str = typer.Option("Never", "--notify-before", help="Never, 15 Minutes, 30 Minutes, 45 Minutes, 1 Hour, 2 Hours"),
        planned_start: str | None = typer.Option(None, "--planned-start", help="ISO date/time with explicit timezone offset"),
        planned_end: str | None = typer.Option(None, "--planned-end", help="ISO date/time with explicit timezone offset"),
        planned_effort: str | None = typer.Option(None, "--planned-effort", help="1h30m or 01:30; sends 1h 30m"),
        dry_run: bool = typer.Option(False, "--dry-run", help="review only; email resolution may read, no POST or journal writes"),
        read_only: bool = typer.Option(False, "--read-only", help="refuse task creation"),
        yes: bool = typer.Option(False, "--yes", "-y", help="confirm the full previously reviewed --expect-plan"),
        expected: str | None = typer.Option(None, "--expect-plan", help="full hash from the exact task review"),
        no_input: bool = typer.Option(False, "--no-input", help="never prompt"),
        json_out: bool = typer.Option(False, "--json", help="emit structured result"),
    ) -> None:
        try:
            # Refuse sends before even resolving an email in read-only mode.
            if read_only and not dry_run:
                raise TaskError("read-only mode: no task was sent")
            options: dict[str, Any] = dict(note=note, notify_before=notify_before,
                           planned_start=planned_start, planned_end=planned_end, planned_effort=planned_effort)
            # Validate local inputs before resolving any remote values.
            status_label(status)
            build_plan(id_, title, status_id=1, **options)
            plan = build_plan(id_, title, owner_id=resolve_owner(owner), group_id=resolve_group(group),
                              status=status, status_id=resolve_status(status), **options)
            if dry_run:
                typer.echo(json.dumps(send(plan, reviewed_hash=None, dry_run=True), indent=2))
                return
            typer.echo(json.dumps(plan.review(), indent=2), err=True)
            if expected is not None and expected != plan.hash:
                raise TaskError("task review hash does not match; review the exact task again")
            if yes:
                if expected != plan.hash:
                    raise TaskError("--yes requires the full --expect-plan from a prior task review")
            elif no_input:
                raise TaskError("sending requires --yes and --expect-plan in no-input mode")
            elif not typer.confirm("Create this exact reviewed task?", default=False):
                typer.echo("Cancelled; no task was sent.")
                return
            result = send(plan, reviewed_hash=plan.hash)
            if json_out:
                typer.echo(json.dumps(result, indent=2))
            else:
                typer.echo(f"Task {result['task_id']} submitted on change {plan.change_id}; exact-ID/state read-back: {result['verified']}")
                if result.get("warning"):
                    typer.echo(result["warning"], err=True)
        except (ValueError, TaskError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None
