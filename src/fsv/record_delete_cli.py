"""CLI for reviewed problem/article deletion (see fsv.record_delete)."""
from __future__ import annotations

import json
from typing import Any, Callable, Optional

import typer

from fsv import record_delete as flow
from fsv.integrated_assets_cli import guard
from fsv.reviewed_write import ReviewedWriteError


def _run(build: Callable[[], Any], client: Any, *, dry_run: bool, yes: bool, expect_plan: str | None,
         confirm_destructive: bool) -> None:
    plan = build()
    if dry_run:
        typer.echo(json.dumps({"status": "dry_run", **flow.review(plan, client)}, indent=2, ensure_ascii=False))
        return
    if not yes or expect_plan != plan.hash:
        raise ReviewedWriteError("review with --dry-run, then --yes --expect-plan FULL_HASH"
                                 " (plus --confirm-destructive where the review requires it)")
    result = flow.send(plan, client, reviewed_hash=expect_plan, confirm_destructive=confirm_destructive)
    typer.echo(json.dumps(result, indent=2, ensure_ascii=False))
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


def register_problems(app: typer.Typer, factory: Callable[[], Any]) -> None:
    _command(app, "delete", "Move one exact problem to the trash (reviewed; restorable).",
             lambda c, i: flow.problem_plan(c, i, "delete"), factory, "PROBLEM_ID")
    _command(app, "restore", "Restore one exact problem from the trash (reviewed).",
             lambda c, i: flow.problem_plan(c, i, "restore"), factory, "PROBLEM_ID")
    _command(app, "delete-forever", "Permanently delete one trashed problem (reviewed; --confirm-destructive).",
             lambda c, i: flow.problem_plan(c, i, "delete-forever"), factory, "PROBLEM_ID")


def register_articles(app: typer.Typer, factory: Callable[[], Any]) -> None:
    _command(app, "delete", "Move one knowledge article to the trash (reviewed; --confirm-destructive).",
             lambda c, i: flow.article_plan(c, i, "delete"), factory, "ARTICLE_ID")
    _command(app, "delete-forever", "Permanently delete one article (reviewed; --confirm-destructive).",
             lambda c, i: flow.article_plan(c, i, "delete-forever"), factory, "ARTICLE_ID")
