"""Regenerate crates/fsv-core/src/pystr/entities.rs from CPython's html module.

Run: cd /Users/phusit.somb/lab/work/fsv && uv run python /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_html_entities.py
"""
import html.entities as he
from html import _invalid_charrefs, _invalid_codepoints
from pathlib import Path

OUT = Path(__file__).resolve().parents[2] / "crates" / "fsv-core" / "src" / "pystr" / "entities.rs"


def lit(s: str) -> str:
    return '"' + "".join(
        c if (c.isascii() and c.isprintable() and c not in '"\\') else "\\u{%x}" % ord(c) for c in s
    ) + '"'


out = [
    "// Generated from CPython `html.entities.html5` and `html._invalid_charrefs`/`_invalid_codepoints`.",
    "// Regenerate with port/golden/gen_html_entities.py; do not edit by hand.",
    "",
    "pub(crate) static HTML5: &[(&str, &str)] = &[",
]
for k, v in sorted(he.html5.items()):
    out.append(f"    ({lit(k)}, {lit(v)}),")
out.append("];\n")
out.append("pub(crate) static INVALID_CHARREFS: &[(u32, &str)] = &[")
for k, v in sorted(_invalid_charrefs.items()):
    out.append(f"    ({k}, {lit(v)}),")
out.append("];\n")
out.append("pub(crate) static INVALID_CODEPOINTS: &[u32] = &[")
cps = sorted(_invalid_codepoints)
for i in range(0, len(cps), 12):
    out.append("    " + ", ".join(str(c) for c in cps[i:i + 12]) + ",")
out.append("];")
OUT.write_text("\n".join(out) + "\n")
print(f"wrote {OUT}")
