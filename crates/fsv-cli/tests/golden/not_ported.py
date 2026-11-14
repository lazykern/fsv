"""Python command paths the Rust tree does not have yet.

dump_cli_tree.py and dump_completion.py drop these subtrees so the fixtures
hold only ported commands. Add a path when Python gains a command the Rust
tree lacks; remove it when the command lands.
"""

NOT_PORTED: set[str] = set()
