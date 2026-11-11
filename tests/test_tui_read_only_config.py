import pytest

from fsv import config
from fsv.tui import actions


@pytest.mark.parametrize(
    "stored, expected",
    [("on", True), ("true", True), ("1", True), ("off", False), ("false", False), ("0", False), (True, True), (False, False), (None, False)],
)
def test_read_only_config_values(stored, expected):
    cfg = config.load_config()
    if stored is None:
        cfg.pop("tui", None)
    else:
        cfg["tui"] = {"read_only": stored}
    config.save_config(cfg)
    assert actions.resolve_read_only(False) is expected


def test_flag_wins_over_config():
    cfg = config.load_config()
    cfg["tui"] = {"read_only": "off"}
    config.save_config(cfg)
    assert actions.resolve_read_only(True) is True
