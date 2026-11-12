#!/usr/bin/env python3
"""Def-by-def coverage audit: Python fsv -> Rust port.

Usage: python3 /tmp/fsv_coverage.py [--no-cargo] [--out PATH] [--json PATH]

Rules
  * Python side: every top-level def and every method of a top-level class
    in fsv/src/fsv/**/*.py (nested defs, e.g. commands built inside
    `_make_subapp`, are not listed).
  * Rust side: only files reachable through `mod` declarations from a crate
    root (src/lib.rs, src/main.rs, tests/*.rs) of a crate that compiles
    under `cargo test --workspace --offline --no-run`.
  * covered  = a matching Rust fn exists and test code references it
               (direct call/path in a #[test] fn, a test-module helper or an
               integration test file; CLI command fns also count when a CLI
               integration test runs that command path via argv).
    fn-only  = matching Rust fn, no test reference found.
    missing  = no matching Rust fn in compiled code.
    n/a      = framework shim with no Rust counterpart expected.
"""
from __future__ import annotations

import argparse
import ast
import json
import os
import re
import subprocess
import sys
from collections import defaultdict
from dataclasses import dataclass, field
from pathlib import Path

PY_ROOT = Path("/Users/phusit.somb/lab/work/fsv/src/fsv")
RS_ROOT = Path("/Users/phusit.somb/lab/work/fsv-rust")
CRATES = RS_ROOT / "crates"

# ---------------------------------------------------------------- python side


@dataclass
class PyDef:
    module: str
    qual: str
    name: str
    cls: str | None
    line: int
    bases: list[str] = field(default_factory=list)
    cmd_path: str | None = None  # Typer command path, e.g. "config defaults show"


def py_module_name(p: Path) -> str:
    rel = p.relative_to(PY_ROOT.parent).with_suffix("")
    parts = list(rel.parts)
    if parts[-1] == "__init__":
        parts = parts[:-1] + ["__init__"]
    return ".".join(parts)


def _dotted(node) -> str:
    if isinstance(node, ast.Name):
        return node.id
    if isinstance(node, ast.Attribute):
        return _dotted(node.value) + "." + node.attr
    if isinstance(node, ast.Call):
        return _dotted(node.func)
    if isinstance(node, ast.Subscript):
        return _dotted(node.value)
    return ""


# Typer app variables -> command path prefix.
TYPER_ROOTS = {("fsv.cli", "app"): "", ("fsv.cli_sr", "app"): "sr", ("fsv.form_cli", "form_app"): "form"}


def typer_groups(module: str, tree: ast.Module) -> dict[str, str]:
    groups = {var: path for (mod, var), path in TYPER_ROOTS.items() if mod == module}
    adds = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute) and node.func.attr == "add_typer":
            parent = _dotted(node.func.value)
            child = _dotted(node.args[0]) if node.args else ""
            name = next((k.value.value for k in node.keywords if k.arg == "name" and isinstance(k.value, ast.Constant)), None)
            if name:
                adds.append((parent, child, name))
    for _ in range(4):
        for parent, child, name in adds:
            if parent in groups and child not in groups:
                groups[child] = (groups[parent] + " " + name).strip()
    return groups


def collect_py() -> list[PyDef]:
    out: list[PyDef] = []
    for p in sorted(PY_ROOT.rglob("*.py")):
        module = py_module_name(p)
        tree = ast.parse(p.read_text(), str(p))
        groups = typer_groups(module, tree)

        def cmd_path(fn) -> str | None:
            for d in fn.decorator_list:
                if isinstance(d, ast.Call) and isinstance(d.func, ast.Attribute) and d.func.attr == "command":
                    var = _dotted(d.func.value)
                    if var not in groups:
                        continue
                    if d.args and isinstance(d.args[0], ast.Constant):
                        leaf = d.args[0].value
                    else:
                        leaf = fn.name.replace("_", "-")
                    return (groups[var] + " " + leaf).strip()
            return None

        for node in tree.body:
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                out.append(PyDef(module, node.name, node.name, None, node.lineno, cmd_path=cmd_path(node)))
            elif isinstance(node, ast.ClassDef):
                bases = [_dotted(b).split(".")[-1] for b in node.bases]
                for sub in node.body:
                    if isinstance(sub, (ast.FunctionDef, ast.AsyncFunctionDef)):
                        out.append(PyDef(module, f"{node.name}.{sub.name}", sub.name, node.name, sub.lineno, bases))
    return out


# ------------------------------------------------------------------ rust side


def clean_rust(src: str) -> tuple[str, dict[int, str]]:
    """Blank out comments and string/char literals (newlines kept).
    Returns cleaned text and {line_no: doc_comment_text}."""
    out = []
    docs: dict[int, str] = {}
    i, n = 0, len(src)
    line = 1

    def is_ident(c):
        return c.isalnum() or c == "_"

    while i < n:
        c = src[i]
        if c == "\n":
            line += 1
            out.append(c)
            i += 1
        elif src.startswith("//", i):
            j = src.find("\n", i)
            j = n if j < 0 else j
            text = src[i:j]
            if text.startswith("///") or text.startswith("//!"):
                docs[line] = text[3:].strip()
            out.append(" ")
            i = j
        elif src.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if src.startswith("/*", j):
                    depth += 1
                    j += 2
                elif src.startswith("*/", j):
                    depth -= 1
                    j += 2
                else:
                    j += 1
            chunk = src[i:j]
            line += chunk.count("\n")
            out.append("\n" * chunk.count("\n") or " ")
            i = j
        elif c == "r" and (i == 0 or not is_ident(src[i - 1]) or src[i - 1] == "b") and re.match(r'r#*"', src[i:i + 300]):
            m = re.match(r'r(#*)"', src[i:])
            hashes = m.group(1)
            end = src.find('"' + hashes, i + len(m.group(0)))
            end = n if end < 0 else end + 1 + len(hashes)
            chunk = src[i:end]
            line += chunk.count("\n")
            out.append('""' + "\n" * chunk.count("\n"))
            i = end
        elif c == '"':
            j = i + 1
            while j < n and src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            chunk = src[i:j + 1]
            line += chunk.count("\n")
            out.append('""' + "\n" * chunk.count("\n"))
            i = j + 1
        elif c == "'":
            m = re.match(r"'(\\(x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]+\}|.)|[^\\'\n])'", src[i:i + 16])
            if m:
                out.append("' '")
                i += len(m.group(0))
            else:
                out.append(c)
                i += 1
        else:
            out.append(c)
            i += 1
    return "".join(out), docs


