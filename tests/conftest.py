"""Hermetic test environment.

fsv reads FSV_CONFIG_DIR / FSV_CACHE_DIR at import time, so the env is pointed
at a throw-away directory *before* any fsv module is imported (module level
below), and again per test (autouse fixture) so no state leaks between tests
or from the developer's real ~/.config/fsv.
"""
from __future__ import annotations

import os
import socket
import sys
import tempfile

import pytest

_BOOT = tempfile.mkdtemp(prefix="fsv-test-")
os.environ["FSV_CONFIG_DIR"] = os.path.join(_BOOT, "config")
os.environ["FSV_CACHE_DIR"] = os.path.join(_BOOT, "cache")
os.environ["FSV_NO_KEYCHAIN"] = "1"  # never touch the real macOS Keychain
os.environ["XDG_STATE_HOME"] = os.path.join(_BOOT, "state")  # write journal
os.environ.pop("FSV_BROWSER", None)


def _import_all_fsv() -> None:
    """Import every fsv module once, up front, so the per-test snapshot below
    is complete and restoring it is exact."""
    import importlib
    import pkgutil

    import fsv

    for mod in pkgutil.walk_packages(fsv.__path__, "fsv."):
        short = mod.name.split(".", 1)[1]
        if short.startswith("_") or short.startswith("forms"):
            continue  # _complete sets FSV_THIN_COMPLETE on import; forms is separate WIP
        try:
            importlib.import_module(mod.name)
        except Exception:  # optional/unfinished modules must not break collection
            pass


_import_all_fsv()


@pytest.fixture(autouse=True)
def _isolated_fsv_env(tmp_path, monkeypatch):
    # Hard guarantee: no test may reach a real host.
    real_getaddrinfo = socket.getaddrinfo

    def _no_dns(host, *a, **k):
        if host in ("localhost", "127.0.0.1", "::1", None, ""):
            return real_getaddrinfo(host, *a, **k)
        raise OSError(f"network disabled in tests (tried to resolve {host!r})")

    monkeypatch.setattr(socket, "getaddrinfo", _no_dns)

    # Some tests purge and re-import fsv.* to re-read env-derived globals.
    # Restore the original module objects afterwards so test files that bound
    # `import fsv.cli as cli` at import time keep talking to live modules.
    saved_modules = {k: v for k, v in sys.modules.items() if k == "fsv" or k.startswith("fsv.")}
    pkg = sys.modules.get("fsv")
    saved_attrs = {k.split(".", 1)[1]: v for k, v in saved_modules.items() if k != "fsv"}

    # fsv.cli monkeypatches typer.main.TyperCommand/TyperGroup at import; a
    # re-import would leave them pointing at a different class generation.
    import typer.main as typer_main
    saved_typer = (typer_main.TyperCommand, typer_main.TyperGroup)

    cfg = tmp_path / "fsv-config"
    cache = tmp_path / "fsv-cache"
    monkeypatch.setenv("FSV_CONFIG_DIR", str(cfg))
    monkeypatch.setenv("FSV_CACHE_DIR", str(cache))
    monkeypatch.setenv("FSV_NO_KEYCHAIN", "1")
    monkeypatch.setenv("XDG_STATE_HOME", str(tmp_path / "fsv-state"))
    monkeypatch.delenv("FSV_COMPLETION_NETWORK", raising=False)

    config = sys.modules.get("fsv.config")
    if config is None:
        import fsv.config as config
    cfg.mkdir(parents=True, exist_ok=True)
    (cfg / "config.json").write_text('{"domain": "acme.freshservice.com"}')
    config.reload()  # paths + DOMAIN from the tmp config

    completion = sys.modules.get("fsv.completion")
    if completion is not None:
        monkeypatch.setattr(completion, "_CFG", config.CONFIG_DIR, raising=False)
    session = sys.modules.get("fsv.session")
    if session is not None:
        monkeypatch.setattr(session, "_SESSION_CACHE", None)
    client = sys.modules.get("fsv.client")
    if client is not None:
        client._instance = None
    yield
    for name in [k for k in sys.modules if k == "fsv" or k.startswith("fsv.")]:
        if name not in saved_modules:
            del sys.modules[name]
    sys.modules.update(saved_modules)
    if pkg is not None:
        for attr, mod in saved_attrs.items():
            if "." not in attr:
                setattr(pkg, attr, mod)
    typer_main.TyperCommand, typer_main.TyperGroup = saved_typer
