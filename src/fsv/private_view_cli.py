"""Reviewed ticket-private-view lifecycle; no shared/default mutations."""
from __future__ import annotations

import json
from typing import Any, Callable, Optional

import typer

from fsv import private_views
from fsv.reviewed_cli import ReviewOutput
from fsv.reviewed_write import ReviewedWriteError


def register(app: typer.Typer, client_factory: Callable[[], Any]) -> None:
    group = typer.Typer(help="Private ticket views: clone, rename or delete an exact self-owned non-default view.")

    @group.command("show")
    def show(id_: int = typer.Argument(..., min=1),
             output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o"),
             json_out: bool = typer.Option(False, "--json")) -> None:
        typer.echo(json.dumps(private_views.detail(client_factory(), id_), indent=2))

    @group.command("write")
    def write(id_: int = typer.Argument(..., min=1, metavar="SOURCE_VIEW"),
              action: str = typer.Argument(..., help="create (clone source) | edit (rename) | delete"),
              name: Optional[str] = typer.Option(None, "--name"),
              dry_run: bool = typer.Option(False, "--dry-run"),
              yes: bool = typer.Option(False, "--yes"),
              expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
              output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o"),
              json_out: bool = typer.Option(False, "--json")) -> None:
        try:
            c = client_factory()
            plan = private_views.build_plan(c, id_, action=action, name=name)
            if dry_run:
                typer.echo(json.dumps({"status": "dry_run", **plan.review()}, indent=2))
                return
            if not yes or expect_plan != plan.hash:
                raise ReviewedWriteError("review with --dry-run, then pass --yes and the full --expect-plan hash")
            typer.echo(json.dumps(private_views.send(plan, c, reviewed_hash=expect_plan), indent=2))
        except (ValueError, ReviewedWriteError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None

    app.add_typer(group, name="private-views")
