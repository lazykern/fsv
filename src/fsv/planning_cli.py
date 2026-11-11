"""CLI for reviewed native planning text, separate from attachment workflows."""
from __future__ import annotations

import json
from typing import Any, Callable, Optional

import typer

from fsv import planning_text
from fsv.resources import Resource
from fsv.reviewed_write import ReviewedWriteError


def register(app: typer.Typer, resource: Resource, client_factory: Callable[[], Any]) -> None:
    @app.command("planning-text", help="Review a native planning/analysis text section; no files.")
    def planning(id_: str = typer.Argument(..., metavar="ID"),
                 section: str = typer.Option(..., "--section", help="exact metadata label/name; problems: Root Cause, Impact, Symptoms"),
                 description: str = typer.Option(..., "--description", help="text/HTML; reviewed verbatim"),
                 dry_run: bool = typer.Option(False, "--dry-run"),
                 yes: bool = typer.Option(False, "--yes"),
                 expect_plan: Optional[str] = typer.Option(None, "--expect-plan")) -> None:
        try:
            c = client_factory()
            plan = planning_text.build_plan(c, resource.name, id_, section, description)
            if dry_run:
                typer.echo(json.dumps({"status": "dry_run", **plan.review()}, indent=2))
                return
            if not yes or expect_plan != plan.hash:
                raise ReviewedWriteError("review with --dry-run, then pass --yes and the full --expect-plan hash")
            typer.echo(json.dumps(planning_text.send(plan, c, reviewed_hash=expect_plan), indent=2))
        except (ValueError, ReviewedWriteError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None
