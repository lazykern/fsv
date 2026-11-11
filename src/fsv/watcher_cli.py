"""CLI registration for self-watch management."""
from __future__ import annotations

import json
from typing import Any, Callable, Optional

import typer

from fsv.reviewed_cli import ReviewOutput

from fsv import watchers
from fsv.resources import Resource, parse_id
from fsv.reviewed_write import ReviewedWriteError


def register(app: typer.Typer, resource: Resource, client_factory: Callable[[], Any]) -> None:
    group = typer.Typer(help="Read watcher membership and review self-only watch changes.")

    @group.command("list")
    def list_(id_: str = typer.Argument(..., metavar="ID"),
              output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o", help="output format (JSON only)"),
              json_out: bool = typer.Option(False, "--json", help="alias for --output json")) -> None:
        try:
            typer.echo(json.dumps({"watcher_ids": watchers.inventory(client_factory(), resource.name,
                                 watchers.positive_id(parse_id(id_, resource)))}))
        except (ValueError, ReviewedWriteError) as exc:
            raise typer.BadParameter(str(exc)) from None

    @group.command("set")
    def set_(id_: str = typer.Argument(..., metavar="ID"),
             watch: bool = typer.Option(..., "--watch/--unwatch"),
             dry_run: bool = typer.Option(False, "--dry-run"),
             yes: bool = typer.Option(False, "--yes"),
             output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o", help="output format (JSON only)"),
             json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
             expect_plan: Optional[str] = typer.Option(None, "--expect-plan")) -> None:
        try:
            c = client_factory()
            plan = watchers.build_plan(c, resource.name, id_, watch=watch)
            if dry_run:
                typer.echo(json.dumps({"status": "dry_run", **plan.review()}, indent=2))
                return
            if not yes or expect_plan != plan.hash:
                raise ReviewedWriteError("review with --dry-run, then pass --yes and the full --expect-plan hash")
            typer.echo(json.dumps(watchers.send(plan, c, reviewed_hash=expect_plan), indent=2))
        except (ValueError, ReviewedWriteError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None

    app.add_typer(group, name="watchers")
