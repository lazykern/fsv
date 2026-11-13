"""CLI glue for ``fsv changes create`` / ``fsv changes update`` on the form
engine: inputs -> plan -> review -> confirmation -> freshness -> send."""
from __future__ import annotations

import json
import os
from typing import Any, Callable

import typer

from fsv import change_flow as flow
from fsv import config
from fsv.change_flow import FlowExit, PartialFailure, Plan
from fsv.drafts import Draft, render_template, resolve_draft
from fsv.formmeta import ClientResolver, is_status, load_meta, resolve_requester
from fsv.forms import FormError, LookupValue
from fsv.render import console, err
from fsv.review import NO_PAYLOAD


def _emit(obj: Any) -> None:
    console.print_json(json.dumps(obj, default=str, ensure_ascii=False), indent=2)


def _print_plan(plan: Plan, *, as_json: bool, to_stdout: bool = True) -> None:
    if as_json:
        print(json.dumps(plan.to_json(), indent=2, default=str, ensure_ascii=False))
        return
    out = console if to_stdout else err
    plan.review.render(out)
    if to_stdout:
        if plan.payload is None or not plan.ok:
            out.print(f"[dim]{NO_PAYLOAD}[/dim]", highlight=False)
        else:
            out.print("[bold]Payload[/bold]" + (f" (PUT changes/{plan.target})" if plan.action == "update"
                                                 else " (POST /api/_/changes)")
                      + "",
                      highlight=False)
            print(json.dumps(plan.payload, indent=2, default=str, ensure_ascii=False))
        if plan.hash:
            out.print(f"plan_hash: {plan.hash}", highlight=False)
            out.print(f"[dim]send exactly this with: --yes --expect-plan {plan.hash[:16]}[/dim]", highlight=False)


def _pick(cands: list[LookupValue]) -> LookupValue:
    shown = cands[:20]
    err.print("several requesters match; candidates:", highlight=False)
    for i, c in enumerate(shown, 1):
        err.print(f"  {i}. {c}", highlight=False)
    n = typer.prompt("Pick a number (0 to cancel)", type=int, default=0)
    if 1 <= n <= len(shown):
        return shown[n - 1]
    raise FlowExit(2, "no requester chosen")


class _Requester:
    """Requester for a draft: --requester > draft ``requester:`` > config
    ``defaults.requester`` > (terminal, not editing) a prompt. Never the
    logged-in user by default."""

    def __init__(self, client: Any, flag: str | None, tty: bool) -> None:
        self.client = client
        self.flag = flag
        self.tty = tty
        self.cache: dict[str, LookupValue] = {}
        self.prompted: str | None = None

    def value(self, draft: Draft) -> str | None:
        v = self.flag or draft.requester or self.prompted
        if not v:
            v = str((config.load_config().get("defaults") or {}).get("requester") or "") or None
        return v

    def ask(self) -> None:
        self.prompted = typer.prompt("Requester (email)", default="", show_default=False) or None

    def resolve(self, draft: Draft) -> tuple[LookupValue | None, str | None]:
        v = self.value(draft)
        if not v:
            return None, None
        if v not in self.cache:
            try:
                self.cache[v] = resolve_requester(self.client, v, _pick if self.tty else None)
            except ValueError as e:
                return None, str(e)
        return self.cache[v], None


def _menu(options: str) -> str:
    while True:
        ans = typer.prompt(options, default="n", show_default=False).strip().lower()[:1]
        if ans in ("y", "e", "s", "n"):
            return ans


def _abort_editor(e: Exception) -> FlowExit:
    return FlowExit(1, str(e))


def _ack_blocking(plan: Plan) -> bool:
    msgs = plan.review.blocking_unacked
    if not msgs:
        return False
    for m in msgs:
        err.print(f"[magenta]blocking rule message:[/] {m}", highlight=False)
    return typer.confirm("Acknowledge and continue?", default=False)


