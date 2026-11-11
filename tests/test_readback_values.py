"""Conservative scalar normalization never treats absent values as accepted."""
import pytest

from fsv.readback import submitted_value_matches


@pytest.mark.parametrize("actual,expected,matched", [
    ({"a": None, "extra": 2}, {"a": None}, True),
    ({"extra": 2}, {"a": None}, False),
    (None, "", False), ("", None, False),
    ("7.0", 7, True), ("wrong", 7, False), ("NaN", 7, False),
    (7, "7", True), (7, "007", False),
    (True, 1, False), (1, True, False), ("false", False, True),
    ("true", True, True), ("1", True, False),
    (["7", "8"], [7, 8], True), ([8, 7], [7, 8], False),
    ([7], [7, 8], False),
])
def test_submitted_values(actual, expected, matched):
    assert submitted_value_matches(actual, expected) is matched
