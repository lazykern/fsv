"""``fsv form``: inspect the form metadata and rules the web uses.

* ``explain``: visible / required / hidden for a set of answers, with the
  rule that caused each.
* ``fields``: labels, help text and choices in web order.
* ``dump`` / ``diff``: snapshot the metadata and compare later (admins edit
  rules and choices; a diff says when drafts and presets need a look).
"""
from __future__ import annotations

import json
import time
from typing import Any, List, Mapping, Optional

import typer
from rich.markup import escape
from rich.table import Table

from fsv.drafts import resolve_draft
from fsv.change_flow import real_conflicts
from fsv.formmeta import (
    ClientResolver,
    apply_answers,
    existing_from_record,
    find_field,
    load_meta,
    module_name,
    name_existing,
)
from fsv.forms import FormError, FormState, canonical_json, plain_text
from fsv.render import console, err

form_app = typer.Typer(
    name="form",
    help="inspect form fields and business rules (explain, fields, dump, diff)",
    no_args_is_help=True,
)

SNAPSHOT_VERSION = 1


def _client() -> Any:
    from fsv.cli import _client as cli_client

    return cli_client()


def _fail(msg: str, code: int = 2) -> None:
    err.print(f"[red]error[/red]: {escape(msg)}", highlight=False)
    raise typer.Exit(code)


def _module(text: str) -> str:
    try:
        return module_name(text)
    except ValueError as e:
        _fail(str(e))
        raise


def _guard(fn: Any) -> Any:
    from fsv.errors import APIError, SessionError, exit_code_for

    try:
        return fn()
    except (SessionError, APIError, FormError, ValueError) as e:
        _fail(str(e), exit_code_for(e) if not isinstance(e, FormError) else 2)


def _one_line(text: str, limit: int = 100) -> str:
    s = " ".join(plain_text(text or "").split())
    return s if len(s) <= limit else s[: limit - 1] + "…"


@form_app.command("explain", epilog=(
    "[bold]Examples:[/bold]  fsv form explain changes --set 'Change Type=Emergency'  |  "
    "fsv form explain changes --from change.yaml  |  fsv form explain changes --id CHN-12 --set 'Stage=Live'"
))
def explain(
    module: str = typer.Argument("changes", help="form: changes"),
    set_: Optional[List[str]] = typer.Option(None, "--set", help="answer LABEL=VALUE (repeatable)"),
    from_: Optional[str] = typer.Option(None, "--from", help="draft YAML with answers"),
    preset: Optional[str] = typer.Option(None, "--preset", help="preset name or path"),
    id_: Optional[str] = typer.Option(None, "--id", help="explain the edit form of this record"),
    all_: bool = typer.Option(False, "--all", help="also list fields hidden by metadata/sections"),
    output: str = typer.Option("table", "--output", "-o", help="table|json"),
) -> None:
    """Why each field is visible, required or hidden, by rule."""
    mod = _module(module)
    c = _client()
    record = None
    if id_:
        from fsv.resources import CHANGES, parse_id

        cid = parse_id(id_, CHANGES) if not id_.isdigit() else int(id_)
        data = _guard(lambda: c.int_get(f"changes/{cid}"))
        record = data.get("change", data)
    meta = _guard(lambda: load_meta(c, mod, "edit" if record else "new",
                                    module_id=(record.get("display_id") or record.get("id")) if record else None))
    draft = _guard(lambda: resolve_draft(from_, preset, set_))
    spec = meta.spec
    state = _guard(lambda: FormState(spec, meta.engine(), mode="edit" if record else "new",
                                     resolver=ClientResolver(c),
                                     existing=(name_existing(existing_from_record(spec, record), spec, record, ClientResolver(c))
                                              if record else None)))
    answers = dict(draft.answers)
    errors = apply_answers(state, answers)
    ev = state.evaluated
    rows: list[dict[str, Any]] = []
    by_field: dict[str, list[Any]] = {}
    for x in state.explain():
        by_field.setdefault(x.field, []).append(x)
    for f in spec.ordered():
        xs = by_field.get(f.api_name, [])
        vis = f.api_name in ev.visible
        if not vis and not all_ and not any(x.source == "rule" for x in xs):
            continue
        why = [str(x).split(": ", 1)[1] if ": " in str(x) else str(x) for x in xs if x.source != "metadata"]
        rows.append({
            "field": f.api_name, "label": f.label, "visible": vis, "required": f.api_name in ev.required,
            "disabled": f.api_name in ev.disabled, "value": state.display(f.api_name) if vis else "",
            "populated_by": ev.populated.get(f.api_name), "why": why,
        })
    out = {"rows": rows, "messages": [{"text": m.text, "blocking": m.blocking, "rule": m.rule_name} for m in ev.messages],
           "conflicts": [c.describe() for c in real_conflicts(ev)], "errors": errors,
           "metadata": meta.as_of()}
    if output == "json":
        print(json.dumps(out, indent=2, default=str, ensure_ascii=False))
        return
    t = Table(title=f"{mod} form" + (" (edit)" if record else " (new)"))
    for col in ("Field", "State", "Value", "Why"):
        t.add_column(col, overflow="fold")
    for r in rows:
        state_s = ("visible" if r["visible"] else "[dim]hidden[/dim]") + (
            ", [bold]required[/bold]" if r["required"] else "") + (", read-only" if r["disabled"] else "")
        val = escape(r["value"] or "") + (f" [dim](rule {r['populated_by']})[/dim]" if r["populated_by"] else "")
        t.add_row(escape(r["label"]), state_s, val, escape("; ".join(r["why"])) or "[dim]form metadata[/dim]")
    console.print(t)
    for m in out["messages"]:
        console.print(f"[magenta]{'BLOCKING ' if m['blocking'] else ''}message[/] {escape(m['text'])} "
                      f"[dim](rule {escape(str(m['rule']))})[/dim]", highlight=False)
    for cf in out["conflicts"]:
        console.print(f"[yellow]conflict[/] {escape(cf)}", highlight=False)
    for e in errors:
        console.print(f"[red]answer error[/] {escape(e)}", highlight=False)
    if errors:
        raise typer.Exit(2)


