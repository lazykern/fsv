"""Problem form create and reviewed captured child workflows."""
from __future__ import annotations

import json
from typing import Any, Callable, Optional

import typer

from fsv import problem_workflows as flow
from fsv.drafts import resolve_draft
from fsv.forms import FormError
from fsv.reviewed_cli import ReviewOutput
from fsv.reviewed_write import ReviewedWriteError


def register(app: typer.Typer, client_factory: Callable[[], Any]) -> None:
    def finish(plan, client, dry_run, yes, expect_plan):
        if dry_run:
            typer.echo(json.dumps({"status": "dry_run", **plan.review()}, indent=2))
            return
        if not yes or expect_plan != plan.hash:
            raise ReviewedWriteError("review with --dry-run, then pass --yes and the full --expect-plan hash")
        typer.echo(json.dumps(flow.send(plan, client, reviewed_hash=expect_plan), indent=2))

    @app.command("create", help="Create a problem through fresh form fields and business rules; no files.")
    def create(from_: Optional[str] = typer.Option(None, "--from", help="YAML answers draft"),
               set_: Optional[list[str]] = typer.Option(None, "--set", help="LABEL=VALUE (repeatable)"),
               requester: str = typer.Option(..., "--requester", help="exact requester email/name lookup"),
               dry_run: bool = typer.Option(False, "--dry-run"),
               yes: bool = typer.Option(False, "--yes"),
               expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
               output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o"),
               json_out: bool = typer.Option(False, "--json")) -> None:
        try:
            draft = resolve_draft(from_, None, set_)
            if draft.attachments or draft.planning or draft.requester:
                raise ValueError("use answers plus --requester; files, planning and special draft keys are unsupported")
            c = client_factory()
            finish(flow.create_plan(c, draft.answers, requester), c, dry_run, yes, expect_plan)
        except (ValueError, FormError, ReviewedWriteError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None

    @app.command("write-child", help="Review a body-only note or basic self-owned one-minute task; note visibility is not proven.")
    def child(target: str = typer.Argument(..., metavar="PROBLEM"),
              kind: str = typer.Argument(..., help="note | task"),
              text: str = typer.Argument(..., help="note body HTML/text or task title"),
              dry_run: bool = typer.Option(False, "--dry-run"),
              yes: bool = typer.Option(False, "--yes"),
              expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
              output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o"),
              json_out: bool = typer.Option(False, "--json")) -> None:
        try:
            c = client_factory()
            finish(flow.child_plan(c, target, kind=kind, text=text), c, dry_run, yes, expect_plan)
        except (ValueError, FormError, ReviewedWriteError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None