@dataclass
class RsFn:
    crate: str
    file: Path
    line: int
    name: str
    impl: str | None
    in_test: bool
    is_test: bool
    body: str
    doc: str


@dataclass
class RsFile:
    crate: str
    path: Path
    in_test: bool
    fns: list[RsFn] = field(default_factory=list)
    types: set[str] = field(default_factory=set)
    display_for: set[str] = field(default_factory=set)
    raw: str = ""


FN_RE = re.compile(r"(?:^|[\s>;}])fn\s+([A-Za-z_]\w*)")
MOD_RE = re.compile(r"(?:^|\s)mod\s+([A-Za-z_]\w*)\s*$")
QUAL_RE = re.compile(r'\s*((pub(\([^)]*\))?|const|async|unsafe|default|extern(\s*"")?)\s+)*')
TYPE_RE = re.compile(r"\b(?:struct|enum|trait|type|union)\s+([A-Z]\w*)")


def impl_type(seg: str) -> str | None:
    m = re.search(r"\bimpl\b(.*)$", seg, re.S)
    if not m:
        return None
    rest = m.group(1).strip()
    if rest.startswith("<"):
        depth = 0
        for k, ch in enumerate(rest):
            depth += ch == "<"
            depth -= ch == ">"
            if depth == 0:
                rest = rest[k + 1:]
                break
    rest = re.split(r"\bwhere\b", rest)[0]
    parts = re.split(r"\bfor\b", rest)
    ty = parts[-1].strip()
    ty = re.sub(r"<.*", "", ty, flags=re.S).strip().lstrip("&").replace("dyn ", "").strip()
    return ty.split("::")[-1] or None


def parse_rust_file(crate: str, path: Path, in_test: bool) -> tuple[RsFile, list[tuple[list[str], str, bool]]]:
    raw = path.read_text()
    text, docs = clean_rust(raw)
    rf = RsFile(crate, path, in_test, raw=raw)
    raw_lines = raw.split("\n")
    rf.types = set(TYPE_RE.findall(text))
    for m in re.finditer(r"\bimpl\b[^{;]*?\b(?:fmt::)?Display\s+for\s+([A-Z]\w*)", text):
        rf.display_for.add(m.group(1))
    mods: list[tuple[list[str], str, bool, str | None]] = []  # (inline mod path, name, cfg_test, #[path])
    # clean_rust blanks string literals, so `#[path = "…"]` values come from the raw text.
    raw_paths = {m.group(2): m.group(1) for m in re.finditer(
        r'#\[path\s*=\s*"([^"]+)"\](?:\s*#\[[^\]]*\])*\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;', raw)}
    # stack entries: (kind, name, test, impl, fn_index, start_offset)
    stack: list[dict] = []
    seg_start = 0
    line_starts = [0]
    for k, ch in enumerate(text):
        if ch == "\n":
            line_starts.append(k + 1)

    def line_of(off):
        lo, hi = 0, len(line_starts) - 1
        while lo < hi:
            mid = (lo + hi + 1) // 2
            if line_starts[mid] <= off:
                lo = mid
            else:
                hi = mid - 1
        return lo + 1

    def ctx_test():
        return in_test or any(e["test"] for e in stack)

    def ctx_impl():
        for e in reversed(stack):
            if e["kind"] == "impl":
                return e["impl"]
            if e["kind"] in ("mod", "fn"):
                return None
        return None

    depth = 0
    for k, ch in enumerate(text):
        if ch in "([":
            depth += 1
            continue
        if ch in ")]":
            depth = max(depth - 1, 0)
            continue
        if ch not in "{};" or (ch == ";" and depth):
            continue
        seg = text[seg_start:k]
        if ch == "{":
            entry = {"kind": "block", "test": False, "impl": None, "fn": None}
            fm = None
            for fm in FN_RE.finditer(seg):
                pass
            mm = MOD_RE.search(seg)
            head = re.sub(r"#!?\[[^\]]*\]", "", seg).strip()
            hm = None
            for hm in re.finditer(r"\bfn\s+[A-Za-z_]\w*", head):
                pass
            fn_ok = bool(fm and hm and QUAL_RE.fullmatch(head[: hm.start()]))
            impl_ok = bool(re.match(r"(unsafe\s+)?impl\b", head))
            mod_ok = bool(re.fullmatch(r"(pub(\([^)]*\))?\s+)?mod\s+\w+", head))
            trait_ok = re.match(r"(pub(\([^)]*\))?\s+)?(unsafe\s+)?trait\s+(\w+)", head)
            if fn_ok:
                is_test = bool(re.search(r"#\[(?:\w+::)*test\b", seg))
                test = ctx_test() or is_test
                ln = line_of(seg_start + fm.start(1))
                doc_lines = []
                d = ln - 1
                while d > 0:
                    src_line = raw_lines[d - 1].strip() if d - 1 < len(raw_lines) else ""
                    if d in docs:
                        doc_lines.insert(0, docs[d])
                    elif not src_line.startswith("#["):
                        break
                    d -= 1
                fn = RsFn(crate, path, ln, fm.group(1), ctx_impl(), test, is_test, "", " ".join(doc_lines))
                rf.fns.append(fn)
                entry.update(kind="fn", test=is_test, fn=fn, start=k)
            elif mod_ok and mm:
                entry.update(kind="mod", name=mm.group(1), test=bool(re.search(r"#\[cfg\(test\)\]", seg)))
            elif impl_ok:
                it = impl_type(head)
                if it:
                    entry.update(kind="impl", impl=it)
            elif trait_ok:
                entry.update(kind="impl", impl=trait_ok.group(4))
            stack.append(entry)
        elif ch == "}":
            if stack:
                e = stack.pop()
                if e["kind"] == "fn":
                    e["fn"].body = text[e["start"]:k + 1]
        else:  # ';'
            mm = MOD_RE.search(seg)
            if mm:
                path_names = [e["name"] for e in stack if e["kind"] == "mod"]
                cfg_test = bool(re.search(r"#\[cfg\(test\)\]", seg)) or any(e["test"] for e in stack)
                path_attr = raw_paths.get(mm.group(1)) if "#[path" in seg and not path_names else None
                mods.append((path_names, mm.group(1), cfg_test, path_attr))
        seg_start = k + 1
    return rf, mods


