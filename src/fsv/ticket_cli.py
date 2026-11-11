"""Command-line face for reviewed incident creation."""
from __future__ import annotations

import json
from typing import Any, Callable

import typer

from fsv import change_flow as flow, ticket_flow
from fsv.change_cli import _Requester, _run_form, form_tty
from fsv.drafts import Draft, resolve_draft
from fsv.formmeta import ClientResolver, load_meta
from fsv.render import console, err


def print_plan(plan: flow.Plan, as_json: bool) -> None:
    if as_json:
        print(json.dumps(plan.to_json(), indent=2, default=str, ensure_ascii=False))
        return
    plan.review.render(console)
    if plan.ok and plan.payload is not None:
        console.print("Payload (POST /api/_/tickets)")
        print(json.dumps(plan.payload, indent=2, default=str, ensure_ascii=False))
        console.print(f"plan_hash: {plan.hash}\nSend with --yes --expect-plan {plan.hash[:16]}")
    else:
        console.print("No payload: fix the problems above.")


def create_command(client_factory: Callable[[], Any], *, from_file: str | None, sets: list[str],
                   preset: str | None, edit: bool, requester: str | None, dry_run: bool,
                   as_json: bool, yes: bool, expect_plan: str | None, allow_duplicate: bool,
                   ack: bool, no_input: bool) -> None:
    tty = flow.interactive() and not no_input
    form = not (from_file or sets or preset or edit) and tty and form_tty()
    template_only = not (from_file or sets or preset or edit or form) and dry_run
    if not (from_file or sets or preset or edit or form or template_only):
        raise flow.FlowExit(2, "pass --from or --set, or open the interactive form in a terminal")
    if edit and not tty:
        raise flow.FlowExit(2, "--edit needs a terminal; use --from or --set")
    client = client_factory()
    meta = load_meta(client, "ticket", "new", fresh=dry_run or yes or not tty)
    try:
        draft = resolve_draft(from_file, preset, sets, Draft())
    except ValueError as e:
        raise flow.FlowExit(2, str(e)) from None
    who = _Requester(client, requester, tty)
    resolver = ClientResolver(client)

    def build(d: Draft, ack_all: bool = ack, m: Any = None) -> flow.Plan:
        req, problem = who.resolve(d)
        plan = ticket_flow.build_create_plan(m or meta, d, resolver=resolver, requester=req, ack_all=ack_all)
        if problem:
            plan.errors.insert(0, problem)
            plan.hash = plan.review.plan_hash = ""
        return plan

    session = None
    if template_only:
        plan = build(draft)
        print(flow.template_for(plan, draft, title="fsv incident draft: fill in, then use tickets create --dry-run"), end="")
        return
    if form:
        from fsv.formui.session import ticket_session
        req, problem = who.resolve(draft)
        if problem:
            raise flow.FlowExit(2, problem)
        session = ticket_session(client, meta=meta, requester=req, dry_run=dry_run)
        plan = _run_form(session)
    elif edit:
        from fsv.editor import EditorAbort
        try:
            draft, plan = flow.edit_loop(lambda d: build(d), draft, title="New incident")
        except EditorAbort as e:
            raise flow.FlowExit(1, str(e)) from None
    else:
        plan = build(draft)
    if dry_run:
        print_plan(plan, as_json)
        if not plan.ok:
            raise flow.FlowExit(2)
        flow.expect(plan, expect_plan)
        return
    if session is None and tty and not yes:
        plan.review.render(err)
        if not plan.ok or plan.review.blocking_unacked:
            raise flow.FlowExit(2, "fix the form or acknowledge blocking messages before creating")
        if not typer.confirm("Create this incident?", default=False):
            raise flow.FlowExit(1, "cancelled; nothing was sent")
    elif session is None:
        if not yes:
            raise flow.FlowExit(2, "review with --dry-run, then use --yes --expect-plan HASH")
        if not tty and not expect_plan:
            raise flow.FlowExit(2, "non-interactive create requires --expect-plan HASH")
    if not plan.ok or plan.review.blocking_unacked:
        raise flow.FlowExit(2, "cannot create: " + "; ".join(plan.errors or plan.review.blocking_unacked))
    flow.expect(plan, expect_plan)
    ack_now = ack or bool(plan.state.acknowledged)
    rebuild = (lambda m: session.build_plan(ack_now, m)) if session else (lambda m: build(plan.draft, ack_now, m))
    result = ticket_flow.send(plan, client, rebuild, expected=expect_plan, allow_duplicate=allow_duplicate)
    if as_json:
        print(json.dumps({"created": result["result"], "plan_hash": result["plan_hash"]}, default=str))
    else:
        console.print(f"[green]created[/green] incident {result['id']}", highlight=False)
