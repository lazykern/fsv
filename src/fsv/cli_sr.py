"""``fsv sr``: service requests from the service catalog.

* ``catalog``: categories and items (name, short description, id)
* ``fields``:  an item's form as the web shows it
* ``create``:  draft -> review -> confirm -> upload -> place request
* ``get``:     a request's item values (by label) and attachments

Mounted by ``fsv.cli`` as ``fsv sr``.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
from typing import Any, Callable, List, Optional

import typer
from rich.markup import escape
from rich.table import Table

from fsv import catalog as cat
from fsv import change_flow as flow
from fsv import config, yamlio
from fsv.change_flow import FlowExit
from fsv.drafts import (
    Draft,
    _kind_text,
    _kv,
    _label_unique,
    _one_line,
    _value_for_yaml,
    load_draft_file,
    load_preset,
    parse_sets,
    draft_from_data,
)
from fsv.forms import Field, FormError, FormState, LookupValue
from fsv.render import console, err
from fsv.review import NO_PAYLOAD

app = typer.Typer(
    name="sr",
    help="service requests: browse the catalog, see an item's form, place and read requests",
    no_args_is_help=True,
)

MAX_CHOICES_SHOWN = 12


def _client() -> Any:
    from fsv.cli import _client as cli_client

    return cli_client()


def _fail(msg: str, code: int = 2) -> None:
    err.print(f"[red]error[/red]: {escape(msg)}", highlight=False)
    raise typer.Exit(code)


def _guard(fn: Callable[[], Any]) -> Any:
    from fsv.errors import APIError, SessionError, exit_code_for

    try:
        return fn()
    except FlowExit as e:
        if e.message:
            err.print(f"[red]error[/red]: {escape(e.message)}" if e.code else escape(e.message), highlight=False)
        raise typer.Exit(e.code) from None
    except FormError as e:
        _fail(str(e), 2)
    except (SessionError, APIError, ValueError) as e:
        _fail(" ".join(str(e).split()), exit_code_for(e))


def _json(obj: Any) -> None:
    print(json.dumps(obj, indent=2, default=str, ensure_ascii=False))


# ---------------------------------------------------------------------------
# catalog


@app.command("catalog", epilog=(
    "[bold]Examples:[/bold]  fsv sr catalog  |  fsv sr catalog --category Access  |  "
    "fsv sr catalog --search laptop  |  fsv sr catalog --popular -o json"
))
def catalog_cmd(
    category: Optional[str] = typer.Option(None, "--category", "-c", help="category name or id"),
    search: Optional[str] = typer.Option(None, "--search", "-s", help="words in the name or description"),
    popular: bool = typer.Option(False, "--popular", help="only frequently requested items"),
    categories_only: bool = typer.Option(False, "--categories", help="list the categories only"),
    output: str = typer.Option("table", "--output", "-o", help="table|json"),
) -> None:
    """List catalog categories and items (name, short description, id)."""
    c = _client()
    cats = _guard(lambda: cat.list_categories(c))
    if categories_only:
        if output == "json":
            _json(cats)
            return
        t = Table(title="Service catalog categories")
        for col in ("ID", "Category", "Description"):
            t.add_column(col, overflow="fold")
        for x in cats:
            t.add_row(str(x.get("id")), escape(str(x.get("name") or "")), escape(_one_line(x.get("description") or "")))
        console.print(t)
        return
    items = _guard(lambda: cat.list_items(c, popular=popular))
    ids = None
    if category:
        picked = _guard(lambda: cat.resolve_category(cats, category))
        ids = [x.get("id") for x in picked]
    rows = cat.filter_items(items, category_ids=ids, search=search, categories=cats)
    names = {x.get("id"): str(x.get("name") or "") for x in cats}
    for r in rows:
        r.setdefault("category_name", names.get(r.get("category_id"), ""))
    rows.sort(key=lambda r: (cat.norm(r.get("category_name") or ""), cat.norm(r.get("name") or "")))
    if output == "json":
        _json([{"display_id": r.get("display_id"), "id": r.get("id"), "name": r.get("name"),
                "short_description": r.get("short_description"), "category_id": r.get("category_id"),
                "category": r.get("category_name")} for r in rows])
        return
    t = Table(title="Service catalog" + (" (frequently requested)" if popular else ""))
    for col in ("ID", "Item", "Category", "Short description"):
        t.add_column(col, overflow="fold")
    for r in rows:
        t.add_row(str(r.get("display_id")), escape(str(r.get("name") or "")), escape(str(r.get("category_name") or "")),
                  escape(_one_line(r.get("short_description") or "")))
    console.print(t)
    if not rows:
        err.print("[yellow]no items match[/yellow]", highlight=False)
    else:
        err.print(f"[dim]{len(rows)} item(s); see one with: fsv sr fields ID[/dim]", highlight=False)


# ---------------------------------------------------------------------------
# fields


def _reveals(spec: Any, f: Field) -> dict[str, list[str]]:
    """normalised choice value -> labels of the fields that choice reveals."""
    out: dict[str, list[str]] = {}
    for s in f.sections:
        for ch in s.choices:
            out.setdefault(ch, []).extend(spec.by_name[n].label for n in s.fields if n in spec.by_name)
    return out


def _attachment_rule(item: Any) -> str:
    if not item.allow_attachments:
        return "not accepted"
    return "required: at least one general file" if item.attachment_mandatory else "optional"


def fields_doc(meta: cat.ItemMeta) -> dict[str, Any]:
    spec = meta.spec
    item = spec.item
    assert item is not None
    state = FormState(spec, meta.engine(), mode="new")
    ev = state.evaluated
    diagnostics, blockers = cat.rule_diagnostics(meta, state)
    uncertain = bool(blockers or cat.rules_warnings(meta.rules))
    effective = None if uncertain else (item.attachment_mandatory if ev.attachments_required is None
                                         else ev.attachments_required)
    rule = _attachment_rule(item)
    if item.allow_attachments and uncertain:
        rule += " (base; effective requirement unknown)"
    elif item.allow_attachments and ev.attachments_required is not None:
        rule += "; effective at load: " + ("required" if effective else "optional") + " (business rule)"
    rows = []
    for f in spec.ordered():
        sp = spec.section_parent(f.api_name)
        visible = f.api_name in ev.visible
        d = ev.decision(f.api_name, "visible")
        row: dict[str, Any] = {
            "name": f.api_name, "label": f.label, "kind": f.kind,
            "required": f.api_name in ev.required if visible else f.required_base,
            "visible_at_load": visible,
            "hidden_by_rule": (d.rule_name or d.rule_id) if d is not None and not visible and d.action == "hide" else None,
            "placeholder": f.placeholder or None, "help": f.help or None,
        }
        if f.kind == "notice":  # read-only information block, never an input
            row["content"] = " ".join(cat.plain_text(f.content).split())
            row["required"] = False
            row["input"] = False
        if sp is not None:
            parent, choices = sp
            labels = [c.label for c in parent.choices if cat.norm(c.value) in {cat.norm(x) for x in choices}]
            row["shown_when"] = {"field": parent.label, "choices": labels or list(choices)}
        if f.choices:
            rv = _reveals(spec, f)
            row["choices"] = [{"value": c.value, "label": c.label, "description": c.description or None,
                               "reveals": rv.get(cat.norm(c.value), [])} for c in f.choices if c.visible]
        if f.kind == "attachment":
            row["allowed_file_types"] = list(f.allowed_file_types)
            row["max_files"] = f.max_files
        rows.append(row)
    return {
        "item": {"id": item.id, "display_id": item.display_id, "name": item.name,
                 "short_description": item.short_description, "category": item.category_name,
                 "description": cat.plain_text(item.description_html),
                 "attachments": {"allowed": item.allow_attachments, "mandatory": item.attachment_mandatory,
                                 "effective_mandatory": effective if item.allow_attachments else False,
                                 "rule": rule},
                 "subject_template": item.subject_template or None},
        "fields": rows,
        "rules": len(getattr(meta.engine(), "rules", []) or []),
        "warnings": list(spec.warnings) + diagnostics + blockers + [cat.REQUESTED_FOR_NOTE],
        "metadata": meta.as_of(),
    }


@app.command("fields", epilog=(
    "[bold]Examples:[/bold]  fsv sr fields 77  |  fsv sr fields 'Feature Request'  |  fsv sr fields 77 -o json"
))
def fields_cmd(
    item: str = typer.Argument(..., metavar="ITEM", help="catalog item id or name"),
    output: str = typer.Option("table", "--output", "-o", help="table|json"),
) -> None:
    """Show an item's form like the web: labels, required, help, choices,
    fields revealed by a choice, and the attachment rules."""
    c = _client()
    meta = _guard(lambda: cat.load_item_meta(c, item))
    doc = _guard(lambda: fields_doc(meta))
    if output == "json":
        _json(doc)
        return
    it = doc["item"]
    console.print(f"[bold]{escape(it['name'])}[/bold]  [dim]#{it['display_id']}"
                  f"{' · ' + escape(it['category']) if it['category'] else ''}[/dim]", highlight=False)
    if it["short_description"]:
        console.print(escape(it["short_description"]), highlight=False)
    if it["description"]:
        console.print(f"[dim]{escape(_one_line(it['description'], 600))}[/dim]", highlight=False)
    console.print(f"Attachments: {escape(it['attachments']['rule'])}", highlight=False)
    t = Table(show_lines=False)
    for col in ("Field", "Required", "Type", "Details"):
        t.add_column(col, overflow="fold")
    for r in doc["fields"]:
        if r["kind"] == "notice":
            t.add_row(f"[dim]{escape(r['label'])}[/dim]", "", "note", escape(_one_line(r.get("content") or "", 400)))
            continue
        indent = "  ↳ " if r.get("shown_when") else ""
        req = "[bold]yes[/bold]" if r["required"] else "no"
        if r.get("shown_when"):
            req += " [dim](when shown)[/dim]" if r["required"] else ""
        kind = "file(s)" if r["kind"] == "attachment" else r["kind"]
        details: list[str] = []
        if r.get("shown_when"):
            sw = r["shown_when"]
            details.append(f"shown when {sw['field']} = {' / '.join(sw['choices'])}")
        if r.get("hidden_by_rule"):
            details.append(f"hidden at load by rule {r['hidden_by_rule']}")
        if r.get("help"):
            details.append(_one_line(r["help"]))
        if r.get("placeholder"):
            details.append(f"e.g. {_one_line(r['placeholder'], 80)}")
        for ch in r.get("choices") or []:
            line = f"• {ch['label']}"
            if ch.get("description"):
                line += f" — {ch['description']}"
            if ch.get("reveals"):
                line += f" → reveals: {', '.join(ch['reveals'])}"
            details.append(line)
        if r["kind"] == "attachment":
            types = ", ".join(r.get("allowed_file_types") or []) or "any type"
            mx = r.get("max_files")
            details.append(f"types: {types}" + (f" · at most {mx} file(s)" if mx else ""))
        t.add_row(escape(indent + r["label"]), req, kind, escape("\n".join(details)))
    console.print(t)
    for w in doc["warnings"]:
        err.print(f"[yellow]warning[/yellow]: {escape(w)}", highlight=False)
    md = doc["metadata"]
    console.print(f"[dim]item form as of {escape(str(md.get('updated_at')))}; {doc['rules']} business rule(s) "
                  f"apply. Create with: fsv sr create {it['display_id']} (interactive form)[/dim]", highlight=False)


# ---------------------------------------------------------------------------
# create: drafts


def render_sr_template(plan: cat.SRPlan, *, errors: list[str] | None = None, requester: str | None = None,
                       base_dir: str | None = None) -> str:
    """The annotated YAML draft for an item: visible fields in web order
    with ``# REQUIRED · type · help`` and choices above each; file fields
    take paths (relative to ``base_dir``, where the draft is saved; default
    the current directory); general attachments and the requester at the
    top level."""
    state, draft, meta = plan.state, plan.draft, plan.meta

    def rel(p: str) -> str:
        return os.path.relpath(p, base_dir) if base_dir else os.path.relpath(p)

    spec = state.spec
    item = spec.item
    assert item is not None
    ev = state.evaluated
    ref = meta.display_id
    lines = [f"# New service request: {item.name} (catalog item {ref})"]
    for e in errors or []:
        lines.append(f"# ERROR: {' '.join(str(e).split())}")
    lines += [
        "# Fill in values after the colons. Lines starting with # are ignored.",
        "# Fields appear with your answers, like the web form; after saving, new",
        "# required fields are added and the file reopens if needed.",
        f"# Full item form: fsv sr fields {ref}",
        "# Save and close to continue; close without changes to abort.",
        f"# {cat.SAFETY}",
        f"# {cat.REQUESTED_FOR_NOTE}",
        "",
        "# REQUIRED · email or exact name of the requester (approvals route via their manager)",
    ]
    who = requester or (plan.requester.email if plan.requester is not None else None) or draft.requester
    lines.append(f"requester: {yamlio.scalar(who)}" if who else "requester:")
    lines.append("# Optional · exact email or name of the person receiving this request")
    lines.append(f"requested_for: {yamlio.scalar(draft.requested_for)}" if draft.requested_for else "requested_for:")
    lines.append("answers:")
    shown = False
    for name in ev.visible:
        f = spec.by_name[name]
        if f.kind == "notice":
            text = _one_line(f.content or f.label, 300)
            if text:
                lines.append(f"  # NOTE: {text}")
            continue
        key = f.label if _label_unique(spec, f) else f.api_name
        meta_bits = ["REQUIRED" if name in ev.required else "optional"]
        if f.kind == "attachment":
            meta_bits.append("file(s): paths relative to this draft")
            if f.allowed_file_types:
                meta_bits.append("types " + ", ".join(f.allowed_file_types))
            if f.max_files:
                meta_bits.append(f"at most {f.max_files} file(s)")
            lines.append("  # " + " · ".join(meta_bits))
            have = [rel(str(p)) for p in ev.values.get(name) or ()]
            lines.append(f"  {yamlio.key(key)}: {yamlio.flow_list(have)}")
            shown = True
            continue
        meta_bits.append(_kind_text(f))
        if f.help:
            meta_bits.append(_one_line(f.help))
        if name in ev.disabled:
            meta_bits.append("read-only (set by a rule)")
        lines.append("  # " + " · ".join(meta_bits))
        if f.choices:
            pool = [c for c in state.choices(name) if c.visible]
            labels = [c.label for c in pool]
            more = f" | … ({len(labels)} total; fsv sr fields {ref})" if len(labels) > MAX_CHOICES_SHOWN else ""
            lines.append(f"  # choices: {' | '.join(labels[:MAX_CHOICES_SHOWN])}{more}")
            for c in pool[:MAX_CHOICES_SHOWN]:
                if c.description:
                    lines.append(f"  #   {c.label} — {_one_line(c.description, 120)}")
            rv = _reveals(spec, f)
            for c in pool[:MAX_CHOICES_SHOWN]:
                if rv.get(cat.norm(c.value)):
                    lines.append(f"  #   {c.label} → adds: {', '.join(rv[cat.norm(c.value)])}")
        if f.placeholder and not f.choices:
            lines.append(f"  # e.g. {_one_line(f.placeholder, 80)}")
        lines.append(_kv(key, _value_for_yaml(state, f)))
        shown = True
    for k, v in draft.answers.items():
        f = spec.find(k)
        if f is None:
            lines.append("  # not a field on this item: fix the label or delete the line")
            lines.append(_kv(k, v))
            shown = True
        elif f.api_name not in ev.visible and f.kind != "notice":
            lines.append("  # hidden now (its section's choice is not selected): not sent")
            lines.append(_kv(k, v))
            shown = True
    if not shown:
        lines.append("  {}")
    lines.append("")
    if item.allow_attachments:
        need = ev.attachments_required
        mandatory = item.attachment_mandatory if need is None else need
        lines.append(f"# {'REQUIRED' if mandatory else 'optional'} · general attachments for the request "
                     "(uploaded only after you confirm)")
        lines.append("attachments: " + yamlio.flow_list([rel(p) for p in draft.attachments]))
    else:
        lines.append("# This item does not accept general attachments.")
    blocking = [m for m in ev.messages if m.blocking]
    if blocking:
        lines.append("")
        for m in blocking:
            lines.append(f"# BLOCKING MESSAGE: {_one_line(m.text)}")
        lines.append("# Acknowledge with: ack: [all]")
        lines.append("ack: " + yamlio.flow_list(draft.ack))
    return "\n".join(lines) + "\n"


def resolve_sr_draft(spec: Any, file: str | None, preset: str | None, sets: list[str] | None,
                     attach: list[str] | None) -> Draft:
    """preset < file < --set < --attach. File-field paths resolve against
    the draft (or preset) file's folder, ``--set`` paths against the
    current directory."""
    d = load_draft_file(file, allow_requested_for=True) if file else Draft()
    name = preset or d.preset
    base = load_preset(name, allow_requested_for=True) if name else Draft()
    base.answers = cat.file_answers(spec, base.answers, base.base_dir)
    d.answers = cat.file_answers(spec, d.answers, d.base_dir)
    sets_d = Draft(answers=cat.file_answers(spec, parse_sets(sets), os.getcwd()))
    out = base.merged(d).merged(sets_d)
    if attach:
        out.attachments = [*out.attachments, *[os.path.abspath(os.path.expanduser(p)) for p in attach]]
    out.preset = name
    return out


def _yaml_norm(v: Any) -> Any:
    if isinstance(v, list):
        return [cat.norm(x) for x in v]
    return None if v in (None, "") else cat.norm(v)


def parse_sr_template(text: str, base_dir: Path | None = None) -> Draft:
    return draft_from_data(yamlio.loads(text), base_dir or Path.cwd(), where="draft", allow_requested_for=True)


def edit_loop(build: Callable[[Draft], cat.SRPlan], draft: Draft, *,
              edit_text: Callable[..., Any] | None = None, max_rounds: int = 20) -> tuple[Draft, cat.SRPlan]:
    """Open the annotated draft; reopen it with the errors on top until the
    form is complete. Raises EditorAbort."""
    from fsv.editor import EditorAbort, edit_text as _edit_text

    edit = edit_text or _edit_text
    plan = build(draft)
    errors: list[str] = []
    for _ in range(max_rounds):
        text = render_sr_template(plan, errors=errors)
        baseline = parse_sr_template(text).answers
        new = edit(text, parse_sr_template, suffix=".yaml", comment_prefix="#")
        keep = set(draft.answers)
        answers = {k: v for k, v in new.answers.items() if k in keep or _yaml_norm(v) != _yaml_norm(baseline.get(k))}
        answers = cat.file_answers(plan.state.spec, answers, new.base_dir)
        draft = Draft(answers=answers, requester=new.requester, requested_for=new.requested_for,
                      description_html=new.description_html,
                      attachments=new.attachments, planning=new.planning, ack=new.ack, preset=draft.preset,
                      base_dir=new.base_dir)
        plan = build(draft)
        if plan.ok:
            return draft, plan
        errors = list(plan.errors)
    raise EditorAbort("aborted: too many rounds with errors")


# ---------------------------------------------------------------------------
# create: requester, review, send


class _Requester:
    """--requester > draft ``requester:`` > config ``defaults.sr_requester``
    > (terminal) a prompt. Never the logged-in user by default."""

    def __init__(self, client: Any, flag: str | None, tty: bool) -> None:
        self.client = client
        self.flag = flag
        self.tty = tty
        self.prompted: str | None = None
        self.cache: dict[str, LookupValue] = {}

    def value(self, draft: Draft) -> str | None:
        v = self.flag or draft.requester or self.prompted
        if not v:
            v = str((config.load_config().get("defaults") or {}).get("sr_requester") or "") or None
        return v

    def ask(self) -> None:
        self.prompted = typer.prompt("Requester (email; approvals route via their manager)", default="",
                                     show_default=False).strip() or None
        if not self.prompted:
            raise FlowExit(2, "no requester given; nothing was sent")

    def resolve(self, draft: Draft) -> tuple[LookupValue | None, str | None]:
        from fsv.change_cli import _pick
        from fsv.formmeta import resolve_requester

        v = self.value(draft)
        if not v:
            return None, None
        if v not in self.cache:
            try:
                self.cache[v] = resolve_requester(self.client, v, _pick if self.tty else None)
            except ValueError as e:
                return None, str(e)
        return self.cache[v], None


def _print_plan(plan: cat.SRPlan, *, as_json: bool) -> None:
    if as_json:
        _json(plan.to_json())
        return
    plan.review.render(console)
    console.print(f"[bold red]⚠ {escape(cat.SAFETY)}[/bold red]", highlight=False)
    if plan.payload is None or not plan.ok:
        console.print(f"[dim]{NO_PAYLOAD}[/dim]", highlight=False)
    else:
        console.print(f"[bold]Payload[/bold] (POST /api/_/catalog/service-items/{plan.meta.display_id}/place-request; "
                      "<upload N: name> becomes the attachment id after upload)", highlight=False)
        print(json.dumps(plan.payload, indent=2, default=str, ensure_ascii=False))
    if plan.hash:
        console.print(f"plan_hash: {plan.hash}", highlight=False)
        console.print(f"[dim]send exactly this with: --yes --expect-plan {plan.hash[:16]}[/dim]", highlight=False)


def _confirm(plan: cat.SRPlan, draft: Draft, build: Callable[[Draft, bool], cat.SRPlan]) -> tuple[cat.SRPlan, Draft]:
    """[y] place request / [e] edit / [s] save draft / [n] cancel."""
    from fsv.change_cli import _menu
    from fsv.editor import EditorAbort

    ack = False
    while True:
        plan.review.render(err)
        err.print(f"[bold red]⚠ {escape(cat.SAFETY)}[/bold red]", highlight=False)
        if plan.ok:
            choice = _menu("[y] place request  [e] edit  [s] save draft  [n] cancel")
        else:
            choice = _menu("Problems must be fixed first: [e] edit  [s] save draft  [n] cancel")
            if choice == "y":
                continue
        if choice == "y":
            if plan.review.blocking_unacked:
                for m in plan.review.blocking_unacked:
                    err.print(f"[magenta]blocking rule message:[/] {escape(m)}", highlight=False)
                if not typer.confirm("Acknowledge and continue?", default=False):
                    continue
                ack = True
                plan = build(draft, ack)
                if not plan.ok:
                    continue
            return plan, draft
        if choice == "e":
            try:
                draft, plan = edit_loop(lambda d, a=ack: build(d, a), draft)
            except EditorAbort as e:
                err.print(f"[yellow]{escape(str(e))}[/yellow]; keeping the previous draft", highlight=False)
            continue
        if choice == "s":
            path = typer.prompt("Save draft to", default="sr-draft.yaml")
            if os.path.exists(path) and not typer.confirm(f"{path} exists; overwrite?", default=False):
                continue
            dest = Path(os.path.expanduser(path))
            dest.write_text(render_sr_template(plan, base_dir=str(dest.resolve().parent)), encoding="utf-8")
            err.print(f"[green]saved[/green] {escape(path)}; continue later with --from {escape(path)}",
                      highlight=False)
            raise FlowExit(0)
        raise FlowExit(1, "cancelled; nothing was uploaded or sent")


def _send(plan: cat.SRPlan, client: Any, *, rebuild: Callable[[cat.ItemMeta], cat.SRPlan], tty: bool, yes: bool,
          expect_plan: str | None, allow_duplicate: bool) -> dict[str, Any]:
    cat.duplicate_check(plan, allow_duplicate)
    if plan.identity:
        checked = rebuild(plan.meta)
        if not checked.ok or checked.hash != plan.hash:
            raise FlowExit(4, "recipient identities changed since review; review again")
    new, diff = cat.freshen(plan, client, rebuild)
    if diff is not None:
        err.print("[yellow]the catalog item or its rules changed since the review; the plan is now different:[/yellow]",
                  highlight=False)
        for line in diff:
            err.print(f"  {escape(line)}", highlight=False)
        if not new.ok:
            new.review.render(err)
            raise FlowExit(2, "the updated form needs more answers: " + "; ".join(new.errors))
        if expect_plan or yes or not tty:
            raise FlowExit(4, f"plan changed to {new.hash or '(incomplete)'}; review it again with --dry-run")
        new.review.render(err)
        if not typer.confirm("Place the updated request?", default=False):
            raise FlowExit(1, "cancelled; nothing was uploaded or sent")
        cat.duplicate_check(new, allow_duplicate)
    flow.expect(new, expect_plan)  # type: ignore[arg-type]
    try:
        return cat.execute(new, client, allow_duplicate=allow_duplicate)
    except cat.SendFailure as e:
        from fsv.errors import exit_code_for

        if e.uploaded:
            err.print("uploaded attachment ids (not attached to any request): "
                      + ", ".join(f"{u['id']} ({u['name']})" for u in e.uploaded), highlight=False)
        code = exit_code_for(e.error)
        if e.step == "upload":
            what = "nothing was placed"
        elif isinstance(e.error, cat.APIError) and 400 <= e.error.status < 500 and e.error.status not in (403, 408, 429):
            what = "the server refused it; nothing was placed"
        else:
            what = "the outcome is unknown: check `fsv journal tail` and the requester's requests before retrying"
        detail = f"HTTP {e.error.status}" if isinstance(e.error, cat.APIError) else type(e.error).__name__
        raise FlowExit(code, f"{e.step} failed: {detail}; {what}") from None


def create_command(
    client_factory: Callable[[], Any],
    item_ref: str,
    *,
    from_file: str | None,
    sets: list[str] | None,
    preset: str | None,
    edit: bool,
    requester: str | None,
    attach: list[str] | None,
    dry_run: bool,
    as_json: bool,
    yes: bool,
    expect_plan: str | None,
    allow_duplicate: bool,
    ack: bool,
    no_input: bool,
    requested_for: str | None = None,
    cc: list[str] | None = None,
) -> None:
    from fsv.editor import EditorAbort
    from fsv.formmeta import ClientResolver

    from fsv.change_cli import _run_form, form_tty

    tty = flow.interactive() and not no_input
    has_input = bool(from_file or sets or preset or attach)
    template_only = False
    form = False
    if not has_input and not edit:
        if tty and form_tty():
            form = True  # the in-terminal item form (review inside; --dry-run sends nothing)
        elif dry_run:
            template_only = True
        elif tty:
            edit = True  # stdout is not a terminal: fall back to the YAML draft in $EDITOR
        else:
            raise FlowExit(2, "nothing to request: pass --from FILE or --set LABEL=VALUE (the interactive form and "
                              "--edit need a terminal); start with `fsv sr create ITEM --dry-run` to get a draft")
    if (form or template_only) and (requested_for is not None or cc):
        raise FlowExit(2, "--requested-for and --cc require --from, --set, --preset or --edit; the interactive form and template cannot retain them")
    if edit and not tty:
        raise FlowExit(2, "--edit needs a terminal (no TTY or --no-input); use --from FILE")

    client = client_factory()
    meta = cat.load_item_meta(client, item_ref)
    try:
        draft = resolve_sr_draft(meta.spec, from_file, preset, sets, attach)
    except ValueError as e:
        raise FlowExit(2, str(e)) from None
    who = _Requester(client, requester, tty)
    if tty and not form and not edit and not template_only and not who.value(draft):
        who.ask()
    resolver = ClientResolver(client)

    def build(d: Draft, ack_all: bool = ack, m: cat.ItemMeta | None = None) -> cat.SRPlan:
        req, problem = who.resolve(d)
        from fsv.formmeta import resolve_requester
        from fsv.ticket_notes import _EMAIL, _valid_recipient
        target = None
        beneficiary = requested_for if requested_for is not None else d.requested_for
        if beneficiary is not None:
            try:
                target = resolve_requester(client, beneficiary)
            except ValueError as e:
                raise FlowExit(2, f"requested-for: {e}") from None
        resolved_cc = []
        resolved_cc_ids = []
        for email in cc or ():
            if not _valid_recipient(email) or not _EMAIL.fullmatch(email):
                raise FlowExit(2, "CC must be a plain email address")
            try:
                person = resolve_requester(client, email)
                if not person.email or person.email.casefold() != email.casefold():
                    raise ValueError("not an exact email match")
                resolved_cc.append(person.email)
                resolved_cc_ids.append(person.id)
            except ValueError:
                raise FlowExit(2, "each CC email must resolve to exactly one non-deleted user") from None
        plan = cat.build_sr_plan(m or meta, d, resolver=resolver, requester=req, ack_all=ack_all,
                                 requested_for=target, cc_emails=tuple(resolved_cc), cc_ids=tuple(resolved_cc_ids))
        if problem:
            plan.errors[:] = [problem] + [e for e in plan.errors if e != cat.REQUESTER_MISSING]
            plan.review.errors = plan.errors
            plan.hash = plan.review.plan_hash = ""
        return plan

    if template_only:
        print(render_sr_template(build(draft)), end="")
        err.print(f"[dim]fill this in, then: fsv sr create {meta.display_id} --from FILE --dry-run[/dim]",
                  highlight=False)
        return
    session = None
    try:
        if form:
            from fsv.formui.session import sr_session

            req = None
            if requester:  # --requester preselects; never the logged-in user
                req, problem = who.resolve(draft)
                if problem:
                    err.print(f"[yellow]--requester[/yellow]: {escape(problem)}", highlight=False)
            session = sr_session(client, meta=meta, requester=req, dry_run=dry_run)
            try:
                plan = _run_form(session)
            except FlowExit as e:
                if e.code == 1:
                    raise FlowExit(1, "cancelled; nothing was uploaded or sent") from None
                raise
            draft = plan.draft
        elif edit:
            draft, plan = edit_loop(lambda d: build(d), draft)
        else:
            plan = build(draft)
    except EditorAbort as e:
        raise FlowExit(1, str(e)) from None
    if session is not None:
        _s = session

        def build(d: Draft, ack_all: bool = ack, m: cat.ItemMeta | None = None) -> cat.SRPlan:  # noqa: F811
            return _s.build_plan(ack_all or _s.ack, m)

    if dry_run:
        _print_plan(plan, as_json=as_json)
        if not plan.ok:
            raise FlowExit(2)
        flow.expect(plan, expect_plan)  # type: ignore[arg-type]
        return

    if session is not None:
        flow.expect(plan, expect_plan)  # type: ignore[arg-type]  # reviewed and confirmed in the form
    elif tty and not yes:
        plan, draft = _confirm(plan, draft, lambda d, a: build(d, a or ack))
    else:
        if not tty and not yes:
            plan.review.render(err)
            raise FlowExit(2, "not a terminal: review with --dry-run, then send with --yes --expect-plan HASH")
        if not plan.ok:
            plan.review.render(err)
            raise FlowExit(2, "cannot place the request: " + "; ".join(plan.errors))
        if plan.review.blocking_unacked:
            plan.review.render(err)
            raise FlowExit(2, "blocking rule message(s) need --ack: " + "; ".join(plan.review.blocking_unacked))
        flow.expect(plan, expect_plan)  # type: ignore[arg-type]
    ack_now = ack or bool(plan.state.acknowledged)
    final = plan
    result = _send(plan, client, rebuild=lambda m: build(final.draft, ack_now, m), tty=tty, yes=yes,
                   expect_plan=expect_plan, allow_duplicate=allow_duplicate)
    ref = result["created"]
    if as_json:
        _json({"created": ref, "result": result["result"], "plan_hash": result["plan_hash"],
               "uploaded": result["uploaded"], "steps": result["steps"], "verification": result["verification"]})
        return
    url = cat.domain_url(f"/a/tickets/{ref['id']}") if ref.get("id") is not None else ""
    for u in result["uploaded"]:
        console.print(f"  [green]ok[/green] uploaded {escape(u['name'])} -> {u['id']} ({escape(u['target'])})",
                      highlight=False)
    console.print(f"[green]placed[/green] {escape(ref['display'])}  {url}", highlight=False)
    if result["verification"].get("warning"):
        err.print(result["verification"]["warning"], highlight=False)


@app.command("create", epilog=(
    "[bold]Examples:[/bold]  fsv sr create 77 --dry-run > sr.yaml  |  "
    "fsv sr create 77 --from sr.yaml --dry-run  |  "
    "fsv sr create 77 --from sr.yaml --yes --expect-plan 1a2b3c4d5e6f  |  "
    "fsv sr create 'Feature Request' --edit"
))
def create_cmd(
    item: str = typer.Argument(..., metavar="ITEM", help="catalog item id or name"),
    from_: Optional[str] = typer.Option(None, "--from", help="draft YAML (requester, answers, attachments)"),
    set_: Optional[List[str]] = typer.Option(None, "--set", help="answer LABEL=VALUE (repeatable; file fields take a path)"),
    preset: Optional[str] = typer.Option(None, "--preset", help="preset name or path (partial answers)"),
    edit: bool = typer.Option(False, "--edit", help="advanced: the annotated YAML draft in $EDITOR (default in a terminal: the interactive form)"),
    requester: Optional[str] = typer.Option(None, "--requester", help="requester email or exact name"),
    requested_for: Optional[str] = typer.Option(None, "--requested-for", help="exact user email or name"),
    cc: Optional[List[str]] = typer.Option(None, "--cc", help="plain CC email (repeatable)"),
    attach: Optional[List[str]] = typer.Option(None, "--attach", help="general attachment (repeatable)"),
    dry_run: bool = typer.Option(False, "--dry-run", help="show the review, payload and plan hash; send nothing"),
    output: str = typer.Option("table", "--output", "-o", help="table|json (with --dry-run: the plan)"),
    yes: bool = typer.Option(False, "--yes", "-y", help="place without the prompt (required without a terminal)"),
    expect_plan: Optional[str] = typer.Option(None, "--expect-plan", help="refuse unless the plan hash matches"),
    allow_duplicate: bool = typer.Option(False, "--allow-duplicate", help="send a plan the journal says was sent"),
    ack: bool = typer.Option(False, "--ack", help="acknowledge blocking rule messages"),
    no_input: bool = typer.Option(False, "--no-input", help="never prompt"),
) -> None:
    """Place a service request from a catalog item (review first; files are
    uploaded only after you confirm)."""
    _guard(lambda: create_command(
        _client, item, from_file=from_, sets=set_, preset=preset, edit=edit, requester=requester, attach=attach,
        dry_run=dry_run, as_json=output == "json", yes=yes, expect_plan=expect_plan,
        allow_duplicate=allow_duplicate, ack=ack, no_input=no_input, requested_for=requested_for, cc=cc))


# ---------------------------------------------------------------------------
# get


def _attachment_rows(ticket: dict[str, Any], items: list[dict[str, Any]],
                     labels: list[dict[str, str]]) -> list[dict[str, Any]]:
    """Ticket, requested-item general and file-field attachments, deduplicated by id."""
    out: list[dict[str, Any]] = []
    by_id: dict[str, dict[str, Any]] = {}
    for a in ticket.get("attachments") or []:
        if isinstance(a, dict):
            row = {**a, "source": "request"}
            out.append(row)
            if a.get("id") is not None:
                by_id[str(a["id"])] = row
    for idx, it in enumerate(items):
        lab = labels[idx]
        for a in it.get("attachments") or []:
            if not isinstance(a, dict):
                continue
            key = str(a["id"]) if a.get("id") is not None else None
            if key is not None and key in by_id:
                continue
            row = {**a, "source": "requested item"}
            out.append(row)
            if key is not None:
                by_id[key] = row
        for name, v in (it.get("custom_fields") or {}).items():
            if not cat.is_file_value(v):
                continue
            for x in v:
                key = str(x.get("id") if isinstance(x, dict) else x)
                src = lab.get(name, name)
                if key in by_id:
                    by_id[key]["source"] = src
                    continue
                row = {**x} if isinstance(x, dict) else {"id": x, "name": f"attachment-{x}"}
                row["source"] = src
                out.append(row)
                by_id[key] = row
    return out


def _download(c: Any, att: dict[str, Any], directory: Path, force: bool) -> dict[str, Any]:
    from fsv.create import _attachment_url

    name = str(att.get("name") or f"attachment-{att.get('id', '')}")
    row = {"id": att.get("id"), "name": name, "source": att.get("source")}
    try:
        target = cat.safe_target(directory, name)
    except ValueError as e:
        return {**row, "status": "rejected", "error": str(e)}
    url = _attachment_url(att)
    if not url and att.get("id") is not None:
        url = cat.domain_url(f"/helpdesk/attachments/{att['id']}?download=true")
    if not url:
        return {**row, "status": "error", "error": "no download URL"}
    if Path(target).exists() and not force:
        return {**row, "path": str(target), "status": "exists"}
    r = c._client.get(url, follow_redirects=True)
    if r.status_code >= 400:
        return {**row, "status": "error", "error": f"HTTP {r.status_code}"}
    Path(directory).expanduser().mkdir(parents=True, exist_ok=True)
    try:
        with open(target, "wb" if force else "xb") as fh:
            fh.write(r.content)
    except FileExistsError:
        # Another process created the file after the preflight check.
        return {**row, "path": str(target), "status": "exists"}
    return {**row, "path": str(target), "size": len(r.content), "status": "downloaded"}


@app.command("get", epilog=(
    "[bold]Examples:[/bold]  fsv sr get SR-123 --items  |  fsv sr get SR-123 --items --attachments  |  "
    "fsv sr get SR-123 --download ./ref -o json"
))
def get_cmd(
    sr_id: str = typer.Argument(..., metavar="SR-ID"),
    items: bool = typer.Option(False, "--items", help="requested item values, by label"),
    attachments: bool = typer.Option(False, "--attachments", help="list attachments"),
    download: Optional[str] = typer.Option(None, "--download", metavar="DIR", help="download attachments into DIR; prefix names starting with - with ./"),
    force: bool = typer.Option(False, "--force", help="overwrite files that exist in DIR"),
    output: str = typer.Option("table", "--output", "-o", help="table|json"),
) -> None:
    """A service request's requested-item values (labels, not internal
    names) and its attachments."""
    from fsv.resources import TICKETS, parse_id
    from fsv.service import get_requested_items

    if download is not None and download.startswith("-"):
        raise typer.BadParameter("expected a directory, not an option; use ./ for names starting with -",
                                 param_hint="--download")
    download_dir = Path(download) if download is not None else None
    if not items and not attachments and download is None:
        items = attachments = True
    tid = _guard(lambda: parse_id(sr_id, TICKETS))
    c = _client()
    raw = _guard(lambda: c.int_get(f"tickets/{tid}"))
    ticket = raw.get("ticket", raw) if isinstance(raw, dict) else {}
    reqs = _guard(lambda: get_requested_items(tid, client=c))
    labels = [_guard(lambda it=it: cat.item_labels(c, it)) for it in reqs]
    names = {str(a.get("id")): str(a.get("name")) for a in ticket.get("attachments") or []
             if isinstance(a, dict) and a.get("name")}
    out_items = []
    for it, lab in zip(reqs, labels):
        meta = it.get("item") or {}
        vals = []
        for name, v in (it.get("custom_fields") or {}).items():
            if v in (None, "", [], False):
                continue
            if cat.is_file_value(v):
                text = ", ".join(str(x.get("name") or x.get("id")) if isinstance(x, dict)
                                 else names.get(str(x), f"attachment {x}") for x in v)
            else:
                text = cat.value_text(v)
            vals.append({"label": lab.get(name, name), "name": name, "value": text, "raw": v})
        stage = it.get("stage")
        out_items.append({"item": meta.get("name"), "display_id": meta.get("display_id"),
                          "stage": stage.get("name") if isinstance(stage, dict) else stage, "values": vals})
    att_rows = _attachment_rows(ticket, reqs, labels) if (attachments or download is not None) else []
    downloads: list[dict[str, Any]] = []
    if download_dir is not None:
        for a in att_rows:
            downloads.append(_guard(lambda a=a: _download(c, a, download_dir, force)))
    shown_id = ticket.get("human_display_id") or f"SR-{ticket.get('display_id') or tid}"
    doc: dict[str, Any] = {"id": shown_id, "subject": ticket.get("subject")}
    if items:
        doc["items"] = out_items
    if attachments or download is not None:
        doc["attachments"] = [{"id": a.get("id"), "name": a.get("name"), "size": a.get("size") or a.get("file_size"),
                               "source": a.get("source")} for a in att_rows]
    if download is not None:
        doc["downloads"] = downloads
    bad = [d for d in downloads if d["status"] in ("rejected", "error")]
    if output == "json":
        for it in doc.get("items", []):
            for v in it["values"]:
                v.pop("raw", None)
        _json(doc)
        if bad:
            raise typer.Exit(2)
        return
    console.rule(f"[bold cyan]{escape(str(shown_id))}[/]  {escape(str(ticket.get('subject') or ''))}")
    if items:
        if not out_items:
            console.print("[yellow]no requested items (not a service request?)[/yellow]", highlight=False)
        for it in out_items:
            console.print(f"[bold]{escape(str(it['item'] or 'requested item'))}[/bold]"
                          + (f"  [dim]#{it['display_id']} · stage {escape(str(it['stage'] or '-'))}[/dim]"),
                          highlight=False)
            t = Table(show_header=False, box=None, pad_edge=False)
            t.add_column(style="cyan", no_wrap=True)
            t.add_column(overflow="fold")
            for v in it["values"]:
                t.add_row(escape(v["label"]), escape(v["value"]))
            console.print(t)
    if attachments or download is not None:
        console.print("[bold]Attachments[/bold]", highlight=False)
        if not att_rows:
            console.print("  (none)", highlight=False)
        for a in doc["attachments"]:
            size = f"{a['size']:,} B" if isinstance(a.get("size"), int) else ""
            console.print(f"  {escape(str(a['name']))}  {size}  [dim]id {a['id']} · {escape(str(a['source']))}[/dim]",
                          highlight=False)
    for d in downloads:
        style = {"downloaded": "green", "exists": "yellow"}.get(d["status"], "red")
        extra = d.get("error") or ""
        console.print(f"  [{style}]{d['status']}[/{style}] {escape(str(d['name']))}  {escape(str(extra))}".rstrip(),
                      highlight=False)
    if download is not None and any(d["status"] in ("downloaded", "exists") for d in downloads):
        console.print(f"Saved to {escape(str(download))}", highlight=False, soft_wrap=True)
    if bad:
        raise typer.Exit(2)


__all__ = ["app", "create_command", "render_sr_template", "fields_doc"]
