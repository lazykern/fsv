"""Attachment downloads never create a bare ./<ID> directory by default."""
import json
from pathlib import Path

import pytest
from typer.testing import CliRunner

import fsv.cli as cli
import fsv.create as create


class Client:
    def int_get(self, path, params=None):
        assert path == "problems/9"
        return {"problem": {"id": 9, "attachments": [{"id": 1, "name": "acme.txt"}], "description": ""}}


@pytest.fixture
def calls(monkeypatch, tmp_path):
    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr(cli, "_client", lambda: Client())
    seen = []

    def fake(att, out_dir, force=False, c=None):
        seen.append(Path(out_dir))
        return {"name": att["name"], "path": str(Path(out_dir) / att["name"]), "size": 4, "skipped": False}
    monkeypatch.setattr(create, "download_attachment", fake)
    return seen


def test_default_download_dir_is_under_downloads_and_announced(calls, tmp_path):
    r = CliRunner().invoke(cli.app, ["problems", "download", "PRB-9", "--json"])
    assert r.exit_code == 0, r.output
    assert calls == [Path("downloads") / "PRB-9"]
    assert json.loads(r.stdout)[0]["path"] == str(Path("downloads") / "PRB-9" / "acme.txt")
    assert "output directory: downloads/PRB-9 (default ./downloads/<ID>; choose with --dir)" in r.stderr
    assert not (tmp_path / "PRB-9").exists()


def test_explicit_dir_is_used_exactly(calls, tmp_path):
    r = CliRunner().invoke(cli.app, ["problems", "download", "PRB-9", "--dir", str(tmp_path / "evidence"), "--json"])
    assert r.exit_code == 0, r.output
    assert calls == [tmp_path / "evidence"]
    assert "(--dir)" in r.stderr


def test_change_download_default_matches():
    assert cli._download_dir(None, "CHN-4") == Path("downloads") / "CHN-4"
    assert cli._download_dir(Path("x"), "CHN-4") == Path("x")
