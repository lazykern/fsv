"""Ticket task CLI: all writes require an exact full-hash review."""
from __future__ import annotations

import json
from typing import Any, Callable, Optional

import typer

from fsv import ticket_tasks as flow
from fsv.resources import TICKETS, parse_id
from fsv.reviewed_write import ReviewedWriteError


def register(app: typer.Typer, client_factory: Callable[[], Any]) -> None:
    tasks = typer.Typer(help="List, add, change status, or delete ticket tasks.")
    app.add_typer(tasks, name="tasks")

    @tasks.command("list")
    def list_(target: str, json_out: bool = typer.Option(False, "--json")) -> None:
        typer.echo(json.dumps(flow.inventory(client_factory(), parse_id(target, TICKETS)), indent=2))

    def run(target, action, task, title, status, sets, dry_run, yes, expected, owner=None, group=None):
        try:
            values = {}
            for item in sets or []:
                key, sep, value = item.partition("=")
                if not sep or not key or key in values:
                    raise ValueError("--set requires unique FIELD=VALUE")
                values[key] = value
            client = client_factory()
            plan = flow.build_plan(client, target, action=action, task=task, title=title, status=status, values=values, owner=owner, group=group)
            if dry_run:
                typer.echo(json.dumps({"status": "dry_run", **plan.review()}, indent=2))
                return
            if not yes or expected != plan.hash:
                raise ReviewedWriteError("review with --dry-run, then --yes --expect-plan FULL_HASH")
            result = flow.send(plan, client, reviewed_hash=expected)
            typer.echo(json.dumps(result, indent=2))
            if not result.get("verified"):
                raise typer.Exit(4)
        except (ValueError, ReviewedWriteError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None

    @tasks.command("add")
    def add(target: str, title: str = typer.Option(..., "--title"), status: str = typer.Option("Open", "--status"),
            owner: Optional[str] = typer.Option(None, "--owner"), group: Optional[str] = typer.Option(None, "--group"),
            set_: Optional[list[str]] = typer.Option(None, "--set"), dry_run: bool = typer.Option(False, "--dry-run"),
            yes: bool = typer.Option(False, "--yes"), expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
            json_out: bool = typer.Option(False, "--json")) -> None:
        run(target, "add", None, title, status, set_, dry_run, yes, expect_plan, owner, group)

    @tasks.command("update")
    def update(target: str, task: str, title: Optional[str] = typer.Option(None, "--title"),
               owner: Optional[str] = typer.Option(None, "--owner"), group: Optional[str] = typer.Option(None, "--group"),
               dry_run: bool = typer.Option(False, "--dry-run"), yes: bool = typer.Option(False, "--yes"),
               expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
               json_out: bool = typer.Option(False, "--json")) -> None:
        run(target, "update", task, title, None, [], dry_run, yes, expect_plan, owner, group)

    @tasks.command("status")
    def status_(target: str, task: str, to: str = typer.Option(..., "--to"), dry_run: bool = typer.Option(False, "--dry-run"),
                yes: bool = typer.Option(False, "--yes"), expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
                json_out: bool = typer.Option(False, "--json")) -> None:
        run(target, "status", task, None, to, [], dry_run, yes, expect_plan)

    @tasks.command("delete")
    def delete(target: str, task: str, dry_run: bool = typer.Option(False, "--dry-run"),
               yes: bool = typer.Option(False, "--yes"), expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
               json_out: bool = typer.Option(False, "--json")) -> None:
        run(target, "delete", task, None, "Open", [], dry_run, yes, expect_plan)
