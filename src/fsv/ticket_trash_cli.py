"""CLI: ticket trash list, restore, and permanent deletion (reviewed)."""
from __future__ import annotations

from fsv.tenant import tenant_json

import json
from typing import Any, Callable, List, Optional

import typer

from fsv import ticket_trash
from fsv.errors import APIError
from fsv.reviewed_write import ReviewedWriteError


def _fail(exc: Exception) -> None:
    if isinstance(exc, APIError):
        exc = ReviewedWriteError(f"read failed (HTTP {exc.status}); nothing was sent")
    typer.echo(str(exc), err=True)
    raise typer.Exit(2) from None


def register(app: typer.Typer, client_factory: Callable[[], Any]) -> None:
    trash = typer.Typer(help="Deleted tickets (the Trash view; every agent's deleted tickets).")

    @trash.command("list", help="One page of the Trash view (read-only).")
    def trash_list(page: int = typer.Option(1, "--page", min=1), per_page: int = typer.Option(30, "--per-page", min=1, max=100),
                   json_out: bool = typer.Option(False, "--json")) -> None:
        try:
            typer.echo(tenant_json(ticket_trash.trash(client_factory(), page=page, per_page=per_page), indent=2))
        except (ValueError, ReviewedWriteError, APIError) as exc:
            _fail(exc)

    app.add_typer(trash, name="trash")

    @app.command("restore", help="Review, then restore one deleted ticket from the trash; verified by deleted=false.")
    def restore(ticket: str = typer.Argument(..., metavar="TICKET"),
                dry_run: bool = typer.Option(False, "--dry-run"), yes: bool = typer.Option(False, "--yes", "-y"),
                expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
                json_out: bool = typer.Option(False, "--json")) -> None:
        try:
            c = client_factory()
            plan = ticket_trash.build_restore(c, ticket)
            if dry_run:
                typer.echo(tenant_json({"status": "dry_run", **plan.review()}, indent=2))
                return
            if not yes or expect_plan != plan.hash:
                raise ReviewedWriteError("review with --dry-run, then --yes --expect-plan FULL_HASH")
            result = ticket_trash.send_restore(plan, c, reviewed_hash=expect_plan)
            typer.echo(tenant_json(result, indent=2))
            if not result.get("verified"):
                raise typer.Exit(4)
        except (ValueError, ReviewedWriteError, APIError) as exc:
            _fail(exc)

    @app.command("delete-forever", help="PERMANENTLY delete explicit tickets that are already in the trash. "
                                        "Needs --max-records and --confirm-destructive; no filters, no empty-trash.")
    def delete_forever(tickets: List[str] = typer.Argument(..., metavar="TICKET..."),
                       max_records: Optional[int] = typer.Option(None, "--max-records", help="required bound, 1 to 50"),
                       confirm_destructive: bool = typer.Option(False, "--confirm-destructive"),
                       dry_run: bool = typer.Option(False, "--dry-run"), yes: bool = typer.Option(False, "--yes", "-y"),
                       expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
                       json_out: bool = typer.Option(False, "--json")) -> None:
        try:
            c = client_factory()
            plan = ticket_trash.build_forever(c, list(tickets), max_records=max_records)
            typer.echo("Tickets to delete PERMANENTLY: " + ", ".join(map(str, plan.ids)), err=True)
            for row in json.loads(plan.subjects_json):
                typer.echo(f"  {row['id']}: {row['subject']}", err=True)
            if dry_run:
                typer.echo(tenant_json({"status": "dry_run", **plan.review()}, indent=2))
                return
            if not yes or expect_plan != plan.hash:
                raise ReviewedWriteError("review with --dry-run, then --yes --expect-plan FULL_HASH --confirm-destructive")
            result = ticket_trash.send_forever(plan, c, reviewed_hash=expect_plan,
                                               confirm_destructive=confirm_destructive)
            typer.echo(tenant_json(result, indent=2))
            if not result.get("verified"):
                raise typer.Exit(4)
        except (ValueError, ReviewedWriteError, APIError) as exc:
            _fail(exc)
