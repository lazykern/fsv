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

    def incident_action(problem: str, incident: str, unlink: bool, dry_run: bool, yes: bool, expect_plan: str | None) -> None:
        from fsv import problem_incidents
        from fsv.change_associations import AssociationError
        from fsv.errors import APIError
        try:
            client = client_factory()
            plan = problem_incidents.build_plan(client, problem, incident, unlink=unlink)
            if dry_run:
                typer.echo(json.dumps({"status": "dry_run", **plan.review(), "problem_id": plan.target,
                                       "incident_id": incident}, indent=2))
                return
            if not yes or expect_plan != plan.hash:
                raise ReviewedWriteError("review with --dry-run, then pass --yes and the full --expect-plan hash")
            typer.echo(json.dumps(problem_incidents.send(plan, client, incident, reviewed_hash=expect_plan), indent=2))
        except (ValueError, AssociationError, ReviewedWriteError, APIError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None

    @app.command("link-incident", help="Review an incident association to a problem.")
    def link_incident(problem: str = typer.Argument(..., metavar="PROBLEM"), incident: str = typer.Argument(..., metavar="INCIDENT"),
                      dry_run: bool = typer.Option(False, "--dry-run"), yes: bool = typer.Option(False, "--yes"),
                      expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
                      output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o"),
                      json_out: bool = typer.Option(False, "--json")) -> None:
        incident_action(problem, incident, False, dry_run, yes, expect_plan)

    @app.command("unlink-incident", help="Review removal of a problem-incident association.")
    def unlink_incident(problem: str = typer.Argument(..., metavar="PROBLEM"), incident: str = typer.Argument(..., metavar="INCIDENT"),
                        dry_run: bool = typer.Option(False, "--dry-run"), yes: bool = typer.Option(False, "--yes"),
                        expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
                        output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o"),
                        json_out: bool = typer.Option(False, "--json")) -> None:
        incident_action(problem, incident, True, dry_run, yes, expect_plan)

    @app.command("note", help="Review a problem note, optionally with local attachments; visibility is not inferred.")
    def note(target: str, body: str = typer.Option(..., "--body"), attach: Optional[list[str]] = typer.Option(None, "--attach"),
             max_bytes: Optional[int] = typer.Option(None, "--max-bytes", min=1, help="explicit total attachment size limit in bytes; replaces the tenant-published limit (required to send when the tenant publishes none)"),
             dry_run: bool = typer.Option(False, "--dry-run"), yes: bool = typer.Option(False, "--yes"),
             expect_plan: Optional[str] = typer.Option(None, "--expect-plan"), json_out: bool = typer.Option(False, "--json")) -> None:
        from fsv import attachments
        from fsv.attachments_cli import finish as finish_attachments
        try:
            client = client_factory()
            if attach:
                rebuild = lambda: attachments.problem_note_plan(client, target, body, list(attach), max_bytes=max_bytes)
                finish_attachments(rebuild(), client, dry_run=dry_run, yes=yes, expected=expect_plan, rebuild=rebuild)
            else:
                finish(flow.child_plan(client, target, kind="note", text=body), client, dry_run, yes, expect_plan)
        except (ValueError, ReviewedWriteError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None

    @app.command("status", help="Review a metadata-backed problem status transition, including closure checks.")
    def status(target: str, to: str = typer.Option(..., "--to"), dry_run: bool = typer.Option(False, "--dry-run"),
               yes: bool = typer.Option(False, "--yes"), expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
               json_out: bool = typer.Option(False, "--json")) -> None:
        from fsv import problem_status
        from fsv.errors import APIError
        try:
            client = client_factory()
            plan = problem_status.build_plan(client, target, to)
            if dry_run:
                typer.echo(json.dumps({"status": "dry_run", **plan.review()}, indent=2))
                return
            if not yes or expect_plan != plan.hash:
                raise ReviewedWriteError("review with --dry-run, then --yes --expect-plan FULL_HASH")
            result = problem_status.send(plan, client, reviewed_hash=expect_plan)
            typer.echo(json.dumps(result, indent=2))
            if not result.get("verified"):
                raise typer.Exit(4)
        except (ValueError, FormError, ReviewedWriteError, APIError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None

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
