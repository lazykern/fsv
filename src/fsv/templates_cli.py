"""CLI: ticket templates (read-only) and change maintenance windows."""
from __future__ import annotations

import json
from typing import Any, Callable, List, Optional

import typer

from fsv.integrated_assets_cli import guard


def register_templates(app: typer.Typer, factory: Callable[[], Any]) -> None:
    from fsv import templates as T
    sub = typer.Typer(help="Ticket templates (read-only; apply with `fsv tickets create --template NAME`).")
    app.add_typer(sub, name="templates")

    @sub.command("list", help="List ticket templates.")
    def list_(json_out: bool = typer.Option(False, "--json")) -> None:
        def run() -> None:
            rows = T.list_templates(factory())
            if json_out:
                typer.echo(json.dumps(rows, indent=2, ensure_ascii=False))
                return
            for r in rows:
                typer.echo(f"{r['id']}\t{r['name']}\t{r.get('type') or ''}")
            if not rows:
                typer.echo("No ticket templates.")
        guard(run)

    @sub.command("show", help="Show one ticket template and the fields it prefills.")
    def show(name: str = typer.Argument(..., metavar="NAME_OR_ID"), json_out: bool = typer.Option(False, "--json")) -> None:
        def run() -> None:
            row = T.show(factory(), name)
            _draft, applied, ignored = T.to_draft(row)
            doc = {"id": row["id"], "name": row.get("name"), "prefills": applied, "not_applied": ignored,
                   "template_data": row["template_data"]}
            typer.echo(json.dumps(doc, indent=2, ensure_ascii=False))
        guard(run)


def register_windows(app: typer.Typer, factory: Callable[[], Any]) -> None:
    from fsv import change_windows as W
    sub = typer.Typer(help="Change maintenance windows (reviewed create/delete).")
    app.add_typer(sub, name="windows")

    @sub.command("list", help="List maintenance windows.")
    def list_(json_out: bool = typer.Option(False, "--json")) -> None:
        def run() -> None:
            rows = W.list_windows(factory())
            if json_out:
                typer.echo(json.dumps(rows, indent=2, ensure_ascii=False))
                return
            for r in rows:
                typer.echo(f"{r.get('id')}\t{r.get('name')}\t{r.get('start_time')} -> {r.get('end_time')}")
            if not rows:
                typer.echo("No maintenance windows.")
        guard(run)

    @sub.command("create", help="Create a maintenance window (overlap check, then reviewed POST).")
    def create(name: str = typer.Option(..., "--name"), start: str = typer.Option(..., "--start", help="YYYY-MM-DD HH:MM"),
               end: str = typer.Option(..., "--end", help="YYYY-MM-DD HH:MM"),
               criteria: List[str] = typer.Option(..., "--criteria", help="change_type=<label or id> (repeatable; at least one)"),
               description: str = typer.Option("", "--description"),
               no_alert_suppression: bool = typer.Option(False, "--no-alert-suppression"),
               allow_overlap: bool = typer.Option(False, "--allow-overlap", help="create even if windows overlap or the overlap answer is not understood"),
               dry_run: bool = typer.Option(False, "--dry-run"), yes: bool = typer.Option(False, "--yes", "-y"),
               expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
               json_out: bool = typer.Option(False, "--json")) -> None:
        from fsv.reviewed_write import ReviewedWriteError

        def run() -> None:
            c = factory()
            build = lambda: W.create_plan(c, name, description, start, end, list(criteria),
                                          alert_suppression=not no_alert_suppression)
            plan = build()
            if dry_run:
                typer.echo(json.dumps({"status": "dry_run", **plan.review()}, indent=2, ensure_ascii=False))
                return
            if not yes or expect_plan != plan.hash:
                raise ReviewedWriteError("review with --dry-run, then --yes --expect-plan FULL_HASH")
            result = W.send_create(plan, c, reviewed_hash=expect_plan, allow_overlap=allow_overlap, rebuild=build)
            typer.echo(json.dumps(result, indent=2, ensure_ascii=False))
            if not result.get("verified"):
                raise typer.Exit(4)
        guard(run)

    @sub.command("delete", help="Delete one maintenance window (reviewed).")
    def delete(ident: int = typer.Argument(..., min=1, metavar="WINDOW_ID"),
               dry_run: bool = typer.Option(False, "--dry-run"), yes: bool = typer.Option(False, "--yes", "-y"),
               expect_plan: Optional[str] = typer.Option(None, "--expect-plan"),
               json_out: bool = typer.Option(False, "--json")) -> None:
        from fsv.reviewed_write import ReviewedWriteError

        def run() -> None:
            c = factory()
            plan = W.delete_plan(c, ident)
            if dry_run:
                typer.echo(json.dumps({"status": "dry_run", **plan.review()}, indent=2, ensure_ascii=False))
                return
            if not yes or expect_plan != plan.hash:
                raise ReviewedWriteError("review with --dry-run, then --yes --expect-plan FULL_HASH")
            result = W.send_delete(plan, c, reviewed_hash=expect_plan)
            typer.echo(json.dumps(result, indent=2, ensure_ascii=False))
            if not result.get("verified"):
                raise typer.Exit(4)
        guard(run)
