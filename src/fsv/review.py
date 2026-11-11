"""The review screen every write face ends in: what will be sent, in the
web's words, before anything is sent."""
from __future__ import annotations

from dataclasses import asdict, dataclass, field as dc_field
from typing import Any

from rich.console import Console
from rich.markup import escape
from rich.table import Table


@dataclass
class Review:
    title: str
    rows: list[tuple[str, str]] = dc_field(default_factory=list)  # label, display value (web order)
    changes: list[tuple[str, str, str]] = dc_field(default_factory=list)  # update: label, old, new
    notify: list[str] = dc_field(default_factory=list)
    messages: list[dict[str, Any]] = dc_field(default_factory=list)  # text, blocking, acknowledged, rule
    dropped: list[str] = dc_field(default_factory=list)
    notes: list[str] = dc_field(default_factory=list)  # populated / disabled / replaced / conflicts
    attachments: list[dict[str, Any]] = dc_field(default_factory=list)  # target, name, size, sha256, path
    risks: list[str] = dc_field(default_factory=list)
    errors: list[str] = dc_field(default_factory=list)
    warnings: list[str] = dc_field(default_factory=list)
    metadata: dict[str, Any] = dc_field(default_factory=dict)
    plan_hash: str = ""

    def to_dict(self) -> dict[str, Any]:
        d = asdict(self)
        d["rows"] = [{"label": a, "value": b} for a, b in self.rows]
        d["changes"] = [{"label": a, "old": b, "new": c} for a, b, c in self.changes]
        return d

    @property
    def blocking_unacked(self) -> list[str]:
        return [m["text"] for m in self.messages if m.get("blocking") and not m.get("acknowledged")]

    def render(self, console: Console) -> None:
        p = console.print
        p(f"[bold]──────── Review: {escape(self.title)} ────────[/bold]", highlight=False)
        if self.rows:
            t = Table(show_header=False, box=None, pad_edge=False)
            t.add_column(style="cyan", no_wrap=True)
            t.add_column(overflow="fold")
            for label, value in self.rows:
                t.add_row(escape(label), escape(value))
            p(t)
        if self.changes:
            t = Table(show_header=True, box=None, pad_edge=False)
            t.add_column("Field", style="cyan", no_wrap=True)
            t.add_column("Old", overflow="fold")
            t.add_column("", no_wrap=True)
            t.add_column("New", overflow="fold")
            for label, old, new in self.changes:
                t.add_row(escape(label), escape(old or "—"), "→", escape(new or "—"))
            p(t)

        def section(title: str, items: list[str], style: str = "") -> None:
            if not items:
                return
            p(f"[bold]{title}[/bold]", highlight=False)
            for it in items:
                p(f"  {('[' + style + ']') if style else ''}{escape(it)}{('[/' + style + ']') if style else ''}",
                  highlight=False)

        section("Will notify / create work for", self.notify, "yellow")
        msgs = []
        for m in self.messages:
            tag = "BLOCKING (needs --ack or y)" if m.get("blocking") and not m.get("acknowledged") else (
                "acknowledged" if m.get("blocking") else "message")
            msgs.append(f"[{tag}] {m['text']}")
        section("Messages", msgs, "magenta")
        section("Dropped (hidden by rules, not sent)", self.dropped, "dim")
        section("Rule notes", self.notes, "dim")
        if self.attachments:
            section("Attachments (uploaded after you confirm)", [
                f"{a['target']}: {a['name']}  {a['size']:,} B  sha256:{a['sha256'][:16]}" for a in self.attachments
            ])
        section("Requires confirmation", self.risks, "yellow")
        section("Warnings", self.warnings, "yellow")
        section("Problems (must be fixed before sending)", self.errors, "red")
        md = self.metadata
        if md:
            asof = md.get("updated_at") or "unknown"
            p(f"[dim]form metadata as of {escape(str(asof))} (fetched {escape(str(md.get('fetched_at')))}, "
              f"{escape(str(md.get('source')))}, fingerprint {escape(str(md.get('fingerprint')))})[/dim]",
              highlight=False)
        if self.plan_hash:
            p(f"[bold]plan[/bold] {self.plan_hash}", highlight=False)


NO_PAYLOAD = "No payload yet — fix the problems above"
