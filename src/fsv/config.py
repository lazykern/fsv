from __future__ import annotations

import json
import os
import sys
import tempfile
import time
from pathlib import Path
from urllib.parse import urlparse

# Paths are derived from FSV_CONFIG_DIR / FSV_CACHE_DIR.  They are module
# globals (kept for backwards compatibility) but are recomputed by
# refresh_paths(), which tests call after pointing the env at a tmp dir.
CONFIG_DIR: Path
CACHE_DIR: Path
CONFIG_FILE: Path
SESSION_FILE: Path
SCHEMA_DIR: Path
FILTERS_DIR: Path


def refresh_paths() -> None:
    """Recompute all path globals from the current environment."""
    global CONFIG_DIR, CACHE_DIR, CONFIG_FILE, SESSION_FILE, SCHEMA_DIR, FILTERS_DIR
    CONFIG_DIR = Path(os.environ.get("FSV_CONFIG_DIR") or Path.home() / ".config" / "fsv")
    CACHE_DIR = Path(os.environ.get("FSV_CACHE_DIR") or Path.home() / ".cache" / "fsv")
    CONFIG_FILE = CONFIG_DIR / "config.json"
    SESSION_FILE = CONFIG_DIR / "session.json"
    SCHEMA_DIR = CACHE_DIR / "schema"
    FILTERS_DIR = CACHE_DIR / "filters"


refresh_paths()


def display_path(path: Path) -> str:
    """Human-friendly path (``~`` for home) for messages."""
    try:
        return "~/" + str(path.relative_to(Path.home()))
    except ValueError:
        return str(path)


def atomic_write_text(path: Path, text: str, mode: int = 0o600) -> None:
    """Write *text* to *path* atomically (tempfile + os.replace) with *mode*."""
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=path.parent, prefix=f".{path.name}.", suffix=".tmp")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(text)
        os.chmod(tmp, mode)
        os.replace(tmp, path)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


def ensure_dirs() -> None:
    CONFIG_DIR.mkdir(parents=True, exist_ok=True)
    CONFIG_DIR.chmod(0o700)
    SCHEMA_DIR.mkdir(parents=True, exist_ok=True)
    FILTERS_DIR.mkdir(parents=True, exist_ok=True)


def load_config() -> dict:
    if not CONFIG_FILE.exists():
        return {}
    try:
        data = json.loads(CONFIG_FILE.read_text())
        if not isinstance(data, dict):
            raise ValueError("config root is not an object")
    except Exception as e:
        # Never silently discard (and later overwrite) a corrupt config.
        backup = CONFIG_FILE.with_name(f"{CONFIG_FILE.name}.corrupt-{int(time.time())}")
        try:
            os.replace(CONFIG_FILE, backup)
            where = f"backed up to {backup}"
        except OSError:
            where = "could not be backed up"
        print(f"warning: {CONFIG_FILE} is unreadable ({e}); {where}; starting with empty config",
              file=sys.stderr)
        return {}
    return data


def save_config(data: dict) -> None:
    ensure_dirs()
    atomic_write_text(CONFIG_FILE, json.dumps(data, indent=2, sort_keys=True) + "\n")


def normalize_domain(value: str, warn: bool = False) -> str:
    raw = value.strip()
    if not raw:
        raise ValueError("empty domain")
    if any(ch.isspace() for ch in raw):
        raise ValueError("domain must not contain whitespace")
    parsed = urlparse(raw if "://" in raw else f"https://{raw}")
    netloc = parsed.netloc or parsed.path.split("/", 1)[0]
    if "@" in netloc:
        raise ValueError("domain must not contain user info ('@')")
    host = netloc.lower().rstrip("/")
    if ":" in host:
        raise ValueError("domain must not contain a port")
    if not host or "." not in host or host.startswith(".") or host.endswith("."):
        raise ValueError("domain must look like acme.freshservice.com")
    if warn and not host.endswith(".freshservice.com"):
        print(f"warning: {host} is not a *.freshservice.com domain; "
              "session cookies will be sent to it", file=sys.stderr)
    return host