# A `#[path]` module file -> the file that declares it (same Rust module scope).
PATH_PARENT: dict[Path, Path] = {}


def collect_rust(compiled: set[str] | None):
    files: dict[Path, RsFile] = {}
    all_rs = {p for p in CRATES.rglob("*.rs") if "/target/" not in str(p)}
    for crate_dir in sorted(CRATES.iterdir()):
        if not (crate_dir / "Cargo.toml").exists():
            continue
        crate = crate_dir.name
        if compiled is not None and crate not in compiled:
            continue
        roots = []
        for r in ("src/lib.rs", "src/main.rs"):
            if (crate_dir / r).exists():
                roots.append((crate_dir / r, False))
        roots += [(p, False) for p in sorted((crate_dir / "src/bin").glob("*.rs"))] if (crate_dir / "src/bin").exists() else []
        roots += [(p, True) for p in sorted((crate_dir / "tests").glob("*.rs")) if p not in BAD_ROOTS] if (crate_dir / "tests").exists() else []
        queue = list(roots)
        while queue:
            p, test = queue.pop()
            if p in files:
                continue
            rf, mods = parse_rust_file(crate, p, test)
            files[p] = rf
            base = p.parent if p.name in ("lib.rs", "main.rs", "mod.rs") or (p.parent.name in ("tests", "bin") and p.parent.parent == crate_dir) else p.parent / p.stem
            for inline, name, cfg_test, path_attr in mods:
                d = base.joinpath(*inline)
                # `#[path]` on an out-of-line mod is relative to the declaring file's directory.
                cands = [p.parent / path_attr] if path_attr else [d / f"{name}.rs", d / name / "mod.rs"]
                for cand in cands:
                    if cand.exists():
                        queue.append((cand, test or cfg_test))
                        if path_attr:
                            PATH_PARENT[cand] = p
                        break
    counted = {c for c in (compiled or {d.name for d in CRATES.iterdir()})}
    orphans = sorted(p for p in all_rs - set(files) if p.relative_to(CRATES).parts[0] in counted and p not in BAD_ROOTS
                     and not any(str(p).startswith(str(b.with_suffix("")) + "/") for b in BAD_ROOTS))
    return files, orphans


BAD_ROOTS: set[Path] = set()


def compiled_crates() -> tuple[set[str], str]:
    """Same targets as `cargo test --workspace --no-run`, but `--keep-going`
    so one broken target does not hide the others. A crate counts when its
    lib (or bin) target built; an integration test file counts when its own
    target built."""
    cmd = ["cargo", "build", "--workspace", "--tests", "--offline", "--keep-going", "--message-format=json"]
    proc = subprocess.run(cmd, cwd=RS_ROOT, capture_output=True, text=True)
    built: set[tuple[str, str]] = set()
    failed: set[tuple[str, str]] = set()
    for ln in proc.stdout.splitlines():
        try:
            msg = json.loads(ln)
        except ValueError:
            continue
        reason = msg.get("reason")
        if reason not in ("compiler-artifact", "compiler-message"):
            continue
        src = msg.get("target", {}).get("src_path", "")
        m = re.search(r"/crates/(fsv-[\w-]+)/", src)
        if not m:
            continue
        key = (m.group(1), src)
        if reason == "compiler-artifact" and msg.get("profile", {}).get("test"):
            built.add(key)
        elif reason == "compiler-message" and msg["message"].get("level") == "error":
            failed.add(key)
    crates = set()
    for crate, src in built - failed:
        if src.endswith(("/src/lib.rs", "/src/main.rs")):
            crates.add(crate)
    for crate, src in failed:
        if "/tests/" in src:
            BAD_ROOTS.add(Path(src))
    status = f"`{' '.join(cmd[:-1])}` exit {proc.returncode}; crates whose lib/bin test target built: {', '.join(sorted(crates))}"
    bad = sorted({f"{c} ({Path(s).relative_to(CRATES / c)})" for c, s in failed})
    if bad:
        status += f"; targets with compile errors: {', '.join(bad)}"
    return crates, status


# -------------------------------------------------------------- matching

