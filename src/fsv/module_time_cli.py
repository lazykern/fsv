"""Change/problem captured time-entry CLI."""
from __future__ import annotations

import json
from typing import Any, Callable, Optional

import typer

from fsv.reviewed_cli import ReviewOutput

from fsv import module_time as ticket_time
from fsv.resources import CHANGES, PROBLEMS, parse_id
from fsv.reviewed_write import ReviewedWriteError


def register(app: typer.Typer, client_factory: Callable[[], Any], module: str) -> None:
    group = typer.Typer(help="Reviewed self-owned manual time entries; problem updates remain blocked.")

    @group.command("list")
    def list_(id_: str = typer.Argument(..., metavar="ID"),
              output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o", help="output format (JSON only)"),
              json_out: bool = typer.Option(False, "--json", help="alias for --output json")) -> None:
        typer.echo(json.dumps(ticket_time.inventory(client_factory(), module, parse_id(id_, CHANGES if module == "changes" else PROBLEMS)), indent=2))

    @group.command("write")
    def write(id_: str = typer.Argument(..., metavar="ID"),
              action: str = typer.Argument(..., help="create | update | delete"),
              entry: Optional[int] = typer.Option(None, "--entry", help="exact existing entry ID for update/delete"),
              spent: Optional[str] = typer.Option(None, "--spent", help="manual duration HH:MM"),
              executed_at: Optional[str] = typer.Option(None, "--executed-at", help="ISO timestamp with explicit offset"),
              note: Optional[str] = typer.Option(None, "--note"),
              billable: Optional[bool] = typer.Option(None, "--billable/--non-billable"),
              dry_run: bool = typer.Option(False, "--dry-run"),
              yes: bool = typer.Option(False, "--yes"),
              output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o", help="output format (JSON only)"),
              json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
              expect_plan: Optional[str] = typer.Option(None, "--expect-plan")) -> None:
        try:
            c = client_factory()
            plan = ticket_time.build_plan(c, module, id_, action=action, entry_id=entry, spent=spent,
                                          executed_at=executed_at, note=note, billable=billable)
            if dry_run:
                typer.echo(json.dumps({"status": "dry_run", **plan.review()}, indent=2))
                return
            if not yes or expect_plan != plan.hash:
                raise ReviewedWriteError("review with --dry-run, then pass --yes and the full --expect-plan hash")
            typer.echo(json.dumps(ticket_time.send(plan, c, reviewed_hash=expect_plan), indent=2))
        except (ValueError, ReviewedWriteError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None

    app.add_typer(group, name="time")
