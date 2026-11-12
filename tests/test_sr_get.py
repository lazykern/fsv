"""``fsv sr get``: requested-item values by label, attachments, and safe
downloads (mocked httpx transport; no network)."""
from __future__ import annotations

import json
import os

import pytest
from typer.testing import CliRunner

import acme_catalog as A
import fsv.change_flow as flow
import fsv.cli_sr as cli_sr
from fsv import catalog as cat

runner = CliRunner()


@pytest.fixture
def fake(monkeypatch):
    f = A.AcmeCatalog()
    c = f.client()
    monkeypatch.setattr(cli_sr, "_client", lambda: c)
    monkeypatch.setattr(flow, "interactive", lambda: False)
    monkeypatch.setattr(cli_sr.console, "width", 240)
    return f


def run(*args):
    r = runner.invoke(cli_sr.app, list(args))
    if r.exception and not isinstance(r.exception, SystemExit):
        raise r.exception
    r.flat = " ".join(r.output.split())
    return r


def test_items_by_label_not_internal_name(fake):
    r = run("get", "SR-123", "--items", "-o", "json")
    assert r.exit_code == 0, r.output
    doc = json.loads(r.output)
    assert doc["id"] == "SR-123"
    vals = {v["label"]: v["value"] for v in doc["items"][0]["values"]}
    assert vals == {
        "Title": "Dark mode",
        "Details": "Please add dark mode.",  # label from the catalog item form (not embedded)
        "1. Expected benefit": "More revenue  (Total USD)",
        "1.1 Revenue target": "+5%",
        "Needed by": "2030-12-03",
        "Overview deck (.pptx)": "deck.pptx",  # file field id -> attachment name
    }
    assert "attachments" not in doc
    assert not fake.writes()


def test_table_and_attachment_sources(fake):
    r = run("get", "SR-123")
    assert r.exit_code == 0
    assert "Feature Request" in r.flat and "Title Dark mode" in r.flat and "title_100" not in r.flat
    assert "deck.pptx 11 B id 801 · Overview deck (.pptx)" in r.flat
    assert "brief.pdf" in r.flat
    doc = json.loads(run("get", "123", "--attachments", "-o", "json").output)
    assert {a["name"]: a["source"] for a in doc["attachments"]} == {
        "deck.pptx": "Overview deck (.pptx)", "../escape.sh": "request", "brief.pdf": "request"}


def test_requested_item_general_attachments_and_deduplication():
    general = {"id": 9901, "name": "overview.txt", "size": 11, "attachment_url": "https://example.com/file"}
    field = {"id": 9902, "name": "field.txt", "size": 12}
    rows = cli_sr._attachment_rows(
        {"attachments": []},
        [{"attachments": [general, field], "custom_fields": {"brief": [field]}}],
        [{"brief": "Brief"}],
    )
    assert [(r["id"], r["source"]) for r in rows] == [(9901, "requested item"), (9902, "Brief")]
    assert rows[0]["attachment_url"] == "https://example.com/file"


def test_download_is_safe_and_reports_rejections(fake, tmp_path):
    out = tmp_path / "ref"
    r = run("get", "SR-123", "--download", str(out), "-o", "json")
    assert r.exit_code == 2  # one name was rejected
    doc = json.loads(r.output)
    st = {d["name"]: d["status"] for d in doc["downloads"]}
    assert st == {"deck.pptx": "downloaded", "../escape.sh": "rejected", "brief.pdf": "downloaded"}
    assert (out / "deck.pptx").read_bytes() == A.DOWNLOADS[801]
    assert (out / "brief.pdf").read_bytes() == A.DOWNLOADS[803]
    assert sorted(os.listdir(out)) == ["brief.pdf", "deck.pptx"]
    assert not (tmp_path / "escape.sh").exists()
    # existing files are kept unless --force
    (out / "deck.pptx").write_bytes(b"mine")
    doc = json.loads(run("get", "SR-123", "--download", str(out), "-o", "json").output)
    assert {d["name"]: d["status"] for d in doc["downloads"]}["deck.pptx"] == "exists"
    assert (out / "deck.pptx").read_bytes() == b"mine"
    run("get", "SR-123", "--download", str(out), "--force")
    assert (out / "deck.pptx").read_bytes() == A.DOWNLOADS[801]


@pytest.mark.parametrize("args", [
    ("--download", "--help"),
    ("--download", "--force"),
    ("--download=--help",),
    ("--download",),
])
def test_download_rejects_flags_and_missing_value_before_client(args, tmp_path, monkeypatch):
    work = tmp_path / "work"
    work.mkdir()
    monkeypatch.chdir(work)

    def no_client():
        raise AssertionError("client must not be called")

    monkeypatch.setattr(cli_sr, "_client", no_client)
    r = run("get", "SR-123", *args)
    assert r.exit_code == 2, r.output
    assert "--download" in r.output
    assert list(work.iterdir()) == []


def test_download_option_like_directory_with_dot_slash(fake, tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    r = run("get", "SR-123", "--download", "./--name", "-o", "json")
    assert r.exit_code == 2  # one attachment name is rejected
    assert (tmp_path / "--name" / "deck.pptx").read_bytes() == A.DOWNLOADS[801]


def test_get_help_remains_available_without_client(monkeypatch):
    monkeypatch.setattr(cli_sr, "_client", lambda: pytest.fail("client must not be called"))
    r = run("get", "--help")
    assert r.exit_code == 0
    assert "--download" in r.output


def test_bad_id_prefix(fake):
    r = run("get", "CHN-5")
    assert r.exit_code == 2


@pytest.mark.parametrize("name", ["..", "../x", "a/../b", "a/b", "a\\b", "/etc/passwd", "C:evil", "x\x00y",
                                  "tab\tname", "", "   ", "report..pdf"])
def test_safe_target_rejects(tmp_path, name):
    with pytest.raises(ValueError):
        cat.safe_target(tmp_path, name)


def test_safe_target_accepts_and_stays_inside(tmp_path):
    p = cat.safe_target(tmp_path, "Q3 plan (final).pptx")
    assert str(p) == str((tmp_path / "Q3 plan (final).pptx").resolve())


def test_safe_target_refuses_symlink_escape(tmp_path):
    outside = tmp_path / "outside.txt"
    outside.write_text("x")
    d = tmp_path / "dl"
    d.mkdir()
    (d / "link.txt").symlink_to(outside)
    with pytest.raises(ValueError, match="outside"):
        cat.safe_target(d, "link.txt")


def test_created_ref_shapes():
    assert cat.created_ref({"service_request": {"id": 9, "display_id": 3}}) == {"id": 9, "display": "SR-3"}
    assert cat.created_ref({"id": 9, "human_display_id": "SR-3"}) == {"id": 9, "display": "SR-3"}
    assert cat.created_ref("ok")["id"] is None


def test_mounted_under_fsv_sr(fake):
    import fsv.cli as cli

    r = runner.invoke(cli.app, ["sr", "--help"])
    assert r.exit_code == 0
    for cmd in ("catalog", "fields", "create", "get"):
        assert cmd in r.output
