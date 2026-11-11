"""Python command paths the Rust tree does not have yet.

dump_cli_tree.py and dump_completion.py drop these subtrees so the fixtures
hold only ported commands. Remove a path when its command lands.
"""

NOT_PORTED = {
    "fsv changes asset-picker",
    "fsv changes columns",
    "fsv changes export",
    "fsv problems columns",
    "fsv problems create",
    "fsv problems export",
    "fsv problems write-child",
    "fsv tickets canned",
    "fsv tickets columns",
    "fsv tickets export",
    "fsv tickets private-views",
    "fsv tickets scenarios",
}
