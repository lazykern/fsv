"""Make the target tenant impossible to miss.

fsv's config dir decides which Freshservice tenant a command talks to, and the
default dir points at whichever tenant ``fsv auth login`` configured last. One
shared hook, :func:`enter_command`, runs from ``FSVCommand.invoke`` for every
CLI command, so no command can bypass it:

* tenant commands print ``fsv: tenant <domain> (config <dir>)`` once on stderr
  (``--quiet`` / ``FSV_QUIET=1`` suppress it; local commands print nothing);
* ``--tenant DOMAIN`` must equal the configured domain when given;
* ``--yes`` (a real write) needs the tenant confirmed: ``--tenant DOMAIN``,
  ``FSV_ALLOW_TENANT=DOMAIN`` or ``fsv config set tenant.confirmed DOMAIN``;
* ``--expect-plan`` reviewed on another tenant is refused by name.

Plan hashes are bound to the tenant (:func:`bind_plan`), and write-capable
commands add ``tenant``/``config_dir`` to their JSON objects (:func:`tenant_json`).
"""
from __future__ import annotations

import hashlib
import json
import os
import re
import sys
from pathlib import Path
from typing import Any

from fsv import config

# Command paths (prefixes) that never talk to a tenant: no banner, no checks.
LOCAL_COMMANDS = ("help", "setup", "auth login", "auth logout", "completion", "config",
                  "cache status", "cache clear", "journal")
# --yes commands that never reach a tenant (local state only).
LOCAL_YES_COMMANDS = frozenset({"auth logout"})
_TRUE = {"1", "true", "yes", "on"}
_HEX = re.compile(r"^[0-9a-f]{64}$")

_state: dict[str, Any] = {"depth": 0, "write": False, "new": []}
_RAW_OF: dict[str, str] = {}      # bound plan hash -> tenant-free hash
_TENANT_OF: dict[str, str] = {}   # bound plan hash -> domain


class TenantRefusal(Exception):
    """A command refused because the tenant is wrong or unconfirmed."""


def _norm(value: str | None) -> str:
    raw = (value or "").strip()
    try:
        return config.normalize_domain(raw)
    except ValueError:
        return raw.lower()


def config_dir() -> str:
    return config.display_path(config.CONFIG_DIR)


def banner() -> str:
    return f"fsv: tenant {config.DOMAIN or '(none configured)'} (config {config_dir()})"


def is_local(path: str) -> bool:
    return any(path == p or path.startswith(p + " ") for p in LOCAL_COMMANDS)


def quiet(flag: bool = False) -> bool:
    return bool(flag) or os.environ.get("FSV_QUIET", "").strip().lower() in _TRUE


def confirmed_tenant() -> str:
    """``tenant.confirmed`` from this config dir ('' when unset)."""
    value = (config.load_config().get("tenant") or {})
    return _norm(value.get("confirmed")) if isinstance(value, dict) else ""


def confirmation(tenant_opt: str | None = None) -> str | None:
    """How the configured tenant is confirmed for writes, or None."""
    domain = config.DOMAIN
    if not domain:
        return None
    if tenant_opt and _norm(tenant_opt) == domain:
        return "--tenant"
    if _norm(os.environ.get("FSV_ALLOW_TENANT")) == domain:
        return "FSV_ALLOW_TENANT"
    if confirmed_tenant() == domain:
        return "tenant.confirmed"
    return None


def refusal(tenant_opt: str | None = None) -> str:
    domain = config.DOMAIN or "(none configured)"
    why = []
    env = os.environ.get("FSV_ALLOW_TENANT")
    if env and _norm(env) != domain:
        why.append(f"FSV_ALLOW_TENANT={_norm(env)} does not match")
    stale = confirmed_tenant()
    if stale and stale != domain:
        why.append(f"tenant.confirmed={stale} does not match")
    reason = f" ({'; '.join(why)})" if why else ""
    return (f"refusing to write: tenant {domain} (config {config_dir()}) is not confirmed{reason}. "
            f"Confirm it with --tenant {domain}, FSV_ALLOW_TENANT={domain}, or once with "
            f"`fsv config set tenant.confirmed {domain}`. Wrong tenant? Set FSV_CONFIG_DIR "
            "to the intended tenant's config dir")


def _command_path(ctx: Any) -> str:
    names: list[str] = []
    while ctx is not None and ctx.parent is not None:
        names.append(ctx.info_name or "")
        ctx = ctx.parent
    return " ".join(reversed(names))


def _option_values(command: Any, ctx: Any) -> dict[str, Any]:
    out: dict[str, Any] = {}
    for param in getattr(command, "params", []):
        for opt in getattr(param, "opts", []):
            out[opt] = ctx.params.get(param.name)
    return out


def write_capable(opts: dict[str, Any]) -> bool:
    return any(o in opts for o in ("--yes", "--dry-run", "--expect-plan"))


