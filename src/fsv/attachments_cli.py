"""Reviewed attachment CLI and shared output gate."""
from __future__ import annotations

from fsv.tenant import tenant_json

import json
from typing import Any, Callable, Optional

import typer

from fsv import attachments as flow
from fsv.reviewed_write import ReviewedWriteError


def finish(plan: flow.AttachedPlan, client: Any, *, dry_run: bool, yes: bool,
           expected: str | None, rebuild: Callable[[], flow.AttachedPlan]) -> None:
    if dry_run:
        typer.echo(tenant_json({"status": "dry_run", **plan.review()}, indent=2))
        return
    if not yes or expected != plan.hash:
        raise ReviewedWriteError("review with --dry-run, then --yes --expect-plan FULL_HASH")
    result = flow.send(plan, client, reviewed_hash=expected, rebuild=rebuild)
    typer.echo(tenant_json(result, indent=2))
    if not result.get("verified"):
        raise typer.Exit(4)


def register(app: typer.Typer, client_factory: Callable[[], Any]) -> None:
    sub = typer.Typer(help="Reviewed attachment deletion with independent owner readback.")
    app.add_typer(sub, name="attachments")

    @sub.command("delete")
    def delete(ident: str, owner: str = typer.Option(..., "--owner", help="tickets/ID, problems/ID or changes/ID"),
               child: Optional[int] = typer.Option(None, "--child", help="exact conversation/note ID"),
               section: Optional[str] = typer.Option(None, "--section", help="exact native planning section key"),
               dry_run: bool = typer.Option(False, "--dry-run"), yes: bool = typer.Option(False, "--yes"),
               expect_plan: Optional[str] = typer.Option(None, "--expect-plan"), json_out: bool = typer.Option(False, "--json")) -> None:
        try:
            client = client_factory()
            plan = flow.delete_plan(client, ident, owner, child=child, section=section)
            if dry_run:
                typer.echo(tenant_json({"status": "dry_run", **plan.review(), "owner": owner, "child": child, "section": section}, indent=2))
                return
            if not yes or expect_plan != plan.hash:
                raise ReviewedWriteError("review with --dry-run, then --yes --expect-plan FULL_HASH")
            result = flow.delete(plan, client, owner, child=child, section=section, reviewed_hash=expect_plan)
            typer.echo(tenant_json(result, indent=2))
            if not result.get("verified"):
                raise typer.Exit(4)
        except (ValueError, ReviewedWriteError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None
