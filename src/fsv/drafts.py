"""Change drafts: YAML files, presets, ``--set`` answers, the annotated
editor template, and the local hints file.

Draft file (``--from``), every key optional::

    preset: acme-normal          # name in <config>/presets/ or a path
    requester: jane@example.com
    answers:                     # label or internal name -> value
      Change Type: Normal
      Services: [Billing, Search]
    description_md: |            # or description_html: "<p>...</p>"
      Rotate the **billing** keys.
    attachments: [evidence/plan.pdf]
    planning:                    # planning field label -> files/text
      Rollout Plan:
        description_md: Steps are in the attached file.
        files: [evidence/rollout.xlsx]
    ack: [all]                   # acknowledge blocking rule messages

Presets have the same shape; they are partial answers replayed through the
engine (never payload fragments), so they keep working when the form
changes and are validated like any other answer.

User files live in ``$FSV_CONFIG_DIR`` if set, else ``$XDG_CONFIG_HOME/fsv``,
else ``~/.config/fsv``: ``presets/*.yaml`` and ``hints.yaml``.
"""
from __future__ import annotations

import os
from dataclasses import dataclass, field as dc_field
from pathlib import Path
from typing import Any, Mapping

from fsv import yamlio
from fsv.forms import Field, FormState, LookupValue, plain_text
from fsv.formmeta import find_field, is_description, is_requester, is_status
from fsv.safety import norm

DRAFT_KEYS = {"preset", "requester", "answers", "description_md", "description_html", "attachments", "planning", "ack"}
MAX_CHOICES_SHOWN = 12


def user_dir() -> Path:
    if os.environ.get("FSV_CONFIG_DIR"):
        return Path(os.environ["FSV_CONFIG_DIR"])
    if os.environ.get("XDG_CONFIG_HOME"):
        return Path(os.environ["XDG_CONFIG_HOME"]) / "fsv"
    return Path.home() / ".config" / "fsv"


def presets_dir() -> Path:
    return user_dir() / "presets"


def hints_path() -> Path:
    return user_dir() / "hints.yaml"


@dataclass
class Draft:
    answers: dict[str, Any] = dc_field(default_factory=dict)
    requester: str | None = None
    description_html: str | None = None
    attachments: list[str] = dc_field(default_factory=list)
    planning: dict[str, dict[str, Any]] = dc_field(default_factory=dict)
    ack: list[str] = dc_field(default_factory=list)
    preset: str | None = None
    base_dir: Path | None = None  # relative file paths resolve against this

    def merged(self, other: "Draft") -> "Draft":
        """``other`` wins key by key (answers are merged, not replaced)."""
        return Draft(
            answers={**self.answers, **other.answers},
            requester=other.requester or self.requester,
            description_html=other.description_html if other.description_html is not None else self.description_html,
            attachments=other.attachments or self.attachments,
            planning={**self.planning, **other.planning},
            ack=[*self.ack, *other.ack],
            preset=other.preset or self.preset,
            base_dir=other.base_dir or self.base_dir,
        )


def _path(p: Any, base: Path | None) -> str:
    s = os.path.expanduser(str(p))
    if base is not None and not os.path.isabs(s):
        s = str(base / s)
    return s


def _str_list(v: Any, what: str) -> list[str]:
    if v is None:
        return []
    if isinstance(v, (str, int)):
        return [str(v)]
    if isinstance(v, list) and all(isinstance(x, (str, int)) for x in v):
        return [str(x) for x in v]
    raise ValueError(f"{what}: expected a list of strings")


