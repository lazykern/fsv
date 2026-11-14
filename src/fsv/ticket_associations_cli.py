"""CLI: ticket-side problem, change and child-ticket associations (reviewed)."""
from __future__ import annotations

from fsv.tenant import tenant_json

import json
from typing import Any, Callable, Optional

import typer

from fsv import ticket_associations as links
from fsv.errors import APIError
from fsv.reviewed_write import ReviewedWriteError

KIND_HELP = "initiating (change_cause: the change caused this ticket) | initiated-by (change: this ticket initiated the change)"


def _fail(exc: Exception) -> None:
    if isinstance(exc, APIError):
        exc = ReviewedWriteError(f"read failed (HTTP {exc.status}); nothing was sent")
    typer.echo(str(exc), err=True)
    raise typer.Exit(2) from None


def register(app: typer.Typer, client_factory: Callable[[], Any]) -> None:
    specs = [("problems", "problem", "PROBLEM", "Problems linked to an incident: list, candidates, link, unlink."),
             ("changes", "change", "CHANGE", "Changes linked to a ticket in either direction: list, candidates, link, unlink."),
             ("children", "child", "CHILD_TICKET", "Child tickets of a parent ticket: list, candidates, link, unlink.")]
    for name, kind, metavar, help_text in specs:
        app.add_typer(_group(kind, metavar, help_text, client_factory), name=name)


def _group(kind: str, metavar: str, help_text: str, client_factory: Callable[[], Any]) -> typer.Typer:
    group = typer.Typer(help=help_text)
    needs_type = kind == "change"

    @group.command("list", help="Complete associated list for the exact ticket (read-only).")
    def list_(ticket: str = typer.Argument(..., metavar="TICKET"),
              ctype: Optional[str] = typer.Option(None, "--kind", help=KIND_HELP if needs_type else "changes only"),
              json_out: bool = typer.Option(False, "--json")) -> None:
        try:
            if needs_type and not ctype:
                raise ValueError("--kind is required: " + KIND_HELP)
            typer.echo(tenant_json(links.listing(client_factory(), ticket, kind, ctype), indent=2))
        except (ValueError, ReviewedWriteError, APIError) as exc:
            _fail(exc)

    @group.command("candidates", help="Picker list or search results to choose from (read-only, one page).")
    def candidates(ticket: str = typer.Argument(..., metavar="TICKET"),
                   search: Optional[str] = typer.Option(None, "--search"),
                   ctype: Optional[str] = typer.Option(None, "--kind", help=KIND_HELP if needs_type else "changes only"),
                   json_out: bool = typer.Option(False, "--json")) -> None:
        try:
            if needs_type and not ctype and not search:
                raise ValueError("--kind is required for the picker list: " + KIND_HELP)
            typer.echo(tenant_json(links.candidates(client_factory(), ticket, kind, search, ctype or "change"
                                                   if needs_type else None), indent=2))
        except (ValueError, ReviewedWriteError, APIError) as exc:
            _fail(exc)

    def mutate(unlink: bool):
        def command(ticket: str = typer.Argument(..., metavar="TICKET"),
                    item: str = typer.Argument(..., metavar=metavar),
                    ctype: Optional[str] = typer.Option(None, "--kind", help=KIND_HELP if needs_type else "changes only"),
                    dry_run: bool = typer.Option(False, "--dry-run"), yes: bool = typer.Option(False, "--yes", "-y"),
                    expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
                    json_out: bool = typer.Option(False, "--json")) -> None:
            try:
                if needs_type and not ctype:
                    raise ValueError("--kind is required: " + KIND_HELP)
                if not needs_type and ctype:
                    raise ValueError("--kind applies to changes only")
                c = client_factory()
                plan = links.build_plan(c, ticket, kind, item, unlink=unlink, ctype=ctype)
                if dry_run:
                    typer.echo(tenant_json({"status": "dry_run", **plan.review()}, indent=2))
                    return
                if not yes or expect_plan != plan.hash:
                    raise ReviewedWriteError("review with --dry-run, then --yes --expect-plan FULL_HASH")
                result = links.send(plan, c, reviewed_hash=expect_plan)
                typer.echo(tenant_json(result, indent=2))
                if not result.get("verified"):
                    raise typer.Exit(4)
            except (ValueError, ReviewedWriteError, APIError) as exc:
                _fail(exc)
        return command

    group.command("link", help="Review, then link once and verify by the complete associated list.")(mutate(False))
    group.command("unlink", help="Review, then unlink once and verify by the complete associated list.")(mutate(True))
    return group
