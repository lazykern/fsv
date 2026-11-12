"""Golden values for `console.print` word wrap (fsv.render.console / err).

Usage (from the Python worktree):
    cd /Users/phusit.somb/lab/work/fsv && uv run python \
        ../fsv-rust/port/golden/gen_fsv_core_term_wrap.py \
        ../fsv-rust/crates/fsv-core/tests/fixtures/term_wrap.json
"""
import io
import json
import os
import sys

os.environ["TERM"] = "xterm-256color"
os.environ.pop("NO_COLOR", None)
os.environ.pop("FORCE_COLOR", None)

from rich.console import Console  # noqa: E402
from rich.markup import escape  # noqa: E402

from provenance import write_sidecar  # noqa: E402

MARKUP = [
    "[red]error[/red]: " + escape("cannot create: Field 'Change Type' is required; Field 'Planned Start Date' is required"),
    "--no-validate: sending without the client-side required check: Field 'Impact' is required; Field 'Group' is required",
    "hello world foo",
    "a\tb\tlonger words here and there\tx",
    "[cyan]x[/cyan]\t[bold]tabbed[/bold]\ty",
    "a" * 76 + " end",
    "word   spaced    words      at    the  boundary  of  the   line width here x",
    "[bold]line one is long enough to wrap around the twenty[/bold]\nsecond [cyan]line[/cyan] too long to fit in",
    "trailing   ",
    "   leading spaces before a long sentence that wraps",
    "中文字符中文字符中文字符 wide chars 中文字符中文字符",
    "",
    "one\n\ntwo\n",
    "[green]status[/]    Pending approval from the change advisory board  (id=3)",
    "path /a/very/long/path/without/any/spaces/that/needs/folding/at/the/width.txt ok",
]
SOFT = [
    "Saved to " + escape("/very/long/path/that/goes/on/and/on/for/ever/and/ever/file[1].pdf"),
    "Saved to a\tb\nnext [bold]line[/bold]",
]

rows = []
for width in (20, 60, 80):
    for soft, cases in ((False, MARKUP), (True, SOFT)):
        for m in cases:
            for stream in ("out", "err"):
                for color in (False, True):
                    buf = io.StringIO()
                    con = Console(
                        file=buf,
                        width=width,
                        force_terminal=color,
                        color_system="standard" if color else None,
                        style="dim" if stream == "err" else None,
                        highlight=False,
                    )
                    con.print(m, soft_wrap=soft) if soft else con.print(m)
                    rows.append(dict(width=width, markup=m, soft=soft, stream=stream, color=color, out=buf.getvalue()))

with open(sys.argv[1], "w", encoding="utf-8") as fh:
    json.dump(rows, fh, ensure_ascii=False, indent=1)
    fh.write("\n")
write_sidecar(sys.argv[1])
