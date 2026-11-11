# Upstream ledger

Python commits that landed while the port ran, and what the Rust port still
needs for each.

- **Last triaged Python commit: `b7d96a7`** (2026-11-12 04:43 +0700,
  `fix(cli): accept consistent JSON options on reviewed commands`).
  Range: `273ffde~1..b7d96a7`, 32 commits.
- Rust state graded: `rust-port` at `8790060` **plus the uncommitted/staged
  working tree** at triage time (other workers had edits in `fsv-api`,
  `fsv-core`, `fsv-forms`, `fsv-flows`, `fsv-views`, `fsv-completion`).
  Re-check a row if its file changed since then.
- Python had uncommitted work after `b7d96a7`, not graded here:
  `src/fsv/canned.py`, `canned_cli.py`, `scenarios.py`,
  `tests/test_canned.py`, and edits to `cli.py` / `ticket_conversation.py`.
- Next triage: `git -C /Users/phusit.somb/lab/work/fsv log --reverse b7d96a7..HEAD`.

Statuses: `ported` (Rust has the behavior), `partial` (some is missing),
`missing` (nothing in Rust).

Counts: **16 ported, 9 partial, 7 missing.**

## Ledger

| commit | date | subject | status | crates |
|---|---|---|---|---|
| `273ffde` | 11-12 00:38 | fix(writes): gate status changes and avoid replay | partial | fsv-core (client ported; safety missing), fsv-tui tests |
| `9d7caa9` | 11-12 00:42 | fix(updates): validate legacy choices and booleans | ported | fsv-cli `lookup.rs:346`, `:546`, `:570` |
| `b05933f` | 11-12 00:46 | fix(tickets): align note recipients and readback | ported | fsv-api `notes.rs:24`, `:33`, `:263-302` |
| `612c117` | 11-12 01:00 | fix(writes): retain ambiguous 403 outcomes | ported | fsv-api `service.rs:298`; fsv-flows `ticket.rs:352` |
| `257d913` | 11-12 01:08 | feat(tickets): support incident CC recipients | ported | fsv-flows `ticket.rs:186-218`; fsv-cli `tickets.rs:541`, `:588` |
| `72f7aca` | 11-12 01:12 | feat(sr): support requested-for and CC recipients | ported | fsv-api `catalog.rs:43`, `:1095`, `:1210-1298`; fsv-cli `sr.rs:53`, `:406-440` |
| `ff348e8` | 11-12 01:17 | feat(tickets): review tag and planned date updates | ported | fsv-api `properties.rs:142-321`; fsv-cli `resource.rs:1326-1410` |
| `c70df11` | 11-12 01:23 | feat(tickets): review reply CC and pending status | ported | fsv-api `conversations.rs`; fsv-cli `tickets.rs:306`, `:474-520` |
| `fcd5db4` | 11-12 01:36 | fix(sr): guard ambiguous outcomes and recipient identity | ported | fsv-api `catalog.rs:847`, `:1444`, `:1499`, `:1547`, `:1573-1611`, `:1685`; fsv-cli `sr.rs:330`; fsv-app `form.rs:463` |
| `eeda2a6` | 11-12 01:44 | fix(tickets): verify incident CC after creation | ported | fsv-flows `ticket.rs:395-426`; fsv-cli `tickets.rs:696-706` |
| `7575578` | 11-12 01:48 | fix(writes): reject control characters in recipients | ported | fsv-flows `ticket.rs:188`; fsv-api `catalog.rs:1073`; fsv-cli `sr.rs:80` |
| `92a660a` | 11-12 02:01 | fix(tui): handle download filename collisions | ported | fsv-tui `app.rs:3389-3420`; fsv-cli `sr.rs:1030-1038`; fsv-api `downloads.rs:142` |
| `1eedd9b` | 11-12 02:09 | feat(tui): add reviewed change approval actions | ported | fsv-tui `app.rs:2894-2918`, `:3168-3200`, `modals.rs:462`; fsv-app `tui.rs:1012-1066` |
| `b83231c` | 11-12 02:14 | fix(tickets): nest custom fields in update payloads | ported | fsv-cli `resource.rs:1461-1466` |
| `fd060d7` | 11-12 02:20 | fix(tickets): show actual identity in update previews | ported | fsv-cli `resource.rs:1486-1501` |
| `42b561f` | 11-12 02:27 | fix(sr): accept requested-for in catalog drafts | ported | fsv-flows `sr.rs:27-120`, `:314`; fsv-cli `sr.rs:406` |
| `ccc3a64` | 11-12 02:58 | fix(forms): resolve metadata-backed custom lookup selections | ported | fsv-forms `state.rs:840`; fsv-api `formmeta.rs:787-943`; fsv-formui `lookups.rs` |
| `7903e2a` | 11-12 03:06 | fix(changes): route legacy updates through reviewed form plans | partial | fsv-flows (plan JSON keys) |
| `d18bab9` | 11-12 03:11 | fix(schema): expose stale metadata and synchronous refresh | partial | fsv-cli (`lookup --refresh`), fsv-completion |
| `f761a01` | 11-12 03:20 | feat(tickets): review metadata-backed property updates | missing | fsv-api, fsv-cli, fsv-completion |
| `a5a8381` | 11-12 03:25 | feat(tickets): add reviewed BCC replies and whole-ticket forwards | missing | fsv-api, fsv-cli, fsv-completion |
| `ddd756a` | 11-12 03:30 | feat(watchers): add reviewed ticket and change self-watch workflows | missing | fsv-core/fsv-api (`me`), fsv-api, fsv-cli, fsv-completion |
| `9622e3f` | 11-12 03:35 | feat(tickets): add reviewed nonbillable self time CRUD | missing | fsv-api, fsv-cli, fsv-completion |
| `5bc5d4d` | 11-12 03:45 | feat(tui): review ticket recipient and status conversation options | partial | fsv-app (fsv-tui done) |
| `f711010` | 11-12 03:51 | feat(planning): add reviewed native text saves and TUI editor | partial | fsv-api, fsv-cli, fsv-app, fsv-completion (fsv-tui done) |
| `3057492` | 11-12 04:00 | feat(tui): edit reviewed ticket properties and SR beneficiaries | partial | fsv-api, fsv-app (fsv-tui done) |
| `f7bf04a` | 11-12 04:07 | feat(problems): add reviewed known-error mark and TUI action | partial | fsv-api, fsv-cli, fsv-app, fsv-completion (fsv-tui done) |
| `1dc7ab6` | 11-12 04:13 | test(writes): exercise ticket property and mail transport guards | partial | fsv-api |
| `c0210a7` | 11-12 04:20 | feat(tickets): add reviewed explicit-ID bulk priority | missing | fsv-api, fsv-cli, fsv-completion |
| `774f4f1` | 11-12 04:26 | fix(tickets): invalidate old beneficiary rule context | missing | fsv-api |
| `4d778c3` | 11-12 04:38 | fix(readback): compare durations and offset timestamps semantically | partial | fsv-api |
| `b7d96a7` | 11-12 04:43 | fix(cli): accept consistent JSON options on reviewed commands | missing | fsv-cli, fsv-completion |

