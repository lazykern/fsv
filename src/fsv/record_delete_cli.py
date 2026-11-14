"""CLI for reviewed problem/article deletion (see fsv.record_delete)."""
from __future__ import annotations

from fsv.tenant import tenant_json

import json
from typing import Any, Callable, List, Optional

import typer

from fsv import record_delete as flow
from fsv.integrated_assets_cli import guard
from fsv.reviewed_write import ReviewedWriteError


def _run(build: Callable[[], Any], client: Any, *, dry_run: bool, yes: bool, expect_plan: str | None,
         confirm_destructive: bool) -> None:
    plan = build()
    if dry_run:
        typer.echo(tenant_json({"status": "dry_run", **flow.review(plan, client)}, indent=2, ensure_ascii=False))
        return
    if not yes or expect_plan != plan.hash:
        raise ReviewedWriteError("review with --dry-run, then --yes --expect-plan FULL_HASH"
                                 " (plus --confirm-destructive where the review requires it)")
    result = flow.send(plan, client, reviewed_hash=expect_plan, confirm_destructive=confirm_destructive)
    typer.echo(tenant_json(result, indent=2, ensure_ascii=False))
    if not result.get("verified"):
        raise typer.Exit(4)


def _command(app: typer.Typer, name: str, help_: str, build: Callable[[Any, str], Any],
             factory: Callable[[], Any], metavar: str) -> None:
    @app.command(name, help=help_)
    def command(ident: str = typer.Argument(..., metavar=metavar),
                dry_run: bool = typer.Option(False, "--dry-run", help="review the exact target; nothing is sent"),
                yes: bool = typer.Option(False, "--yes", "-y"),
                expect_plan: Optional[str] = typer.Option(None, "--expect-plan", help="full hash from the review"),
                confirm_destructive: bool = typer.Option(False, "--confirm-destructive",
                                                         help="required for permanent or untracked deletes"),
                json_out: bool = typer.Option(False, "--json", help="JSON output (always JSON)")) -> None:
        def run() -> None:
            c = factory()
            _run(lambda: build(c, ident), c, dry_run=dry_run, yes=yes, expect_plan=expect_plan,
                 confirm_destructive=confirm_destructive)
        guard(run)


def _forever_command(app: typer.Typer, kind: str, factory: Callable[[], Any], metavar: str) -> None:
    label = "problems" if kind == "problem" else "articles"

    @app.command("delete-forever", help=f"PERMANENTLY delete explicit {label} already in the trash (reviewed). "
                                        "More than one ID needs --max-records (1 to 50); always --confirm-destructive.")
    def command(idents: List[str] = typer.Argument(..., metavar=metavar + "..."),
                max_records: Optional[int] = typer.Option(None, "--max-records",
                                                          help="bound, 1 to 50; required for more than one ID"),
                dry_run: bool = typer.Option(False, "--dry-run", help="review the exact targets; nothing is sent"),
                yes: bool = typer.Option(False, "--yes", "-y"),
                expect_plan: Optional[str] = typer.Option(None, "--expect-plan", help="full hash from the review"),
                confirm_destructive: bool = typer.Option(False, "--confirm-destructive",
                                                         help="required: the deletes are permanent"),
                json_out: bool = typer.Option(False, "--json", help="JSON output (always JSON)")) -> None:
        def run() -> None:
            c = factory()
            batch = flow.build_forever(c, kind, list(idents), max_records=max_records)
            typer.echo(f"{label.capitalize()} to delete PERMANENTLY: " + ", ".join(map(str, batch.ids)), err=True)
            for row in batch.rows:
                typer.echo(f"  {row['id']}: {row['title']}", err=True)
            if dry_run:
                typer.echo(tenant_json({"status": "dry_run", **batch.review()}, indent=2, ensure_ascii=False))
                return
            if not yes or expect_plan != batch.hash:
                raise ReviewedWriteError("review with --dry-run, then --yes --expect-plan FULL_HASH --confirm-destructive")
            result = flow.send_forever(batch, c, reviewed_hash=expect_plan, confirm_destructive=confirm_destructive)
            typer.echo(tenant_json(result, indent=2, ensure_ascii=False))
            if not result.get("verified"):
                raise typer.Exit(4)
        guard(run)


def register_problems(app: typer.Typer, factory: Callable[[], Any]) -> None:
    _command(app, "delete", "Move one exact problem to the trash (reviewed; restorable).",
             lambda c, i: flow.problem_plan(c, i, "delete"), factory, "PROBLEM_ID")
    _command(app, "restore", "Restore one exact problem from the trash (reviewed).",
             lambda c, i: flow.problem_plan(c, i, "restore"), factory, "PROBLEM_ID")
    _forever_command(app, "problem", factory, "PROBLEM_ID")


def register_articles(app: typer.Typer, factory: Callable[[], Any]) -> None:
    _command(app, "delete", "Move one knowledge article to the trash (reviewed; --confirm-destructive).",
             lambda c, i: flow.article_plan(c, i, "delete"), factory, "ARTICLE_ID")
    _forever_command(app, "article", factory, "ARTICLE_ID")
