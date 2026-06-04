from __future__ import annotations


class APIError(Exception):
    def __init__(self, status: int, body: object) -> None:
        super().__init__(f"HTTP {status}: {body}")
        self.status = status
        self.body = body


class SessionError(Exception):
    pass
