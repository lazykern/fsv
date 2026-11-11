from __future__ import annotations

import base64
import json
import stat
import subprocess

import pytest

import fsv.config as config
import fsv.session as session
from fsv.errors import SessionError

COOKIES = {"_x_m": "a", "_x_d": "b", "_x_w": "c", "fw-session-id": "tok"}


def test_keychain_write_keeps_secret_out_of_argv(monkeypatch):
    monkeypatch.setattr(session, "_keychain_available", lambda: True)
    seen = {}

    def fake_run(argv, **kw):
        seen["argv"], seen["input"] = argv, kw.get("input")
        return subprocess.CompletedProcess(argv, 0, "", "")

    monkeypatch.setattr(subprocess, "run", fake_run)
    secret = json.dumps({"cookies": {"fw-session-id": "SUPER-SECRET"}})
    session._keychain_write(secret)
    assert "SUPER-SECRET" not in " ".join(seen["argv"])
    assert seen["argv"] == ["security", "-i"]
    assert "add-generic-password" in seen["input"]
    assert "SUPER-SECRET" not in seen["input"]  # base64-encoded, so no quoting issues
    encoded = seen["input"].split(" -w ")[1].strip()
    assert base64.b64decode(encoded).decode() == secret


def test_keychain_write_failure_raises(monkeypatch):
    monkeypatch.setattr(session, "_keychain_available", lambda: True)
    monkeypatch.setattr(subprocess, "run", lambda argv, **kw: subprocess.CompletedProcess(argv, 1, "", "denied"))
    with pytest.raises(SessionError):
        session._keychain_write("x")


def test_keychain_read_decodes_new_and_legacy(monkeypatch):
    monkeypatch.setattr(session, "_keychain_available", lambda: True)
    payload = json.dumps({"domain": "acme.freshservice.com"})
    for stored in (base64.b64encode(payload.encode()).decode(), payload):
        monkeypatch.setattr(subprocess, "run",
                            lambda argv, _s=stored, **kw: subprocess.CompletedProcess(argv, 0, _s + "\n", ""))
        assert session._keychain_read() == payload


def test_keychain_disabled_in_tests():
    assert session._keychain_available() is False
    assert session.current_backend() == "file"


def test_file_session_roundtrip_is_private_and_atomic():
    assert session.save_cookies(COOKIES, "file") == "file"
    assert stat.S_IMODE(config.SESSION_FILE.stat().st_mode) == 0o600
    assert sorted(p.name for p in config.CONFIG_DIR.iterdir() if p.name.startswith(".")) == []
    session._SESSION_CACHE = None
    assert session.load_cookies() == COOKIES


def test_update_cookie_rewrites_atomically():
    session.save_cookies(COOKIES, "file")
    session.update_cookie("fw-session-id", "rotated")
    session._SESSION_CACHE = None
    assert session.load_cookies()["fw-session-id"] == "rotated"
    assert stat.S_IMODE(config.SESSION_FILE.stat().st_mode) == 0o600


def test_argon_params_are_clamped():
    raw = json.dumps({"backend": "argon", "kdf": "argon2id",
                      "params": {"time_cost": 3, "memory_cost": 1 << 40, "parallelism": 4},
                      "salt": "", "nonce": "", "data": ""})
    with pytest.raises(SessionError, match="KDF"):
        session._argon_decrypt(raw)