def _configured_domain() -> str:
    domain = load_config().get("domain")
    if not domain:
        return ""
    try:
        return normalize_domain(str(domain))
    except ValueError:
        return ""


def _urls(domain: str) -> tuple[str, str, str]:
    if not domain:
        return "", "", ""
    return f"https://{domain}/api/v2", f"https://{domain}/api/_", f"https://{domain}/"


DOMAIN = _configured_domain()
API_V2, API_INT, LOGIN_URL = _urls(DOMAIN)


def set_domain(value: str) -> str:
    global DOMAIN, API_V2, API_INT, LOGIN_URL
    domain = normalize_domain(value, warn=True)
    cfg = load_config()
    cfg["domain"] = domain
    save_config(cfg)
    DOMAIN = domain
    API_V2, API_INT, LOGIN_URL = _urls(domain)
    return domain


def require_domain() -> str:
    if DOMAIN:
        return DOMAIN
    raise RuntimeError("no Freshservice domain; run `fsv auth login --domain acme.freshservice.com`")


def cache_domain_key(domain: str | None = None) -> str:
    raw = (domain or DOMAIN or _configured_domain()).strip()
    if not raw:
        return "default"
    try:
        return normalize_domain(raw)
    except ValueError:
        return raw.lower().replace("/", "-")


def cache_workspace_key() -> str:
    """The workspace dimension of schema/form-field cache keys: the
    ``forms.workspace_id`` override, else ``auto`` (the form metadata's own
    workspace). Switching the override never reuses another workspace's
    cached fields."""
    raw = (load_config().get("forms") or {}).get("workspace_id")
    text = str(raw).strip() if raw not in (None, "") else ""
    return f"ws{text}" if text and text.isascii() and text.isalnum() else "wsauto"


def cache_scope_key(domain: str | None = None) -> str:
    """``<domain>--ws<id>``: schema and form-field caches are keyed by tenant
    AND workspace. Older ``<domain>--<name>`` files are simply never read."""
    return f"{cache_domain_key(domain)}--{cache_workspace_key()}"


def schema_cache_path(name: str, domain: str | None = None) -> Path:
    return SCHEMA_DIR / f"{cache_scope_key(domain)}--{name}.json"


def schema_cache_candidates(name: str, domain: str | None = None) -> list[Path]:
    return [schema_cache_path(name, domain)]


def filters_cache_path(name: str, domain: str | None = None) -> Path:
    return FILTERS_DIR / f"{cache_domain_key(domain)}--{name}.json"


def filters_cache_candidates(name: str, domain: str | None = None) -> list[Path]:
    return [filters_cache_path(name, domain)]


def get_default_where(res_name: str) -> list[str]:
    return list((load_config().get("defaults") or {}).get(res_name, {}).get("where") or [])


def set_default_where(res_name: str, exprs: list[str]) -> None:
    cfg = load_config()
    cfg.setdefault("defaults", {}).setdefault(res_name, {})["where"] = list(exprs)
    save_config(cfg)


def clear_default_where(res_name: str) -> None:
    cfg = load_config()
    (cfg.get("defaults") or {}).pop(res_name, None)
    save_config(cfg)


def groups_cache_path(domain: str | None = None) -> Path:
    return CACHE_DIR / f"groups--{cache_domain_key(domain)}.json"


def groups_cache_candidates(domain: str | None = None) -> list[Path]:
    return [groups_cache_path(domain)]


def reload() -> None:
    """Re-read paths from env and the domain from the config file (tests/tools)."""
    global DOMAIN, API_V2, API_INT, LOGIN_URL
    refresh_paths()
    DOMAIN = _configured_domain()
    API_V2, API_INT, LOGIN_URL = _urls(DOMAIN)
