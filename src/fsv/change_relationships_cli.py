"""Reviewed change relationship list/add/remove CLI."""
from __future__ import annotations

import json
from typing import Optional
import typer

from fsv import change_relationships as flow
from fsv.resources import CHANGES, parse_id
from fsv.errors import APIError
from fsv.reviewed_write import ReviewedWriteError


def register(app, client_factory):
    sub = typer.Typer(help="Captured change relationships; project add creates a new project.")
    app.add_typer(sub, name="relationships")

    @sub.command("list")
    def list_(target: str, kind: str, json_out: bool = typer.Option(False, "--json")):
        try:
            typer.echo(json.dumps(flow.inventory(client_factory(), parse_id(target, CHANGES), kind), indent=2))
        except APIError as exc:
            typer.echo(str(flow._refusal(exc, f"the {kind} relationships of {target}")).replace("; nothing was sent", ""),
                       err=True)
            raise typer.Exit(2) from None
        except (ValueError, ReviewedWriteError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None

    @sub.command("write")
    def write(target: str, kind: str, action: str,
              item: Optional[str] = typer.Option(None, "--item"),
              project: Optional[str] = typer.Option(None, "--project-json", help="create-and-associate project details as JSON"),
              project_key: Optional[str] = typer.Option(None, "--project-key"), name: Optional[str] = typer.Option(None, "--name"),
              dry_run: bool = typer.Option(False, "--dry-run"), yes: bool = typer.Option(False, "--yes"),
              expect_plan: Optional[str] = typer.Option(None, "--expect-plan"), json_out: bool = typer.Option(False, "--json")):
        try:
            client = client_factory()
            plan = flow.build_plan(client, target, kind=kind, action=action, item=item,
                                   project=json.loads(project) if project else None, project_key=project_key, name=name)
            if dry_run:
                typer.echo(json.dumps({"status": "dry_run", **plan.review()}, indent=2))
                return
            if not yes or expect_plan != plan.hash:
                raise ReviewedWriteError("review with --dry-run, then --yes --expect-plan FULL_HASH")
            result = flow.send(plan, client, reviewed_hash=expect_plan)
            typer.echo(json.dumps(result, indent=2))
            if not result.get("verified"):
                raise typer.Exit(4)
        except APIError as exc:
            # Plan building maps its reads; this is a last-resort one-liner.
            typer.echo(f"error: {exc}; nothing was sent", err=True)
            raise typer.Exit(2) from None
        except (ValueError, ReviewedWriteError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None
