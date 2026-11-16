"""Reviewed private saved-view lifecycle (tickets, changes); no shared/default mutations."""
from __future__ import annotations

from fsv.tenant import tenant_json

import json
from typing import Any, Callable, Optional

import typer

from fsv import private_views
from fsv.errors import APIError
from fsv.reviewed_cli import ReviewOutput
from fsv.reviewed_write import ReviewedWriteError


def register(app: typer.Typer, client_factory: Callable[[], Any], module: str = "tickets") -> None:
    kind = private_views.KINDS[module]
    actions = "create (clone source) | rename | delete"
    group = typer.Typer(help=f"Private {module[:-1]} views: list, show, and reviewed {actions} "
                             "on an exact self-owned non-default view visible only to you.")

    def _fail(exc: Exception) -> None:
        typer.echo(str(exc), err=True)
        raise typer.Exit(2) from None

    @group.command("list", help="Your own non-default saved views (the list carries no visibility).")
    def list_(output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o"),
              json_out: bool = typer.Option(False, "--json")) -> None:
        try:
            typer.echo(tenant_json(private_views.own_views(client_factory(), kind), indent=2))
        except (ReviewedWriteError, APIError) as exc:
            _fail(exc)

    @group.command("show", help="Exact view read plus whether fsv treats it as private (Myself, no groups).")
    def show(id_: int = typer.Argument(..., min=1),
             output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o"),
             json_out: bool = typer.Option(False, "--json")) -> None:
        try:
            typer.echo(tenant_json(private_views.assess(client_factory(), id_, kind), indent=2))
        except (ReviewedWriteError, APIError) as exc:
            _fail(exc)

    @group.command("write")
    def write(id_: int = typer.Argument(..., min=1, metavar="SOURCE_VIEW"),
              action: str = typer.Argument(..., help=actions),
              name: Optional[str] = typer.Option(None, "--name"),
              dry_run: bool = typer.Option(False, "--dry-run"),
              yes: bool = typer.Option(False, "--yes"),
              expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
              confirm_destructive: bool = typer.Option(False, "--confirm-destructive",
                                                       help="required with --yes for delete"),
              output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o"),
              json_out: bool = typer.Option(False, "--json")) -> None:
        try:
            c = client_factory()
            try:
                plan = private_views.build_plan(c, id_, action=action, name=name, kind=kind)
            except private_views.NothingToChange as exc:
                if expect_plan is not None:
                    from fsv import journal
                    raise ReviewedWriteError(journal.stale_plan_refusal(expect_plan, (kind.prefix + "edit",))
                                             or "nothing to change; the reviewed plan does not exist. "
                                                "Nothing was sent") from None
                typer.echo(tenant_json({"status": "nothing_to_change", "view_id": id_, "message": str(exc)},
                                       indent=2))
                return
            if dry_run:
                typer.echo(tenant_json({"status": "dry_run", **plan.review()}, indent=2))
                return
            if not yes or expect_plan != plan.hash:
                raise ReviewedWriteError("review with --dry-run, then pass --yes and the full --expect-plan hash")
            if plan.method == "DELETE" and not confirm_destructive:
                raise ReviewedWriteError("deleting a view cannot be undone; pass --confirm-destructive with "
                                         "--yes --expect-plan. Nothing was sent")
            typer.echo(tenant_json(private_views.send(plan, c, reviewed_hash=expect_plan), indent=2))
        except (ValueError, ReviewedWriteError) as exc:
            _fail(exc)
        except APIError as exc:
            _fail(ReviewedWriteError(f"read failed (HTTP {exc.status}); nothing was sent"))

    app.add_typer(group, name="private-views")