# Python module -> (preferred crates, preferred Rust file stems)
def prefs(module: str) -> tuple[list[str], list[str]]:
    parts = module.split(".")[1:]
    stem = parts[-1]
    if parts[0] == "forms":
        return ["fsv-forms"], [stem if stem != "__init__" else "lib"]
    if parts[0] == "formui":
        extra = {"screens": ["screen", "dialogs", "review", "mod"], "widgets": ["widgets", "combo", "date", "calendar", "text", "mod"],
                 "app": ["app", "kit"], "__init__": ["lib"]}
        return ["fsv-formui", "fsv-forms"], extra.get(stem, [stem])
    if parts[0] == "tui":
        extra = {"app": ["app", "view", "runtime", "site", "keys", "input", "filter", "format", "markup", "backend"], "__init__": ["lib"]}
        return ["fsv-tui"], extra.get(stem, [stem])
    table = {
        "__init__": (["fsv-cli", "fsv-app"], ["lib", "main"]),
        "cli": (["fsv-cli", "fsv-app", "fsv-core"], ["app", "cmd", "util", "pending", "types", "mod", "misc", "resource", "auth", "config", "cache", "journal", "completion"]),
        "cli_sr": (["fsv-cli", "fsv-app", "fsv-flows", "fsv-api"], ["sr"]),
        "change_cli": (["fsv-cli", "fsv-app", "fsv-flows"], ["changes"]),
        "ticket_cli": (["fsv-cli", "fsv-app", "fsv-flows"], ["tickets"]),
        "form_cli": (["fsv-cli", "fsv-app"], ["form"]),
        "completion": (["fsv-completion", "fsv-cli"], ["dynamic", "emit", "model", "quote", "completion"]),
        "completion_gen": (["fsv-completion", "fsv-cli"], ["emit", "model", "generated"]),
        "_complete": (["fsv-completion", "fsv-cli"], ["dynamic", "quote"]),
        "client": (["fsv-core"], ["client", "mod", "transport", "cookies", "html", "log"]),
        "errors": (["fsv-core"], ["error"]),
        "yamlio": (["fsv-core"], ["yaml"]),
        "resources": (["fsv-core"], ["resource"]),
        "session": (["fsv-core"], ["session", "mod", "browser", "paste"]),
        "change_approvals": (["fsv-api"], ["approvals"]),
        "change_tasks": (["fsv-api"], ["tasks"]),
        "change_associations": (["fsv-api"], ["associations"]),
        "ticket_notes": (["fsv-api"], ["notes"]),
        "ticket_conversation": (["fsv-api"], ["conversations"]),
        "ticket_properties": (["fsv-api"], ["properties"]),
        "change_flow": (["fsv-flows"], ["change", "meta"]),
        "ticket_flow": (["fsv-flows"], ["ticket"]),
        "change_approval_actions": (["fsv-flows", "fsv-api"], ["change", "approvals"]),
        "drafts": (["fsv-flows"], ["drafts"]),
        "views": (["fsv-views", "fsv-api"], ["detail", "source", "coverage", "render", "lib", "site", "record", "format"]),
        "render": (["fsv-views", "fsv-api"], ["render", "flow", "coverage", "format"]),
        "pages": (["fsv-views", "fsv-api"], ["pages", "list"]),
    }
    if stem in table:
        return table[stem]
    for core in ("config", "cache", "editor", "journal", "mdlite", "review", "reviewed_snapshot", "safety"):
        if stem == core:
            return ["fsv-core"], [stem]
    if stem.endswith("_cli"):
        return ["fsv-cli"], ["cmd", stem.removesuffix("_cli"), "mod"]
    for api in ("service", "query", "schema", "catalog", "formmeta", "task_status", "state_flow", "create",
                "bulk_priority", "planning_text", "problem_known_error", "reviewed_write", "ticket_time", "watchers",
                "ticket_lifecycle", "ticket_merge", "ticket_assignment", "ticket_bulk", "ticket_note_mutation", "problem_incidents",
                "ticket_tasks", "problem_status", "attachments", "integrated_assets", "sr_approvals", "sr_recipient_readback", "sr_requested_items", "change_task_update", "change_task_delete", "change_relationships", "solutions", "approval_recipients", "agent_lookup"):
        if stem == api:
            return ["fsv-api"], [stem]
    return [], [stem]


def snake(name: str) -> str:
    s = name.strip("_")
    s = re.sub(r"(?<=[a-z0-9])([A-Z])", r"_\1", s)
    return s.lower()


STOP = set("""new get set list len push insert remove clear run render parse from into default fmt eq show status path draw
update apply value key keys name text send close open load save write read iter next filter map find contains is_empty start stop
ls tail refresh install doctor login logout help search setup tui main dispatch handle submit cancel reset items rows label kind
values on_key compose refresh_view""".split())

TEXTUAL_BASES = {"App", "Screen", "ModalScreen", "Widget", "Static", "Input", "ListView", "ListItem", "DataTable", "Message",
                 "Vertical", "Horizontal", "Container", "VerticalScroll", "OptionList", "TextArea", "Button", "Label", "Tree",
                 "Markdown", "RichLog", "Log", "Footer", "Header", "Select", "Checkbox", "Switch", "TabbedContent", "Provider",
                 "SuggestFromList", "Suggester", "ScrollableContainer", "Grid", "LoadingIndicator", "ProgressBar", "Tabs"}
TEXTUAL_HOOKS = re.compile(r"^(compose|on_mount|on_unmount|on_resize|on_show|on_hide|render|render_line|get_content_width|get_content_height|"
                           r"watch_\w+|validate_\w+|compute_\w+|__rich\w*__|startup_hook)$")
DUNDER = re.compile(r"^__\w+__$")


def first_tick(doc: str) -> str:
    m = re.match(r"\s*`([^`]+)`", doc or "")
    return m.group(1).strip() if m else ""


