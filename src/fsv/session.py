from __future__ import annotations

import base64
import getpass
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path
from typing import Literal

from argon2.low_level import Type, hash_secret_raw
from cryptography.exceptions import InvalidTag
from cryptography.hazmat.primitives.ciphers.aead import AESGCM

from fsv import config
from fsv.config import CONFIG_DIR, SESSION_FILE, ensure_dirs
from fsv.errors import SessionError

REQUIRED_COOKIES = ("_x_m", "_x_d", "_x_w", "fw-session-id")
STORE_PREF_FILE = CONFIG_DIR / "store"
KEYCHAIN_SERVICE = "fsv"
KEYCHAIN_ACCOUNT = "fsv-session"
_SESSION_CACHE: tuple[Backend, str] | None = None

Backend = Literal["file", "argon", "keychain"]


# ---------- backend selection ----------


def current_backend() -> Backend:
    if STORE_PREF_FILE.exists():
        v = STORE_PREF_FILE.read_text().strip()
        if v in ("file", "argon", "keychain"):
            return v  # type: ignore[return-value]
    if SESSION_FILE.exists():
        return "file"
    if _keychain_available():
        return "keychain"
    return "file"


def set_backend(backend: Backend) -> None:
    ensure_dirs()
    STORE_PREF_FILE.write_text(backend)


# ---------- keychain (macOS) ----------


def _keychain_available() -> bool:
    return sys.platform == "darwin"


def _keychain_write(value: str) -> None:
    if not _keychain_available():
        raise SessionError("keychain backend is macOS only")
    subprocess.run(
        ["security", "add-generic-password", "-U",
         "-a", KEYCHAIN_ACCOUNT, "-s", KEYCHAIN_SERVICE,
         "-w", value],
        check=True, capture_output=True,
    )


def _keychain_read() -> str:
    if not _keychain_available():
        raise SessionError("keychain backend is macOS only")
    r = subprocess.run(
        ["security", "find-generic-password",
         "-a", KEYCHAIN_ACCOUNT, "-s", KEYCHAIN_SERVICE, "-w"],
        capture_output=True, text=True,
    )
    if r.returncode != 0:
        raise SessionError("no session in Keychain; run `fsv auth login`")
    return r.stdout.strip()


def _keychain_delete() -> None:
    if not _keychain_available():
        return
    subprocess.run(
        ["security", "delete-generic-password",
         "-a", KEYCHAIN_ACCOUNT, "-s", KEYCHAIN_SERVICE],
        capture_output=True,
    )


# ---------- argon2 file encryption ----------


def _b64e(b: bytes) -> str:
    return base64.b64encode(b).decode("ascii")


def _b64d(s: str) -> bytes:
    return base64.b64decode(s.encode("ascii"))


def _derive_key(passphrase: str, salt: bytes, params: dict[str, int]) -> bytes:
    return hash_secret_raw(
        passphrase.encode("utf-8"),
        salt,
        time_cost=params["time_cost"],
        memory_cost=params["memory_cost"],
        parallelism=params["parallelism"],
        hash_len=32,
        type=Type.ID,
    )


def _passphrase(confirm: bool = False) -> str:
    p1 = getpass.getpass("Passphrase: ")
    if not p1:
        raise SessionError("empty passphrase")
    if confirm:
        p2 = getpass.getpass("Confirm passphrase: ")
        if p1 != p2:
            raise SessionError("passphrases do not match")
    return p1


def _argon_encrypt(raw: str) -> str:
    params = {"time_cost": 3, "memory_cost": 65536, "parallelism": 4}
    salt = os.urandom(16)
    nonce = os.urandom(12)
    key = _derive_key(_passphrase(confirm=True), salt, params)
    ct = AESGCM(key).encrypt(nonce, raw.encode("utf-8"), config.DOMAIN.encode("utf-8"))
    return json.dumps({
        "version": 1,
        "backend": "argon",
        "kdf": "argon2id",
        "cipher": "aes-256-gcm",
        "params": params,
        "salt": _b64e(salt),
        "nonce": _b64e(nonce),
        "data": _b64e(ct),
    })


