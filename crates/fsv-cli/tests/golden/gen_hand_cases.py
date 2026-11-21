#!/usr/bin/env python3
"""Rewrite the `tag: "hand"` rows of ../fixtures/diff_cases.json.

Hand-written differential cases for command families whose Rust tests build
their own routes. `@HASH@` in args is the plan hash Python prints for
`hash_from`; both sides are given Python's, so a Rust hash that differs
fails the send.
"""
import json
import os

HERE = os.path.dirname(os.path.abspath(__file__))
PATH = os.path.join(HERE, "../fixtures/diff_cases.json")
TAG = "hand"
R = []


def case(fixture, args, **kw):
    R.append({"fixture": fixture, "args": args, "tag": TAG, **kw})


def reviewed(fixture, review, send_extra=(), **kw):
    """The dry-run, the send with Python's hash, and the send without --expect-plan."""
    case(fixture, [*review, "--dry-run"], **kw)
    case(fixture, [*review, *send_extra, "--yes", "--expect-plan", "@HASH@"], hash_from=[*review, "--dry-run"], **kw)
    case(fixture, [*review, *send_extra, "--yes"], **kw)
    case(fixture, [*review, *send_extra, "--yes", "--expect-plan", "0" * 64], **kw)


# downloads --max-bytes
for a in (["tickets", "download", "5", "--max-bytes", "3"],
          ["tickets", "download", "5", "--max-bytes", "7", "--json"],
          ["tickets", "download", "5", "--max-bytes", "6", "--json"],
          ["tickets", "download", "5", "--all", "--max-bytes", "4", "--json"],
          ["tickets", "download", "5", "--conversations", "--max-bytes", "3"],
          ["changes", "download", "5", "--max-bytes", "3"],
          ["changes", "download", "5", "--all", "--max-bytes", "100", "--json"],
          ["problems", "download", "9", "--max-bytes", "1"],
          ["tickets", "download", "5", "--max-bytes", "0"]):
    case("downloads", a)

# assets --per-page
for a in (["assets", "search", "acme", "--per-page", "2", "--json"],
          ["assets", "search", "acme", "--per-page", "2", "--page", "3", "--json"],
          ["assets", "search", "acme", "--per-page", "0"],
          ["assets", "search", "acme", "--per-page", "101"],
          ["assets", "search", "--per-page", "100"],
          ["assets", "search", "--kind", "services", "--per-page", "1", "--page", "2"],
          ["changes", "assets", "search", "acme", "--per-page", "5"]):
    case("integrated_assets", a)
for a in (["tickets", "assets", "candidates", "151", "--per-page", "2"],
          ["changes", "assets", "candidates", "51", "--per-page", "1", "--json"],
          ["tickets", "assets", "candidates", "151", "--per-page", "101"],
          ["tickets", "assets", "candidates", "151", "--per-page", "0"]):
    case("legacy_assets", a)

# attachments delete --confirm-destructive
review = ["attachments", "delete", "62", "--owner", "tickets/6"]
case("attachments", [*review, "--dry-run"])
case("attachments", [*review, "--yes", "--expect-plan", "@HASH@"], hash_from=[*review, "--dry-run"])
case("attachments", [*review, "--yes", "--expect-plan", "@HASH@", "--confirm-destructive"], hash_from=[*review, "--dry-run"])
case("attachments", [*review, "--yes", "--confirm-destructive"])
case("attachments", [*review, "--confirm-destructive", "--dry-run"])
case("attachments", [*review, "--confirm-destructive"])

# private views: tickets and changes
for module, fx, vid in (("tickets", "private_views", "703"), ("tickets", "private_views", "704"),
                        ("changes", "private_views_native", "811"), ("changes", "private_views_native", "812")):
    base = [module, "private-views"]
    case(fx, [*base, "list"])
    case(fx, [*base, "show", vid])
    for action, extra in (("create", ["--name", "Globex copy"]), ("edit", ["--name", "Acme source"]),
                          ("edit", ["--name", "Globex changes"]), ("edit", ["--name", "Globex renamed"]),
                          ("delete", [])):
        rv = [*base, "write", vid, action, *extra]
        case(fx, [*rv, "--dry-run"])
        case(fx, [*rv, "--yes", "--expect-plan", "@HASH@"], hash_from=[*rv, "--dry-run"])
        case(fx, [*rv, "--yes", "--expect-plan", "@HASH@", "--confirm-destructive"], hash_from=[*rv, "--dry-run"])
        case(fx, [*rv, "--expect-plan", "abc"])
        case(fx, [*rv, "--yes"])

