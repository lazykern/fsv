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
             sets: Optional[list[str]] = typer.Option(None, "--set", help="status only: FIELD=VALUE the status's rules require; repeatable"),
             json_out: bool = typer.Option(False, "--json", help="accepted for symmetry; stdout is always JSON only (selection list goes to stderr)")):
        try:
            answers: dict[str, str] = {}
            for expression in sets or []:
                key, sep, value = expression.partition("=")
                if not sep or not key.strip() or key.strip() in answers:
                    raise ValueError("--set requires unique FIELD=VALUE answers")
                answers[key.strip()] = value
            client = client_factory()
            plan = ticket_bulk.build_plan(client, list(ids or []), action=action, where=list(where or []),
                select_all=select_all, max_records=max_records, status=status, agent=agent, group=group, primary=primary,
                note_primary=note_primary, note_secondary=note_secondary, answers=answers or None)
            # Human inspection list (destructive actions rely on it): stderr
            # only, so stdout stays pure JSON; JSON also carries the selection.
            typer.echo("Selected ticket IDs: " + ", ".join(map(str, plan.ids)), err=True)
            for row in json.loads(plan.selected_json):
                typer.echo(f"  {row['id']}: {row['subject']}", err=True)
            if dry_run:
                typer.echo(json.dumps({"status": "dry_run", **plan.review()}, indent=2))
                return
            if not yes or expect_plan != plan.hash:
                raise ReviewedWriteError("review with --dry-run, then --yes --expect-plan FULL_HASH")
            result = ticket_bulk.send(plan, client, reviewed_hash=expect_plan, confirm_destructive=confirm_destructive)
            result = {**result, "selected_tickets": json.loads(plan.selected_json)}
            typer.echo(json.dumps(result, indent=2))
            if not result.get("verified"):
                raise typer.Exit(4)
        except (ValueError, PropertyError, ReviewedWriteError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None