def _argon_decrypt(raw: str) -> str:
    data = json.loads(raw)
    if data.get("backend") != "argon" or data.get("kdf") != "argon2id":
        raise SessionError("invalid encrypted session file")
    key = _derive_key(_passphrase(), _b64d(data["salt"]), data["params"])
    try:
        pt = AESGCM(key).decrypt(_b64d(data["nonce"]), _b64d(data["data"]), config.DOMAIN.encode("utf-8"))
    except InvalidTag as e:
        raise SessionError("wrong passphrase or corrupted session") from e
    return pt.decode("utf-8")


# ---------- unified read/write ----------


def _read_file_session(backend: Backend) -> str:
    global _SESSION_CACHE
    if _SESSION_CACHE and _SESSION_CACHE[0] == backend:
        return _SESSION_CACHE[1]
    if not SESSION_FILE.exists():
        raise SessionError("no session; run `fsv auth login`")
    raw = SESSION_FILE.read_text()
    payload = _argon_decrypt(raw) if backend == "argon" else raw
    _SESSION_CACHE = (backend, payload)
    return payload


def load_cookies() -> dict[str, str]:
    backend = current_backend()
    raw = _keychain_read() if backend == "keychain" else _read_file_session(backend)
    data = json.loads(raw)
    domain = config.require_domain()
    if data.get("domain") != domain:
        raise SessionError(f"session is for {data.get('domain')}, not {domain}; run `fsv auth login`")
    cookies = data.get("cookies", {})
    missing = [c for c in REQUIRED_COOKIES if c not in cookies]
    if missing:
        raise SessionError(f"session missing cookies {missing}; run `fsv auth login`")
    return cookies