# problem and change notes
for fx in ("problem_add_note", "problem_note_reviewed"):
    pid = "12"
    reviewed(fx, ["problems", "add-note", pid, "Acme evidence"], ["--json"])
    reviewed(fx, ["problems", "add-note", pid, "Acme evidence"])
    case(fx, ["problems", "add-note", pid, "Acme evidence", "--public", "--dry-run"])
    case(fx, ["problems", "add-note", pid, "Acme evidence", "--no-input"])
    case(fx, ["problems", "add-note", pid, "", "--dry-run"])
    case(fx, ["problems", "add-note", "999", "Acme evidence", "--dry-run"])
reviewed("problem_add_note", ["changes", "add-note", "3", "Acme change note"])
reviewed("problem_add_note", ["changes", "add-note", "3", "Acme change note", "--public"])
case("problem_add_note", ["changes", "add-note", "3", "Acme change note", "--public", "--no-input"])
case("problem_add_note", ["changes", "add-note", "3", "Acme change note", "--public", "--yes"])
case("problem_add_note", ["changes", "add-note", "3", "Acme change note", "--yes", "--no-input"])
case("problem_add_note", ["changes", "add-note", "3", "Acme change note"], stdin="y\n", tty=True)
case("problem_add_note", ["changes", "add-note", "3", "Acme change note", "--public"], stdin="n\n", tty=True)

# plain ticket reply
for fx in ("ticket_reply_plain", "ticket_reply_plain_500", "ticket_reply_plain_mismatch", "ticket_reply_plain_rejected"):
    reviewed(fx, ["tickets", "reply", "123", "Acme is working on it"])
    reviewed(fx, ["tickets", "reply", "123", "Acme is working on it"], ["--json"])
case("ticket_reply_plain", ["tickets", "reply", "123", "Acme is working on it", "--no-input"])
case("ticket_reply_plain", ["tickets", "reply", "123", "Acme is working on it", "--cc", "ann@acme.com", "--dry-run"])
case("ticket_reply_plain", ["tickets", "reply", "123", "", "--dry-run"])

# ticket and problem update (reviewed by default), --due-by
for fx in ("ticket_reviewed", "ticket_reviewed_ack"):
    reviewed(fx, ["tickets", "update", "5", "--priority", "High"])
    reviewed(fx, ["tickets", "update", "5", "--priority", "High"], ["--json"])
    reviewed(fx, ["tickets", "update", "5", "--ack", "--priority", "High"])
    reviewed(fx, ["tickets", "update", "5", "--set", "Cost Code=new"])
    case(fx, ["tickets", "update", "5", "--priority", "High", "--no-input"])
    case(fx, ["tickets", "update", "5", "--priority", "High"], stdin="y\n", tty=True)
    case(fx, ["tickets", "update", "5", "--priority", "High"], stdin="n\n", tty=True)
    case(fx, ["tickets", "update", "5", "--priority", "High", "--no-review", "--yes"])
    case(fx, ["tickets", "update", "5", "--priority", "High", "--no-review", "--dry-run"])
    case(fx, ["tickets", "update", "5", "--priority", "Nope", "--dry-run"])
reviewed("ticket_update", ["tickets", "update", "5", "--status", "Resolved"])
case("ticket_update", ["tickets", "update", "5", "--status", "Resolved", "--no-input"])
for pid, extra in (("74", ["--set", "Owner Note=Acme new owner"]), ("74", ["--set", "cf_owner_note=Acme new owner", "--json"]),
                   ("71", ["--set", "subject=Acme renamed"]), ("76", ["--priority", "High"]), ("72", ["--set", "subject=x"])):
    reviewed("problem_update", ["problems", "update", pid, *extra])