def _confirm_loop(plan: Plan, draft: Draft, build: Callable[[Draft, bool], Plan], *, title: str,
                  verb: str) -> tuple[Plan, Draft]:
    """Interactive review: [y] send / [e] edit / [s] save draft / [n] cancel."""
    from fsv.editor import EditorAbort

    ack = False
    while True:
        plan.review.render(err)
        if plan.ok:
            choice = _menu(f"[y] {verb}  [e] edit  [s] save draft  [n] cancel")
        else:
            choice = _menu("Problems must be fixed first: [e] edit  [s] save draft  [n] cancel")
            if choice == "y":
                continue
        if choice == "y":
            if plan.review.blocking_unacked:
                if not _ack_blocking(plan):
                    continue
                ack = True
                plan = build(draft, ack)
                if not plan.ok:
                    continue
            return plan, draft
        if choice == "e":
            try:
                draft, plan = flow.edit_loop(lambda d: build(d, ack), draft, title=title)
            except EditorAbort as e:
                err.print(f"[yellow]{e}[/yellow]; keeping the previous draft", highlight=False)
            continue
        if choice == "s":
            path = typer.prompt("Save draft to", default="change-draft.yaml")
            if os.path.exists(path) and not typer.confirm(f"{path} exists; overwrite?", default=False):
                continue
            saved = flow.save_draft(plan, draft, path)
            err.print(f"[green]saved[/green] {saved}; continue later with --from {saved}", highlight=False)
            raise FlowExit(0)
        raise FlowExit(1, "cancelled; nothing was sent")


def form_tty() -> bool:
    """The in-terminal form can open (patched in tests)."""
    from fsv.formui.app import form_available

    return form_available()


def _run_form(session: Any) -> Any:
    """Open the form; returns the reviewed plan (submit) or exits."""
    from fsv.formui.app import run_form

    result = run_form(session)
    if result.action == "save_draft":
        path = result.saved_path or ""
        raise FlowExit(0, f"saved {path}; continue later with --from {path} (nothing was sent)")
    if result.action != "submit" or result.plan is None:
        raise FlowExit(1, "cancelled; nothing was sent")
    return result.plan


def _send(plan: Plan, client: Any, *, rebuild: Callable[[Any], Plan], tty: bool, yes: bool,
          expect_plan: str | None, allow_duplicate: bool, duplicate: str = "append",
          attach_flow: str | None = None) -> dict[str, Any]:
    flow.check_duplicate(plan, allow_duplicate)
    new, diff = flow.freshen(plan, client, rebuild)
    if diff is not None:
        err.print("[yellow]form metadata changed since the review; the plan is now different:[/yellow]",
                  highlight=False)
        for line in diff:
            err.print(f"  {line}", highlight=False)
        if not new.ok:
            new.review.render(err)
            raise FlowExit(2, "the updated form needs more answers: " + "; ".join(new.errors))
        if expect_plan or yes or not tty:
            raise FlowExit(4, f"plan changed to {new.hash or '(incomplete)'}; review it again with --dry-run")
        new.review.render(err)
        if not typer.confirm("Send the updated plan?", default=False):
            raise FlowExit(1, "cancelled; nothing was sent")
        flow.check_duplicate(new, allow_duplicate)
    plan = new
    flow.expect(plan, expect_plan)
    return flow.execute(plan, client, duplicate=duplicate, attach_flow=attach_flow)


def _report_partial(e: PartialFailure, what: str) -> FlowExit:
    err.print(f"[red]error[/red]: {what} {e.created_id} was written, but a later step failed: {e.error}",
              highlight=False)
    if e.uploaded:
        err.print("uploaded attachment ids: " + ", ".join(f"{u['id']} ({u['name']})" for u in e.uploaded),
                  highlight=False)
    for s in e.steps:
        err.print(f"  {s}", highlight=False)
    return FlowExit(5, f"partial failure; {what} id {e.created_id}")


# ---------------------------------------------------------------------------


