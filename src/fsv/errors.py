from __future__ import annotations


def _strip_tags(text: str) -> str:
    import html
    import re
    text = re.sub(r"(?is)<(script|style).*?</\1>", " ", text)
    text = re.sub(r"<[^>]+>", " ", text)
    return " ".join(html.unescape(text).split())


def server_message(body: object, limit: int = 240) -> str:
    """One-line human message extracted from an error response body."""
    def one(x: object) -> str:
        if isinstance(x, dict):
            msg = next((x[k] for k in ("message", "description", "error", "detail", "msg") if x.get(k)), None)
            field = x.get("field")
            if msg is None and x.get("errors"):
                return one(x["errors"])
            text = one(msg) if msg is not None else ""
            return f"{field}: {text}" if field and text else text
        if isinstance(x, (list, tuple)):
            return "; ".join(t for t in (one(i) for i in x[:5]) if t)
        if x is None:
            return ""
        return _strip_tags(str(x))
    text = one(body)
    if isinstance(body, dict) and body.get("errors") and body.get("description"):
        extra = one(body["errors"])
        if extra and extra not in text:
            text = f"{text} ({extra})"
    if len(text) > limit:
        text = text[: limit - 1].rstrip() + "…"
    return text


def _describe_request(method: str | None, target: str | None) -> str:
    if not method or not target:
        return ""
    path = target.split("?", 1)[0]
    segs = [s for s in path.split("/") if s and s not in ("api", "_", "v2", "a")]
    named = [s for s in segs if not s.isdigit()]
    noun = (named[-1] if named else "resource").replace("_", " ")
    is_item = bool(segs) and segs[-1].isdigit()
    m = method.upper()
    if m == "GET":
        return f"{'fetching' if is_item else 'listing'} {noun}" if not is_item else f"fetching {noun.rstrip('s')}"
    verb = {"POST": "creating", "PUT": "updating", "PATCH": "updating", "DELETE": "deleting"}.get(m)
    if verb:
        return f"{verb} {noun.rstrip('s') if is_item or m != 'POST' else noun}"
    return f"on {m} {path}"


class APIError(Exception):
    """Server/engine error. ``str()`` is a clean single line; ``detail()`` has the raw body."""

    def __init__(self, status: int, body: object, method: str | None = None, target: str | None = None) -> None:
        self.status = status
        self.body = body
        self.method = method
        self.target = target
        what = _describe_request(method, target)
        msg = server_message(body)
        # Zero is a local guard failure, never a real HTTP response.
        head = (f"HTTP {status}" if status else "Error") + (f" {what}" if what else "")
        super().__init__(f"{head}: {msg}" if msg else head)

    def detail(self) -> str:
        where = f"{(self.method or '').upper()} {self.target}".strip()
        body = self.body if isinstance(self.body, str) else repr(self.body)
        return f"{where}\n{body}" if where else body


class UsageError(APIError, ValueError):
    """Local validation; retain APIError compatibility for library callers."""

    def __init__(self, message: str) -> None:
        super().__init__(0, message)
        # Local choices/help must not use the server-body truncation policy.
        self.args = (message,)


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
    if isinstance(exc, UsageError):
        return EXIT_USAGE
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
