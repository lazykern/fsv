from __future__ import annotations

import json
import os
import stat
import time

import pytest

import fsv.cache as cache
import fsv.config as config


def test_mark_stale_actually_makes_entry_stale(tmp_path):
    p = tmp_path / "c.json"
    cache.save(p, "schema", [{"name": "x"}])
    assert cache.load(p)[1] is False
    cache.mark_stale(p)
    doc, stale = cache.load(p)
    assert doc is not None and stale is True


def test_cache_list_json_is_a_miss_not_a_crash(tmp_path):
    p = tmp_path / "c.json"
    p.write_text("[1, 2, 3]")
    assert cache.load(p) == (None, True)
    cache.update_meta(p, checked_at=True)  # must not raise
    cache.mark_stale(p)
    assert json.loads(p.read_text()) == [1, 2, 3]


def test_cache_save_is_atomic_and_private(tmp_path):
    p = tmp_path / "sub" / "c.json"
    cache.save(p, "schema", {"a": 1})
    assert stat.S_IMODE(p.stat().st_mode) == 0o600
    assert [f.name for f in p.parent.iterdir()] == ["c.json"]  # no temp leftovers


def test_atomic_write_failure_keeps_old_file(tmp_path, monkeypatch):
    d = tmp_path / "d"
    p = d / "f.json"
    config.atomic_write_text(p, "old")

    def boom(*a, **k):
        raise OSError("disk full")

    monkeypatch.setattr(os, "replace", boom)
    with pytest.raises(OSError):
        config.atomic_write_text(p, "new")
    assert p.read_text() == "old"
    assert [f.name for f in d.iterdir()] == ["f.json"]


def test_save_config_private_and_atomic():
    config.save_config({"domain": "acme.freshservice.com"})
    assert stat.S_IMODE(config.CONFIG_FILE.stat().st_mode) == 0o600
    assert config.load_config()["domain"] == "acme.freshservice.com"


def test_corrupt_config_is_backed_up_and_warned(capsys):
    config.ensure_dirs()
    config.CONFIG_FILE.write_text("{not json")
    assert config.load_config() == {}
    err = capsys.readouterr().err
    assert "unreadable" in err and "corrupt" in err
    backups = list(config.CONFIG_DIR.glob("config.json.corrupt-*"))
    assert len(backups) == 1 and backups[0].read_text() == "{not json"
    # later saves must not destroy the backup
    config.save_config({"a": 1})
    assert backups[0].exists()


def test_non_object_config_is_treated_as_corrupt(capsys):
    config.ensure_dirs()
    config.CONFIG_FILE.write_text("[1]")
    assert config.load_config() == {}
    assert list(config.CONFIG_DIR.glob("config.json.corrupt-*"))


@pytest.mark.parametrize("bad", ["user@acme.freshservice.com", "https://u:p@acme.freshservice.com",
                                 "acme.freshservice.com:8443", "acme .freshservice.com", "acme", ".com"])
def test_normalize_domain_rejects(bad):
    with pytest.raises(ValueError):
        config.normalize_domain(bad)


def test_normalize_domain_accepts_and_cleans():
    assert config.normalize_domain("https://ACME.freshservice.com/a/dashboard") == "acme.freshservice.com"


def test_normalize_domain_warns_for_foreign_host(capsys):
    assert config.normalize_domain("portal.example.org", warn=True) == "portal.example.org"
    assert "not a *.freshservice.com" in capsys.readouterr().err
    config.normalize_domain("acme.freshservice.com", warn=True)
    assert capsys.readouterr().err == ""


def test_paths_follow_env(monkeypatch, tmp_path):
    monkeypatch.setenv("FSV_CONFIG_DIR", str(tmp_path / "c2"))
    monkeypatch.setenv("FSV_CACHE_DIR", str(tmp_path / "k2"))
    config.reload()
    assert config.SESSION_FILE == tmp_path / "c2" / "session.json"
    assert config.SCHEMA_DIR == tmp_path / "k2" / "schema"
    assert config.display_path(tmp_path / "x") == str(tmp_path / "x")