def draft_from_data(data: Any, base_dir: Path | None = None, where: str = "draft") -> Draft:
    from fsv.mdlite import to_html

    if data is None:
        return Draft(base_dir=base_dir)
    if not isinstance(data, Mapping):
        raise ValueError(f"{where}: top level must be a mapping (preset/answers/description_md/...)")
    unknown = [k for k in data if k not in DRAFT_KEYS]
    if unknown:
        raise ValueError(f"{where}: unknown key(s) {', '.join(map(str, unknown))}; "
                         f"expected: {', '.join(sorted(DRAFT_KEYS))}")
    answers = data.get("answers") or {}
    if not isinstance(answers, Mapping):
        raise ValueError(f"{where}: answers must be a mapping of field label -> value")
    if data.get("description_md") is not None and data.get("description_html") is not None:
        raise ValueError(f"{where}: use description_md or description_html, not both")
    desc = None
    if data.get("description_md") is not None:
        md = str(data["description_md"])
        desc = to_html(md) if md.strip() else None
    elif data.get("description_html") is not None and str(data["description_html"]).strip():
        desc = str(data["description_html"])
    planning: dict[str, dict[str, Any]] = {}
    raw_planning = data.get("planning") or {}
    if not isinstance(raw_planning, Mapping):
        raise ValueError(f"{where}: planning must map a planning field label to files or text")
    for label, spec in raw_planning.items():
        entry: dict[str, Any] = {"files": [], "description_html": None}
        if isinstance(spec, (list, str)) and not (isinstance(spec, str) and "\n" in spec):
            entry["files"] = [_path(p, base_dir) for p in _str_list(spec, f"planning.{label}")]
        elif isinstance(spec, Mapping):
            bad = [k for k in spec if k not in ("files", "description_md", "description_html")]
            if bad:
                raise ValueError(f"{where}: planning.{label}: unknown key(s) {', '.join(map(str, bad))}")
            entry["files"] = [_path(p, base_dir) for p in _str_list(spec.get("files"), f"planning.{label}.files")]
            if spec.get("description_md") is not None:
                entry["description_html"] = to_html(str(spec["description_md"]))
            elif spec.get("description_html") is not None:
                entry["description_html"] = str(spec["description_html"])
        else:
            raise ValueError(f"{where}: planning.{label}: expected files or a mapping")
        planning[str(label)] = entry
    return Draft(
        answers={str(k): v for k, v in answers.items()},
        requester=None if data.get("requester") in (None, "") else str(data["requester"]),
        description_html=desc,
        attachments=[_path(p, base_dir) for p in _str_list(data.get("attachments"), "attachments")],
        planning=planning,
        ack=_str_list(data.get("ack"), "ack"),
        preset=None if not data.get("preset") else str(data["preset"]),
        base_dir=base_dir,
    )


def load_draft_file(path: str) -> Draft:
    p = Path(os.path.expanduser(path))
    try:
        data = yamlio.load_file(str(p))
    except OSError as e:
        raise ValueError(f"cannot read {path}: {e.strerror}") from None
    return draft_from_data(data, p.parent, where=str(path))


def preset_path(name: str) -> Path:
    """A preset by name (``<config>/presets/<name>.yaml``) or by path."""
    cand = Path(os.path.expanduser(name))
    if cand.suffix in (".yaml", ".yml") or os.sep in name or cand.exists():
        if not cand.exists():
            raise ValueError(f"preset file not found: {name}")
        return cand
    for ext in (".yaml", ".yml"):
        p = presets_dir() / f"{name}{ext}"
        if p.exists():
            return p
    have = sorted(p.stem for p in presets_dir().glob("*.y*ml")) if presets_dir().exists() else []
    hint = f"; available: {', '.join(have)}" if have else f"; put presets in {presets_dir()}"
    raise ValueError(f"preset {name!r} not found{hint}")


def load_preset(name: str) -> Draft:
    d = load_draft_file(str(preset_path(name)))
    if d.preset:
        raise ValueError(f"preset {name!r}: presets cannot reference other presets")
    return d


def parse_sets(exprs: list[str] | None) -> dict[str, str]:
    out: dict[str, str] = {}
    for expr in exprs or []:
        if "=" not in expr:
            raise ValueError(f"--set needs LABEL=VALUE, got {expr!r}")
        k, _, v = expr.partition("=")
        if not k.strip():
            raise ValueError(f"--set needs LABEL=VALUE, got {expr!r}")
        out[k.strip()] = v
    return out


def resolve_draft(file: str | None, preset: str | None, sets: list[str] | None, extra: Draft | None = None) -> Draft:
    """preset < file < --set < explicit flags (``extra``)."""
    d = load_draft_file(file) if file else Draft()
    name = preset or d.preset
    base = load_preset(name) if name else Draft()
    out = base.merged(d)
    out = out.merged(Draft(answers=parse_sets(sets)))
    if extra is not None:
        out = out.merged(extra)
    out.preset = name
    return out


# ---------------------------------------------------------------------------
# annotated YAML template


def _kind_text(f: Field) -> str:
    k = f.kind
    if k == "datetime":
        return "date-time (YYYY-MM-DD HH:MM, optional +HH:MM)"
    if k == "date":
        return "date (YYYY-MM-DD)"
    if k == "checkbox":
        return "yes/no"
    if k == "multi_select":
        return "multi-select (list)"
    if k.startswith("lookup"):
        return "lookup (name or email)"
    return k


def _one_line(text: str, limit: int = 160) -> str:
    s = " ".join(plain_text(text or "").split())
    return s if len(s) <= limit else s[: limit - 1] + "…"