case("problem_update", ["problems", "update", "74", "--set", "Owner Note=Acme new owner", "--no-input"])
case("problem_update", ["problems", "update", "74", "--set", "Owner Note=Acme new owner", "--no-review", "--yes"])
case("problem_update", ["problems", "update", "74", "--set", "Owner Note=Acme new owner"], stdin="y\n", tty=True)
case("problem_update", ["problems", "update", "71", "--planned-effort", "2h", "--dry-run"])
case("problem_update", ["problems", "update", "71", "--planned-effort", "2h", "--yes", "--expect-plan", "@HASH@"],
     hash_from=["problems", "update", "71", "--planned-effort", "2h", "--dry-run"])
case("problem_update", ["problems", "update", "71", "--planned-effort", "2h", "--yes", "--expect-plan", "f" * 64])
case("problem_update", ["problems", "update", "76", "--planned-start", "2030-01-02T09:00:00Z", "--yes", "--expect-plan", "@HASH@"],
     hash_from=["problems", "update", "76", "--planned-start", "2030-01-02T09:00:00Z", "--dry-run"])
reviewed("change_helpers", ["changes", "update", "5", "--due-by", "2026-06-09T18:00:00+07:00"])
case("change_helpers", ["changes", "update", "5", "--due-by", "2026-06-09T18:00:00+07:00", "--no-input"])
case("change_helpers", ["changes", "update", "5", "--due-by", "none", "--dry-run"])
case("change_helpers", ["changes", "update", "5", "--due-by", "2026-06-09", "--dry-run"])

# planning text with Python's hash
for fx, args in (("planning_edit", ["problems", "planning-text", "123", "--section", "Root Cause", "--description", "Acme new"]),
                 ("planning_edit", ["problems", "planning-text", "124", "--section", "Root Cause", "--clear"]),
                 ("planning_clear_empty", ["problems", "planning-text", "124", "--section", "Root Cause", "--clear"]),
                 ("planning_clear_empty", ["changes", "planning-text", "5", "--section", "Rollout plan", "--clear"])):
    reviewed(fx, args)

# cache clear/status
for target in ("schema", "forms", "views", "groups", "all", "bogus", None):
    args = ["cache", "clear"] + ([target] if target else [])
    case("ticket_update", args, pre=[["tickets", "update", "5", "--status", "Resolved", "--dry-run"], ["cache", "status"]])
case("ticket_update", ["cache", "status"], pre=[["tickets", "update", "5", "--status", "Resolved", "--dry-run"]])
case("ticket_update", ["cache", "status"], pre=[["tickets", "update", "5", "--status", "Resolved", "--dry-run"], ["cache", "clear", "forms"]])
case("ticket_update", ["cache", "status"], pre=[["tickets", "update", "5", "--status", "Resolved", "--dry-run"], ["cache", "clear", "schema"]])
case("ticket_update", ["cache", "status", "--json"], pre=[["tickets", "update", "5", "--status", "Resolved", "--dry-run"]])
case("auth", ["cache", "status"], pre=[["cache", "refresh"]])
for target in ("schema", "forms"):
    case("auth", ["cache", "clear", target], pre=[["cache", "refresh"]])
    case("auth", ["cache", "status"], pre=[["cache", "refresh"], ["cache", "clear", target]])

# schema cache inside the TTL, refetch on refusal, replay of a sent plan
for fx in ("ticket_reviewed", "ticket_update", "problem_update"):
    mod = "problems" if fx == "problem_update" else "tickets"
    pid = "74" if fx == "problem_update" else "5"
    extra = ["--set", "Owner Note=Acme new owner"] if fx == "problem_update" else ["--priority", "High"]
    dry = [mod, "update", pid, *extra, "--dry-run"]
    case(fx, dry, pre=[dry])
    case(fx, [mod, "update", pid, "--set", "nope=1", "--dry-run"], pre=[dry])
    case(fx, [mod, "update", pid, "--priority", "Bogus", "--dry-run"], pre=[dry])
    case(fx, ["cache", "status"], pre=[dry])
    case(fx, ["cache", "clear", "forms"], pre=[dry])
    send = [mod, "update", pid, *extra, "--yes", "--expect-plan", "@HASH@"]
    case(fx, send, pre=[send], hash_from=dry)
    case(fx, [*send, "--json"], pre=[send], hash_from=dry)

