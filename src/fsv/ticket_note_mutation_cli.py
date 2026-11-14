"""CLI for reviewed ticket note edits and deletes."""
from __future__ import annotations

from fsv.tenant import tenant_json

import json
from typing import Any, Callable, Optional

import typer

from fsv import ticket_note_mutation as flow
from fsv.reviewed_cli import ReviewOutput
from fsv.reviewed_write import ReviewedWriteError


def register(app: typer.Typer, client_factory: Callable[[], Any]) -> None:
    def run(ticket_id: str, note_id: str, body: str | None, dry_run: bool, yes: bool, expected: str | None) -> None:
        try:
            client = client_factory()
            plan = flow.build_plan(client, ticket_id, note_id, body=body)
            if dry_run:
                typer.echo(tenant_json({"status": "dry_run", **flow.review(plan, client)}, indent=2, ensure_ascii=False))
                return
            if not yes or expected != plan.hash:
                raise ReviewedWriteError("review with --dry-run, then pass --yes and the full --expect-plan hash")
            typer.echo(tenant_json(flow.send(plan, client, reviewed_hash=expected), indent=2, ensure_ascii=False))
        except (ValueError, ReviewedWriteError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None

    @app.command("note-edit", help="Review an edit to an exact ticket note.")
    def edit(id_: str = typer.Argument(..., metavar="ID"), conversation_id: str = typer.Argument(..., metavar="CONV_ID"),
             body: str = typer.Option(..., "--body"), dry_run: bool = typer.Option(False, "--dry-run"),
             yes: bool = typer.Option(False, "--yes"), expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
             output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o"),
             json_out: bool = typer.Option(False, "--json")) -> None:
        run(id_, conversation_id, body, dry_run, yes, expect_plan)

    @app.command("note-delete", help="Review deletion of an exact ticket note.")
    def delete(id_: str = typer.Argument(..., metavar="ID"), conversation_id: str = typer.Argument(..., metavar="CONV_ID"),
               dry_run: bool = typer.Option(False, "--dry-run"), yes: bool = typer.Option(False, "--yes"),
               expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
               output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o"),
               json_out: bool = typer.Option(False, "--json")) -> None:
        run(id_, conversation_id, None, dry_run, yes, expect_plan)
