"""Native list preferences/export requests, separate from local ls CSV output."""
from __future__ import annotations

import json
from typing import Any, Callable, Optional

import typer

from fsv import list_preferences as flow
from fsv.resources import Resource
from fsv.reviewed_cli import ReviewOutput
from fsv.reviewed_write import ReviewedWriteError


def register(app: typer.Typer, resource: Resource, client_factory: Callable[[], Any]) -> None:
    group = typer.Typer(help="Inspect/set native list columns (shared account preferences may be affected).")

    def finish(plan, client, dry_run, yes, expect_plan):
        if dry_run:
            typer.echo(json.dumps({"status": "dry_run", **plan.review()}, indent=2))
            return
        if not yes or expect_plan != plan.hash:
            raise ReviewedWriteError("review with --dry-run, then pass --yes and the full --expect-plan hash")
        typer.echo(json.dumps(flow.send(plan, client, reviewed_hash=expect_plan), indent=2))

    @group.command("list")
    def list_(output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o"),
              json_out: bool = typer.Option(False, "--json")) -> None:
        typer.echo(json.dumps(flow.headers(client_factory(), resource.name), indent=2))

    @group.command("set")
    def set_(column: list[str] = typer.Option(..., "--column", help="exact runtime column name, in desired order (repeatable)"),
             freeze: Optional[list[str]] = typer.Option(None, "--freeze", help="captured change requester freeze only; display_id is automatic"),
             dry_run: bool = typer.Option(False, "--dry-run"),
             yes: bool = typer.Option(False, "--yes"),
             expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
             output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o"),
             json_out: bool = typer.Option(False, "--json")) -> None:
        try:
            c = client_factory()
            finish(flow.columns_plan(c, resource.name, column, freeze or []), c, dry_run, yes, expect_plan)
        except (ValueError, ReviewedWriteError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None

    @app.command("export", help="Review-only: print the native CSV/30-day export request (body keys, recipient "
                                "mailbox); fsv never sends it (delivery unknown).")
    def export(view: str = typer.Option(..., "--view", help="saved view ID or unique name"),
               field: list[str] = typer.Option(..., "--field", help="captured export field key (repeatable)"),
               dry_run: bool = typer.Option(False, "--dry-run"),
               yes: bool = typer.Option(False, "--yes", hidden=True),
               expect_plan: Optional[str] = typer.Option(None, "--expect-plan", hidden=True),
               output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o"),
               json_out: bool = typer.Option(False, "--json")) -> None:
        try:
            if yes or expect_plan is not None:
                raise ReviewedWriteError("export is review-only in fsv (delivery cannot be verified); nothing was "
                                         "sent; use --dry-run to review, and the web UI to export")
            c = client_factory()
            plan = flow.export_plan(c, resource.name, view, field)
            typer.echo(json.dumps({"status": "dry_run", **plan.review()}, indent=2))
        except (ValueError, ReviewedWriteError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None

    app.add_typer(group, name="columns")
