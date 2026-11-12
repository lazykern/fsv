"""Python command paths the Rust tree does not have yet.

dump_cli_tree.py and dump_completion.py drop these subtrees so the fixtures
hold only ported commands. Add a path when Python gains a command the Rust
tree lacks; remove it when the command lands.
"""

NOT_PORTED: set[str] = {
    # 9910ddc ticket_note_mutation_cli
    "fsv tickets note-edit",
    "fsv tickets note-delete",
    # 17ada1b problem_cli incident associations
    "fsv problems link-incident",
    "fsv problems unlink-incident",
    # 5d7d777 problem_cli status lifecycle
    "fsv problems status",
    # c46d3b0 turns the "fsv tickets tasks" leaf into a group; the leaf's
    # behaviour moves to this hidden path.
    "fsv tickets tasks-legacy",
}
