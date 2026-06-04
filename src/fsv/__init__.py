from __future__ import annotations

from fsv.errors import APIError, SessionError

__all__ = ["APIError", "SessionError"]


def main() -> None:
    from fsv.cli import app

    app()
