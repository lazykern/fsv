"""Download size guard: --max-bytes (default: the tenant's published
attachment limit) refuses files by listed size, Content-Length or streamed
bytes, writes nothing for them, and streams the rest to disk."""
import json

import httpx
import pytest
from typer.testing import CliRunner

from fsv import cli, config, create

BODY = b"Acme evidence\n" * 8  # 112 bytes


class Fake:
    def __init__(self, page_mib=None, chunked=False):
        self.gets = []
        self.page_mib = page_mib
        self.chunked = chunked
        self._client = httpx.Client(transport=httpx.MockTransport(self.handler))

    def handler(self, req):
        self.gets.append(str(req.url))
        if self.chunked:
            return httpx.Response(200, stream=httpx.ByteStream(BODY), headers={"content-type": "text/plain"})
        return httpx.Response(200, content=BODY)

    def _send(self, method, url, **kw):
        script = "" if self.page_mib is None else f"window.MAX_ATTACHMENT_LIMIT = {self.page_mib};"
        return httpx.Response(200, text=f"<html><script>{script}</script></html>")

    def int_get(self, path, params=None):
        assert path == "tickets/123"
        return {"ticket": {"id": 123, "attachments": [
            {"id": 1, "name": "small.txt", "size": len(BODY), "attachment_url": "https://files.example.com/small"},
            {"id": 2, "name": "big.bin", "size": 3 * 1024 * 1024, "attachment_url": "https://files.example.com/big"}]}}


@pytest.fixture
def out(tmp_path):
    return tmp_path / "out"


def att(**kw):
    return {"name": "acme.txt", "attachment_url": "https://files.example.com/acme", **kw}


def test_listed_size_over_limit_is_refused_without_a_request(out):
    c = Fake()
    r = create.download_attachment(att(size=500), out, c=c, max_bytes=100)
    assert "listed size 500 bytes exceeds the 100-byte limit" in r["refused"]
    assert c.gets == [] and list(out.iterdir()) == []


@pytest.mark.parametrize("chunked", [False, True])
def test_declared_or_streamed_size_over_limit_writes_nothing(out, chunked):
    c = Fake(chunked=chunked)
    r = create.download_attachment(att(), out, c=c, max_bytes=50)
    assert r["refused"] and len(c.gets) == 1
    assert list(out.iterdir()) == []  # no file, no .part left behind


def test_under_limit_streams_to_disk(out):
    c = Fake(chunked=True)
    r = create.download_attachment(att(size=len(BODY)), out, c=c, max_bytes=len(BODY))
    assert r == {"name": "acme.txt", "path": str(out / "acme.txt"), "size": len(BODY), "skipped": False}
    assert (out / "acme.txt").read_bytes() == BODY and [p.name for p in out.iterdir()] == ["acme.txt"]


def test_ticket_download_defaults_to_the_tenant_limit_and_exits_1_on_refusal(tmp_path, monkeypatch):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    c = Fake(page_mib=1)
    monkeypatch.setattr(cli, "_client", lambda: c)
    r = CliRunner().invoke(cli.app, ["tickets", "download", "INC-123", "--dir", str(tmp_path), "--json"])
    assert r.exit_code == 1, r.output
    rows = {row["name"]: row["status"] for row in json.loads(r.stdout)}
    assert rows == {"small.txt": "downloaded", "big.bin": "refused"}
    assert c.gets == ["https://files.example.com/small"] and not (tmp_path / "big.bin").exists()


def test_unknown_tenant_limit_notes_and_downloads_unless_max_bytes(tmp_path, monkeypatch):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    c = Fake(page_mib=None)
    monkeypatch.setattr(cli, "_client", lambda: c)
    r = CliRunner().invoke(cli.app, ["tickets", "download", "INC-123", "--dir", str(tmp_path / "a"), "--json"])
    assert r.exit_code == 0 and "size limit is unknown" in r.stderr
    r = CliRunner().invoke(cli.app, ["tickets", "download", "INC-123", "--dir", str(tmp_path / "b"),
                                     "--json", "--max-bytes", "10"])
    assert r.exit_code == 1 and "size limit is unknown" not in r.stderr
    assert {row["status"] for row in json.loads(r.stdout)} == {"refused"}