def create_command(
    client_factory: Callable[[], Any],
    *,
    from_file: str | None,
    sets: list[str] | None,
    preset: str | None,
    edit: bool,
    requester: str | None,
    attach: list[str] | None,
    legacy: dict[str, Any],
    dry_run: bool,
    as_json: bool,
    yes: bool,
    expect_plan: str | None,
    allow_duplicate: bool,
    ack: bool,
    no_input: bool,
    no_validate: bool,
    portal_url: str,
    attach_flow: str | None = None,
    devices: list[int] | None = None,
    max_bytes: int | None = None,
) -> None:
    from fsv.editor import EditorAbort

    tty = flow.interactive() and not no_input
    attach_flow = flow.resolve_attach_flow(attach_flow)
    if max_bytes is not None and attach_flow != "before-create":
        raise FlowExit(2, "--max-bytes requires the reviewed --attach-flow before-create transaction")
    legacy = {k: v for k, v in legacy.items() if v is not None}
    has_input = bool(from_file or sets or preset or legacy or attach)
    template_only = False
    form = False
    if not has_input and not edit:
        if tty and form_tty():
            form = True  # the in-terminal form (review inside; --dry-run sends nothing)
        elif dry_run:
            template_only = True
        elif tty:
            edit = True  # stdout is not a terminal: fall back to the YAML draft in $EDITOR
        else:
            raise FlowExit(2, "nothing to create: pass --from FILE, --set LABEL=VALUE or --preset NAME "
                              "(the interactive form and --edit need a terminal); see `fsv changes create --help`")
    if edit and not tty:
        raise FlowExit(2, "--edit needs a terminal (no TTY or --no-input); use --from FILE")

    client = client_factory()
    meta = load_meta(client, "change", "new", fresh=dry_run or yes or not tty)
    extra = Draft(attachments=[os.path.expanduser(p) for p in attach or []])
    description = legacy.pop("description", None)
    status = legacy.pop("status", None)
    extra.answers = flow.legacy_answers(meta, **legacy)
    if status is not None:
        sf = next((f for f in meta.spec.fields if is_status(f)), None)
        if sf is not None:
            extra.answers[sf.api_name] = status
    if description is not None:
        extra.description_html = description
    try:
        draft = resolve_draft(from_file, preset, sets, extra)
    except ValueError as e:
        raise FlowExit(2, str(e)) from None
    who = _Requester(client, requester, tty)
    if tty and not form and not edit and not dry_run and not template_only and not who.value(draft):
        who.ask()
    resolver = ClientResolver(client)

    def build(d: Draft, ack_all: bool = ack, m: Any = None) -> Plan:
        req, problem = who.resolve(d)
        plan = flow.build_create_plan(m or meta, d, resolver=resolver, requester=req, ack_all=ack_all,
                                      allow_incomplete=no_validate, devices=tuple(devices or ()))
        if problem:
            plan.errors[:] = [problem] + [e for e in plan.errors if not e.startswith("requester is not set")]
            plan.review.errors = plan.errors
        return plan

    if template_only:
        plan = build(draft)
        print(flow.template_for(plan, draft, title="fsv change draft: fill in, then "
                                                  "`fsv changes create --from FILE --dry-run`"), end="")
        return
    session = None
    try:
        if form:
            from fsv.formui.session import change_session

            req = None
            if requester:  # --requester preselects; never the logged-in user
                req, problem = who.resolve(draft)
                if problem:
                    err.print(f"[yellow]--requester[/yellow]: {problem}", highlight=False)
            session = change_session(client, meta=meta, requester=req, dry_run=dry_run)
            plan = _run_form(session)
            draft = plan.draft
        elif edit:
            draft, plan = flow.edit_loop(lambda d: build(d), draft, title="New change")
        else:
            plan = build(draft)
    except EditorAbort as e:
        raise _abort_editor(e) from None

    if draft.attachments and attach_flow == "before-create":
        from fsv import attachments, config, journal
        from fsv.attachments_cli import finish
        from fsv.reviewed_write import ReviewedWriteError, WritePlan
        import json
        if draft.planning or devices or session is not None:
            raise FlowExit(2, "initial attachments cannot combine with planning, devices or interactive forms")
        def attached(current):
            if not current.ok:
                raise ReviewedWriteError("cannot attach to incomplete change plan: " + "; ".join(current.errors))
            raw = client.int_get("change_form_fields")
            base = WritePlan("change_create_attachments", 0, config.DOMAIN, "POST", "changes",
                             json.dumps(current.payload, sort_keys=True),
                             journal.plan_hash({"plan": current.hash, "metadata": raw}) or "")
            from dataclasses import replace
            guards = tuple(("create", value) for value in dict.fromkeys(
                [current.hash, getattr(current, "legacy_hash", None)]) if value)
            from fsv.watchers import positive_id
            wrapped = attachments.wrap(base, draft.attachments, raw, client=client, max_bytes=max_bytes)
            return replace(wrapped, requester_id=positive_id(current.requester.id),
                           prior_guards=guards + ((base.action, wrapped.hash),))
        def rebuild_attached():
            fresh = load_meta(client, "change", "new", fresh=True)
            return attached(build(draft, ack, fresh))
        try:
            finish(attached(plan), client, dry_run=dry_run, yes=yes, expected=expect_plan,
                   rebuild=rebuild_attached)
        except ReviewedWriteError as exc:
            raise FlowExit(2, str(exc)) from None
        return

    if dry_run:
        _print_plan(plan, as_json=as_json)
        if not plan.ok:
            raise FlowExit(2)
        flow.expect(plan, expect_plan)
        return

    if session is not None:
        flow.expect(plan, expect_plan)  # reviewed and confirmed in the form
        ack_now = ack or session.ack
        try:
            result = _send(plan, client, rebuild=lambda m: session.build_plan(ack_now, m), tty=tty, yes=yes,
                           expect_plan=expect_plan, allow_duplicate=allow_duplicate, attach_flow=attach_flow)
        except PartialFailure as e:
            raise _report_partial(e, "change") from None
        _report_created(result, as_json=as_json, portal_url=portal_url)
        return

    if tty and not yes:
        plan, draft = _confirm_loop(plan, draft, lambda d, a: build(d, a or ack), title="New change", verb="create")
    else:
        if not tty and not yes:
            plan.review.render(err)
            raise FlowExit(2, "not a terminal: review with --dry-run, then send with --yes --expect-plan HASH")
        if not plan.ok:
            plan.review.render(err)
            raise FlowExit(2, "cannot create: " + "; ".join(plan.errors))
        if plan.review.blocking_unacked:
            plan.review.render(err)
            raise FlowExit(2, "blocking rule message(s) need --ack: " + "; ".join(plan.review.blocking_unacked))
        flow.expect(plan, expect_plan)
    ack_now = ack or bool(plan.state.acknowledged)
    try:
        result = _send(plan, client, rebuild=lambda m: build(plan.draft, ack_now, m), tty=tty, yes=yes,
                       expect_plan=expect_plan, allow_duplicate=allow_duplicate, attach_flow=attach_flow)
    except PartialFailure as e:
        raise _report_partial(e, "change") from None
    _report_created(result, as_json=as_json, portal_url=portal_url)


