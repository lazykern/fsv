"""Selected downloads handle collisions in Textual without overwriting files."""
import asyncio
from types import SimpleNamespace

import pytest
from textual.widgets import Input

from fsv.resources import CHANGES, TICKETS
from fsv.tui import actions
from fsv.tui.app import FsvApp
from fsv.tui.modals import ChoiceScreen, ConfirmScreen, FailureScreen, ValueScreen
from test_tui_integration import offline  # noqa: F401


@pytest.mark.parametrize("resource", [CHANGES, TICKETS])
@pytest.mark.parametrize("rename", [False, True])
def test_collision_skip_or_rename(offline, monkeypatch, tmp_path, resource, rename):
    original = tmp_path / "evidence.txt"
    original.write_bytes(b"original")
    item = {"id": 9000001001, "display_id": 11, "_resource": resource}
    att = {"id": 9000001002, "name": "evidence.txt", "attachment_url": "https://example.com/file"}
    client = SimpleNamespace(_client=SimpleNamespace(get=lambda *a, **kw: SimpleNamespace(status_code=200, content=b"new")))
    monkeypatch.setattr("fsv.tui.app.get_client", lambda: client)

    async def go():
        app = FsvApp()
        async with app.run_test(size=(120, 40)) as pilot:
            app._prepare_download(item, att, str(tmp_path))
            assert isinstance(app.screen, ChoiceScreen)
            if not rename:
                await pilot.press("enter")
                assert not isinstance(app.screen, ConfirmScreen)
                return
            await pilot.press("down", "enter")
            assert isinstance(app.screen, ValueScreen)
            app.screen.query_one(Input).value = "other.txt"
            await pilot.press("enter")
            assert isinstance(app.screen, ConfirmScreen)
            assert any("other.txt" in line for line in app.screen._lines)
            assert not (tmp_path / "other.txt").exists()
            await pilot.press("y")
            await app.workers.wait_for_complete()
            assert (tmp_path / "other.txt").read_bytes() == b"new"
    asyncio.run(go())
    assert original.read_bytes() == b"original"


def test_unsafe_rename_rejected(offline, tmp_path):
    item = {"id": 9000001001, "_resource": CHANGES}
    att = {"id": 9000001002, "name": "a.txt"}
    async def go():
        app = FsvApp()
        async with app.run_test() as pilot:
            app._prepare_download(item, att, str(tmp_path), "../outside.txt")
            await pilot.pause()
            assert isinstance(app.screen, FailureScreen)
    asyncio.run(go())


def test_download_race_does_not_overwrite(tmp_path):
    path = tmp_path / "a.txt"
    def get(*args, **kwargs):
        path.write_bytes(b"racing writer")
        return SimpleNamespace(status_code=200, content=b"download")
    client = SimpleNamespace(_client=SimpleNamespace(get=get))
    att = {"id": 9000001002, "name": "a.txt", "attachment_url": "https://example.com/a"}
    assert actions.download_selected(client, att, str(tmp_path))["status"] == "exists"
    assert path.read_bytes() == b"racing writer"
