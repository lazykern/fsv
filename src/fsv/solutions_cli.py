"""Draft knowledge and problem solution association commands."""
from __future__ import annotations
from typing import Any, Callable
import typer
from fsv import solutions
from fsv.integrated_assets_cli import finish, guard


def register(app: typer.Typer, factory: Callable[[], Any]) -> None:
    from fsv.record_delete_cli import register_articles
    register_articles(app, factory)

    @app.command("create", help="Create a draft workaround/permanent article; does not link or publish it.")
    def create(title: str = typer.Option(..., "--title"), body: str = typer.Option(..., "--body", help="plain text"),
               kind: str = typer.Option(..., "--type", help="workaround or permanent"),
               category: str | None = typer.Option(None, "--category", help="runtime ID/name; unique default if omitted"),
               folder: str | None = typer.Option(None, "--folder", help="runtime ID/name within category; unique default if omitted"),
               dry_run: bool = typer.Option(False, "--dry-run"), yes: bool = typer.Option(False, "--yes", "-y"),
               expected: str | None = typer.Option(None, "--expect-plan"), read_only: bool = typer.Option(False, "--read-only"),
               json_out: bool = typer.Option(False, "--json")) -> None:
        def run():
            c = factory()
            plan = solutions.create_plan(c, title, body, kind, category=category, selected_folder=folder)
            finish(plan, lambda: solutions.send_create(plan, c, reviewed_hash=expected), dry_run=dry_run, yes=yes, expected=expected, read_only=read_only)
        guard(run)


def register_links(app: typer.Typer, factory: Callable[[], Any]) -> None:
    def action(unlink):
        def command(target: str, article: int = typer.Argument(..., min=1), dry_run: bool = typer.Option(False, "--dry-run"),
                    yes: bool = typer.Option(False, "--yes", "-y"), expected: str | None = typer.Option(None, "--expect-plan"),
                    read_only: bool = typer.Option(False, "--read-only"), json_out: bool = typer.Option(False, "--json")) -> None:
            def run():
                c = factory()
                plan = solutions.link_plan(c, target, article, unlink=unlink)
                finish(plan, lambda: solutions.send_link(plan, c, reviewed_hash=expected), dry_run=dry_run, yes=yes, expected=expected, read_only=read_only)
            guard(run)
        return command
    app.command("link-solution")(action(False))
    app.command("unlink-solution")(action(True))
