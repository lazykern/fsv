"""Requested-item review CLI; not ticket-property editing."""
from __future__ import annotations

import json
from typing import Optional
import typer

from fsv import sr_requested_items as flow
from fsv.resources import TICKETS, parse_id
from fsv.reviewed_write import ReviewedWriteError


def register(app, client_factory):
    sub = typer.Typer(help="Requested-item text answers, captured stage 2, add and non-parent removal.")
    app.add_typer(sub, name="items")

    @sub.command("list")
    def list_(target: str, json_out: bool = typer.Option(False, "--json")):
        try:
            typer.echo(json.dumps(flow.inventory(client_factory(), parse_id(target, TICKETS)), indent=2))
        except (ValueError, ReviewedWriteError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None

    @sub.command("write")
    def write(target: str, action: str,
              item: Optional[int] = typer.Option(None, "--item", min=1),
              catalog_item: Optional[str] = typer.Option(None, "--catalog-item"),
              answers: Optional[str] = typer.Option(None, "--answers-json", help="changed API-name text answers as JSON object"),
              stage: Optional[int] = typer.Option(None, "--stage"),
              dry_run: bool = typer.Option(False, "--dry-run"), yes: bool = typer.Option(False, "--yes"),
              expect_plan: Optional[str] = typer.Option(None, "--expect-plan"), json_out: bool = typer.Option(False, "--json")):
        try:
            values = json.loads(answers) if answers else None
            if values is not None and not isinstance(values, dict):
                raise ValueError("answers must be a JSON object")
            client = client_factory()
            plan = flow.build_plan(client, target, action=action, item=item, catalog_item=catalog_item,
                                   answers=values, stage=stage)
            if dry_run:
                typer.echo(json.dumps({"status": "dry_run", **plan.review()}, indent=2))
                return
            if not yes or expect_plan != plan.hash:
                raise ReviewedWriteError("review with --dry-run, then --yes --expect-plan FULL_HASH")
            result = flow.send(plan, client, reviewed_hash=expect_plan)
            typer.echo(json.dumps(result, indent=2))
            if not result.get("verified"):
                raise typer.Exit(4)
        except (ValueError, ReviewedWriteError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None
