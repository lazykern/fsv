"""Captured problem-task update, status-only completion and native deletion."""
from __future__ import annotations

import json
from typing import Optional
import typer

from fsv import problem_task_update as flow
from fsv.reviewed_write import ReviewedWriteError


def register(app, client_factory):
    @app.command("task-write", help="Review a problem task update, complete or delete; completion may rewrite planned dates.")
    def task_write(target: str, task: int, action: str,
                   title: Optional[str] = typer.Option(None, "--title"), status: Optional[str] = typer.Option(None, "--status"),
                   owner: Optional[str] = typer.Option(None, "--owner"), group: Optional[str] = typer.Option(None, "--group"),
                   note: Optional[str] = typer.Option(None, "--note"),
                   due_date: Optional[str] = typer.Option(None, "--due-date"),
                   planned_start: Optional[str] = typer.Option(None, "--planned-start"),
                   planned_end: Optional[str] = typer.Option(None, "--planned-end"),
                   planned_effort: Optional[str] = typer.Option(None, "--planned-effort"),
                   notify_before: Optional[str] = typer.Option(None, "--notify-before"),
                   dry_run: bool = typer.Option(False, "--dry-run"), yes: bool = typer.Option(False, "--yes"),
                   expect_plan: Optional[str] = typer.Option(None, "--expect-plan"), json_out: bool = typer.Option(False, "--json")):
        try:
            client = client_factory()
            inputs = {k: v for k, v in dict(title=title, status=status, owner=owner, group=group, note=note,
                due_date=due_date, planned_start=planned_start, planned_end=planned_end,
                planned_effort=planned_effort, notify_before=notify_before).items() if v is not None}
            if action not in {"update", "complete", "delete"}:
                raise ValueError("task action must be update, complete or delete")
            if action != "update" and inputs:
                raise ValueError("complete/delete do not accept update fields")
            if action == "complete":
                inputs = {"status": "Completed"}
            plan = flow.delete_plan(client, target, task) if action == "delete" else flow.build_plan(client, target, task, inputs)
            if dry_run:
                typer.echo(json.dumps({"status": "dry_run", **plan.review(),
                    "side_effects": "Completion may rewrite planned start/end and stop OLA timers; delete is irreversible."}, indent=2))
                return
            if not yes or expect_plan != plan.hash:
                raise ReviewedWriteError("review with --dry-run, then --yes --expect-plan FULL_HASH")
            sender = flow.delete if action == "delete" else flow.send
            result = sender(plan, client, reviewed_hash=expect_plan)
            typer.echo(json.dumps(result, indent=2))
            if not result.get("verified"):
                raise typer.Exit(4)
        except (ValueError, ReviewedWriteError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None
