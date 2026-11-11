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


# Stable process exit codes (documented in README).
EXIT_OK = 0
EXIT_OTHER = 1
EXIT_USAGE = 2      # validation / usage error
EXIT_SESSION = 3    # session expired or missing
EXIT_CONFLICT = 4   # conflict / duplicate
EXIT_REJECTED = 5   # server rejected the request (4xx/5xx)

_DUPLICATE_RE = None


def exit_code_for(exc: BaseException) -> int:
    """Map an exception to the documented exit code."""
    import re
    global _DUPLICATE_RE
    if _DUPLICATE_RE is None:
        _DUPLICATE_RE = re.compile(r"already (exists|been taken|associated)|duplicate", re.I)
    if isinstance(exc, SessionError):
        return EXIT_SESSION
    if isinstance(exc, APIError):
        if exc.status == 409 or (exc.status in (400, 422) and _DUPLICATE_RE.search(str(exc.body))):
            return EXIT_CONFLICT
        return EXIT_REJECTED
    if isinstance(exc, ValueError):
        return EXIT_USAGE
    cause = exc.__cause__ or exc.__context__
    if isinstance(cause, APIError | SessionError):
        return exit_code_for(cause)
    return EXIT_OTHER
