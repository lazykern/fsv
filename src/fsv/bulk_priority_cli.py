"""CLI for the captured explicit-ticket bulk-priority subset."""
from __future__ import annotations

import json
from typing import Callable, Optional

import typer

from fsv.reviewed_cli import ReviewOutput

from fsv import bulk_priority
from fsv.reviewed_write import ReviewedWriteError
from fsv.ticket_properties import PropertyError


def register(app: typer.Typer, client_factory: Callable) -> None:
    @app.command("bulk-priority", help="Review priority for explicit ticket IDs; no filter/all-pages expansion.")
    def bulk_priority_command(ids: list[str] = typer.Argument(..., metavar="ID..."),
                              priority: str = typer.Option(..., "--priority"),
                              dry_run: bool = typer.Option(False, "--dry-run"),
                              yes: bool = typer.Option(False, "--yes"),
                              output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o", help="output format (JSON only)"),
                              json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
                              expect_plan: Optional[str] = typer.Option(None, "--expect-plan")) -> None:
        try:
            c = client_factory()
            plan = bulk_priority.build_plan(c, ids, priority)
            if dry_run:
                typer.echo(json.dumps({"status": "dry_run", **plan.review()}, indent=2))
                return
            if not yes or expect_plan != plan.hash:
                raise ReviewedWriteError("review with --dry-run, then pass --yes and the full --expect-plan hash")
            result = bulk_priority.send(plan, c, reviewed_hash=expect_plan)
            typer.echo(json.dumps(result, indent=2))
            if not result.get("verified"):
                raise typer.Exit(1)
        except (ValueError, ReviewedWriteError, PropertyError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None