## Live regressions in the Rust TUI (fix first)

The fsv-tui half of `5bc5d4d` / `3057492` is ported, but `LiveBackend`
(`crates/fsv-app/src/tui.rs:478`) keeps the default `Failure::unavailable`
for every reviewed hook. So in the real binary:

- A plain **ticket reply** (`r`, Ctrl+S) now always goes through
  `prepare_ticket_conversation` (`crates/fsv-tui/src/app.rs:2575`) and fails
  with "ticket conversations unavailable". Before the port of `5bc5d4d` it
  used `add_reply`.
- **`e` on a ticket** calls `load_ticket_property_fields`
  (`crates/fsv-tui/src/app.rs:3737`) and fails with "ticket property edits
  unavailable".
- `a` → known error / planning text fail the same way; those were new
  features, so nothing regressed there.

The fsv-app work under `5bc5d4d` and `3057492` removes both regressions. The
reply hook only needs the reply/note-with-CC/Pending part of
`fsv_api::conversations`, which already exists, so it does not wait on the
BCC/forward work.

## Per-commit work

Python tests are in `/Users/phusit.somb/lab/work/fsv/tests/`. Copy expected
values from them.

### `273ffde` fix(writes): gate status changes and avoid replay — partial

Ported: `Client::int_request` fetches CSRF once and never replays
(`crates/fsv-core/src/client/mod.rs:657-675`).

Missing: `safety.write_risks` now flags **every** ticket/problem status
transition (same status is still not a risk). Rust keeps the old
terminal-only rule (`crates/fsv-core/src/safety.rs:140-144`).

- **fsv-core** `safety.rs`: for a non-change, non-task `status` that differs
  from current (or current is unknown), always push
  `"{what} status -> {label} (status semantics and workflows require confirmation)"`.
  Drop the `status_closes` match in `write_risks` (keep `status_closes` as a
  pub fn if other callers use it). Rename and invert the test
  `ticket_only_terminal_status_is_risky` (`safety.rs:246`).
- **fsv-tui** `actions.rs` tests: a ticket status plan to a non-terminal label
  ("Review") now carries a risk containing the label.
- **fsv-cli**: dry-run/confirm golden output for ticket/problem status
  updates gains `requires_confirmation`. Re-run `crates/fsv-cli/tests/write_cmds.rs`.
- Python tests: `tests/test_write_safety.py::test_ticket_every_status_transition_is_risky`,
  `tests/test_qa_regressions.py::test_localized_terminal_status_needs_confirmation`
  ("confirmation" substring; "Pending final closure" is risky),
  `tests/test_tui_actions.py::test_ticket_every_status_transition_warns`,
  `tests/test_change_task_wire.py::test_internal_mutation_never_replayed_after_ambiguous_failure`
  (already covered by `crates/fsv-core/src/client/tests.rs:972`; add a
  403/disconnect × POST/PUT/PATCH/DELETE single-request case if absent).