def _value_for_yaml(state: FormState, f: Field) -> Any:
    v = state.evaluated.values.get(f.api_name)
    if v in (None, "", ()):
        return None
    if f.kind == "multi_select":
        out = []
        for x in v:
            ch = f.choice(x)
            out.append(ch.label if ch is not None else str(x))
        return out
    if f.kind in ("dropdown", "department", "nested"):
        return state.display(f.api_name)
    if f.kind == "checkbox":
        return bool(v)
    if isinstance(v, LookupValue):
        return v.email or v.display or str(v.id)
    if isinstance(v, tuple) and v and isinstance(v[0], LookupValue):
        return [x.email or x.display or str(x.id) for x in v]
    if f.kind == "datetime" and hasattr(v, "strftime"):
        return v.isoformat(timespec="minutes")
    if f.kind == "date" and hasattr(v, "isoformat"):
        return v.isoformat()
    return v


def render_template(
    state: FormState,
    *,
    title: str,
    errors: list[str] | None = None,
    draft: Draft | None = None,
    requester: str | None = None,
    module_cli: str = "changes",
    unknown: Mapping[str, Any] | None = None,
    hidden: Mapping[str, Any] | None = None,
) -> str:
    """The editor draft: visible fields in web order, ``Label: value`` with
    ``# REQUIRED · type · help`` and ``# choices: ...`` above each.
    Status is never listed (the server default is used on create)."""
    draft = draft or Draft()
    spec = state.spec
    ev = state.evaluated
    lines = [f"# {title}"]
    for e in errors or []:
        lines.append(f"# ERROR: {' '.join(str(e).split())}")
    lines += [
        "# Fill in values after the colons. Lines starting with # are ignored.",
        "# Fields appear and disappear with your answers, like the web form; after",
        "# saving, new required fields are added and the file reopens if needed.",
        f"# Full choice lists: fsv form fields {module_cli} --choices LABEL",
        "# Save and close to continue; close without changes to abort.",
        "",
    ]
    if draft.preset:
        lines.append(f"# preset {draft.preset} is applied: its answers are filled in below")
    req_field = next((f for f in spec.fields if is_requester(f)), None)
    if req_field is not None or requester:
        lines.append("# REQUIRED · email or exact name of the requester")
        who = requester or draft.requester
        lines.append(f"requester: {yamlio.scalar(who)}" if who else "requester:")
    lines.append("answers:")
    desc_field = None
    shown_any = False
    for name in ev.visible:
        f = spec.by_name[name]
        if is_status(f) or is_requester(f):
            continue
        if is_description(f):
            desc_field = f
            continue
        if f.kind == "notice":
            text = _one_line(f.content or f.label, 300)
            if text:
                lines.append(f"  # NOTE: {text}")
            continue
        if f.kind == "attachment":
            continue
        meta = ["REQUIRED" if name in ev.required else "optional", _kind_text(f)]
        if f.help:
            meta.append(_one_line(f.help))
        if name in ev.disabled:
            meta.append("read-only (set by a rule)")
        if name in ev.populated:
            meta.append("filled in by a rule")
        r = ev.restricted.get(name)
        prefill = None
        if r is not None:
            rule = r.rule_name or r.rule_id
            offered = [c for c in state.choices(name) if c.visible and c.label and str(c.value) != ""]
            if len(offered) == 1:
                meta.append(f"only option allowed by rule {rule}")
                if _value_for_yaml(state, f) is None:
                    prefill = offered[0].label  # still visible on the review screen
            else:
                shown_n = ", ".join(c.label for c in offered[:MAX_CHOICES_SHOWN])
                meta.append(f"choices limited to {shown_n or '(none)'} by rule {rule}"
                            if len(offered) <= 5 else f"choices limited by rule {rule}")
        lines.append("  # " + " · ".join(meta))
        if f.choices or f.is_nested_level:
            pool = [c for c in state.choices(f.api_name) if c.visible]
            labels = [c.label for c in pool]
            if labels:
                shown = " | ".join(labels[:MAX_CHOICES_SHOWN])
                more = (f" | … ({len(labels)} total; fsv form fields {module_cli} --choices "
                        f"{yamlio.scalar(f.label)})") if len(labels) > MAX_CHOICES_SHOWN else ""
                lines.append(f"  # choices: {shown}{more}")
                for c in pool[:MAX_CHOICES_SHOWN]:
                    if c.description:
                        lines.append(f"  #   {c.label} — {_one_line(c.description, 120)}")
        if f.placeholder and not f.choices:
            lines.append(f"  # e.g. {_one_line(f.placeholder, 80)}")
        value = _value_for_yaml(state, f)
        lines.append(_kv(f.label if _label_unique(spec, f) else f.api_name, prefill if prefill is not None else value))
        shown_any = True
    for k, v in (hidden or {}).items():
        lines.append("  # hidden by a rule now: not sent; kept in case it reappears")
        lines.append(_kv(k, v))
    for k, v in (unknown or {}).items():
        lines.append("  # not a field on this form: fix the label or delete the line")
        lines.append(_kv(k, v))
    if not shown_any and not unknown and not hidden:
        lines.append("  {}")
    lines.append("")
    if desc_field is not None:
        req = "REQUIRED" if desc_field.api_name in ev.required else "optional"
        lines.append(f"# {req} · {desc_field.label} · Markdown (or use description_html for raw HTML)")
        desc_val = state.evaluated.values.get(desc_field.api_name)
        if draft.description_html is not None or desc_val:
            html_text = draft.description_html if draft.description_html is not None else str(desc_val)
            lines.append("description_html: " + yamlio.block(html_text, 2))
        else:
            lines.append("description_md: |")
            lines.append("  ")
        lines.append("")
    lines.append("# Files to attach after the change is created (uploaded only after you confirm)")
    lines.append("attachments: " + yamlio.flow_list([os.path.relpath(p) for p in draft.attachments]))
    if draft.planning:
        lines.append("planning:")
        for label, entry in draft.planning.items():
            lines.append(f"  {yamlio.key(label)}:")
            lines.append("    files: " + yamlio.flow_list([os.path.relpath(p) for p in entry.get("files") or []]))
            if entry.get("description_html"):
                lines.append("    description_html: " + yamlio.block(entry["description_html"], 6))
    else:
        lines.append("# planning:            # planning field label -> files")
        lines.append("#   Rollout Plan: [rollout.xlsx]")
    blocking = [m for m in ev.messages if m.blocking]
    if blocking:
        lines.append("")
        for m in blocking:
            lines.append(f"# BLOCKING MESSAGE: {_one_line(m.text)}")
        lines.append("# Acknowledge with: ack: [all]")
        lines.append("ack: " + yamlio.flow_list(draft.ack))
    return "\n".join(lines) + "\n"


