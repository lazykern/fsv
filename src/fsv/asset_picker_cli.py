"""Read-only asset candidate picker for scripts."""
from __future__ import annotations

from fsv.tenant import tenant_json

import json
from typing import Any, Callable, Optional

import typer

from fsv import asset_picker
from fsv.reviewed_cli import ReviewOutput
from fsv.reviewed_write import ReviewedWriteError


def register(app: typer.Typer, client_factory: Callable[[], Any]) -> None:
    @app.command("asset-picker", help="Search/select a change asset candidate without associating it (read-only).")
    def pick(target: str = typer.Argument(..., metavar="CHANGE"),
             search: str = typer.Option("", "--search"),
             page: int = typer.Option(1, "--page", min=1),
             select: Optional[int] = typer.Option(None, "--select", min=1, help="exact display ID from this page, not internal ID"),
             output: ReviewOutput = typer.Option(ReviewOutput.json, "--output", "-o"),
             json_out: bool = typer.Option(False, "--json")) -> None:
        try:
            result = asset_picker.candidates(client_factory(), target, search, page=page)
            if select is not None:
                result["selected"] = asset_picker.select(result, select)
            typer.echo(tenant_json(result, indent=2))
        except (ValueError, ReviewedWriteError) as exc:
            typer.echo(str(exc), err=True)
            raise typer.Exit(2) from None