### `7903e2a` fix(changes): route legacy updates through reviewed form plans — partial

Ported: `--status/--priority/--agent/--group` on `changes update` go through
the form plan (`crates/fsv-cli/src/cmd/changes.rs:1452-1471`).

Missing: `change_flow.Plan.to_dict()` gained `"risks": self.risks` and
`"requires_confirmation": bool(self.risks)` between `warnings` and `errors`.
`fsv_flows::change::Plan::to_json` (`crates/fsv-flows/src/change.rs:200-209`)
has neither key.

- **fsv-flows** `change.rs`: add both keys in Python order (`review`,
  `warnings`, `risks`, `requires_confirmation`, `errors`). Update the
  change-plan JSON goldens in `crates/fsv-flows/src/change/tests.rs` and
  `fsv-cli` `changes update --dry-run` fixtures.
- **fsv-cli**: update the `changes update` help text: "Legacy
  --status/--priority/--agent/--group flags use the same reviewed, hash-bound
  form path." (it comes with the tree regeneration, below).
- Python tests: `tests/test_create_flags.py::test_update_resolves_labels`,
  `::test_legacy_status_hash_matches_set_and_refuses_mismatch` (legacy hash ==
  `--set` hash; mismatched `--expect-plan` exits 4 with no writes; send is
  `("PUT", "changes/501", {"status": 2})`),
  `tests/test_write_safety.py::test_dry_run_never_asks_and_reports_risks`
  (`out["payload"] == {"status": 2}`, `out["risks"]` non-empty, `plan_hash`
  set), `::test_change_planning_status_needs_yes`, `::test_same_status_is_not_gated`,
  `::test_reassign_gated`.

### `d18bab9` fix(schema): expose stale metadata and synchronous refresh — partial

Ported: `schema::load` returns `stale`/`source`, warns on stale cache,
`force` bypasses cache (`crates/fsv-api/src/schema.rs:261-379`);
`fields --refresh` exists (`crates/fsv-cli/src/cmd/resource.rs:738`).