def ref_regex(n: str) -> re.Pattern:
    n = re.escape(n)
    return re.compile(rf"(?:(?<![\w])|(?<=\.)|(?<=::)){n}\s*(?:::<[^()]*>)?\s*\(|(?:\.|::){n}\b|[(,]\s*(?:\w+::)*{n}\s*[),]")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--no-cargo", action="store_true", help="skip cargo; treat every crate as compiling")
    ap.add_argument("--out", default=str(RS_ROOT / "port/COVERAGE.md"))
    ap.add_argument("--json", default="/tmp/fsv_coverage.json")
    args = ap.parse_args()

    if args.no_cargo:
        compiled, cargo_status = None, "cargo not run (--no-cargo)"
    else:
        compiled, cargo_status = compiled_crates()
    files, orphans = collect_rust(compiled)
    all_crates = sorted(d.name for d in CRATES.iterdir() if (d / "Cargo.toml").exists())
    not_compiled = [c for c in all_crates if compiled is not None and c not in compiled]

    fns = [f for rf in files.values() for f in rf.fns]
    prod = [f for f in fns if not f.in_test]
    by_name: dict[str, list[RsFn]] = defaultdict(list)
    for f in prod:
        by_name[f.name].append(f)
    types = defaultdict(set)
    display = set()
    for rf in files.values():
        for t in rf.types:
            types[t].add(rf.crate)
        display |= rf.display_for

    # orphan fn names (for notes only)
    orphan_names = defaultdict(list)
    for p in orphans:
        try:
            rf, _ = parse_rust_file(p.parts[len(CRATES.parts)], p, False)
            for f in rf.fns:
                orphan_names[f.name].append(f)
        except Exception:
            pass

    # test code segments
    segs: list[tuple[str, Path, str]] = []
    for rf in files.values():
        if rf.in_test:
            segs.append((rf.crate, rf.path, clean_rust(rf.raw)[0]))
        else:
            for f in rf.fns:
                if f.in_test and f.body:
                    segs.append((f.crate, f.file, f.body))

    # CLI dispatch arms and argv evidence
    # "path" -> (file, target fn or None for an inline block, line, inline block text)
    arms: dict[str, list[tuple[Path, str | None, int, str]]] = defaultdict(list)
    arm_re = re.compile(r'((?:"[^"\n]+"\s*\|\s*)*"[^"\n]+")\s*(?:if\s[^\n]*?)?=>\s*')
    for rf in files.values():
        if rf.crate != "fsv-cli" or rf.in_test:
            continue
        for m in arm_re.finditer(rf.raw):
            rest = rf.raw[m.end():]
            ln = rf.raw.count("\n", 0, m.start()) + 1
            if rest.startswith("{"):
                depth, j = 0, 0
                for j, ch in enumerate(rest):
                    depth += ch == "{"
                    depth -= ch == "}"
                    if depth == 0:
                        break
                target, block = None, rest[: j + 1]
            else:
                cm = re.match(r"((?:\w+::)*\w+)\s*\(", rest)
                if not cm:
                    continue
                target, block = cm.group(1), ""
            for s in re.findall(r'"([^"]+)"', m.group(1)):
                arms[s].append((rf.path, target, ln, block))
    argvs: list[list[str]] = []
    for rf in files.values():
        if rf.crate == "fsv-cli" and rf.in_test:
            for m in re.finditer(r'&\[\s*((?:"(?:[^"\\]|\\.)*"\s*,?\s*)+)\]', rf.raw):
                argvs.append(re.findall(r'"((?:[^"\\]|\\.)*)"', m.group(1)))

    argv_tokens = {t for a in argvs for t in a if t and not t.startswith("-")}
    argv_lines = [" " + " ".join(a) + " " for a in argvs]

    def arm_hit(s: str) -> bool:
        if " " not in s:
            return s in argv_tokens
        return any(f" {s} " in line for line in argv_lines)

    golden_roots: set[str] = set()
    for s, lst in arms.items():
        if not arm_hit(s):
            continue
        for _path, target, _ln, block in lst:
            if target:
                golden_roots.add(target.split("::")[-1])
            else:
                golden_roots.update(re.findall(r"\b([a-z_]\w*)\s*\(", block))

    golden_roots -= {"dispatch", "run", "main", "Ok", "Err", "Some"}

    def argv_hits(path: str) -> bool:
        toks = path.split()
        return any(a[: len(toks)] == toks for a in argvs)

    def common(n: str) -> bool:
        return n in STOP or len(by_name.get(n, [])) >= 3

    def stem_of(p: Path) -> str:
        return p.parent.name if p.name == "mod.rs" else p.stem

    deps: dict[str, set[str]] = {}
    for cdir in CRATES.iterdir():
        toml = cdir / "Cargo.toml"
        if toml.exists():
            deps[cdir.name] = set(re.findall(r"^(fsv-[\w-]+)(?:\.workspace)?\s*=", toml.read_text(), re.M))

    def can_call(user: str, owner: str, seen=None) -> bool:
        if user == owner:
            return True
        seen = seen or set()
        for d in deps.get(user, ()):
            if d not in seen:
                seen.add(d)
                if can_call(d, owner, seen):
                    return True
        return False

    def tested(f: RsFn) -> str | None:
        rx = ref_regex(f.name)
        modkey = f.file.parent if f.file.name in ("mod.rs", "lib.rs", "main.rs") else f.file.parent / f.file.stem
        for crate, path, text in segs:
            if not can_call(crate, f.crate):
                continue
            if not rx.search(text):
                continue
            if common(f.name):
                if crate != f.crate:
                    continue
                same = path == f.file or str(path).startswith(str(modkey) + "/") or PATH_PARENT.get(path) == f.file
                mentions = (f.impl and re.search(rf"\b{re.escape(f.impl)}\b", text)) or re.search(rf"\b{re.escape(stem_of(f.file))}::{re.escape(f.name)}\b", text)
                if not (same or mentions):
                    continue
            return str(path.relative_to(CRATES))
        return None

    # transitive reachability (names, same crate, non-common edges)
    tested_cache: dict[int, str | None] = {}

    def is_tested(f):
        k = id(f)
        if k not in tested_cache:
            tested_cache[k] = tested(f)
        return tested_cache[k]

    callers: dict[int, list[RsFn]] = defaultdict(list)

    def reached_via(f: RsFn) -> str | None:
        rx = ref_regex(f.name)
        seen, frontier = {id(f)}, [f]
        for _ in range(4):
            nxt = []
            for g in frontier:
                for h in prod:
                    if id(h) in seen or h.crate != g.crate or not h.body or h is g:
                        continue
                    if common(g.name) and h.file != g.file:
                        continue
                    if ref_regex(g.name).search(h.body[h.body.find("{"):] if "{" in h.body else h.body):
                        if is_tested(h):
                            return f"{h.name} ({h.file.relative_to(CRATES)}:{h.line})"
                        seen.add(id(h))
                        nxt.append(h)
            frontier = nxt[:40]
        return None

    def reached_golden(f: RsFn) -> str | None:
        """fsv-cli fn reachable (name call graph, <=6 hops) from a dispatch target a golden argv runs."""
        if f.name in golden_roots:
            return f"dispatch target {f.name}"
        seen, frontier = {id(f)}, [f]
        for _ in range(6):
            nxt = []
            for g in frontier:
                rx = ref_regex(g.name)
                for h in prod:
                    if id(h) in seen or h.crate != "fsv-cli" or not h.body:
                        continue
                    if common(g.name) and h.file != g.file:
                        continue
                    if rx.search(h.body[h.body.find("{"):] if "{" in h.body else h.body):
                        if h.name in golden_roots:
                            return f"{h.name} ({h.file.relative_to(CRATES)}:{h.line})"
                        seen.add(id(h))
                        nxt.append(h)
            frontier = nxt[:60]
        return None

    port_idx: dict[str, list[RsFn]] = defaultdict(list)
    for f in prod:
        for line in (f.doc or "").splitlines():
            if "Port of" not in line and "port of" not in line:
                continue
            seg = line[line.lower().index("port of") + 7:]
            for tok in re.findall(r"[A-Za-z_][\w]*(?:\.[A-Za-z_][\w]*)+", seg):
                port_idx[tok.removeprefix("fsv.")].append(f)
    na_file = RS_ROOT / "port/coverage_na.txt"
    na_list: dict[str, str] = {}
    if na_file.exists():
        for line in na_file.read_text().splitlines():
            if line.strip() and not line.startswith("#") and ":" in line:
                k, v = line.split(":", 1)
                na_list[k.strip().removeprefix("fsv.")] = v.strip()

    pydefs = collect_py()
    rows = []
    for d in pydefs:
        dkey = f"{d.module.removeprefix('fsv.')}.{d.qual}"
        if dkey in na_list:
            rows.append(dict(module=d.module, qual=d.qual, line=d.line, status="n/a", rust="", crate=prefs(d.module)[0][0] if prefs(d.module)[0] else "?",
                             rust_fn="", evidence="", note="declared n/a: " + na_list[dkey], via="", cmd=d.cmd_path or "", expected=""))
            continue
        skey = f"{d.module.split('.')[-1]}.{d.qual}"
        ported = port_idx.get(dkey) or port_idx.get(skey)
        pcrates, pstems = prefs(d.module)
        key = snake(d.name)
        cands = [] if DUNDER.match(d.name) else list(by_name.get(key, []))
        note = ""
        chosen = None
        via = None
        # CLI command mapping through dispatch arms
        inline_arm = None
        if d.cmd_path:
            toks = d.cmd_path.split()
            targets = arms.get(d.cmd_path, [])
            if not targets and len(toks) > 1:
                targets = [t for t in arms.get(" ".join(toks[1:]), []) if stem_of(t[0]) == toks[0]]
            for path, target, ln, block in targets:
                if target is None:
                    inline_arm = (path, ln, block)
                    continue
                fname = target.split("::")[-1]
                modname = target.split("::")[-2] if "::" in target else None
                for f in by_name.get(fname, []):
                    if f.crate == "fsv-cli" and (modname is None and f.file == path or stem_of(f.file) == modname):
                        chosen, via = f, f"dispatch arm \"{d.cmd_path}\""
                        break
                if chosen:
                    break

        if not chosen and ported:
            tested_p = [f for f in ported if is_tested(f)]
            chosen = (tested_p or ported)[0]
            note = f"doc comment says Port of {dkey}"

        def score(f: RsFn) -> int:
            s = 0
            if f.crate in pcrates:
                s += 3 + (len(pcrates) - pcrates.index(f.crate)) * 0.1
            if stem_of(f.file) in pstems or f.file.stem in pstems:
                s += 2
            if d.cls:
                if f.impl == d.cls or (f.impl and snake(f.impl) == snake(d.cls)):
                    s += 3
            elif f.impl is None:
                s += 1
            pyname = d.qual if d.cls else d.name
            if f.doc and (f"`{pyname}`" in f.doc or f"`{d.name}`" in f.doc or f"{d.module.split('.')[-1]}.{d.name}" in f.doc):
                s += 4
            return s

        if not chosen and inline_arm:
            path, ln, block = inline_arm
            stubs = sorted(set(re.findall(r"pending::(\w+)", block)))
            loc = f"{path.relative_to(CRATES)}:{ln}"
            if stubs:
                note = f"inline dispatch arm at {loc} calls stub pending::{', pending::'.join(stubs)}"
            else:
                note = f"inline dispatch arm at {loc} (no fn to test)"
            rows.append(dict(module=d.module, qual=d.qual, line=d.line, status="missing", rust=loc, crate="fsv-cli",
                             rust_fn="", evidence="", note=note, via="", cmd=d.cmd_path or "",
                             expected="fsv-cli [dispatch arm]"))
            continue
        if not chosen and cands:
            ranked = sorted(cands, key=score, reverse=True)
            best = ranked[0]
            sc = score(best)
            ok = best.crate in pcrates or sc >= 4 or (not common(key) and len(cands) == 1)
            if d.cls and common(key) and not (best.impl and snake(best.impl) == snake(d.cls)) and best.crate not in pcrates:
                ok = False
            if common(key):
                pyname = d.qual if d.cls else d.name
                stem_ok = stem_of(best.file) in pstems or best.file.stem in pstems
                mst = d.module.split('.')[-1]
                doc_ok = bool(best.doc) and (f"{mst}.{d.name}" in best.doc or f"{mst}.{pyname}" in best.doc)
                if not (stem_ok or doc_ok):
                    ok = False
            if ok:
                chosen = best
                if best.crate not in pcrates:
                    note = "matched outside preferred crates"
        if not chosen:
            mstem = d.module.split(".")[-1]
            names = {d.name, d.qual, f"{mstem}.{d.name}", f"{mstem}.{d.qual}", f"{d.name}()", f"{d.qual}()"}
            hits = [f for f in prod if f.crate in pcrates and first_tick(f.doc) in names]
            if d.cls:
                hits = [f for f in hits if f.impl and snake(f.impl) == snake(d.cls)] or [f for f in hits if first_tick(f.doc) != d.name]
            if hits:
                chosen = sorted(hits, key=score, reverse=True)[0]
                note = f"doc comment cites `{first_tick(chosen.doc)}`"
        if not chosen and len(key) >= 4 and not DUNDER.match(d.name):
            ren = [f for f in prod if f.crate in pcrates and (stem_of(f.file) in pstems or f.file.stem in pstems)
                   and f.name != key and (f.name.startswith(key + "_") or f.name.endswith("_" + key))
                   and (not d.cls or (f.impl and snake(f.impl) == snake(d.cls)))]
            if len(ren) == 1:
                chosen = ren[0]
                note = f"renamed to {chosen.name}"
        status = None
        evidence = ""
        stub = None
        if chosen:
            if chosen.file.name == "pending.rs" or re.search(r"\b(not_ported|todo!|unimplemented!)\s*\(", chosen.body):
                stub = f"`{chosen.name}` at {chosen.file.relative_to(CRATES)}:{chosen.line} is a not-ported stub"
            elif re.search(r"pending::\w+", chosen.body):
                calls = sorted(set(re.findall(r"pending::(\w+)", chosen.body)))
                stub = (f"`{chosen.name}` at {chosen.file.relative_to(CRATES)}:{chosen.line} calls stub "
                        f"pending::{', pending::'.join(calls)}")
        if stub:
            rows.append(dict(module=d.module, qual=d.qual, line=d.line, status="missing",
                             rust=f"{chosen.file.relative_to(CRATES)}:{chosen.line}", crate=chosen.crate, rust_fn="",
                             evidence="", note=stub, via="", cmd=d.cmd_path or "", expected=", ".join(pcrates)))
            continue
        if chosen:
            ev = is_tested(chosen)
            if not ev and d.cmd_path and chosen.crate == "fsv-cli" and argv_hits(d.cmd_path):
                ev = f"argv `{d.cmd_path}` in fsv-cli/tests"
            if ev:
                status, evidence = "covered", ev
            else:
                status = "fn-only"
                g = reached_golden(chosen) if chosen.crate == "fsv-cli" else None
                if g:
                    status, evidence = "covered", f"golden binary run via {g}"
                else:
                    r = reached_via(chosen)
                    if r:
                        note = (note + "; " if note else "") + f"reached indirectly from tested {r}"
        else:
            na = None
            if d.module.endswith("__init__") and d.name in ("__getattr__", "__dir__"):
                na = "lazy-import module __getattr__ shim"
            elif d.cls == "_LazyModule":
                na = "import-time _LazyModule shim"
            elif d.cls and set(d.bases) & TEXTUAL_BASES and (TEXTUAL_HOOKS.match(d.name) or d.name == "__init__"):
                na = "Textual framework hook"
            elif d.cls and DUNDER.match(d.name) and d.name not in ("__call__",):
                if d.name == "__str__" and d.cls in display:
                    na = f"Display impl for {d.cls}"
                elif d.cls in types or snake(d.cls) in {snake(t) for t in types}:
                    na = f"dunder; Rust type {d.cls} exists (literal/derive/trait)"
                else:
                    note = f"no Rust type {d.cls}"
            if na:
                status, note = "n/a", na
            else:
                status = "missing"
                if key in orphan_names:
                    note = (note + "; " if note else "") + "only in orphan file " + str(orphan_names[key][0].file.relative_to(CRATES))
                elif cands:
                    note = (note + "; " if note else "") + "same-name fn only at " + ", ".join(
                        f"{c.file.relative_to(CRATES)}:{c.line}" for c in cands[:2])
        loc = f"{chosen.file.relative_to(CRATES)}:{chosen.line}" if chosen else ""
        crate = chosen.crate if chosen else (pcrates[0] if pcrates else "?")
        rows.append(dict(module=d.module, qual=d.qual, line=d.line, status=status, rust=loc, crate=crate,
                         rust_fn=(f"{chosen.impl}::{chosen.name}" if chosen and chosen.impl else chosen.name if chosen else ""),
                         evidence=evidence, note=note, via=via or "", cmd=d.cmd_path or "",
                         expected=", ".join(pcrates) + " [" + ", ".join(pstems) + "]"))

    Path(args.json).write_text(json.dumps(rows, indent=1))
    write_md(Path(args.out), rows, cargo_status, not_compiled, orphans)
    tot = defaultdict(int)
    for r in rows:
        tot[r["status"]] += 1
    print(dict(tot), "->", args.out)
    return 0


