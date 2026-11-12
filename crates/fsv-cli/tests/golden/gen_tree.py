"""Generate crates/fsv-cli/src/generated.rs from the Python tree dumps.

    python3 crates/fsv-cli/tests/golden/gen_tree.py          # rewrite generated.rs
    python3 crates/fsv-cli/tests/golden/gen_tree.py --check  # exit 1 if stale

Inputs: tests/fixtures/cli_tree.json (dump_cli_tree.py) for commands and
options, tests/fixtures/completion_tree.json (dump_completion.py) for the
completion hints. The output is piped through `rustfmt --edition 2024`.

Choices the dumps cannot carry live in the tables below: which leaves share
one `Args` struct (SHARED), the Rust enum behind a choice set (ENUMS),
groups built by a helper fn (GROUP_FNS), and the completion `value_flag`
override (VALUE_FLAG).
"""
import json
import pathlib
import re
import subprocess
import sys

from pyrev import stamp

CRATE = pathlib.Path(__file__).resolve().parents[2]
OUT = CRATE / "src" / "generated.rs"
RES = ("changes", "tickets", "problems")


def per_res(leaf, *res):
    return [f"{r} {leaf}" for r in (res or RES)]


# Leaves that share one struct; the first leaf in tree order defines the
# fields, the others patch differing help or flags with `mut_arg`.
SHARED = {
    "ResLsArgs": per_res("ls"),
    "ResGetArgs": per_res("get"),
    "ResActivityArgs": per_res("activity"),
    "ResTasksArgs": per_res("tasks", "changes", "problems") + ["tickets tasks-legacy"],
    "ResExportArgs": per_res("export"),
    "PlanningTextArgs": per_res("planning-text", "changes", "problems"),
    "ResFieldsArgs": per_res("fields"),
    "ResLookupArgs": per_res("lookup"),
    "ResViewsArgs": per_res("views"),
    "ResSearchArgs": per_res("search"),
    "ResUrlArgs": per_res("url"),
    "ResNotesArgs": per_res("notes", "changes", "problems"),
    "ResAddNoteArgs": per_res("add-note", "changes", "problems"),
    "ResUpdateArgs": per_res("update", "tickets", "problems"),
    "ResDownloadArgs": per_res("download", "tickets", "problems"),
    "ReviewedListArgs": per_res("watchers list", "changes", "tickets") + ["tickets time list"],
    "WatchersSetArgs": per_res("watchers set", "changes", "tickets"),
    "JsonOnlyArgs": per_res("columns list") + ["tickets canned folders", "tickets scenarios list"],
    "PositiveIdArgs": ["tickets private-views show", "tickets canned show", "tickets scenarios show"],
    "ColumnsSetArgs": per_res("columns set"),
}

ENUMS = {
    ("table", "json", "csv", "tsv"): "OutputFormat",
    ("json",): "ReviewOutput",
    ("relevance", "created", "modified"): "SearchSort",
    ("asc", "desc"): "SortOrder",
}

# Subgroups built once by a helper fn and attached under each resource.
GROUP_FNS = {
    "watchers": "// `fsv {tickets,changes} watchers`.",
    "columns": None,
    "private-views": None,
    "canned": None,
    "scenarios": None,
}

# Python's completion generator treats this boolean as taking a value.
VALUE_FLAG = {("fsv changes update", "backup_replaced"), ("fsv changes time write", "billable"),
              ("fsv problems time write", "billable")}

KEYWORDS = {"where", "type", "match", "ref", "self", "move", "loop", "fn", "use", "mod", "impl", "in"}


