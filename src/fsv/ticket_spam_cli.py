"""CLI: single-ticket spam / not spam (reviewed)."""
from __future__ import annotations

from fsv.tenant import tenant_json

import json
from typing import Any, Callable, Optional

import typer

from fsv import ticket_spam
from fsv.errors import APIError
from fsv.reviewed_write import ReviewedWriteError
from fsv.ticket_trash_cli import _fail


def register(app: typer.Typer, client_factory: Callable[[], Any]) -> None:
    def command(spam: bool):
        def run(ticket: str = typer.Argument(..., metavar="TICKET"),
                confirm_destructive: bool = typer.Option(False, "--confirm-destructive", help="required for spam"),
                dry_run: bool = typer.Option(False, "--dry-run"), yes: bool = typer.Option(False, "--yes", "-y"),
                expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
                json_out: bool = typer.Option(False, "--json")) -> None:
            try:
                c = client_factory()
                plan = ticket_spam.build_plan(c, ticket, spam=spam)
                if dry_run:
                    typer.echo(tenant_json({"status": "dry_run", **plan.review()}, indent=2))
                    return
                if not yes or expect_plan != plan.hash:
                    raise ReviewedWriteError("review with --dry-run, then --yes --expect-plan FULL_HASH")
                result = ticket_spam.send(plan, c, reviewed_hash=expect_plan, confirm_destructive=confirm_destructive)
                typer.echo(tenant_json(result, indent=2))
                if not result.get("verified"):
                    raise typer.Exit(4)
            except (ValueError, ReviewedWriteError, APIError) as exc:
                _fail(exc)
        return run

    app.command("spam", help="Review, then mark one ticket as spam (needs --confirm-destructive); "
                             "verified by the spam flag. Bulk: tickets bulk spam.")(command(True))
    app.command("unspam", help="Review, then mark one ticket as not spam; verified by the spam flag.")(command(False))
