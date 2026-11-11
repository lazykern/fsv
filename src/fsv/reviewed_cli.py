"""Output choices for commands whose review/readback contract is JSON-only."""
from enum import Enum


class ReviewOutput(str, Enum):
    json = "json"