def write_md(out: Path, rows, cargo_status, not_compiled, orphans):
    mods = defaultdict(list)
    for r in rows:
        mods[r["module"]].append(r)
    S = ("covered", "fn-only", "missing", "n/a")
    L = []
    L.append("# Python -> Rust def coverage\n")
    L.append("Generated by `/tmp/fsv_coverage.py` (kept outside the repo). Rerun with\n"
             "`python3 /tmp/fsv_coverage.py`. It first runs `cargo build --workspace --tests --offline --keep-going`, which builds "
             "the same targets as `cargo test --workspace --offline --no-run` but continues past a broken target. "
             "`--no-cargo` skips the build and treats every crate as compiling. Raw rows go to `/tmp/fsv_coverage.json`. "
             "Other work was in progress in the worktree while this was generated, so rerun before relying on the numbers.\n")
    L.append("## Method\n")
    L.append(
        "- Python defs: every top-level `def` and every method of a top-level class in `fsv/src/fsv/**/*.py`, listed with `ast`. "
        "Nested defs, such as the per-resource commands built inside `cli._make_subapp`, are not listed.\n"
        "- Rust fns: only files reachable through `mod` declarations from `src/lib.rs`, `src/main.rs` or `tests/*.rs` of a crate that compiled. "
        "Fns in `#[cfg(test)]` modules and integration tests count as test code, not as implementations.\n"
        "- Matching: snake_case name with leading underscores stripped, ranked by expected crate and file for the Python module, "
        "`impl` type equal to the Python class, and doc comments citing the Python name. Typer commands are matched through the "
        "`dispatch` match arms in `fsv-cli` (command path to fn). When no exact name matches, two fallbacks apply, both limited to "
        "the expected crate: a fn whose doc comment opens with the Python name in backticks, and a unique fn in the expected file "
        "whose name extends the Python name (`fields_from` to `fields_from_payload`).\n"
        "- Stubs count as missing: fns in `fsv-cli/src/pending.rs`, any fn calling `not_ported`, `todo!` or `unimplemented!`, "
        "handlers that call `pending::*`, and inline dispatch arms (no fn of their own).\n"
        "- Test evidence: the fn name is called or referenced in a `#[test]` fn, a test-module helper or an integration test file. "
        "Common names such as `new`, `get` or `render` only count when the test is in the same file or module, or names the `impl` type. "
        "CLI command fns also count when a `fsv-cli/tests` argv runs that command path; such a test may only reach argument validation.\n"
        "- Limits: matching is by name. A port renamed beyond the fallbacks above shows as missing, and an unrelated fn with the same "
        "name in the expected crate can show as a match. \"reached indirectly\" notes come from a name-based call graph within one crate and are hints only.\n"
        "- n/a covers Textual hooks (`compose`, `on_mount`, `watch_*` and similar) on Textual subclasses, the lazy-import `__getattr__` shim, "
        "and dunder methods whose class exists as a Rust type. These are only applied when no Rust fn matched.\n"
        "- The Rust TUI and form UI route events through message enums (`App::on_msg`, `App::on_ui`) instead of one method per "
        "Textual `action_*` or `on_*` handler. Those handlers are listed as missing unless a Rust fn shares the name, even when "
        "the behaviour lives in a match arm.\n")
    L.append(f"Build check: {cargo_status}.\n")
    if not_compiled:
        L.append(f"Crates not counted because they did not compile: {', '.join(not_compiled)}.\n")
    if orphans:
        L.append("Orphan `.rs` files (not in any `mod` chain, not counted): " + ", ".join(f"`{p.relative_to(CRATES)}`" for p in orphans) + ".\n")
    else:
        L.append("Orphan `.rs` files: none.\n")

    L.append("## Totals by Rust crate\n")
    L.append("A def is attributed to the crate of its matched fn; missing defs go to the first expected crate for their Python module.\n")
    L.append("| crate | defs | covered | fn-only | missing | n/a |\n|---|---:|---:|---:|---:|---:|")
    bc = defaultdict(lambda: defaultdict(int))
    for r in rows:
        bc[r["crate"]][r["status"]] += 1
    for c in sorted(bc):
        L.append(f"| {c} | {sum(bc[c].values())} | " + " | ".join(str(bc[c][s]) for s in S) + " |")
    tot = defaultdict(int)
    for r in rows:
        tot[r["status"]] += 1
    L.append(f"| **total** | {len(rows)} | " + " | ".join(str(tot[s]) for s in S) + " |\n")

    L.append("## Summary by Python module\n")
    L.append("| module | defs | covered | fn-only | missing | n/a |\n|---|---:|---:|---:|---:|---:|")
    for m in sorted(mods):
        c = defaultdict(int)
        for r in mods[m]:
            c[r["status"]] += 1
        L.append(f"| `{m}` | {len(mods[m])} | " + " | ".join(str(c[s]) for s in S) + " |")
    L.append("")

    L.append("## Gaps by module\n")
    L.append("Only fn-only and missing defs are listed. For a fn-only def, \"Rust\" is the matched fn. For a missing def, it is the crate and "
             "files that were searched, plus any same-name fn that was rejected.\n")
    for m in sorted(mods):
        gaps = [r for r in mods[m] if r["status"] in ("fn-only", "missing")]
        if not gaps:
            continue
        L.append(f"### `{m}`\n")
        for r in sorted(gaps, key=lambda r: r["line"]):
            cmd = f" (command `fsv {r['cmd']}`)" if r["cmd"] else ""
            if r["status"] == "fn-only":
                where = f"`{r['rust_fn']}` at `{r['rust']}`"
            elif r["rust"]:
                where = f"`{r['rust']}`"
            else:
                where = f"searched {r['expected']}"
            note = f"; {r['note']}" if r["note"] else ""
            L.append(f"- {r['status']}: `{r['qual']}` (line {r['line']}){cmd}. Rust: {where}{note}.")
        L.append("")
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text("\n".join(L))


if __name__ == "__main__":
    sys.exit(main())
