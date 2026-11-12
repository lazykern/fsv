"""CLI for irreversible, reviewed ticket merges."""
from __future__ import annotations

import json
from typing import Any, Callable, Optional

import typer

from fsv import ticket_merge as flow
from fsv.reviewed_cli import ReviewOutput
from fsv.reviewed_write import ReviewedWriteError
from fsv.ticket_properties import PropertyError


def register(app: typer.Typer, client_factory: Callable[[], Any]) -> None:
    @app.command("merge", help="Irreversibly merge secondary tickets into the surviving primary.")
    def merge(primary: str = typer.Argument(..., metavar="PRIMARY"),
              secondary: list[str] = typer.Argument(..., metavar="SECONDARY..."),
              note_primary: str = typer.Option(..., "--note-primary"),
              note_secondary: str = typer.Option(..., "--note-secondary"),
              private_primary: bool = typer.Option(True, "--private-primary/--public-primary"),
              private_secondary: bool = typer.Option(True, "--private-secondary/--public-secondary"),
              dry_run: bool = typer.Option(False, "--dry-run"),
              yes: bool = typer.Option(False, "--yes"),
              expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
              output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o"),
              json_out: bool = typer.Option(False, "--json")) -> None:
        try:
            client = client_factory()
            plan = flow.build_plan(client, primary, secondary, note_primary=note_primary,
                                   note_secondary=note_secondary, private_primary=private_primary,
                                   private_secondary=private_secondary)
            if dry_run:
                typer.echo(json.dumps({"status": "dry_run", **flow.review(plan)}, indent=2, ensure_ascii=False))
                return
            if not yes or expect_plan != plan.hash:
                raise ReviewedWriteError("review with --dry-run, then pass --yes and the full --expect-plan hash")
            typer.echo(json.dumps(flow.send(plan, client, reviewed_hash=expect_plan), indent=2, ensure_ascii=False))
        except (ValueError, PropertyError, ReviewedWriteError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None
