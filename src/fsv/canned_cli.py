"""Captured template/scenario discovery and source-bound canned replies."""
from __future__ import annotations

import json
from typing import Any, Callable, Optional

import typer

from fsv import canned, ticket_conversation
from fsv.reviewed_cli import ReviewOutput
from fsv.reviewed_write import ReviewedWriteError


def register(app: typer.Typer, client_factory: Callable[[], Any]) -> None:
    templates = typer.Typer(help="Read canned responses and insert selected literal HTML into a reviewed reply.")
    scenarios = typer.Typer(help="Read/review native scenarios. Execution side effects remain unverified.")

    @templates.command("folders")
    def folders(output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o"),
                json_out: bool = typer.Option(False, "--json")) -> None:
        typer.echo(json.dumps(canned.folders(client_factory()), indent=2))

    @templates.command("list")
    def list_(folder: int = typer.Option(..., "--folder", min=1),
              output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o"),
              json_out: bool = typer.Option(False, "--json")) -> None:
        typer.echo(json.dumps(canned.responses(client_factory(), folder), indent=2))

    @templates.command("show")
    def show(id_: int = typer.Argument(..., min=1),
             output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o"),
             json_out: bool = typer.Option(False, "--json")) -> None:
        typer.echo(json.dumps(canned.show(client_factory(), id_), indent=2))

    @templates.command("reply")
    def reply(target: str = typer.Argument(..., metavar="TICKET"),
              response: int = typer.Option(..., "--response", min=1),
              content_pointer: str = typer.Option(..., "--content-pointer", help="JSON pointer from show; no automatic template expansion"),
              sender: str = typer.Option(..., "--sender", help="explicit configured support mailbox"),
              dry_run: bool = typer.Option(False, "--dry-run"),
              yes: bool = typer.Option(False, "--yes"),
              expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
              output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o"),
              json_out: bool = typer.Option(False, "--json")) -> None:
        try:
            c = client_factory()
            plan = canned.reply_plan(c, target, response, content_pointer, sender=sender)
            if dry_run:
                typer.echo(json.dumps({"status": "dry_run", **plan.review(),
                           "source": {"id": plan.canned_id, "hash": plan.canned_hash},
                           "insertion": "Literal selected HTML only; no attachments, signatures or placeholder expansion."}, indent=2))
                return
            if not yes or expect_plan != plan.hash:
                raise ticket_conversation.ConversationError("review with --dry-run, then pass --yes and the full --expect-plan hash")
            result = ticket_conversation.send(plan, c, reviewed_hash=expect_plan, public_confirmed=True)
            typer.echo(json.dumps(result, indent=2))
        except (ValueError, ReviewedWriteError, ticket_conversation.ConversationError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None

    @scenarios.command("list")
    def scenario_list(output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o"),
                      json_out: bool = typer.Option(False, "--json")) -> None:
        typer.echo(json.dumps(canned.scenarios(client_factory()), indent=2))

    @scenarios.command("show")
    def scenario_show(id_: int = typer.Argument(..., min=1),
                      output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o"),
                      json_out: bool = typer.Option(False, "--json")) -> None:
        try:
            typer.echo(json.dumps(canned.scenario(client_factory(), id_), indent=2))
        except (ValueError, ReviewedWriteError) as exc:
            raise typer.BadParameter(str(exc)) from None

    @scenarios.command("apply", help="Review native scenario execution on one ticket; may send mail or trigger workflows, not just insert reply text.")
    def scenario_apply(target: str = typer.Argument(..., metavar="TICKET"),
                       scenario_id: int = typer.Option(..., "--scenario", min=1),
                       dry_run: bool = typer.Option(False, "--dry-run"),
                       yes: bool = typer.Option(False, "--yes"),
                       expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
                       output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o"),
                       json_out: bool = typer.Option(False, "--json")) -> None:
        from fsv import scenarios as native
        try:
            c = client_factory()
            plan = native.build_plan(c, target, scenario_id)
            if dry_run:
                typer.echo(json.dumps({"status": "dry_run", **plan.review(),
                           "definition": canned.scenario(c, scenario_id),
                           "warning": "Native automation may send mail or change records. No complete side-effect verification contract is captured."}, indent=2))
                return
            if not yes or expect_plan != plan.hash:
                raise ReviewedWriteError("review with --dry-run, then pass --yes and the full --expect-plan hash")
            typer.echo(json.dumps(native.send(plan, c, reviewed_hash=expect_plan), indent=2))
        except (ValueError, ReviewedWriteError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None

    app.add_typer(templates, name="canned")
    app.add_typer(scenarios, name="scenarios")
