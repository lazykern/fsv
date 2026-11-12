"""Runtime choice identities accept integer and digit-string metadata only."""
import pytest

from fsv.choice_ids import runtime_choice_id


@pytest.mark.parametrize("value,minimum,expected", [
    (1, 1, 1), ("1", 1, 1), ("0", 0, 0), (0, 0, 0), ("0", 1, None), (0, 1, None),
    ("900", 0, 900), (True, 0, None), (False, 0, None), ("-1", 0, None), (-1, 0, None),
    (" 1", 0, None), ("1.0", 0, None), (1.0, 0, None), ("١", 0, None), (None, 0, None), ("", 0, None)])
def test_runtime_choice_id(value, minimum, expected):
    assert runtime_choice_id(value, minimum=minimum) == expected