def _label_unique(spec: Any, f: Field) -> bool:
    return sum(1 for g in spec.fields if g.on_form and norm(g.label) == norm(f.label)) == 1


def _kv(k: str, v: Any) -> str:
    key = yamlio.key(k)
    if v is None:
        return f"  {key}:"
    if isinstance(v, (list, tuple)):
        return f"  {key}: {yamlio.flow_list(list(v))}"
    s = str(v) if not isinstance(v, bool) else None
    if s is not None and "\n" in s:
        return f"  {key}: " + yamlio.block(s, 4)
    return f"  {key}: {yamlio.scalar(v)}"


def parse_template(text: str, base_dir: Path | None = None) -> Draft:
    data = yamlio.loads(text)
    return draft_from_data(data, base_dir or Path.cwd(), where="draft")


# ---------------------------------------------------------------------------
# hints (local, tenant-specific knowledge the metadata does not carry)


def load_hints(section: str, path: Path | None = None) -> list[dict[str, Any]]:
    """Entries of ``hints.yaml``::

        changes:
          - when: {"Notify Network Team": "Yes"}
            warn: creates a task for the Acme network team
          - when: {"Change Type": Normal, "Environment": [Staging, Live]}
            warn: the release board reviews this change

    ``when`` maps a field label/internal name to a value, a list of values
    (any of) or ``"*"`` (any value). All conditions must hold.
    """
    p = path or hints_path()
    if not p.exists():
        return []
    data = yamlio.load_file(str(p)) or {}
    if not isinstance(data, Mapping):
        raise ValueError(f"{p}: top level must map a section (e.g. changes) to a list")
    entries = data.get(section) or []
    if not isinstance(entries, list):
        raise ValueError(f"{p}: {section} must be a list of {{when, warn}} entries")
    out = []
    for i, e in enumerate(entries):
        if not isinstance(e, Mapping) or not isinstance(e.get("when"), Mapping) or not e.get("warn"):
            raise ValueError(f"{p}: {section}[{i}] needs 'when' (mapping) and 'warn' (text)")
        out.append({"when": dict(e["when"]), "warn": str(e["warn"])})
    return out


def matching_hints(state: FormState, hints: list[dict[str, Any]]) -> tuple[list[str], list[str]]:
    """-> (warnings that apply, problems with the hints file)."""
    sent = state.payload_values()
    out: list[str] = []
    problems: list[str] = []
    for h in hints:
        ok = True
        for k, want in h["when"].items():
            f = state.spec.find(str(k))
            if f is None:
                problems.append(f"hints: unknown field {k!r}")
                ok = False
                break
            if f.api_name not in sent:
                ok = False
                break
            if want == "*":
                continue
            display = state.display(f.api_name)
            have = {norm(x) for x in display.split(", ")} | {norm(display)}
            wants = want if isinstance(want, list) else [want]
            wants = [("Yes" if w else "No") if isinstance(w, bool) else w for w in wants]
            if not any(norm(w) in have for w in wants):
                ok = False
                break
        if ok:
            out.append(h["warn"])
    return out, problems
