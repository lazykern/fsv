"""Native list preferences/export requests, separate from local ls CSV output."""
from __future__ import annotations

from fsv.tenant import tenant_json

import json
from typing import Any, Callable, Optional

import typer

from fsv import list_preferences as flow
from fsv.resources import Resource
from fsv.reviewed_cli import ReviewOutput
from fsv.reviewed_write import ReviewedWriteError


def register(app: typer.Typer, resource: Resource, client_factory: Callable[[], Any]) -> None:
    group = typer.Typer(help="Inspect/set native list columns (shared account preferences may be affected).")

    def finish(plan, client, dry_run, yes, expect_plan):
        if dry_run:
            typer.echo(tenant_json({"status": "dry_run", **plan.review()}, indent=2))
            return
        if not yes or expect_plan != plan.hash:
            raise ReviewedWriteError("review with --dry-run, then pass --yes and the full --expect-plan hash")
        typer.echo(tenant_json(flow.send(plan, client, reviewed_hash=expect_plan), indent=2))

    @group.command("list")
    def list_(output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o"),
              json_out: bool = typer.Option(False, "--json")) -> None:
        typer.echo(tenant_json(flow.headers(client_factory(), resource.name), indent=2))

    @group.command("set")
    def set_(column: list[str] = typer.Option(..., "--column", help="exact runtime column name, in desired order (repeatable)"),
             freeze: Optional[list[str]] = typer.Option(None, "--freeze", help="captured change requester freeze only; display_id is automatic"),
             dry_run: bool = typer.Option(False, "--dry-run"),
             yes: bool = typer.Option(False, "--yes"),
             expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
             output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o"),
             json_out: bool = typer.Option(False, "--json")) -> None:
        try:
            c = client_factory()
            finish(flow.columns_plan(c, resource.name, column, freeze or []), c, dry_run, yes, expect_plan)
        except (ValueError, ReviewedWriteError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None

    @app.command("export", help="Native CSV/30-day export of a saved view, emailed by the server to the signed-in "
                                "agent (submit-only). Without --yes: review only (body, fields, view, recipient). "
                                "Send: --yes --expect-plan FULL_HASH --confirm-send-to-me.")
    def export(view: str = typer.Option(..., "--view", help="saved view ID or unique name"),
               field: list[str] = typer.Option(..., "--field", help="captured export field key (repeatable)"),
               dry_run: bool = typer.Option(False, "--dry-run"),
               yes: bool = typer.Option(False, "--yes"),
               expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
               confirm_send_to_me: bool = typer.Option(False, "--confirm-send-to-me",
                                                       help="required with --yes: the server emails the export "
                                                            "to your own mailbox"),
               output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o"),
               json_out: bool = typer.Option(False, "--json")) -> None:
        try:
            c = client_factory()
            plan = flow.export_plan(c, resource.name, view, field)
            if dry_run or not yes:
                if expect_plan is not None and not dry_run:
                    raise ReviewedWriteError("--expect-plan sends only with --yes --confirm-send-to-me; nothing was sent")
                typer.echo(tenant_json({"status": "dry_run", **plan.review()}, indent=2))
                return
            if expect_plan != plan.hash:
                from fsv import journal
                raise ReviewedWriteError((journal.stale_plan_refusal(expect_plan, ("list_export",))
                                          if expect_plan else None)
                                         or "review first, then pass --yes with the full --expect-plan hash; "
                                            "nothing was sent")
            if not confirm_send_to_me:
                raise ReviewedWriteError("the server emails this export to your own mailbox; pass "
                                         "--confirm-send-to-me with --yes --expect-plan. Nothing was sent")
            typer.echo(tenant_json(flow.send(plan, c, reviewed_hash=expect_plan), indent=2))
        except (ValueError, ReviewedWriteError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None

    app.add_typer(group, name="columns")
