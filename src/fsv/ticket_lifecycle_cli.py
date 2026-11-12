"""Hash-bound ticket lifecycle commands."""
from __future__ import annotations

import json
from typing import Any, Callable, Optional

import typer

from fsv import ticket_lifecycle as flow
from fsv.reviewed_write import ReviewedWriteError


def register(app: typer.Typer, client_factory: Callable[[], Any]) -> None:
    def run(target, status, note, sets, dry_run, yes, expect_plan, ack=False):
        try:
            answers = {}
            for expression in sets:
                key, sep, value = expression.partition("=")
                if not sep or not key or key in answers:
                    raise ValueError("--set requires unique FIELD=VALUE answers")
                answers[key] = value
            client = client_factory()
            plan = flow.build_plan(client, target, status, note=note, answers=answers, ack=ack)
            if dry_run:
                typer.echo(json.dumps({"status": "dry_run", **plan.review()}, indent=2))
                return
            if not yes or expect_plan != plan.hash:
                raise ReviewedWriteError("review with --dry-run, then pass --yes and the full --expect-plan hash")
            typer.echo(json.dumps(flow.send(plan, client, reviewed_hash=expect_plan), indent=2))
        except (ValueError, ReviewedWriteError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None

    @app.command("resolve", help="Save a resolution note, verify it, then resolve using runtime requirements.")
    def resolve(id_: str = typer.Argument(..., metavar="ID"),
                note: str = typer.Option(..., "--note"),
                sets: list[str] = typer.Option([], "--set", help="custom FIELD=VALUE; repeatable"),
                dry_run: bool = typer.Option(False, "--dry-run"),
                yes: bool = typer.Option(False, "--yes"),
                expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
                ack: bool = typer.Option(False, "--ack", help="acknowledge blocking rule messages (bound to the review hash)")) -> None:
        run(id_, "Resolved", note, sets, dry_run, yes, expect_plan, ack)

    @app.command("close", help="Close a ticket with a persisted resolution note and required fields.")
    def close(id_: str = typer.Argument(..., metavar="ID"),
              sets: list[str] = typer.Option([], "--set"),
              dry_run: bool = typer.Option(False, "--dry-run"),
              yes: bool = typer.Option(False, "--yes"),
              expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
              ack: bool = typer.Option(False, "--ack", help="acknowledge blocking rule messages (bound to the review hash)")) -> None:
        run(id_, "Closed", None, sets, dry_run, yes, expect_plan, ack)

    @app.command("reopen", help="Set the runtime Open status; preserve the resolution note.")
    def reopen(id_: str = typer.Argument(..., metavar="ID"),
               dry_run: bool = typer.Option(False, "--dry-run"),
               yes: bool = typer.Option(False, "--yes"),
               expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
               ack: bool = typer.Option(False, "--ack", help="acknowledge blocking rule messages (bound to the review hash)")) -> None:
        run(id_, "Open", None, [], dry_run, yes, expect_plan, ack)

    @app.command("status", help="Reviewed runtime status transition with conditional custom answers.")
    def status(id_: str = typer.Argument(..., metavar="ID"),
               status_: str = typer.Argument(..., metavar="STATUS"),
               sets: list[str] = typer.Option([], "--set"),
               dry_run: bool = typer.Option(False, "--dry-run"),
               yes: bool = typer.Option(False, "--yes"),
               expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
               ack: bool = typer.Option(False, "--ack", help="acknowledge blocking rule messages (bound to the review hash)")) -> None:
        run(id_, status_, None, sets, dry_run, yes, expect_plan, ack)
