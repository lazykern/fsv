"""Frozen bulk ticket selection CLI."""
from __future__ import annotations

import json
from typing import Optional
import typer

from fsv import ticket_bulk
from fsv.reviewed_write import ReviewedWriteError
from fsv.ticket_properties import PropertyError


def register(app, client_factory):
    @app.command("bulk", help="Review captured bulk actions on explicit IDs or a complete filtered expansion.")
    def bulk(action: str, ids: Optional[list[str]] = typer.Argument(None),
             where: Optional[list[str]] = typer.Option(None, "--where"),
             select_all: bool = typer.Option(False, "--select-all"),
             max_records: Optional[int] = typer.Option(None, "--max-records", help="Required filtered-write bound, 1 to 50."),
             status: Optional[str] = typer.Option(None, "--status"),
             agent: Optional[str] = typer.Option(None, "--agent"), group: Optional[str] = typer.Option(None, "--group"),
             primary: Optional[str] = typer.Option(None, "--primary"),
             note_primary: Optional[str] = typer.Option(None, "--note-primary"),
             note_secondary: Optional[str] = typer.Option(None, "--note-secondary"),
             confirm_destructive: bool = typer.Option(False, "--confirm-destructive"),
             dry_run: bool = typer.Option(False, "--dry-run"), yes: bool = typer.Option(False, "--yes"),
             expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
             json_out: bool = typer.Option(False, "--json")):
        try:
            client = client_factory()
            plan = ticket_bulk.build_plan(client, list(ids or []), action=action, where=list(where or []),
                select_all=select_all, max_records=max_records, status=status, agent=agent, group=group, primary=primary,
                note_primary=note_primary, note_secondary=note_secondary)
            typer.echo("Selected ticket IDs: " + ", ".join(map(str, plan.ids)), err=True)
            for row in json.loads(plan.selected_json):
                typer.echo(f"  {row['id']}: {row['subject']}", err=True)
            if dry_run:
                typer.echo(json.dumps({"status": "dry_run", **plan.review()}, indent=2))
                return
            if not yes or expect_plan != plan.hash:
                raise ReviewedWriteError("review with --dry-run, then --yes --expect-plan FULL_HASH")
            result = ticket_bulk.send(plan, client, reviewed_hash=expect_plan, confirm_destructive=confirm_destructive)
            typer.echo(json.dumps(result, indent=2))
            if not result.get("verified"):
                raise typer.Exit(4)
        except (ValueError, PropertyError, ReviewedWriteError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None