# boolean flag pairs and hidden aliases
for module in ("changes", "problems"):
    for flag in ("--billable", "--non-billable", "--no-billable"):
        case("module_time", [module, "time", "write", "5", "create", "--spent", "1h", flag, "--dry-run"])
case("change_create", ["changes", "create", "-O", "--set", "Subject=x", "--dry-run"])
case("change_create", ["changes", "create", "--optional", "--set", "Subject=x", "--dry-run"])

# journal kinds after reviewed sends
for fx, send, review in (
        ("problem_add_note", ["problems", "add-note", "12", "Acme evidence"], None),
        ("ticket_reviewed", ["tickets", "update", "5", "--priority", "High"], None),
        ("private_views", ["tickets", "private-views", "write", "703", "edit", "--name", "Globex renamed"], None),
        ("planning_edit", ["problems", "planning-text", "123", "--section", "Root Cause", "--description", "Acme new"], None),
        ("ticket_reply_plain", ["tickets", "reply", "123", "Acme is working on it"], None),
        ("attachments", ["attachments", "delete", "62", "--owner", "tickets/6"], ["--confirm-destructive"])):
    extra = review or []
    sent = [*send, *extra, "--yes", "--expect-plan", "@HASH@"]
    for tail in (["journal", "ls"], ["journal", "ls", "--json"], ["journal", "tail"]):
        case(fx, tail, pre=[sent], hash_from=[*send, "--dry-run"])
    case(fx, sent, pre=[sent], hash_from=[*send, "--dry-run"])


# setup wizard (scripted tty): completion install per shell, reinstall, network completion, defaults, menu errors
for shell in ("/bin/bash", "/bin/zsh", "/usr/bin/fish"):
    env = {"SHELL": shell}
    install = ["setup"]
    case("local", ["setup"], stdin="3\ny\n", tty=True, env=env)
    case("local", ["setup"], stdin="3\nn\n", tty=True, env=env)
    case("local", ["setup"], stdin="3\n\n", tty=True, env=env)
    case("local", ["setup"], stdin="3\nn\n", tty=True, env=env, pre=[{"args": ["setup"], "stdin": "3\ny\n"}])
    case("local", ["setup"], stdin="3\ny\n", tty=True, env=env, pre=[{"args": ["setup"], "stdin": "3\ny\n"}])
    case("local", ["setup"], stdin="3\n\n", tty=True, env=env, pre=[{"args": ["setup"], "stdin": "3\ny\n"}])
    case("local", ["setup"], stdin="3\nmaybe\ny\n", tty=True, env=env)
for ans in ("on", "", "OFF", " on ", "yes", "On\n", "of"):
    case("local", ["setup"], stdin=f"4\n{ans}\n", tty=True)
case("local", ["setup"], stdin="4\non\n", tty=True, pre=[{"args": ["config", "set", "completion.network", "off"]}])
for choice in ("0", "7", "abc", "-1", " 3 ", "", "1.0", "１"):
    case("local", ["setup"], stdin=f"{choice}\n3\nn\n", tty=True, env={"SHELL": "/bin/zsh"})
case("local", ["setup"], stdin="5\n, ,\nstatus=Open,, priority=High\n\n\n\n\n\n\n\n", tty=True)
case("local", ["setup"], stdin="5\nx\n", tty=True)
case("local", ["setup"], stdin="1\n\n", tty=True)
case("local", ["setup"], stdin="1\nhttps://Globex.freshservice.com/\n", tty=True)
case("local", ["setup"], stdin="1\nglobex\n", tty=True)
case("local", ["setup"], stdin="1\nbad domain\n", tty=True)
case("local", ["setup"], stdin="", tty=True)
case("local", ["setup"], stdin="3\n", tty=False, env={"SHELL": "/bin/zsh"})
case("local", ["setup"], stdin="3\ny\n", tty=True, pre=[{"args": ["config", "set", "domain", "globex.freshservice.com"]}], env={"SHELL": "/bin/zsh"})

rows = [r for r in json.load(open(PATH)) if r.get("tag") != TAG]
json.dump(rows + R, open(PATH, "w"), indent=1)
open(PATH, "a").write("\n")
print(len(R), "hand cases")
