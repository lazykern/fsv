from __future__ import annotations


class APIError(Exception):
    def __init__(self, status: int, body: object, method: str | None = None, target: str | None = None) -> None:
        where = f" {method.upper()} {target}" if method and target else ""
        super().__init__(f"HTTP {status}{where}: {body}")
        self.status = status
        self.body = body
        self.method = method
        self.target = target


class SessionError(Exception):
    pass