def save_cookies(cookies: dict[str, str], backend: Backend | None = None) -> Backend:
    global _SESSION_CACHE
    if backend is None:
        backend = current_backend()
    payload = json.dumps({"saved_at": time.time(), "domain": config.require_domain(), "cookies": cookies})
    if backend == "keychain":
        _keychain_write(payload)
        SESSION_FILE.unlink(missing_ok=True)
    else:
        ensure_dirs()
        data = _argon_encrypt(payload) if backend == "argon" else payload
        fd = os.open(SESSION_FILE, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(fd, "w") as f:
            f.write(data)
        SESSION_FILE.chmod(0o600)
        _keychain_delete()
    _SESSION_CACHE = (backend, payload)
    set_backend(backend)
    return backend


def update_cookie(name: str, value: str) -> None:
    """Best-effort single-cookie update. Silent on argon (needs passphrase) or errors."""
    global _SESSION_CACHE
    backend = current_backend()
    if backend == "argon":
        return
    try:
        raw = _keychain_read() if backend == "keychain" else _read_file_session(backend)
        data = json.loads(raw)
        if data.get("cookies", {}).get(name) == value:
            return
        data["cookies"][name] = value
        payload = json.dumps(data)
        if backend == "keychain":
            _keychain_write(payload)
        else:
            fd = os.open(SESSION_FILE, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
            with os.fdopen(fd, "w") as f:
                f.write(payload)
            SESSION_FILE.chmod(0o600)
        _SESSION_CACHE = (backend, payload)
    except Exception:
        pass


def logout() -> None:
    global _SESSION_CACHE
    _SESSION_CACHE = None
    SESSION_FILE.unlink(missing_ok=True)
    _keychain_delete()
    STORE_PREF_FILE.unlink(missing_ok=True)


def cookie_header() -> str:
    return "; ".join(f"{k}={v}" for k, v in load_cookies().items())


def session_age_hours() -> float | None:
    try:
        backend = current_backend()
        raw = _keychain_read() if backend == "keychain" else _read_file_session(backend)
        saved = json.loads(raw).get("saved_at")
        return None if not saved else (time.time() - saved) / 3600
    except SessionError:
        return None



# ---------- login helpers ----------


def parse_cookie_header(s: str) -> dict[str, str]:
    out: dict[str, str] = {}
    for part in s.split(";"):
        if "=" not in part:
            continue
        k, _, v = part.strip().partition("=")
        if k and v:
            out[k.strip()] = v.strip()
    return out


def validate(cookies: dict[str, str]) -> None:
    missing = [c for c in REQUIRED_COOKIES if c not in cookies]
    if missing:
        raise SessionError(f"missing required cookies: {missing}")


def _read_secret(prompt: str) -> str:
    """Read hidden input from tty.

    Handles both single-line (cookie value) and multi-line (full headers block)
    pastes.  After each newline we wait up to 80 ms for more data; if more
    arrives we keep accumulating (it's a paste), otherwise we stop (user hit
    Enter).  Remaining tty input is flushed so nothing leaks to the shell.
    """
    import select
    import termios
    import tty

    sys.stderr.write(prompt)
    sys.stderr.flush()

    if not sys.stdin.isatty():
        return sys.stdin.read().rstrip("\n")

    fd = sys.stdin.fileno()
    old = termios.tcgetattr(fd)
    lines: list[str] = []
    chars: list[str] = []
    try:
        tty.setcbreak(fd)  # no echo, no canonical buffer
        while True:
            ch = os.read(fd, 1).decode("utf-8", errors="replace")
            if ch == "\x03":  # Ctrl-C
                raise KeyboardInterrupt
            if ch == "\x04":  # Ctrl-D
                break
            if ch in ("\x7f", "\x08"):  # backspace
                if chars:
                    chars.pop()
                continue
            if ch in ("\r", "\n"):
                lines.append("".join(chars))
                chars = []
                # More data arriving within 80 ms → multi-line paste, keep reading
                ready, _, _ = select.select([sys.stdin], [], [], 0.08)
                if not ready:
                    break
                continue
            chars.append(ch)
        if chars:
            lines.append("".join(chars))
    finally:
        termios.tcflush(fd, termios.TCIFLUSH)  # discard any extra pasted lines
        termios.tcsetattr(fd, termios.TCSADRAIN, old)
        sys.stderr.write("\n")
        sys.stderr.flush()

    return "\n".join(lines)


_CURL_COOKIE_RE = re.compile(
    r"-H\s+['\"]cookie:\s*([^'\"]+)['\"]",
    re.IGNORECASE,
)


def _extract_cookie_from_input(raw: str) -> str:
    """Extract cookie value from any paste format:

    1. Bare cookie string:              ``_x_m=a; _x_d=b``
    2. Standard HTTP header line:       ``Cookie: _x_m=a; _x_d=b``
    3. Chrome DevTools alternating fmt: header name on one line, value on next::

           cookie
           _x_m=a; _x_d=b

    4. cURL (bash/cmd):                 ``-H 'cookie: _x_m=a'`` / ``-H "cookie: _x_m=a"``
    5. Firefox JSON headers:            ``[{"name":"cookie","value":"_x_m=a"}]``
    6. Fetch API headers object:        ``{"cookie": "_x_m=a"}``
    """
    lines = raw.splitlines()

    # 3. Chrome DevTools alternating format
    for i, line in enumerate(lines[:-1]):
        if line.strip().lower() == "cookie":
            return lines[i + 1].strip()

    # 2. Standard HTTP format: "Cookie: value"
    for line in lines:
        if line.lower().startswith("cookie:"):
            return line.split(":", 1)[1].strip()

    # 4. cURL format: -H 'Cookie: value' or -H "Cookie: value"
    m = _CURL_COOKIE_RE.search(raw)
    if m:
        return m.group(1).strip()

    # 5 & 6. JSON formats (Firefox header list or fetch headers object)
    try:
        data = json.loads(raw)
        if isinstance(data, list):
            for item in data:
                if isinstance(item, dict) and item.get("name", "").lower() == "cookie":
                    return str(item["value"]).strip()
        elif isinstance(data, dict):
            headers = data.get("headers", data)  # fetch() wraps under "headers"
            if isinstance(headers, dict):
                for k, v in headers.items():
                    if k.lower() == "cookie":
                        return str(v).strip()
    except (json.JSONDecodeError, ValueError, TypeError):
        pass

    # 1. Bare cookie string
    return raw.strip()


def login_interactive() -> dict[str, str]:
    sys.stderr.write(f"Open your Freshservice portal: https://{config.DOMAIN}/\n")
    sys.stderr.write("DevTools → Network tab → click any /api/_/ request → Headers tab\n")
    sys.stderr.write("RIGHT-CLICK the Cookie header → Copy value  (don't drag-select, value is truncated)\n")
    sys.stderr.write("Also accepts: Cookie: header line, all request headers, cURL, or fetch snippet.\n")
    sys.stderr.write("Input is hidden — paste and press Enter:\n")
    raw = _read_secret("> ")
    if not raw:
        raise SessionError("empty input")
    line = _extract_cookie_from_input(raw)
    if not line:
        raise SessionError("could not find Cookie header in pasted input")
    cookies = parse_cookie_header(line)
    validate(cookies)
    return cookies