def _report_created(result: dict[str, Any], *, as_json: bool, portal_url: str) -> None:
    cid = result["id"]
    if as_json:
        print(json.dumps({"created": result["result"], "plan_hash": result["plan_hash"], "steps": result["steps"],
                          "uploaded": result["uploaded"],
                          **({"device_verification": result["device_verification"]} if "device_verification" in result else {})}, indent=2, default=str, ensure_ascii=False))
        return
    from fsv.resources import CHANGES, format_id

    console.print(f"[green]created[/] {format_id(result['result'] if isinstance(result['result'], dict) else {'id': cid}, CHANGES)}"
                  f"  {portal_url}/{cid}", highlight=False)
    for s in result["steps"][1:]:
        console.print(f"  [green]ok[/] {s['step']}", highlight=False)


def update_command(
    client_factory: Callable[[], Any],
    cid: int,
    *,
    from_file: str | None,
    sets: list[str] | None,
    preset: str | None,
    edit: bool,
    attach: list[str] | None,
    legacy: dict[str, Any],
    duplicate: str,
    dry_run: bool,
    as_json: bool,
    yes: bool,
    expect_plan: str | None,
    ack: bool,
    no_input: bool,
    status_label: Callable[[Any], str] | None,
    subject: str,
    form: bool = False,
) -> None:
    """``form``: open the in-terminal edit form (pre-filled; the edit
    form's rules; only changed fields are sent)."""
    from fsv.editor import EditorAbort

    tty = flow.interactive() and not no_input
    if form and not (tty and form_tty()):
        raise FlowExit(2, "nothing to update: the interactive form needs a terminal; pass --set LABEL=VALUE, "
                          "--from FILE, --status/--priority/--agent/--group or --edit")
    if edit and not tty:
        raise FlowExit(2, "--edit needs a terminal (no TTY or --no-input); use --from FILE or --set")
    client = client_factory()
    data = client.int_get(f"changes/{cid}")
    record = data.get("change", data) if isinstance(data, dict) else {}
    meta = load_meta(client, "change", "edit", fresh=dry_run or yes or not tty,
                     module_id=record.get("display_id") or record.get("id") or cid, record=record)
    legacy = {k: v for k, v in legacy.items() if v is not None}
    status = legacy.pop("status", None)
    extra = Draft(attachments=[os.path.expanduser(p) for p in attach or []])
    extra.answers = flow.legacy_answers(meta, **legacy)
    if status is not None:
        sf = next((f for f in meta.spec.fields if is_status(f)), None)
        if sf is None:
            raise FlowExit(2, "this form has no status field")
        extra.answers[sf.api_name] = status
    try:
        draft = resolve_draft(from_file, preset, sets, extra)
    except ValueError as e:
        raise FlowExit(2, str(e)) from None
    resolver = ClientResolver(client)
    lifecycle_cache: dict[str, Any] = {}

    def lifecycle() -> Any:
        # One read of the runtime state flow per command (edges, criteria, locks).
        if "flow" not in lifecycle_cache:
            from fsv.state_flow import get_state_flow
            lifecycle_cache["flow"] = get_state_flow(record["state_flow_id"], client, record.get("workspace_id"))
        return lifecycle_cache["flow"]

    def build(d: Draft, ack_all: bool = ack, m: Any = None) -> Plan:
        return flow.build_update_plan(m or meta, d, record, cid, resolver=resolver, ack_all=ack_all,
                                      status_label=status_label,
                                      lifecycle=lifecycle if record.get("state_flow_id") else None)

    session = None
    try:
        if form:
            from fsv.formui.session import change_session

            session = change_session(client, mode="edit", target=cid, record=record, meta=meta, dry_run=dry_run,
                                     status_label=status_label, title=f"Edit {subject}")
            plan = _run_form(session)
            draft = plan.draft
        elif edit:
            draft, plan = flow.edit_loop(lambda d: build(d), draft, title=f"Edit {subject}")
        else:
            plan = build(draft)
    except EditorAbort as e:
        raise _abort_editor(e) from None
    except FormError as e:
        raise FlowExit(2, str(e)) from None
    if session is not None:
        _s = session

        def build(d: Draft, ack_all: bool = ack, m: Any = None) -> Plan:  # noqa: F811 - the form's own plan
            return _s.build_plan(ack_all or _s.ack, m)

    if dry_run:
        _print_plan(plan, as_json=as_json)
        if not plan.ok:
            raise FlowExit(2)
        flow.expect(plan, expect_plan)
        return
    if plan.ok and not plan.hash:
        console.print(f"[yellow]no changes[/] {subject}", highlight=False)
        return
    if session is not None:
        flow.expect(plan, expect_plan)  # reviewed and confirmed in the form
    elif tty and not yes:
        plan, draft = _confirm_loop(plan, draft, lambda d, a: build(d, a or ack), title=f"Edit {subject}",
                                    verb="update")
        if not plan.hash:
            console.print(f"[yellow]no changes[/] {subject}", highlight=False)
            return
    else:
        if not plan.ok:
            plan.review.render(err)
            raise FlowExit(2, "cannot update: " + "; ".join(plan.errors))
        if plan.risks and not yes:
            raise FlowExit(2, f"{subject}: " + "; ".join(plan.risks) + ". Pass --yes to confirm.")
        if plan.review.blocking_unacked:
            raise FlowExit(2, "blocking rule message(s) need --ack: " + "; ".join(plan.review.blocking_unacked))
        flow.expect(plan, expect_plan)
    from fsv.reviewed_write import ReviewedWriteError
    try:
        result = _send(plan, client, rebuild=lambda m: build(plan.draft, ack, m), tty=tty, yes=yes,
                       expect_plan=expect_plan, allow_duplicate=True, duplicate=duplicate)
    except PartialFailure as e:
        raise _report_partial(e, "change") from None
    except ReviewedWriteError as e:
        # A failed send always names the server's status and message.
        if as_json:
            print(json.dumps({"updated": None, "plan_hash": plan.hash, "verified": False, **e.as_dict()},
                             indent=2, default=str, ensure_ascii=False))
        raise FlowExit(5 if e.http_status else 1, f"{subject}: {e}") from None
    if as_json:
        print(json.dumps({"updated": result["result"], "plan_hash": result["plan_hash"], "steps": result["steps"],
                          "verified": result.get("verified", False), "fields": result.get("fields", {}),
                          "warning": result.get("warning")}, indent=2, default=str, ensure_ascii=False))
        if not result.get("verified"):
            raise FlowExit(4, "Change update readback unverified; do not resend")
        return
    if not result.get("verified"):
        raise FlowExit(4, "Change update readback unverified; do not resend")
    console.print(f"[green]updated[/] {subject}", highlight=False)
    for s in result["steps"][1:]:
        console.print(f"  [green]ok[/] {s['step']}", highlight=False)


__all__ = ["create_command", "update_command", "FlowExit", "render_template"]