Missing: `fsv {changes,tickets,problems} lookup --refresh` ("fetch current
schema choices synchronously") → `schema.load(res, c, force=True)`.

- **fsv-cli**: add `refresh: bool` to `ResLookupArgs`
  (`crates/fsv-cli/src/generated.rs:830`) and pass it to `schema::load` in
  `lookup` (`crates/fsv-cli/src/cmd/resource.rs:837`).
- **fsv-completion**: the new flag shows up in the completion tree/goldens.
- Python tests: `tests/test_schema_cache.py` (all four new tests; mostly
  covered by `schema.rs:538-620`).

### `f761a01` feat(tickets): review metadata-backed property updates — missing

`tickets update --reviewed` turns `--set FIELD=VALUE` and
`--status/--priority/--agent/--group` into **answers** for
`ticket_properties.build_plan`. Rust `properties::build_plan` has no
`answers` (`crates/fsv-api/src/properties.rs:142`).

- **fsv-api** `properties.rs`:
  - `PropertyPlan` gains `answers_json` (default `"{}"`) and `metadata_hash`
    (default `""`). `hash()` adds `"metadata_hash"` to the intent **only when
    non-empty**, so legacy tag/date hashes do not change.
  - `build_plan(..., answers: Option<&Map>)`: `"specify --set, --tag,
    --planned-start or --planned-end"` when body and answers are both empty.
    Build `FormState` with `ClientResolver` (`formmeta.rs:787`; note it takes
    `Arc<dyn Api + Send + Sync>`, so the signature needs an Arc or a
    borrowed resolver). For each answer: `"{label}: duplicate property
    answer"`, `"{label}: field is not writable"` (`!sent || synthetic ||
    kind == notice`), `state.set`. Then for each answer:
    hidden → `"{label}: answered field is hidden"`; encode with
    `fsv_forms::payload::encode_value`; a status field whose choice is
    missing or labelled resolved/closed (casefold) →
    `"combined resolution/closure requirements are not proven; use the web
    form"`; custom → `body.custom_fields[api_name]`; else key =
    `requester_id` for `requester` or `TICKET_KEYS` mapping, refused with
    `"{label}: update wire is not proven"` unless in the allowed set
    `{subject, description, requester_id, group_id, responder_id, status,
    priority, urgency, impact, source, category, type}`. Recompute
    `missing`/`before` after answers.
  - Plan keeps `answers_json` (sorted JSON) and `metadata_hash =
    meta.fingerprint().hash` when answers are given.
  - `send` rebuilds with the stored answers; readback for `custom_fields`
    matches when every expected key is present and equal (subset, not whole
    dict).
- **fsv-cli** `resource.rs` `update` (`:1325`): add `--reviewed` ("tickets
  only: evaluate --set and core flags as a hash-bound property plan"). Enter
  the property path when `reviewed || tag || planned dates`. Build answers
  from `--set` (`"--set requires unique FIELD=VALUE answers"`) and core flags
  keyed `status`/`priority`/`agent`/`group` (`"duplicate property answer:
  {name}"`). Drop the old "do not combine ticket properties…" refusal. The
  `--expect-plan` misuse message becomes `"--expect-plan requires
  --reviewed, --tag, --planned-start or --planned-end"`.
- **fsv-completion**: `--reviewed` on `tickets update` / `problems update`.
- Python tests: `tests/test_ticket_properties.py::test_full_properties_runtime_enums_and_custom_readback`
  (`priority == 73`, `subject == "Globex"`, `custom_fields == {"cost_code":
  "new"}`, one write, verified), `::test_reviewed_cli_uses_full_hash_and_refuses_unproven_closure`,
  `::test_legacy_property_hash_identity_is_unchanged` (pins the old hash).

### `a5a8381` feat(tickets): add reviewed BCC replies and whole-ticket forwards — missing

Rust `ConversationArgs` has no `bcc`/`sender`, and `kind` accepts only
`reply`/`note` (`crates/fsv-api/src/conversations.rs:186-219`).

- **fsv-api** `conversations.rs`:
  - `ConversationArgs` gains `bcc: &[String]`, `sender: Option<&str>`; kind
    `forward` is allowed. `path()` maps reply→`reply`, forward→`forward`,
    note→`notes`.
  - Review warning: `"Replies and forwards send mail; notes may trigger
    tenant workflows. Explicit sender addresses are user-asserted, not
    verified mailbox configuration."`
  - Validation, in Python order: sender not bare `EMAIL` → `"sender must be
    a plain configured support email address"`; forward without
    sender/notify or with status/cc/bcc → `"whole-ticket forward requires
    explicit sender and recipients; no combined status/CC/BCC wire is
    proven"`; note with bcc/sender → `"note sender/BCC wire is not proven"`;
    bcc with cc/status → `"combined BCC/CC/status wire is not proven"`; bcc
    without sender or with a non-bare email → `"BCC requires a configured
    --sender and bare recipient emails"`.
  - Reply payload: with bcc set `bcc_emails` and **remove** `cc_emails`
    when there is no CC; with a sender set `from_email`. Forward payload:
    `{"body", "from_email", "notify_emails", "conversation_from_summary":
    false, "private": true}`; non-bare notify → `"forward requires bare
    recipient emails"`.
  - `send`: forward also needs `public_confirmed`. Before sending, call
    `recipients` again for each bcc address and (forward) each notify
    address. Readback `expected` includes `cc_emails` only if present,
    `bcc_emails` if present, `from_email` if present.
- **fsv-cli** `tickets.rs`: `reply --bcc` (repeatable, "BCC plain email from
  user lookup (repeatable; requires --sender)") and `--sender`; the
  simple-path test becomes `cc empty && bcc empty && sender none && status
  none && expect none`. BCC addresses go through `recipients` (result
  discarded) before `build_plan`. New `tickets forward ID BODY --to…
  --sender [--dry-run] [--yes] [--expect-plan]` ("Review and forward a whole
  ticket (not an individual conversation)."): dry-run prints `{"status":
  "dry_run", **review}`; otherwise requires `--yes` and an exact
  `--expect-plan`, else `"review with --dry-run, then pass --yes and the
  full --expect-plan hash"` (exit 2).
- **fsv-completion**: new command and flags.
- Python tests: `tests/test_ticket_conversation.py::test_bcc_and_whole_ticket_forward_exact_wire_hash_and_readback`
  (reply, forward), `::test_bcc_missing_readback_is_unavailable_not_verified`,
  `::test_bcc_and_forward_cli_require_exact_review`, `::test_forward_uncaptured_variants_refused`.

### `ddd756a` feat(watchers): add reviewed ticket and change self-watch workflows — missing

New Python modules `reviewed_write.py` (shared single-attempt primitive),
`watchers.py`, `watcher_cli.py`. Nothing in Rust.

- **fsv-api** (new `reviewed_write.rs`), the shared base for watchers, time,
  planning text, known error and bulk priority:
  - `WritePlan {action, target: i64, tenant, method, path, payload_json,
    context_hash}`; `hash = plan_hash({"action","target","tenant","method",
    "path","payload","context"})`; `review()` = `{"action","method","path":
    "/api/_/"+path,"payload","plan_hash","warning": "Single-attempt write.
    Unknown outcomes must be inspected, never automatically resent."}`.
  - `execute(plan, api, journal, reviewed_hash, rebuild, verify)`: None →
    `{"status":"cancelled","verified":false}`; tenant/hash →
    `"tenant or full review hash changed; review again"`; last journal event
    started/done → `"write was sent or outcome unknown; inspect target and
    journal before retry"`; rebuild hash differs → `"target or metadata
    changed since review; review again"`; csrf failure → `"could not prepare
    session; no write attempted"`; journal `started` not persisted →
    `"could not persist ambiguity journal; no write attempted"`; one
    `send_once` (body only when payload is not null; then Content-Type JSON);
    `definite_rejection` → journal failed + `"write rejected (HTTP {s}); no
    retry"`; else `"write outcome unknown; inspect target and journal before
    retry"`; done → `{"status":"submitted","plan_hash","verified":false}`
    merged with `verify(response)`; a verify error sets `"Independent
    readback failed; do not resend."`; unverified sets (setdefault)
    `"Readback was not fully verified; do not resend."`. Errors are exit 2
    (`ReviewedWriteError`).
  - Note: `path` may carry a query string (`changes/1/unwatch?user_id=9`,
    `…/planning-fields?id=KEY`). `send_once` must send it verbatim.
- **fsv-api** `Api` trait (`crates/fsv-api/src/service.rs:21`): add `me()`
  (`fsv_core::client::Client::me`, `client/mod.rs:778`) plus the
  fsv-cli `lazy.rs` / testkit / mock impls. Watchers, time entries and
  their self-identity checks need it; never use a cached identity.
- **fsv-api** new `watchers.rs`: `positive_id` (`"expected a positive numeric
  identity"`; rejects bools), `self_id`, `inventory` (GET
  `{module}/{id}/watchers`; requires a `watcher_ids` list and no
  `has_more/has_next/next_page`; duplicates and a `total` mismatch refused;
  sorted), `build_plan` (tickets: POST `tickets/{id}/watch` / PUT
  `tickets/{id}/unwatch` body `{"user_id"}`; changes: POST
  `changes/{id}/watchers` body `{"user_id"}` / DELETE
  `changes/{id}/unwatch?user_id={u}` no body; action
  `{module}_self_{watch|unwatch}`; context `plan_hash({"self","before","watch"})`;
  `payload_json = json.dumps(payload, sort_keys=True)` so `null` for
  DELETE), `send` (rebuild even for no-ops; already-in-state →
  `{"status":"already_in_requested_state","verified":true,"plan_hash"}`;
  verify compares the full expected membership).
- **fsv-cli**: `fsv {tickets,changes} watchers list ID` → `{"watcher_ids":
  […]}` (errors as Click BadParameter); `watchers set ID --watch|--unwatch
  [--dry-run] [--yes] [--expect-plan]` with the same `--yes`/hash rule as
  forward.
- **fsv-completion**: new group and commands.
- Python tests: `tests/test_watchers.py` (all; exact methods, ambiguous
  403/429/500 leaves `started`, stale identity, partial membership, no-op,
  readback/journal failures, CLI hash flow).

### `9622e3f` feat(tickets): add reviewed nonbillable self time CRUD — missing

New `ticket_time.py` / `ticket_time_cli.py`. Depends on `reviewed_write`,
`watchers.positive_id/self_id`, and `readback` (see `4d778c3`).

- **fsv-api** new `ticket_time.rs`:
  - `inventory`: `collect_pages(api, "tickets/{id}/time_entries",
    "time_entries", {"page":1,"per_page":30}, what="time entries")`
    (`crates/fsv-api/src/service.rs:356`); not complete → `"complete
    time-entry inventory is unavailable"`; duplicate ids refused; sorted by id.
  - `_duration`: `^\d{2,}:[0-5]\d$`, not all zeros, else `"manual time must
    be a positive HH:MM duration"`.
  - `build_plan(action create|update|delete, entry_id, spent, executed_at,
    note)` with every Python message (create needs spent+executed_at and no
    entry; executed_at via `tasks::date`, `"executed-at needs an explicit
    offset"`; payload `{executed_at, timer_running:false, billable:false,
    time_spent, note or "", agent_id: self}` POST; update/delete need a
    self-owned, `billable is False`, `timer_running is False` row, else
    `"exact self-owned nonbillable manual entry is unavailable"`; update
    keeps unchanged note/time and turns server `HH:MM:SS` into `HH:MM`
    (`"entry has sub-minute precision; provide --spent explicitly"`);
    `payload_json` with `ensure_ascii=False`, sorted). Context
    `plan_hash({"self","entries"})`, action `ticket_time_{action}`.
  - `send`: rebuild passes `note = payload.note or None`; create readback
    needs the exact returned `time_entry.id`; delete verifies absence;
    fields use `same_duration` / `same_instant` / strict type+value equality
    (`0` is not `false`).
- **fsv-cli**: `fsv tickets time list ID` (JSON inventory, indent 2) and
  `fsv tickets time write ID ACTION [--entry N] [--spent] [--executed-at]
  [--note] [--dry-run] [--yes] [--expect-plan]`.
- **fsv-completion**: new group and commands.
- Python tests: `tests/test_ticket_time.py` (all),
  `tests/test_readback_normalization.py::test_normalized_exact_id_transport_readback`,
  `::test_note_only_update_preserves_normalized_minutes` (`{"time_spent":
  "125:05", "note": "Globex"}`).

### `5bc5d4d` feat(tui): review ticket recipient and status conversation options — partial

Ported in fsv-tui: Ctrl+O option menu, `ConversationRequest` with
cc/notify/bcc/sender/Pending, plain ticket replies routed to the reviewed
path (`crates/fsv-tui/src/app/reviewed.rs:479-585`, `app.rs:2567-2580`,
`modals.rs:210-259`).

Missing:

- **fsv-app** `tui.rs` `impl Backend for LiveBackend`: implement
  `ticket_conversation_plan` (recipients for cc and notify; bcc goes through
  `recipients` only as a check; `conversations::build_plan` with kind
  reply/note, public, set_status; return a `ReviewPlan` whose `raw` is the
  payload, because the TUI reads `private` from it) and
  `send_ticket_conversation` (`conversations::send(…, public_confirmed)`).
  This fixes the plain-reply regression above.
- **fsv-api**: bcc/sender come from `a5a8381`. Until then the backend should
  refuse a request with bcc/sender using the Python validation text, not
  drop the values silently.
- Python tests: `tests/test_tui_ticket_options.py` (both),
  `tests/test_tui_actions.py` reply test (plain reply calls the reviewed
  send with `"<div>thanks</div>"`, after a PUBLIC consent screen).

### `f711010` feat(planning): add reviewed native text saves and TUI editor — partial

Ported in fsv-tui: `a` → "Edit native planning/analysis text", section
picker, editor, review, outcome (`reviewed.rs:394-475`, `modals.rs:1328`).

Missing:

- **fsv-api** new `planning_text.rs` on `reviewed_write`:
  `PROBLEM_SECTIONS = {"root cause": "problem_cause", "impact":
  "problem_impact", "symptoms": "problem_symptom"}`; `definitions(module)`
  (changes: `create::planning_definitions(schema::load(CHANGES, force=true).fields)`;
  problems: `{"name": key, "label": label.title()}`; else `"planning wire is
  proven only for changes and the three native problem analysis
  sections"`); `inventory` (GET `{module}/{id}/planning-fields`, list key
  `change_planning_fields` | `problem_planning_fields`, completeness and
  identity checks); `build_plan` (empty description refused; changes via
  `create::resolve_planning_definition`; problems exact label/name, else
  `"choose Root Cause, Impact or Symptoms; other analysis wires are not
  proven"`; an existing row with `attachments` missing or non-empty →
  `"planning attachments are present or unavailable; text-only preservation
  is not proven"`; POST `{module}/{id}/planning-fields?id={quote(key,
  safe='')}` body `{"description"}`; action `{module}_planning_text`;
  context `plan_hash({"definitions","rows"})`); `send` parses `id` from the
  query and verifies `description` on the matched row.
  Helpers exist: `fsv_api::create::{planning_definitions,
  planning_server_key, resolve_planning_definition, match_planning_row}`
  (`create.rs:1013-1142`).
- **fsv-cli**: `fsv {changes,problems} planning-text ID --section S
  --description D [--dry-run] [--yes] [--expect-plan]`.
- **fsv-app**: `planning_definitions`, `planning_row` (`match_planning_row`
  on `inventory`), `planning_text_plan`, `send_planning_text`.
- **fsv-completion**: new commands.
- Python tests: `tests/test_planning_text.py` (all four; keys
  `acme_plan` for changes "Rollout Plan", `problem_cause` for "Root Cause").

### `3057492` feat(tui): edit reviewed ticket properties and SR beneficiaries — partial

Ported in fsv-tui: `e` on a ticket opens the property picker → value/choice →
review → outcome (`reviewed.rs:305-357`, `app.rs:3737`).

Missing:

- **fsv-api** `properties.rs` (on top of `f761a01`): a beneficiary answer
  (`api_name == "requested_for" && field_type == "default_requested_for"`)
  is writable even when `!sent`; allowed keys gain `requested_for_id`; key
  is `{api_name}_id` for `requester` and `requested_for`.
- **fsv-app** `tui.rs`: `ticket_property_fields` (`load_meta("ticket",
  "edit", fresh, module_id)`; keep fields where `(sent || field_type ==
  default_requested_for) && !synthetic && is_input && kind != unsupported &&
  (is_custom || api_name in {subject, description, requester,
  requested_for, group, agent, status, priority, urgency, impact, source,
  category, ticket_type})`; dropdowns carry visible `(value, label)`
  choices), `ticket_property_plan` (`build_plan(answers={name: value})`),
  `send_ticket_property`. This fixes the `e` regression above.
- Python tests: `tests/test_tui_ticket_properties.py::test_tui_property_field_picker_review_and_outcome`,
  `::test_requested_for_update_is_property_only_and_resolves_current_user`,
  `::test_readonly_property_picker_never_prepares`,
  `tests/test_tui_views.py` (`e` on a ticket opens the property picker).

### `f7bf04a` feat(problems): add reviewed known-error mark and TUI action — partial

Ported in fsv-tui: `a` → "Mark as known error", review, outcome
(`reviewed.rs:356-392`, `app.rs:3184-3197`).

Missing:

- **fsv-api** new `problem_known_error.rs` on `reviewed_write`: `record`
  (GET `problems/{id}`, unwrap `problem`; exact id and a **bool**
  `known_error`, else `"exact problem identity/known-error state is
  unavailable"`); `build_plan` PUT `problems/{id}` `{"known_error": true}`
  (`json.dumps` without sort/indent: `{"known_error": true}`), action
  `problem_known_error_mark`, context `plan_hash(row)`; `send` rebuilds,
  then None → cancelled; hash/tenant → `"problem or review changed; review
  again"`; already true → `{"status": "already_in_requested_state",
  "verified": true}`; verify re-reads the flag.
- **fsv-cli**: `fsv problems mark-known-error ID [--dry-run] [--yes]
  [--expect-plan]` ("Review marking a problem as a known error; no reversal
  wire is proven.").
- **fsv-app**: `known_error_plan`, `send_known_error`.
- **fsv-completion**: new command.
- Python tests: `tests/test_problem_known_error.py` (all four).

### `1dc7ab6` test(writes): exercise ticket property and mail transport guards — partial

Missing source change: before sending, `ticket_conversation.send` re-resolves
formatted CC recipients and refuses if they differ, without changing the
reviewed bytes.

- **fsv-api** `conversations.rs` `send`: when `payload.cc_emails` is
  non-empty, take each value's address (`email.utils.parseaddr(v)[1]`), call
  `recipients(api, emails)` and compare with the stored list; on a
  difference → `"CC recipient identity changed; review again"` (exit 2, no
  POST).
- **fsv-api** tests: port the transport-level cases (exact HTTP bytes and
  headers, Acme/Globex runtime status ids, 403/429/500/disconnect leave the
  `started` guard, wrong-id/missing/contradicting readback never resends,
  Resolved/Closed refused).
- Python tests: `tests/test_ticket_conversation_http.py` (all),
  `tests/test_ticket_properties_http.py` (all; the property side is already
  implemented, so these are test ports only).

### `c0210a7` feat(tickets): add reviewed explicit-ID bulk priority — missing

Depends on `reviewed_write` and on `f761a01` (property answers).

- **fsv-api** new `bulk_priority.rs`: 1–100 ids (`"specify between 1 and 100
  explicit ticket IDs"`), positive and unique (`"ticket IDs must be positive
  and unique"`); one `properties::build_plan(id, answers={"priority":
  label})` per id; all payload priorities must be one non-null enum, else
  `"ticket metadata resolves different priorities; separate reviews are
  required"`; body `{"priority", "ids", "reply": {}}` PUT
  `bulk_ticket_actions/update_multiple`, action `ticket_bulk_priority`,
  target = first id, context `plan_hash([p.hash…])`, `payload_json` sorted.
  `review()` = WritePlan review + `before` (`[{"ticket_id", "priority"}]`) +
  `priority_label`, with warning `"Explicit tickets only. Bulk actions may
  notify people; partial per-ticket outcomes are possible."`. Verify reads
  each ticket with `properties::record` and reports per-id
  matched/mismatched/unavailable.
- **fsv-cli**: `fsv tickets bulk-priority ID... --priority P [--dry-run]
  [--yes] [--expect-plan]`; exit 1 when the result is unverified; `ValueError`,
  `ReviewedWriteError` and `PropertyError` → exit 2.
- **fsv-completion**: new command (`--priority` gets no completer in Python).
- Python tests: `tests/test_bulk_priority.py` (all; fixtures
  `("acme.example.com", 123, 87)`, `("globex.example.com", 742, 26)`).

### `774f4f1` fix(tickets): invalidate old beneficiary rule context — missing

- **fsv-api** `properties.rs` (with `f761a01`/`3057492`): when any answer's
  field is `requested_for`, set `engine.context.requested_for = None`
  before building `FormState`, as `catalog.rs:1095` already does for SR
  create.
- Python tests: `tests/test_tui_ticket_properties.py::test_beneficiary_edit_never_reuses_previous_person_rule_context`.

### `4d778c3` fix(readback): compare durations and offset timestamps semantically — partial

Rust compares planned/task dates with `tasks::same_datetime`
(`crates/fsv-api/src/tasks.rs:179`), which treats two naive datetimes as
equal and runs `py_str` on non-string values. Python's `same_instant`
returns False unless **both** values are strings with an explicit offset.

- **fsv-api** (new `readback.rs`, or next to `tasks.rs`):
  `same_instant(actual: &Value, expected: &Value) -> bool` (string only,
  `Z`→`+00:00`, `fromisoformat`, offset required, compare UTC; any error →
  false); `duration_seconds(&Value) -> Result<i64>` (`^\d{2,}:[0-5]\d(?::[0-5]\d)?$`,
  no 24h wrap); `same_duration`. Switch `properties::send`
  (`properties.rs:295-303`) and the task readback (`tasks.rs:440`) to
  `same_instant`. `ticket_time` uses all three.
- Python tests: `tests/test_readback_normalization.py::test_duration_semantics`,
  `::test_timestamp_semantics` (expected `"2030-01-02T03:00:00+07:00"`;
  naive `"2030-01-02T03:00:00"`, date-only, `None`, `False` and `"invalid"`
  are false), plus the two transport tests listed under `9622e3f`.

### `b7d96a7` fix(cli): accept consistent JSON options on reviewed commands — missing

`ReviewOutput` is a one-value enum (`json`). Each reviewed command added in
this range takes `--output/-o` (`"output format (JSON only)"`, default
`json`) and `--json` (`"alias for --output json"`). Their output stays the
same. Any other `--output` value fails as a Click choice error (exit 2)
**before** a client is opened. The commands are `{tickets,changes} watchers
list|set`, `tickets time list|write`, `{changes,problems} planning-text`,
`problems mark-known-error`, `tickets bulk-priority` and `tickets forward`.

- **fsv-cli**: add a `ReviewOutput` value enum (`json` only) in `types.rs`
  and both options on each of those commands as you add them. They change
  nothing except accepting the flags.
- **fsv-completion**: `--output` completes to `json` only on those commands.
- Python tests: `tests/test_reviewed_cli_output.py::test_review_and_send_output_aliases`
  (flag sets `[]`, `--json`, `--output json`, `-o json`, `--output=json
  --json`), `::test_unsupported_format_refused_before_client`,
  `::test_read_output_aliases`.

## Suggested ordering, one owner per crate

Dependencies run fsv-core → fsv-api → fsv-cli / fsv-app → fsv-completion.
fsv-tui needs no new work beyond test updates.

1. **fsv-core** (independent, small)
   - `273ffde`: ticket/problem status risk text in `safety.rs` and its tests.
2. **fsv-flows** (independent, small)
   - `7903e2a`: `risks` and `requires_confirmation` in `Plan::to_json` and
     the goldens.
3. **fsv-api** (critical path; land in this order)
   1. `Api::me()` and its impls (fsv-cli `lazy.rs`, mocks).
   2. `readback.rs` (`4d778c3`), switching properties and tasks to it.
   3. `reviewed_write.rs` (`ddd756a` base).
   4. `properties.rs` answers + `--reviewed` model (`f761a01`), beneficiary
      key (`3057492`), context reset (`774f4f1`).
   5. `conversations.rs`: CC re-check (`1dc7ab6`), bcc/sender/forward
      (`a5a8381`).
   6. New modules: `watchers.rs` (`ddd756a`), `problem_known_error.rs`
      (`f7bf04a`), `planning_text.rs` (`f711010`), `ticket_time.rs`
      (`9622e3f`), `bulk_priority.rs` (`c0210a7`, after step 4).
   7. Port the HTTP transport tests (`1dc7ab6`).
4. **fsv-app** (start right away for the regressions)
   - Now: `ticket_conversation_plan` / `send_ticket_conversation` with the
     existing `conversations` API (`5bc5d4d`); refuse bcc/sender until fsv-api
     step 5 lands.
   - After fsv-api step 4: `ticket_property_fields/plan/send` (`3057492`).
   - After fsv-api step 6: `known_error_plan/send` (`f7bf04a`),
     `planning_definitions/row/text_plan/send` (`f711010`).
5. **fsv-cli** (the tree can start now; commands follow fsv-api)
   - Re-dump `crates/fsv-cli/tests/fixtures/cli_tree.json` with
     `crates/fsv-cli/tests/golden/dump_cli_tree.py` from the Python repo, and
     update `generated.rs` to match (`tests/tree.rs` checks it). The
     `gen_tree.py` named in the `generated.rs` header is not in the
     worktree, so either restore it or edit `generated.rs` by hand. The tree
     diff against Python at `b7d96a7` is: new `changes planning-text`, `changes watchers
     list|set`, `problems mark-known-error`, `problems planning-text`,
     `tickets bulk-priority`, `tickets forward`, `tickets time list|write`,
     `tickets watchers list|set`; `--refresh` on `{changes,problems,tickets}
     lookup`; `--reviewed` on `{tickets,problems} update`; `--bcc` and
     `--sender` on `tickets reply`. Every new command also has
     `--output/-o {json}` and `--json` (`b7d96a7`).
   - Commands: `lookup --refresh` (`d18bab9`), `update --reviewed`
     (`f761a01`), `reply --bcc/--sender` and `forward` (`a5a8381`),
     `watchers` (`ddd756a`), `time` (`9622e3f`), `planning-text` and
     `mark-known-error` (`f711010`, `f7bf04a`), `bulk-priority` (`c0210a7`),
     the `changes update` help text (`7903e2a`), and the JSON-only output
     options on every new reviewed command (`b7d96a7`).
   - Refresh the write goldens for `273ffde`.
6. **fsv-completion** (after the fsv-cli tree)
   - Re-dump `completion_tree.json` (`crates/fsv-cli/tests/golden/dump_completion.py`)
     and the bash/zsh/fish goldens for the new commands and flags. None of
     the new options has a Python completer.
7. **fsv-tui** (tests only)
   - `273ffde`: a ticket non-terminal status plan now has a risk.

No work for **fsv-forms**, **fsv-views** or **fsv-formui** in this range.
