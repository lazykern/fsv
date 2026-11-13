"""CLI for integrated inventory; legacy CI command names are aliases."""
from __future__ import annotations

import json
from typing import Any, Callable
import typer
from fsv import integrated_assets as assets
from fsv.errors import APIError, SessionError
from fsv.reviewed_write import ReviewedWriteError


def finish(plan: Any, send: Callable, *, dry_run: bool, yes: bool, expected: str | None, read_only: bool = False) -> None:
    if dry_run:
        typer.echo(json.dumps({"status": "dry_run", **plan.review()}, indent=2))
        return
    if read_only:
        raise ReviewedWriteError("read-only mode; no write attempted")
    if not yes or expected != plan.hash:
        raise ReviewedWriteError("review with --dry-run, then --yes --expect-plan FULL_HASH")
    typer.echo(json.dumps(send(), indent=2))


def guard(fn: Callable) -> None:
    try:
        fn()
    except (ValueError, ReviewedWriteError, APIError, SessionError) as exc:
        typer.echo(str(exc), err=True)
        raise typer.Exit(2) from None


def register_search(app: typer.Typer, factory: Callable[[], Any]) -> None:
    @app.command("search")
    def search(query: str = typer.Argument(""), kind: str = typer.Option("devices", "--kind"),
               page: int = typer.Option(1, "--page", min=1), json_out: bool = typer.Option(False, "--json")) -> None:
        guard(lambda: typer.echo(json.dumps(assets.search(factory(), query, kind=kind, page=page), indent=2)))


def register(app: typer.Typer, module: str, factory: Callable[[], Any]) -> None:
    from fsv.cli import ChangeLegacyGroup
    group = typer.Typer(cls=ChangeLegacyGroup, help="Integrated Devices and Business services; IDs are inventory IDs, not legacy display IDs.")
    app.add_typer(group, name="assets")

    @group.command("list")
    @group.command("ls", hidden=True)
    def listing(target: str, kind: str = typer.Option("devices", "--kind"), json_out: bool = typer.Option(False, "--json")) -> None:
        guard(lambda: typer.echo(json.dumps(assets.listing(factory(), module, target, kind), indent=2)))

    @group.command("candidates", help="Assets offered for association: legacy assets-to-associate on legacy "
                                      "records, else (or when it answers 403/404) the integrated inventory search.")
    def candidates(target: str, query: str = typer.Option("", "--search", help="integrated search text"),
                   kind: str = typer.Option("devices", "--kind"), page: int = typer.Option(1, "--page", min=1),
                   json_out: bool = typer.Option(False, "--json")) -> None:
        guard(lambda: typer.echo(json.dumps(assets.candidates(factory(), module, target, query, kind=kind, page=page),
                                            indent=2)))

    if module == "changes":
        @group.command("legacy", hidden=True)
        def legacy(target: str, search: str | None = typer.Option(None, "--search"),
                   json_out: bool = typer.Option(False, "--json")) -> None:
            typer.echo("deprecated: use assets search or assets list", err=True)
            if search is None:
                listing(target, "devices", json_out)
            else:
                guard(lambda: typer.echo(json.dumps(assets.search(factory(), search), indent=2)))

    def mutate(remove: bool):
        def command(target: str, ids: list[int] = typer.Argument(...), kind: str = typer.Option("devices", "--kind"),
                    dry_run: bool = typer.Option(False, "--dry-run"), yes: bool = typer.Option(False, "--yes", "-y"),
                    expected: str | None = typer.Option(None, "--expect-plan"), read_only: bool = typer.Option(False, "--read-only"),
                    json_out: bool = typer.Option(False, "--json")) -> None:
            def run():
                c = factory()
                plan = assets.build_plan(c, module, target, ids, kind=kind, remove=remove)
                finish(plan, lambda: assets.send(plan, c, module, ids, kind=kind, remove=remove, reviewed_hash=expected),
                       dry_run=dry_run, yes=yes, expected=expected, read_only=read_only)
            guard(run)
        return command
    add, remove = mutate(False), mutate(True)
    group.command("add")(add)
    group.command("remove")(remove)
    if module == "changes":
        app.command("ci-add", hidden=True)(add)
        app.command("ci-remove", hidden=True)(remove)
        app.command("ci-list", hidden=True)(listing)
        register_search(group, factory)
