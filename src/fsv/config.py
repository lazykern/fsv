from __future__ import annotations

import json
import os
from pathlib import Path
from urllib.parse import urlparse

CONFIG_DIR = Path(os.environ.get("FSV_CONFIG_DIR", Path.home() / ".config" / "fsv"))
CACHE_DIR = Path(os.environ.get("FSV_CACHE_DIR", Path.home() / ".cache" / "fsv"))

CONFIG_FILE = CONFIG_DIR / "config.json"
SESSION_FILE = CONFIG_DIR / "session.json"
SCHEMA_DIR = CACHE_DIR / "schema"
FILTERS_DIR = CACHE_DIR / "filters"


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
    except Exception:
        return {}
    return data if isinstance(data, dict) else {}


def save_config(data: dict) -> None:
    ensure_dirs()
    CONFIG_FILE.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n")
    CONFIG_FILE.chmod(0o600)


def normalize_domain(value: str) -> str:
    raw = value.strip()
    if not raw:
        raise ValueError("empty domain")
    parsed = urlparse(raw if "://" in raw else f"https://{raw}")
    host = (parsed.netloc or parsed.path).split("/", 1)[0].strip().lower()
    host = host.rstrip("/")
    if not host or "." not in host:
        raise ValueError("domain must look like acme.freshservice.com")
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
    domain = normalize_domain(value)
    cfg = load_config()
    cfg["domain"] = domain
    save_config(cfg)
    DOMAIN = domain
    API_V2, API_INT, LOGIN_URL = _urls(domain)
    return domain


def require_domain() -> str:
    if DOMAIN:
        return DOMAIN
    raise RuntimeError("no Freshservice domain; run `fsv auth login --domain yourcompany.freshservice.com`")


def cache_domain_key(domain: str | None = None) -> str:
    raw = (domain or DOMAIN or _configured_domain()).strip()
    if not raw:
        return "default"
    try:
        return normalize_domain(raw)
    except ValueError:
        return raw.lower().replace("/", "-")


def schema_cache_path(name: str, domain: str | None = None) -> Path:
    return SCHEMA_DIR / f"{cache_domain_key(domain)}--{name}.json"


def schema_cache_candidates(name: str, domain: str | None = None) -> list[Path]:
    primary = schema_cache_path(name, domain)
    legacy = SCHEMA_DIR / f"{name}.json"
    return [primary] if primary == legacy else [primary, legacy]


def filters_cache_path(name: str, domain: str | None = None) -> Path:
    return FILTERS_DIR / f"{cache_domain_key(domain)}--{name}.json"


def filters_cache_candidates(name: str, domain: str | None = None) -> list[Path]:
    primary = filters_cache_path(name, domain)
    legacy = FILTERS_DIR / f"{name}.json"
    return [primary] if primary == legacy else [primary, legacy]


def groups_cache_path(domain: str | None = None) -> Path:
    return CACHE_DIR / f"groups--{cache_domain_key(domain)}.json"


def groups_cache_candidates(domain: str | None = None) -> list[Path]:
    primary = groups_cache_path(domain)
    legacy = CONFIG_DIR / "groups.json"
    legacy_cache = CACHE_DIR / "groups.json"
    candidates = [primary]
    for p in (legacy_cache, legacy):
        if p != primary:
            candidates.append(p)
    return candidates
