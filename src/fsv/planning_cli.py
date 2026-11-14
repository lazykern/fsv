"""CLI for reviewed native planning text, separate from attachment workflows."""
from __future__ import annotations

from fsv.tenant import tenant_json

import json
from typing import Any, Callable, Optional

import typer

from fsv.reviewed_cli import ReviewOutput

from fsv import completion, planning_text
from fsv.errors import APIError, SessionError, exit_code_for
from fsv.resources import Resource
from fsv.reviewed_write import ReviewedWriteError


def complete_section(resource: Resource):
    if resource.name == "changes":
        return completion.complete_planning_field_names(resource)

    def complete(incomplete: str):
        for label, key in planning_text.PROBLEM_SECTIONS.items():
            if label.startswith(incomplete.casefold()) or key.startswith(incomplete.casefold()):
                yield (label.title(), key)
    return complete


def register(app: typer.Typer, resource: Resource, client_factory: Callable[[], Any]) -> None:
    if resource.name == "problems":
        @app.command("mark-known-error", help="Review marking a problem as a known error; no reversal wire is proven.")
        def mark_known_error(id_: str = typer.Argument(..., metavar="ID"),
                             dry_run: bool = typer.Option(False, "--dry-run"),
                             yes: bool = typer.Option(False, "--yes"),
                             output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o", help="output format (JSON only)"),
                             json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
                             expect_plan: Optional[str] = typer.Option(None, "--expect-plan")) -> None:
            from fsv import problem_known_error
            try:
                c = client_factory()
                plan = problem_known_error.build_plan(c, id_)
                if dry_run:
                    typer.echo(tenant_json({"status": "dry_run", **plan.review()}, indent=2))
                    return
                if not yes or expect_plan != plan.hash:
                    raise ReviewedWriteError("review with --dry-run, then pass --yes and the full --expect-plan hash")
                typer.echo(tenant_json(problem_known_error.send(plan, c, reviewed_hash=expect_plan), indent=2))
            except (ValueError, ReviewedWriteError) as exc:
                typer.echo(str(exc), err=True)
                raise typer.Exit(2) from None

    @app.command("planning-text", help="Review a native planning/analysis text section; no files.")
    def planning(id_: str = typer.Argument(..., metavar="ID"),
                 section: str = typer.Option(..., "--section", help="exact metadata label/name; problems: Root Cause, Impact, Symptoms", autocompletion=complete_section(resource)),
                 description: Optional[str] = typer.Option(None, "--description", help="text/HTML; reviewed verbatim; mutually exclusive with --clear"),
                 clear: bool = typer.Option(False, "--clear", help="delete existing section text; refuses files or unknown state"),
                 dry_run: bool = typer.Option(False, "--dry-run"),
                 yes: bool = typer.Option(False, "--yes"),
                 output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o", help="output format (JSON only)"),
                 json_out: bool = typer.Option(False, "--json", help="alias for --output json"),
                 expect_plan: Optional[str] = typer.Option(None, "--expect-plan")) -> None:
        try:
            c = client_factory()
            plan = planning_text.build_plan(c, resource.name, id_, section, description, clear=clear)
            if dry_run:
                typer.echo(tenant_json({"status": "dry_run", **plan.review()}, indent=2))
                return
            if not yes or expect_plan != plan.hash:
                raise ReviewedWriteError("review with --dry-run, then pass --yes and the full --expect-plan hash")
            typer.echo(tenant_json(planning_text.send(plan, c, reviewed_hash=expect_plan), indent=2))
        except (ValueError, ReviewedWriteError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None
        except (APIError, SessionError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(exit_code_for(exc)) from None