def rs(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def camel(path):
    return "".join(w.capitalize() for part in path.split() for w in part.split("-")) + "Args"


def plain(text):
    """Rich markup stripped, whitespace collapsed (Click help -> clap)."""
    return " ".join(re.sub(r"\[/?[a-z ]+\]", "", text or "").split())


def walk(node, out):
    out.append(node)
    for c in node["commands"]:
        walk(c, out)
    return out


def own_params(node):
    return [p for p in node["params"] if p["name"] != "verbose"]


# ---------------------------------------------------------------------------
# struct fields


def longs_shorts(p):
    longs = [o[2:] for o in p["opts"] if o.startswith("--")]
    shorts = [o[1:] for o in p["opts"] if not o.startswith("--")]
    return longs, shorts


def field(p, name=None, long=None, shorts=None, aliases=None, overrides=None, unless=None, help_=True):
    """(attrs, field, type) for one clap arg."""
    name = name or p["name"]
    a = []
    if help_ and p["help"]:
        a.append(f"help = {rs(p['help'])}")
    if p["hidden"]:
        a.append("hide = true")
    t, default = p["type"], p["default"]
    if p["kind"] == "argument":
        a.append(f"value_name = {rs(p['metavar'] or name.upper())}")
        if p["nargs"] == -1:
            a += ["num_args = 1..", "required = true"] if p["required"] else ["num_args = 0.."]
            if t in ("integer", "integer range"):
                a.append("value_parser = clap::value_parser!(i64)" + (int_range(p) if t == "integer range" else ""))
                ty = "Vec<i64>"
            else:
                ty = "Vec<String>"
        elif t == "integer range":
            a.append("value_parser = clap::value_parser!(i64)" + int_range(p))
            ty = "i64"
        elif t == "integer":
            ty = "i64"
        elif p["required"]:
            ty = "String"
        elif default is None:
            ty = "Option<String>"
        else:
            a.append(f"default_value = {rs(str(default))}")
            ty = "String"
        return a, name, ty
    a.append(f"long = {rs(long)}")
    for s in shorts:
        a.append(f"short = '{s}'")
    for al in aliases:
        a.append(f"visible_alias = {rs(al)}")
    if p["is_flag"]:
        ty = "bool"
    else:
        if t != "integer range":
            a.append("allow_hyphen_values = true")
        if t == "choice":
            ty = ENUMS[tuple(p["choices"])]
            a.append("value_enum")
            a.append(f"default_value = {rs(str(default))}")
        elif t in ("integer", "integer range"):
            a.append(f"value_name = {rs(p['metavar'] or t.upper())}")
            if p["multiple"]:
                if p["required"]:
                    a.append("required = true")
                ty = "Vec<i64>"
            elif p["required"]:
                ty = "i64"
            elif default is None:
                ty = "Option<i64>"
            else:
                a.append(f"default_value_t = {default}")
                ty = "i64"
            if t == "integer range":
                a.append("value_parser = clap::value_parser!(i64)" + int_range(p))
        elif t == "path":
            a.append(f"value_name = {rs(p['metavar'] or 'PATH')}")
            ty = "Option<PathBuf>"
        elif p["multiple"]:
            a.append(f"value_name = {rs(p['metavar'] or 'TEXT')}")
            if p["required"]:
                a.append("required = true")
            ty = "Vec<String>"
        else:
            a.append(f"value_name = {rs(p['metavar'] or 'TEXT')}")
            if p["required"] or default is None:
                ty = "String" if p["required"] else "Option<String>"
            else:
                a.append(f"default_value = {rs(str(default))}")
                ty = "String"
    if overrides:
        a.append(f"overrides_with = {rs(overrides)}")
    if unless:
        a.append(f"required_unless_present = {rs(unless)}")
    return a, name, ty


def int_range(p):
    lo, hi = p.get("min"), p.get("max")
    if lo is None and hi is None:
        raise SystemExit(f"{p['name']}: integer range without bounds; re-dump cli_tree.json")
    return f".range({'' if lo is None else lo}..{'' if hi is None else f'={hi}'})"


def fields(params):
    out = []
    for p in params:
        if p["kind"] == "option":
            longs, shorts = longs_shorts(p)
            if p["secondary_opts"]:
                neg = p["secondary_opts"][0][2:]
                neg_name = neg.replace("-", "_")
                unless = neg_name if p["required"] else None
                out.append(field(p, long=longs[0], shorts=shorts, aliases=longs[1:], overrides=neg_name, unless=unless))
                q = dict(p, help=None, hidden=False)
                out.append(field(q, name=neg_name, long=neg, shorts=[], aliases=[], overrides=p["name"]))
                continue
            out.append(field(p, long=longs[0], shorts=shorts, aliases=longs[1:]))
        else:
            out.append(field(p))
    return out


def ident(name):
    return "r#" + name if name in KEYWORDS else name


def struct(comment, name, fs):
    lines = [f"// {comment}", "#[derive(Args, Debug, Clone)]"]
    if not fs:
        return "\n".join(lines + [f"pub struct {name} {{}}"])
    lines.append(f"pub struct {name} {{")
    for a, f, ty in fs:
        one = f"    #[arg({', '.join(a)})]"
        if a and len(one) <= 100:
            lines.append(one)
        elif a:
            lines += ["    #[arg("] + [f"        {x}," for x in a[:-1]] + [f"        {a[-1]}", "    )]"]
        lines.append(f"    pub {ident(f)}: {ty},")
    lines.append("}")
    return "\n".join(lines)


# ---------------------------------------------------------------------------
# tree


def patches(base, other):
    """`.mut_arg` calls that turn `base` leaf params into `other`'s."""
    out = []
    for p, q in zip(own_params(base), own_params(other)):
        if p["name"] != q["name"] or p["type"] != q["type"]:
            raise SystemExit(f"{other['path']}: params differ from {base['path']}")
        calls = []
        if p["help"] != q["help"]:
            calls.append(f"a.help({rs(q['help'])})")
        if p["opts"] != q["opts"]:
            longs, shorts = longs_shorts(q)
            calls.append(f".long({rs(longs[0])})" if calls else f"a.long({rs(longs[0])})")
            calls.append(".alias(None)")
            calls += [f".visible_alias({rs(al)})" for al in longs[1:]]
            calls.append(f".short('{shorts[0]}')" if shorts else ".short(None)")
            calls.append(".short_alias(None)")
        if calls:
            out.append(f'.mut_arg({rs(p["name"])}, |a| {"".join(calls)})')
    if len(own_params(base)) != len(own_params(other)):
        raise SystemExit(f"{other['path']}: params differ from {base['path']}")
    return "".join(out)


class Gen:
    def __init__(self, tree):
        self.tree = tree
        self.nodes = walk(tree, [])
        self.leaves = [n for n in self.nodes[1:] if not n["group"]]
        self.struct_of = {}
        for name, paths in SHARED.items():
            for p in paths:
                self.struct_of["fsv " + p] = name
        for n in self.leaves:
            self.struct_of.setdefault(n["path"], camel(n["path"][4:]))
        self.first = {}
        for n in self.leaves:
            self.first.setdefault(self.struct_of[n["path"]], n)

    def structs(self):
        out, done = [], set()
        for n in self.leaves:
            name = self.struct_of[n["path"]]
            if name in done:
                continue
            done.add(name)
            users = [m["path"] for m in self.leaves if self.struct_of[m["path"]] == name]
            out.append(struct(", ".join(users), name, fields(own_params(n))))
        return out

    def leaf_call(self, n, cmd_suffix=""):
        name = self.struct_of[n["path"]]
        last = n["path"].split()[-1]
        about = ""
        if n["hidden"]:
            about += " .hide(true)"
        if n["help"]:
            about += f" .about({rs(plain(n['help']))})"
        if n["epilog"]:
            about += f" .after_help({rs(plain(n['epilog']))})"
        patch = "" if self.first[name] is n else patches(self.first[name], n)
        return f'{name}::augment_args(Command::new({rs(last)}){about}.arg(global_verbose())){patch}'

    def group_fn_name(self, n):
        return n["path"].split()[-1].replace("-", "_") + "_group"

    def subtree(self, n, indent):
        """`.subcommand(...)` lines for `n`'s children at `indent`."""
        pad = " " * indent
        lines = []
        for c in n["commands"]:
            last = c["path"].split()[-1]
            if c["group"] and last in GROUP_FNS and len(c["path"].split()) == 3:
                lines.append(f"{pad}.subcommand({self.group_fn_name(c)}())")
            elif c["group"]:
                head = [f"{pad}.subcommand(Command::new({rs(last)})"]
                if c["help"]:
                    head.append(f"{pad}    .about({rs(plain(c['help']))})")
                if c["epilog"]:
                    head.append(f"{pad}    .after_help({rs(plain(c['epilog']))})")
                body = self.subtree(c, indent + 4)
                body[-1] += ")"
                lines += head + body
            else:
                lines.append(f"{pad}.subcommand({self.leaf_call(c)})")
        return lines

    def group_fns(self):
        out, done = [], set()
        for n in self.nodes:
            last = n["path"].split()[-1]
            if not (n["group"] and last in GROUP_FNS and len(n["path"].split()) == 3) or last in done:
                continue
            done.add(last)
            lines = []
            if GROUP_FNS[last]:
                lines.append(GROUP_FNS[last])
            lines.append(f"fn {self.group_fn_name(n)}() -> Command {{")
            lines.append(f"    Command::new({rs(last)})")
            if n["help"]:
                lines.append(f"        .about({rs(plain(n['help']))})")
            for c in n["commands"]:
                name = self.struct_of[c["path"]]
                if c["help"]:
                    lines += [f"        .subcommand({name}::augment_args(",
                              f"            Command::new({rs(c['path'].split()[-1])})",
                              f"                .about({rs(plain(c['help']))})",
                              "                .arg(global_verbose()),",
                              "        ))"]
                else:
                    lines.append(f"        .subcommand({name}::augment_args(Command::new({rs(c['path'].split()[-1])}).arg(global_verbose())))")
            lines.append("}")
            out.append("\n".join(lines))
        return out

    def build(self):
        t = self.tree
        lines = ["// The full command tree (without the shared settings `crate::app` applies).",
                 "pub fn build() -> Command {",
                 f"    let root = Command::new(\"fsv\")",
                 f"        .about({rs(plain(t['help']))})",
                 f"        .after_help({rs(plain(t['epilog']))});",
                 "    RootArgs::augment_args(root)"]
        body = self.subtree(t, 8)
        lines += body
        lines.append("}")
        return "\n".join(lines)

    def root_args(self):
        return struct("Root flags.", "RootArgs", fields(self.tree["params"]))

    def leaves_table(self):
        rows = [f"    ({rs(n['path'][4:])}, {rs(self.struct_of[n['path']])})," for n in self.leaves]
        return "\n".join(["// Leaf command path (without `fsv`) to its `Args` struct, for review.",
                          "pub const LEAVES: &[(&str, &str)] = &["] + rows + ["];"])


def click_metavar(p):
    """`TyperArgument.make_metavar` for the argument types the tree has."""
    if p["metavar"]:
        return p["metavar"]
    var = p["name"].upper() if p["required"] else f"[{p['name'].upper()}]"
    return var + ("..." if p["nargs"] != 1 else "")


def click_usage(leaves):
    """Per leaf with a required parameter: Click's usage pieces and, in
    declaration order (the order Click checks them), clap's first token for
    each required parameter with Click's `MissingParameter` message."""
    rows = []
    for n in leaves:
        required = [p for p in n["params"] if p["required"]]
        if not required:
            continue
        pieces = ["[OPTIONS]"] + [click_metavar(p) for p in n["params"] if p["kind"] == "argument"]
        missing = []
        for p in required:
            if p["kind"] == "argument":
                token = "<" + (p["metavar"] or p["name"].upper()) + ">"
                msg = f"Missing argument '{click_metavar(p)}'."
            else:
                token = "--" + longs_shorts(p)[0][0]
                msg = f"Missing option {' / '.join(repr(o) for o in p['opts'])}."
            missing.append(f"({rs(token)}, {rs(msg)})")
        if len({m.split(",")[0] for m in missing}) != len(missing):
            raise SystemExit(f"{n['path']}: two required parameters print the same clap token")
        rows.append(f"    ({rs(n['path'][4:])}, {rs(' '.join(pieces))}, &[{', '.join(missing)}]),")
    return "\n".join(["// Click's usage line and missing-parameter message, keyed by the token clap",
                      "// prints for the missing argument; see `app::missing_parameter`.",
                      "pub const CLICK_USAGE: &[(&str, &str, &[(&str, &str)])] = &["] + rows + ["];"])


def hints(ctree, cli_nodes):
    by_path = {n["path"]: n for n in cli_nodes}
    rows = []
    for n in walk_completion(ctree, []):
        path = " ".join(n["path"])
        cli = by_path[path]
        opts = [p for p in n["params"] if path == "fsv" or p["flags"] != ["--verbose", "-v"]]
        entries = []
        for p in opts:
            ids = [c["name"] for c in cli["params"] if p["flags"][0] in c["opts"] + c["secondary_opts"]]
            if len(ids) != 1:
                raise SystemExit(f"{path}: no unique param for {p['flags']}")
            vf = "true" if (path, ids[0]) in VALUE_FLAG else "false"
            comp = f"Some({rs(p['completer'])})" if p["completer"] else "None"
            res = f"Some({rs(p['resource'])})" if p["resource"] else "None"
            flags = ", ".join(rs(f) for f in p["flags"])
            entries.append(f"OptionHint {{ id: {rs(ids[0])}, flags: &[{flags}], completer: {comp}, "
                           f"resource: {res}, value_flag: {vf} }},")
        rows.append(f"    ({rs(path)}, {rs(n['help'] or '')}, &[{' '.join(entries)}]),")
    return "\n".join(["// Completion metadata clap cannot hold: the callback docstring line and",
                      "// the options to complete, in declaration order.",
                      "pub static COMPLETION_HINTS: &[(&str, &str, &[OptionHint])] = &["] + rows + ["];"])


def walk_completion(node, out):
    out.append(node)
    for s in node["subcommands"]:
        walk_completion(s, out)
    return out


HEADER = """// @generated by tests/golden/gen_tree.py from tests/fixtures/cli_tree.json; do not edit.
// {stamp}
#![allow(clippy::all)]
use crate::types::{OutputFormat, ReviewOutput, SearchSort, SortOrder};
use clap::{Arg, ArgAction, Args, Command};
use fsv_completion::OptionHint;
use std::path::PathBuf;
"""

GLOBAL_VERBOSE = """fn global_verbose() -> Arg {
    Arg::new("fsv_verbose")
        .long("verbose")
        .short('v')
        .action(ArgAction::SetTrue)
        .help("print debug info to stderr")
        .help_heading("Global options")
}"""


def generate():
    fx = CRATE / "tests" / "fixtures"
    tree = json.loads((fx / "cli_tree.json").read_text())
    ctree = json.loads((fx / "completion_tree.json").read_text())
    g = Gen(tree)
    parts = [HEADER.rstrip("\n").replace("{stamp}", stamp(tree["_python"]))] + g.structs() + [GLOBAL_VERBOSE, g.root_args()] + g.group_fns()
    parts += [g.build(), g.leaves_table(), click_usage(g.leaves), hints(ctree, g.nodes)]
    raw = "\n\n".join(parts) + "\n"
    return subprocess.run(["rustfmt", "--edition", "2024", "--emit", "stdout"], input=raw,
                          capture_output=True, text=True, check=True).stdout


if __name__ == "__main__":
    text = generate()
    if "--check" in sys.argv[1:]:
        sys.exit(0 if OUT.read_text() == text else 1)
    OUT.write_text(text)
