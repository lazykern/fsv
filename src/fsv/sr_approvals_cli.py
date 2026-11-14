"""Reviewed service-request approval commands."""
from __future__ import annotations

from fsv.tenant import tenant_json
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
        def run() -> None:
            from fsv import approval_model
            out = approvals.listing(factory(), parse_id(target, TICKETS))
            if "approvals" in out:  # the flat (legacy) list: same row keys as `tickets approvals`
                out = {**out, "approvals": approval_model.with_flat(out["approvals"])}
            if json_out or "approvals" not in out:
                typer.echo(tenant_json(out, indent=2))
                return
            from fsv.render import console, err
            console.print(approval_model.flat_table(out["approvals"]))
            if out.get("note"):
                err.print(f"[dim]{out['note']}[/dim]", highlight=False)
        guard(run)

    def legacy(c, target):
        from fsv import approval_model
        from fsv.resources import parse_id, TICKETS
        tid = parse_id(target, TICKETS)
        return tid, approval_model.detect(c, "tickets", tid)[0] == approval_model.LEGACY

    def finish_legacy(plan, c, dry_run, yes, expected, read_only):
        from fsv import legacy_approvals
        finish(plan, lambda: legacy_approvals.send(plan, c, reviewed_hash=expected), dry_run=dry_run, yes=yes,
               expected=expected, read_only=read_only)

    @group.command("request")
    def request(target: str, name: str | None = typer.Option(None, "--name", help="approval group name (approval chains model)"),
                message: str | None = typer.Option(None, "--message", help="email text (approval chains model)"),
                approver: list[str] = typer.Option(..., "--approver"), rule: str = typer.Option("everyone", "--rule"),
                dry_run: bool = typer.Option(False, "--dry-run"), yes: bool = typer.Option(False, "--yes", "-y"),
                expected: str | None = typer.Option(None, "--expect-plan"), read_only: bool = typer.Option(False, "--read-only"),
                json_out: bool = typer.Option(False, "--json")) -> None:
        def run():
            c = factory()
            tid, is_legacy = legacy(c, target)
            if is_legacy:
                from fsv import legacy_approvals
                if name or message:
                    raise ValueError("--name/--message do not apply to legacy flat approvals; the email is the "
                                     "tenant's approval template")
                plan = legacy_approvals.build_request(c, "tickets", tid, approvals.recipients(c, approver), rule=rule)
                finish_legacy(plan, c, dry_run, yes, expected, read_only)
                return
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
                if operation == "remind":
                    tid, is_legacy = legacy(c, target)
                    if is_legacy:
                        from fsv import legacy_approvals
                        finish_legacy(legacy_approvals.build_remind(c, "tickets", tid, ident), c, dry_run, yes,
                                      expected, read_only)
                        return
                plan = approvals.build_plan(c, target, operation, ident=ident, remark=remark)
                finish(plan, lambda: approvals.send(plan, c, reviewed_hash=expected), dry_run=dry_run, yes=yes, expected=expected, read_only=read_only)
            guard(run)
        return command
    for operation in ("approve", "reject", "cancel", "remind"):
        group.command(operation)(action(operation))