@form_app.command("fields", epilog=(
    "[bold]Examples:[/bold]  fsv form fields changes  |  fsv form fields changes --choices 'Change Type'"
))
def fields(
    module: str = typer.Argument("changes", help="form: changes"),
    choices: Optional[str] = typer.Option(None, "--choices", help="list the choices of this field (label or name)"),
    output: str = typer.Option("table", "--output", "-o", help="table|json"),
) -> None:
    """Fields in web order with labels, help text and choices."""
    mod = _module(module)
    c = _client()
    meta = _guard(lambda: load_meta(c, mod, "new"))
    spec = meta.spec
    state = _guard(lambda: FormState(spec, meta.engine(), mode="new"))
    ev = state.evaluated
    if choices:
        f = _guard(lambda: find_field(spec, choices))
        pool = [ch for ch in spec.choices_for(f, ev.values) if ch.visible] if f.is_nested_level else \
            [ch for ch in f.choices if ch.visible]
        if f.is_nested_level and not pool:
            _fail(f"{f.label}: choose the upper level first (its choices depend on it)")
        data = [{"label": ch.label, "value": ch.value, "id": ch.id, "description": ch.description} for ch in pool]
        if output == "json":
            print(json.dumps({"field": f.api_name, "label": f.label, "choices": data}, indent=2, default=str,
                             ensure_ascii=False))
            return
        t = Table(title=f"{f.label} ({f.api_name}): {len(data)} choices")
        t.add_column("Choice", overflow="fold")
        t.add_column("Meaning", overflow="fold")
        for d in data:
            t.add_row(escape(d["label"]), escape(_one_line(d["description"], 200)))
        console.print(t)
        if f.help:
            console.print(f"[dim]{escape(_one_line(f.help, 400))}[/dim]", highlight=False)
        return
    rows = []
    for f in spec.ordered():
        rows.append({
            "label": f.label, "name": f.api_name, "kind": f.kind, "custom": f.is_custom,
            "required_static": f.required_base, "visible_on_load": f.api_name in ev.visible,
            "required_on_load": f.api_name in ev.required, "help": _one_line(f.help, 300),
            "choices": len(f.choices),
            "choice_descriptions": [{"label": c.label, "description": c.description}
                                    for c in f.choices if c.visible and c.description],
        })
    if output == "json":
        print(json.dumps(rows, indent=2, default=str, ensure_ascii=False))
        return
    t = Table(title=f"{mod} form fields (web order; state on a new form)")
    for col in ("Label", "Name", "Type", "On load", "Help"):
        t.add_column(col, overflow="fold")
    for r in rows:
        load = ("visible" if r["visible_on_load"] else "[dim]hidden[/dim]") + (
            ", [bold]required[/bold]" if r["required_on_load"] else "")
        kind = r["kind"] + (f" ({r['choices']})" if r["choices"] else "")
        help_cell = _one_line(r["help"], 80)
        extra = [f"• {d['label']} — {_one_line(d['description'], 80)}" for d in r["choice_descriptions"][:8]]
        if extra:
            help_cell = "\n".join(([help_cell] if help_cell else []) + extra)
        t.add_row(escape(r["label"]), escape(r["name"]), kind, load, escape(help_cell))
    console.print(t)
    console.print("[dim]choices: fsv form fields changes --choices LABEL[/dim]", highlight=False)


@form_app.command("dump", epilog="[bold]Examples:[/bold]  fsv form dump changes > snap.json")
def dump(
    module: str = typer.Argument("changes", help="form: changes"),
    mode: str = typer.Option("new", "--mode", help="new|edit (which form's rules)"),
) -> None:
    """Print a metadata snapshot (fields + rules) as JSON. Keep it local:
    the requesting user's profile is left out."""
    mod = _module(module)
    if mode not in ("new", "edit"):
        _fail("--mode must be new or edit")
    c = _client()
    meta = _guard(lambda: load_meta(c, mod, mode, fresh=True))
    snap = {
        "fsv_form_snapshot": SNAPSHOT_VERSION,
        "module": mod,
        "form_type": meta.form_type,
        "workspace_id": meta.workspace_id,
        "taken_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "fields": meta.raw_fields,
        "business_rules": meta.rules.get("business_rules") or [],
    }
    print(json.dumps(snap, indent=2, default=str, ensure_ascii=False))


