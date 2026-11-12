"""CLI for reviewed ticket assignment and pick-up."""
from __future__ import annotations

import json
from typing import Any, Callable, Optional

import typer

from fsv import ticket_assignment as flow
from fsv.reviewed_cli import ReviewOutput
from fsv.reviewed_write import ReviewedWriteError
from fsv.ticket_properties import PropertyError


def register(app: typer.Typer, client_factory: Callable[[], Any]) -> None:
    def run(ids, agent, group, unassign, pickup, dry_run, yes, expected):
        try:
            client = client_factory()
            plan = flow.build_plan(client, ids, agent=agent, group=group, unassign=unassign, pickup=pickup)
            if dry_run:
                typer.echo(json.dumps({"status": "dry_run", **plan.review()}, indent=2))
                return
            if not yes or expected != plan.hash:
                raise ReviewedWriteError("review with --dry-run, then pass --yes and the full --expect-plan hash")
            typer.echo(json.dumps(flow.send(plan, client, reviewed_hash=expected), indent=2))
        except (ValueError, PropertyError, ReviewedWriteError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None

    @app.command("assign", help="Review ticket assignment, group assignment, or agent unassignment.")
    def assign(id_: str = typer.Argument(..., metavar="ID"),
               agent: Optional[str] = typer.Option(None, "--agent"),
               group: Optional[str] = typer.Option(None, "--group"),
               unassign: bool = typer.Option(False, "--unassign"),
               dry_run: bool = typer.Option(False, "--dry-run"),
               yes: bool = typer.Option(False, "--yes"),
               expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
               output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o"),
               json_out: bool = typer.Option(False, "--json")) -> None:
        run([id_], agent, group, unassign, False, dry_run, yes, expect_plan)

    @app.command("pick-up", help="Review picking up explicit tickets as the current agent.")
    def pick_up(ids: list[str] = typer.Argument(..., metavar="ID..."),
                dry_run: bool = typer.Option(False, "--dry-run"),
                yes: bool = typer.Option(False, "--yes"),
                expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
                output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o"),
                json_out: bool = typer.Option(False, "--json")) -> None:
        run(ids, None, None, False, True, dry_run, yes, expect_plan)