def check(path: str, opts: dict[str, Any], tenant_opt: str | None) -> None:
    """Raise :class:`TenantRefusal` when this command must not run."""
    if is_local(path):
        return
    domain = config.DOMAIN
    if tenant_opt and _norm(tenant_opt) != domain:
        raise TenantRefusal(f"--tenant {_norm(tenant_opt)} does not match the configured tenant "
                            f"{domain or '(none configured)'} (config {config_dir()}); nothing was sent. "
                            "Set FSV_CONFIG_DIR to the intended tenant's config dir")
    expected = opts.get("--expect-plan")
    if expected:
        other = reviewed_elsewhere(str(expected))
        if other:
            raise TenantRefusal(f"plan {str(expected).strip()[:12]} was reviewed on tenant {other}, but this "
                                f"command targets {domain or '(none configured)'} (config {config_dir()}); "
                                "nothing was sent. Review again on this tenant, or set FSV_CONFIG_DIR to "
                                f"{other}'s config dir")
    if opts.get("--yes") and not opts.get("--dry-run") and path not in LOCAL_YES_COMMANDS:
        if confirmation(tenant_opt) is None:
            raise TenantRefusal(refusal(tenant_opt))


def enter_command(command: Any, ctx: Any) -> None:
    """The shared startup hook: banner, tenant assertion, write confirmation."""
    _state["depth"] += 1
    if _state["depth"] > 1:
        return
    root = ctx.find_root()
    params = root.params if root is not ctx else {}
    tenant_opt = params.get("tenant")
    opts = _option_values(command, ctx)
    path = _command_path(ctx)
    _state["write"] = write_capable(opts) and not is_local(path)
    _state["new"] = []
    if is_local(path):
        return
    if not quiet(params.get("quiet", False)):
        print(banner(), file=sys.stderr, flush=True)
    try:
        check(path, opts, tenant_opt)
    except TenantRefusal as e:
        import typer
        print(f"error: {e}", file=sys.stderr, flush=True)
        raise typer.Exit(2) from None


def leave_command() -> None:
    _state["depth"] = max(0, _state["depth"] - 1)
    if _state["depth"]:
        return
    if _state["write"] and _state["new"]:
        _remember(_state["new"])
    _state["write"] = False
    _state["new"] = []


# --------------------------------------------------------------------------
# JSON annotation


def annotate(obj: Any) -> Any:
    """Add ``tenant``/``config_dir`` to a write command's JSON object."""
    if not _state["write"] or not isinstance(obj, dict):
        return obj
    out = dict(obj)
    out.setdefault("tenant", config.DOMAIN)
    out.setdefault("config_dir", str(config.CONFIG_DIR))
    return out


def tenant_json(obj: Any, **kwargs: Any) -> str:
    """``json.dumps`` for command output (annotated in write commands)."""
    return json.dumps(annotate(obj), **kwargs)


# --------------------------------------------------------------------------
# tenant-bound plan hashes


def bind_plan(raw: str, domain: str | None = None) -> str:
    """Bind a tenant-free plan hash to *domain* (default: configured)."""
    d = config.DOMAIN if domain is None else domain
    bound = hashlib.sha256(f"fsv-plan-tenant\0{d}\0{raw}".encode("utf-8")).hexdigest()
    _RAW_OF[bound] = raw
    _TENANT_OF[bound] = d
    if _state["write"]:
        _state["new"].append(bound)
    return bound


def unbind(value: Any) -> Any:
    """Replace bound plan hashes nested in *value* by their tenant-free form,
    so composed hashes (a batch of plan hashes) bind the tenant exactly once
    and stay comparable with journals written before the binding."""
    if isinstance(value, str):
        return _RAW_OF.get(value, value)
    if isinstance(value, dict):
        return {k: unbind(v) for k, v in value.items()}
    if isinstance(value, (list, tuple)):
        return [unbind(v) for v in value]
    return value


def bind_legacy(value: Any) -> Any:
    """A journal plan hash written before tenant binding -> its bound form."""
    if isinstance(value, str) and _HEX.match(value):
        return bind_plan(value)
    return value


def _registry() -> Path:
    base = os.environ.get("XDG_STATE_HOME") or str(Path.home() / ".local" / "state")
    return Path(base) / "fsv" / "plan-tenants.jsonl"


def _remember(hashes: list[str]) -> None:
    """Record which tenant each reviewed plan hash belongs to (hashes and
    domains only). Never raises."""
    try:
        p = _registry()
        p.parent.mkdir(parents=True, exist_ok=True)
        lines = [json.dumps({"plan": h, "tenant": _TENANT_OF.get(h, "")}) for h in dict.fromkeys(hashes)]
        if p.exists() and p.stat().st_size > 2_000_000:
            keep = p.read_text(encoding="utf-8").splitlines()[-5000:]
            config.atomic_write_text(p, "\n".join(keep) + "\n")
        fd = os.open(p, os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o600)
        try:
            os.write(fd, ("\n".join(lines) + "\n").encode("utf-8"))
        finally:
            os.close(fd)
    except Exception:
        pass


def reviewed_elsewhere(expected: str) -> str | None:
    """The other tenant a reviewed plan hash (or >=12-char prefix) belongs
    to, when it is known only for another tenant; else None."""
    e = expected.strip().lower().removeprefix("sha256:")
    if len(e) < 12:
        return None
    seen: dict[str, str] = {h: d for h, d in _TENANT_OF.items() if h.startswith(e)}
    try:
        p = _registry()
        if p.exists():
            for line in p.read_text(encoding="utf-8").splitlines():
                try:
                    row = json.loads(line)
                except ValueError:
                    continue
                h = str(row.get("plan") or "")
                if h.startswith(e):
                    seen.setdefault(h, str(row.get("tenant") or ""))
    except OSError:
        return None
    domains = set(seen.values())
    if not domains or config.DOMAIN in domains:
        return None
    return sorted(domains)[0]