def _choice_labels(f: Mapping[str, Any]) -> list[str]:
    out = []
    for ch in f.get("choices") or []:
        if isinstance(ch, Mapping):
            out.append(str(ch.get("value") or ch.get("label") or ch.get("name") or ch.get("id")))
        else:
            out.append(str(ch))
    return out


def diff_snapshots(old: Mapping[str, Any], new: Mapping[str, Any]) -> list[dict[str, Any]]:
    """Field and rule differences between two snapshots."""
    out: list[dict[str, Any]] = []
    of = {str(f.get("name")): f for f in old.get("fields") or []}
    nf = {str(f.get("name")): f for f in new.get("fields") or []}
    for name in nf.keys() - of.keys():
        out.append({"kind": "field", "change": "added", "name": name, "label": nf[name].get("label")})
    for name in of.keys() - nf.keys():
        out.append({"kind": "field", "change": "removed", "name": name, "label": of[name].get("label")})
    for name in sorted(of.keys() & nf.keys()):
        a, b = of[name], nf[name]
        what = []
        for k in ("label", "field_type", "required", "position", "helptext", "displayed_to_agents"):
            if a.get(k) != b.get(k):
                what.append(f"{k}: {a.get(k)!r} -> {b.get(k)!r}")
        ca, cb = _choice_labels(a), _choice_labels(b)
        if ca != cb:
            added = [x for x in cb if x not in ca]
            removed = [x for x in ca if x not in cb]
            what.append("choices" + (f" +{added}" if added else "") + (f" -{removed}" if removed else "")
                        + ("" if added or removed else " reordered"))
        if what:
            out.append({"kind": "field", "change": "changed", "name": name, "label": b.get("label"), "what": what})

    def key(r: Mapping[str, Any]) -> str:
        return f"{r.get('id')}/{'rev' if r.get('is_reversible') else 'fixed'}"

    orules = {key(r): r for r in old.get("business_rules") or []}
    nrules = {key(r): r for r in new.get("business_rules") or []}
    for k in sorted(nrules.keys() - orules.keys()):
        out.append({"kind": "rule", "change": "added", "id": k, "name": nrules[k].get("name")})
    for k in sorted(orules.keys() - nrules.keys()):
        out.append({"kind": "rule", "change": "removed", "id": k, "name": orules[k].get("name")})
    for k in sorted(orules.keys() & nrules.keys()):
        a, b = orules[k], nrules[k]
        what = [f for f in sorted(set(a) | set(b))
                if f not in ("updated_at", "created_at") and canonical_json(a.get(f)) != canonical_json(b.get(f))]
        if what:
            out.append({"kind": "rule", "change": "changed", "id": k, "name": b.get("name"), "what": what})
    return out


@form_app.command("diff", epilog="[bold]Examples:[/bold]  fsv form diff snap.json  |  fsv form diff old.json --against new.json")
def diff(
    snapshot: str = typer.Argument(..., help="snapshot from `fsv form dump`"),
    against: Optional[str] = typer.Option(None, "--against", help="compare with this snapshot instead of the server"),
    output: str = typer.Option("table", "--output", "-o", help="table|json"),
) -> None:
    """Compare a snapshot with the current metadata (or another snapshot)."""
    def read(path: str) -> dict[str, Any]:
        try:
            with open(path, encoding="utf-8") as fh:
                data = json.load(fh)
        except (OSError, ValueError) as e:
            _fail(f"cannot read snapshot {path}: {e}")
            raise
        if not isinstance(data, dict) or data.get("fsv_form_snapshot") != SNAPSHOT_VERSION:
            _fail(f"{path} is not an fsv form snapshot (make one with `fsv form dump`)")
        return data

    old = read(snapshot)
    if against:
        new = read(against)
    else:
        c = _client()
        mode = "edit" if old.get("form_type") == "edit_form" else "new"
        meta = _guard(lambda: load_meta(c, old.get("module") or "change", mode, fresh=True))
        new = {"fields": meta.raw_fields, "business_rules": meta.rules.get("business_rules") or []}
    changes = diff_snapshots(old, new)
    if output == "json":
        print(json.dumps(changes, indent=2, default=str, ensure_ascii=False))
        return
    if not changes:
        console.print("no changes", highlight=False)
        return
    sign = {"added": "[green]+[/green]", "removed": "[red]-[/red]", "changed": "[yellow]~[/yellow]"}
    for ch in changes:
        ident = ch.get("label") or ch.get("name") or ""
        ref = ch.get("name") if ch["kind"] == "field" else ch.get("id")
        line = f"{sign[ch['change']]} {ch['kind']} {escape(str(ident))} [dim]({escape(str(ref))})[/dim]"
        if ch.get("what"):
            line += ": " + escape("; ".join(map(str, ch["what"])))
        console.print(line, highlight=False)
    console.print(f"[dim]{len(changes)} difference(s)[/dim]", highlight=False)
