"""Reviewed service-request approval commands."""
from __future__ import annotations
import json
from typing import Any, Callable
import typer
from fsv import sr_approvals as approvals
from fsv.integrated_assets_cli import finish, guard


def register(app: typer.Typer, factory: Callable[[], Any]) -> None:
    group = typer.Typer(help="Approval actions send irreversible notifications; delegation is not supported.")
    app.add_typer(group, name="approvals")

    @group.command("list")
    def listing(target: str, json_out: bool = typer.Option(False, "--json")) -> None:
        from fsv.resources import parse_id, TICKETS
        guard(lambda: typer.echo(json.dumps(approvals.listing(factory(), parse_id(target, TICKETS)), indent=2)))

    @group.command("request")
    def request(target: str, name: str = typer.Option(..., "--name"), message: str = typer.Option(..., "--message"),
                approver: list[str] = typer.Option(..., "--approver"), rule: str = typer.Option("everyone", "--rule"),
                dry_run: bool = typer.Option(False, "--dry-run"), yes: bool = typer.Option(False, "--yes", "-y"),
                expected: str | None = typer.Option(None, "--expect-plan"), read_only: bool = typer.Option(False, "--read-only"),
                json_out: bool = typer.Option(False, "--json")) -> None:
        def run():
            c = factory()
            plan = approvals.build_plan(c, target, "request", name=name, message=message, approvers=approver, rule=rule)
            finish(plan, lambda: approvals.send(plan, c, reviewed_hash=expected), dry_run=dry_run, yes=yes, expected=expected, read_only=read_only)
        guard(run)

    def action(operation):
        def command(target: str, ident: int = typer.Argument(..., min=1, help="approval ID; cancel requires group ID"),
                    remark: str = typer.Option("", "--remark"), dry_run: bool = typer.Option(False, "--dry-run"),
                    yes: bool = typer.Option(False, "--yes", "-y"), expected: str | None = typer.Option(None, "--expect-plan"),
                    read_only: bool = typer.Option(False, "--read-only"), json_out: bool = typer.Option(False, "--json")) -> None:
            def run():
                c = factory()
                plan = approvals.build_plan(c, target, operation, ident=ident, remark=remark)
                finish(plan, lambda: approvals.send(plan, c, reviewed_hash=expected), dry_run=dry_run, yes=yes, expected=expected, read_only=read_only)
            guard(run)
        return command
    for operation in ("approve", "reject", "cancel", "remind"):
        group.command(operation)(action(operation))
