# Upstream ledger

Python commits that landed while the port ran, and what the Rust port still
needs for each.

- **Last triaged Python commit: `92002d7`** (2026-11-12 06:17 +0700,
  `Guard native planning text creates against existing content`; see
  "Ledger: `b7d96a7..6797301`", "Ledger: `6797301..dbe553c`" and
  "Ledger: `dbe553c..92002d7`" at the end).
  Range: `273ffde~1..b7d96a7`, 32 commits.
- Rust state graded: `rust-port` at `8790060` **plus the uncommitted/staged
  working tree** at triage time (other workers had edits in `fsv-api`,
  `fsv-core`, `fsv-forms`, `fsv-flows`, `fsv-views`, `fsv-completion`).
  Re-check a row if its file changed since then.
- Python had uncommitted work after `b7d96a7`, not graded here:
  `src/fsv/canned.py`, `canned_cli.py`, `scenarios.py`,
  `tests/test_canned.py`, and edits to `cli.py` / `ticket_conversation.py`.
- Next triage: `git -C /Users/phusit.somb/lab/work/fsv log --reverse 92002d7..HEAD`.

Statuses: `ported` (Rust has the behavior), `partial` (some is missing),
`missing` (nothing in Rust).

Counts: **30 ported, 2 partial, 0 missing.** (fsv-api half of `f761a01`…`4d778c3` landed after triage; the fsv-cli/fsv-completion half after that, with goldens captured from Python `dbe553c`; see the per-commit notes.)

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
| `92a660a` | 11-12 02:01 | fix(tui): handle download filename collisions | ported | fsv-tui `app/writes.rs:407-455`; fsv-cli `sr.rs:1030-1038`; fsv-api `downloads.rs:142` |
| `1eedd9b` | 11-12 02:09 | feat(tui): add reviewed change approval actions | ported | fsv-tui `app/cont.rs:355-397`, `app/writes.rs:181-184, 355-405`, `modals.rs:462`; fsv-app `tui.rs:1012-1066` |
| `b83231c` | 11-12 02:14 | fix(tickets): nest custom fields in update payloads | ported | fsv-cli `resource.rs:1461-1466` |
| `fd060d7` | 11-12 02:20 | fix(tickets): show actual identity in update previews | ported | fsv-cli `resource.rs:1486-1501` |
| `42b561f` | 11-12 02:27 | fix(sr): accept requested-for in catalog drafts | ported | fsv-flows `sr.rs:27-120`, `:314`; fsv-cli `sr.rs:406` |
| `ccc3a64` | 11-12 02:58 | fix(forms): resolve metadata-backed custom lookup selections | ported | fsv-forms `state.rs:840`; fsv-api `formmeta.rs:787-943`; fsv-formui `lookups.rs` |
| `7903e2a` | 11-12 03:06 | fix(changes): route legacy updates through reviewed form plans | ported | fsv-flows and fsv-cli done |
| `d18bab9` | 11-12 03:11 | fix(schema): expose stale metadata and synchronous refresh | ported | fsv-cli; fsv-completion goldens re-dumped (`--refresh`) |
| `f761a01` | 11-12 03:20 | feat(tickets): review metadata-backed property updates | ported | fsv-api (`properties.rs:128`, `:189`, `:374`, `:462`); fsv-cli `update --reviewed`/`--set` answers (`resource.rs` `ticket_properties`); fsv-completion |
| `a5a8381` | 11-12 03:25 | feat(tickets): add reviewed BCC replies and whole-ticket forwards | ported | fsv-api (`conversations.rs:169`, `:206`, `:242`, `:394`); fsv-cli `tickets.rs` `forward`, `reply --bcc/--sender`; fsv-completion |
| `ddd756a` | 11-12 03:30 | feat(watchers): add reviewed ticket and change self-watch workflows | ported | fsv-api (`service.rs:68`, `reviewed_write.rs:23`, `:64`, `watchers.rs:21`, `:70`, `:119`); fsv-cli `LazyApi` forwards `me`/`v2_get`, `reviewed.rs` `watchers_list`/`watchers_set`; fsv-completion |
| `9622e3f` | 11-12 03:35 | feat(tickets): add reviewed nonbillable self time CRUD | ported | fsv-api (`ticket_time.rs:48`, `:71`); fsv-cli `reviewed.rs` `time_list`/`time_write`; fsv-completion |
| `5bc5d4d` | 11-12 03:45 | feat(tui): review ticket recipient and status conversation options | ported | fsv-tui; fsv-api `conversations::build_plan_mail`; fsv-app `tui.rs` `ticket_conversation_plan` |
| `f711010` | 11-12 03:51 | feat(planning): add reviewed native text saves and TUI editor | ported | fsv-api (`planning_text.rs:34`, `:132`); fsv-tui and fsv-app (`LiveBackend::planning_*`); fsv-cli `reviewed.rs` `planning_text`; fsv-completion |
| `3057492` | 11-12 04:00 | feat(tui): edit reviewed ticket properties and SR beneficiaries | ported | fsv-api `properties.rs:218`, `:374`; fsv-tui; fsv-app `tui.rs` `ticket_property_plan` / `send_ticket_property` (`build_plan_with` / `send_with`) |
| `f7bf04a` | 11-12 04:07 | feat(problems): add reviewed known-error mark and TUI action | ported | fsv-api (`problem_known_error.rs:36`); fsv-tui and fsv-app (`LiveBackend::known_error_plan` / `send_known_error`); fsv-cli `reviewed.rs` `mark_known_error`; fsv-completion |
| `1dc7ab6` | 11-12 04:13 | test(writes): exercise ticket property and mail transport guards | partial | fsv-api CC re-check ported (`conversations.rs:389`); the byte/header-level HTTP tests are ported as `FakeApi` tests only |
| `c0210a7` | 11-12 04:20 | feat(tickets): add reviewed explicit-ID bulk priority | ported | fsv-api (`bulk_priority.rs:49`, `:125`); fsv-cli `reviewed.rs` `bulk_priority`; fsv-completion |
| `774f4f1` | 11-12 04:26 | fix(tickets): invalidate old beneficiary rule context | ported | fsv-api `properties.rs:218` |
| `4d778c3` | 11-12 04:38 | fix(readback): compare durations and offset timestamps semantically | ported | fsv-api `readback.rs:15`, `:29`, `:50`; `properties.rs:540`, `tasks.rs:428` |
| `b7d96a7` | 11-12 04:43 | fix(cli): accept consistent JSON options on reviewed commands | ported | fsv-cli `types.rs` `ReviewOutput`; fsv-completion |

## Live regressions in the Rust TUI

The fsv-tui half of `5bc5d4d` / `3057492` routed two existing actions
through new reviewed `Backend` hooks. `LiveBackend` (`crates/fsv-app/src/tui.rs`)
implements all of them. **Fixed:**

- A ticket reply (`r`, Ctrl+S) or note, with CC, notify, Pending, Sender and
  BCC, goes through `ticket_conversation_plan` / `send_ticket_conversation`
  over `conversations::build_plan_mail`. The send re-checks CC and BCC
  identities, as Python's does (`1dc7ab6`).
- `e` on a ticket lists the fields, and an edit goes through
  `properties::build_plan_with` / `send_with`.
- `a` → known error and planning text, and `i` → asset candidates, are wired.

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

### `7903e2a` fix(changes): route legacy updates through reviewed form plans — ported

Ported: `--status/--priority/--agent/--group` on `changes update` go through
the form plan (`crates/fsv-cli/src/cmd/changes.rs:1452-1471`).

Ported (fsv-flows): `fsv_flows::change::Plan::to_json`
(`crates/fsv-flows/src/change.rs:200-211`) emits `"risks"` and
`"requires_confirmation"` (`!risks.is_empty()`) between `warnings` and
`errors`, on every change and ticket plan, as Python's `to_dict`.
`flows.json` regenerated with `gen_flows.py` (11 create, 5 update, 4 ticket
plans gained the keys; nothing else moved). The change and ticket golden
tests now also check top-level key order. No fsv-cli fixture holds a change
form plan JSON, so no fsv-cli golden moved.

Ported (fsv-cli): the `Reviewed::to_json` fallback for a form plan without
a carried `change::Plan` (`crates/fsv-cli/src/cmd/formflow.rs`) emits
`"risks": []` and `"requires_confirmation": false` in the same place (a
formui plan has no risks). The docstring change is not observable: the
command passes `help="Update change fields."`, which Typer shows instead of
the docstring, and the docstring's first line (the completion hint table's
text) did not change.

Not ported, Python tests: `tests/test_create_flags.py::test_update_resolves_labels`,
  `::test_legacy_status_hash_matches_set_and_refuses_mismatch` (legacy hash ==
  `--set` hash; mismatched `--expect-plan` exits 4 with no writes; send is
  `("PUT", "changes/501", {"status": 2})`),
  `tests/test_write_safety.py::test_dry_run_never_asks_and_reports_risks`
  (`out["payload"] == {"status": 2}`, `out["risks"]` non-empty, `plan_hash`
  set), `::test_change_planning_status_needs_yes`, `::test_same_status_is_not_gated`,
  `::test_reassign_gated`.

### `d18bab9` fix(schema): expose stale metadata and synchronous refresh — ported

Ported: `schema::load` returns `stale`/`source`, warns on stale cache,
`force` bypasses cache (`crates/fsv-api/src/schema.rs:261-379`);
`fields --refresh` exists (`crates/fsv-cli/src/cmd/resource.rs:738`).

Ported (fsv-cli): `fsv {changes,tickets,problems} lookup --refresh` ("fetch
current schema choices synchronously") → `schema::load(res, c, true)`.
`ResLookupArgs.refresh` in `generated.rs`; the three `lookup` nodes of
`tests/fixtures/cli_tree.json` were re-dumped from Python (only `--refresh`
added); goldens `lookup_refresh_refetches_schema` and
`lookup_uses_cached_schema` in `read_cmds.rs`.

Ported (fsv-completion): `completion_tree.json` and the bash/fish/zsh
goldens re-dumped; the three `lookup` hints gained `refresh`.

Not ported:

- Python tests: `tests/test_schema_cache.py` (all four new tests; mostly
  covered by `schema.rs:538-620`).

### `f761a01` feat(tickets): review metadata-backed property updates — ported

**fsv-api ported:** `properties::build_plan_with` / `send_with` with
`PropertyArgs.answers`, returning `AnsweredPropertyPlan` (`properties.rs:128`,
`:374`, `:462`). `PropertyPlan` keeps its five fields so existing struct
literals compile; the old `build_plan` / `send` wrap the new ones.

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
- **fsv-cli** (done; `property_answers` / `ticket_properties` in
  `resource.rs`, goldens `update_reviewed_*` and `update_expect_plan_needs_reviewed`
  in `tests/upstream_cmds.rs`; `write_cmds.rs` `update_props_mixed` re-captured)
  `resource.rs` `update` (`:1325`): add `--reviewed` ("tickets
  only: evaluate --set and core flags as a hash-bound property plan"). Enter
  the property path when `reviewed || tag || planned dates`. Build answers
  from `--set` (`"--set requires unique FIELD=VALUE answers"`) and core flags
  keyed `status`/`priority`/`agent`/`group` (`"duplicate property answer:
  {name}"`). Drop the old "do not combine ticket properties…" refusal. The
  `--expect-plan` misuse message becomes `"--expect-plan requires
  --reviewed, --tag, --planned-start or --planned-end"`.
- **fsv-completion** (done): `--reviewed` on `tickets update` / `problems update`.
- Python tests: `tests/test_ticket_properties.py::test_full_properties_runtime_enums_and_custom_readback`
  (`priority == 73`, `subject == "Globex"`, `custom_fields == {"cost_code":
  "new"}`, one write, verified), `::test_reviewed_cli_uses_full_hash_and_refuses_unproven_closure`,
  `::test_legacy_property_hash_identity_is_unchanged` (pins the old hash).

### `a5a8381` feat(tickets): add reviewed BCC replies and whole-ticket forwards — ported

**fsv-api ported:** `conversations::build_plan_mail(.., &MailArgs { bcc, sender })`
(`conversations.rs:206`, `:242`); `ConversationArgs` is unchanged and
`build_plan` = `build_plan_mail` with no BCC/sender. Forward path, warning,
BCC/notify re-lookup and readback at `:169`, `:394`.

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
- **fsv-cli** (done; goldens `reply_bcc_*` / `forward_*` in
  `tests/upstream_cmds.rs`, fixture `ticket_mail.json`) `tickets.rs`: `reply --bcc` (repeatable, "BCC plain email from
  user lookup (repeatable; requires --sender)") and `--sender`; the
  simple-path test becomes `cc empty && bcc empty && sender none && status
  none && expect none`. BCC addresses go through `recipients` (result
  discarded) before `build_plan`. New `tickets forward ID BODY --to…
  --sender [--dry-run] [--yes] [--expect-plan]` ("Review and forward a whole
  ticket (not an individual conversation)."): dry-run prints `{"status":
  "dry_run", **review}`; otherwise requires `--yes` and an exact
  `--expect-plan`, else `"review with --dry-run, then pass --yes and the
  full --expect-plan hash"` (exit 2).
- **fsv-completion** (done): new command and flags.
- Python tests: `tests/test_ticket_conversation.py::test_bcc_and_whole_ticket_forward_exact_wire_hash_and_readback`
  (reply, forward), `::test_bcc_missing_readback_is_unavailable_not_verified`,
  `::test_bcc_and_forward_cli_require_exact_review`, `::test_forward_uncaptured_variants_refused`.

### `ddd756a` feat(watchers): add reviewed ticket and change self-watch workflows — ported

**fsv-api ported:** `Api::me` / `Api::v2_get` (defaulted, `service.rs:68`,
`:72`; `Client` overrides), `reviewed_write.rs`, `watchers.rs`. The other
`Api` impls (fsv-cli `lazy.rs`, fsv-flows testing, fsv-views golden) still
use the default `me()` error until their owners forward it.

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
- **fsv-cli** (done; `cmd/reviewed.rs`, goldens in `tests/upstream_cmds.rs`,
  fixture `watchers.json`): `fsv {tickets,changes} watchers list ID` → `{"watcher_ids":
  […]}` (errors as Click BadParameter); `watchers set ID --watch|--unwatch
  [--dry-run] [--yes] [--expect-plan]` with the same `--yes`/hash rule as
  forward.
- **fsv-completion** (done): new group and commands.
- Python tests: `tests/test_watchers.py` (all; exact methods, ambiguous
  403/429/500 leaves `started`, stale identity, partial membership, no-op,
  readback/journal failures, CLI hash flow).

### `9622e3f` feat(tickets): add reviewed nonbillable self time CRUD — ported

**fsv-api ported:** `ticket_time.rs` (`TimeArgs`, `build_plan`, `send`).

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
- **fsv-cli** (done; `cmd/reviewed.rs`, goldens `time_*` in
  `tests/upstream_cmds.rs`, fixture `ticket_time.json`): `fsv tickets time list ID` (JSON inventory, indent 2) and
  `fsv tickets time write ID ACTION [--entry N] [--spent] [--executed-at]
  [--note] [--dry-run] [--yes] [--expect-plan]`.
- **fsv-completion** (done): new group and commands.
- Python tests: `tests/test_ticket_time.py` (all),
  `tests/test_readback_normalization.py::test_normalized_exact_id_transport_readback`,
  `::test_note_only_update_preserves_normalized_minutes` (`{"time_spent":
  "125:05", "note": "Globex"}`).

### `5bc5d4d` feat(tui): review ticket recipient and status conversation options — partial

Ported in fsv-tui: Ctrl+O option menu, `ConversationRequest` with
cc/notify/bcc/sender/Pending, plain ticket replies routed to the reviewed
path (`crates/fsv-tui/src/app/reviewed.rs:480-575`, `app/cont.rs:34-47`,
`modals.rs:210-259`).

Missing:

- **fsv-app**: done. `ticket_conversation_plan` and
  `send_ticket_conversation` in `tui.rs` call `conversations::build_plan_mail`
  with `MailArgs { bcc, sender }` (golden
  `port/golden/gen_fsv_app_tui_reviewed.py`: a BCC + sender reply sent, the
  accepted and refused option sets, full reviews including the warning).
- **fsv-api**: done (`a5a8381`, CC/BCC re-check from `1dc7ab6`).
- Python tests: `tests/test_tui_ticket_options.py` (both),
  `tests/test_tui_actions.py` reply test (plain reply calls the reviewed
  send with `"<div>thanks</div>"`, after a PUBLIC consent screen).

### `f711010` feat(planning): add reviewed native text saves and TUI editor — ported

**fsv-api ported:** `planning_text.rs` (schema injectable via `ChangeSchema`;
`live_change_schema` = `schema::load_forced`).

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
- **fsv-cli** (done; `cmd/reviewed.rs`, goldens in `tests/upstream_cmds.rs`,
  fixture `planning.json`): `fsv {changes,problems} planning-text ID --section S
  --description D [--dry-run] [--yes] [--expect-plan]`, at `92002d7`: an
  `APIError`/`SessionError` prints `str(exc)` with `exit_code_for` (unknown
  section: exit 2), the `--section` completers are per module, and a section
  with text is refused (see "Ledger: `dbe553c..92002d7`").
- **fsv-app**: done. `planning_definitions`, `planning_row`
  (`create::match_planning_row` on `inventory`), `planning_text_plan`,
  `send_planning_text`; `LiveBackend::with_change_schema` injects the change
  schema in tests.
- **fsv-completion** (done): new commands.
- Python tests: `tests/test_planning_text.py` (all four; keys
  `acme_plan` for changes "Rollout Plan", `problem_cause` for "Root Cause").

### `3057492` feat(tui): edit reviewed ticket properties and SR beneficiaries — partial

**fsv-api ported:** beneficiary answers and `requested_for_id`
(`properties.rs:189-300`). fsv-app can now call `build_plan_with`.

Ported in fsv-tui: `e` on a ticket opens the property picker → value/choice →
review → outcome (`reviewed.rs:307-360`, `app/forms.rs:23-24`).

Missing:

- **fsv-api** `properties.rs` (on top of `f761a01`): a beneficiary answer
  (`api_name == "requested_for" && field_type == "default_requested_for"`)
  is writable even when `!sent`; allowed keys gain `requested_for_id`; key
  is `{api_name}_id` for `requester` and `requested_for`.
- **fsv-app** `tui.rs`: done. `ticket_property_plan` is
  `build_plan_with(answers={name: value})` with a `ClientResolver`, and
  `send_ticket_property` is `send_with`; `ReviewPlan.raw` carries the
  `AnsweredPropertyPlan` (golden: subject, priority, custom text and
  dropdown, beneficiary, plus the status and planned-date refusals).
- Python tests: `tests/test_tui_ticket_properties.py::test_tui_property_field_picker_review_and_outcome`,
  `::test_requested_for_update_is_property_only_and_resolves_current_user`,
  `::test_readonly_property_picker_never_prepares`,
  `tests/test_tui_views.py` (`e` on a ticket opens the property picker).

### `f7bf04a` feat(problems): add reviewed known-error mark and TUI action — ported

**fsv-api ported:** `problem_known_error.rs`.

Ported in fsv-tui: `a` → "Mark as known error", review, outcome
(`reviewed.rs:362-396`, `app/writes.rs:197, 210`).

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
- **fsv-cli** (done; `cmd/reviewed.rs`, goldens `mark_known_error_*` in
  `tests/upstream_cmds.rs`): `fsv problems mark-known-error ID [--dry-run] [--yes]
  [--expect-plan]` ("Review marking a problem as a known error; no reversal
  wire is proven.").
- **fsv-app**: done. `known_error_plan`, `send_known_error`.
- **fsv-completion** (done): new command.
- Python tests: `tests/test_problem_known_error.py` (all four).

### `1dc7ab6` test(writes): exercise ticket property and mail transport guards — partial

**fsv-api ported:** the CC re-check (`conversations.rs:389`) and the readback /
ambiguity cases as `FakeApi` tests. Exact HTTP bytes and headers are not
exercised (no transport-level fake in fsv-api).

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

### `c0210a7` feat(tickets): add reviewed explicit-ID bulk priority — ported

**fsv-api ported:** `bulk_priority.rs` (`BulkPriorityPlan`, `build_plan`, `send`).

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
- **fsv-cli** (done; `cmd/reviewed.rs`, goldens `bulk_priority_*` in
  `tests/upstream_cmds.rs`, fixture `ticket_reviewed.json`): `fsv tickets bulk-priority ID... --priority P [--dry-run]
  [--yes] [--expect-plan]`; exit 1 when the result is unverified; `ValueError`,
  `ReviewedWriteError` and `PropertyError` → exit 2.
- **fsv-completion** (done): new command (`--priority` gets no completer in Python).
- Python tests: `tests/test_bulk_priority.py` (all; fixtures
  `("acme.example.com", 123, 87)`, `("globex.example.com", 742, 26)`).

### `774f4f1` fix(tickets): invalidate old beneficiary rule context — ported

**fsv-api ported:** `properties.rs:218`.

- **fsv-api** `properties.rs` (with `f761a01`/`3057492`): when any answer's
  field is `requested_for`, set `engine.context.requested_for = None`
  before building `FormState`, as `catalog.rs:1095` already does for SR
  create.
- Python tests: `tests/test_tui_ticket_properties.py::test_beneficiary_edit_never_reuses_previous_person_rule_context`.

### `4d778c3` fix(readback): compare durations and offset timestamps semantically — ported

**fsv-api ported:** `readback.rs`; properties and tasks use `same_instant`
(`properties.rs:540`, `tasks.rs:428`). `tasks::same_datetime` is gone.

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

### `b7d96a7` fix(cli): accept consistent JSON options on reviewed commands — ported

`ReviewOutput` is a one-value enum (`json`). Each reviewed command added in
this range takes `--output/-o` (`"output format (JSON only)"`, default
`json`) and `--json` (`"alias for --output json"`). Their output stays the
same. Any other `--output` value fails as a Click choice error (exit 2)
**before** a client is opened. The commands are `{tickets,changes} watchers
list|set`, `tickets time list|write`, `{changes,problems} planning-text`,
`problems mark-known-error`, `tickets bulk-priority` and `tickets forward`.

- **fsv-cli** (done): `ReviewOutput` value enum (`json` only) in `types.rs`
  and both options on each of those commands. They change nothing except
  accepting the flags. Rust rejects other values with clap's usage error
  (exit 2, no request), not Click's rich panel; only the exit code is pinned
  (`json_only_output_rejects_other_formats`).
- **fsv-completion** (done): Python's completion dump records `--output/-o`
  as freeform with no completer on those commands, and so does Rust.
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
   - ~~`7903e2a`: `risks` and `requires_confirmation` in `Plan::to_json` and
     the goldens.~~ Done.
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
     regenerate `generated.rs` with `tests/golden/gen_tree.py`
     (`tests/tree.rs` checks it). The tree
     diff against Python at `b7d96a7` is: new `changes planning-text`, `changes watchers
     list|set`, `problems mark-known-error`, `problems planning-text`,
     `tickets bulk-priority`, `tickets forward`, `tickets time list|write`,
     `tickets watchers list|set`; `--refresh` on `{changes,problems,tickets}
     lookup`; `--reviewed` on `{tickets,problems} update`; `--bcc` and
     `--sender` on `tickets reply`. Every new command also has
     `--output/-o {json}` and `--json` (`b7d96a7`).
   - Commands: all done. ~~`lookup --refresh` (`d18bab9`)~~, ~~`update --reviewed`
     (`f761a01`)~~, ~~`reply --bcc/--sender` and `forward` (`a5a8381`)~~,
     ~~`watchers` (`ddd756a`)~~, ~~`time` (`9622e3f`)~~, ~~`planning-text` and
     `mark-known-error` (`f711010`, `f7bf04a`)~~, ~~`bulk-priority` (`c0210a7`)~~,
     ~~the `changes update` help text (`7903e2a`)~~ (not observable), ~~the JSON-only output
     options on every new reviewed command (`b7d96a7`)~~.
   - Refresh the write goldens for `273ffde`.
6. **fsv-completion** (after the fsv-cli tree) — done
   - Re-dump `completion_tree.json` (`crates/fsv-cli/tests/golden/dump_completion.py`)
     and the bash/zsh/fish goldens for the new commands and flags. None of
     the new options has a Python completer. Re-dumped from Python
     `92002d7` with every path (`crates/fsv-cli/tests/golden/not_ported.py`
     is empty).
7. **fsv-tui** (tests only)
   - `273ffde`: a ticket non-terminal status plan now has a risk.

No work for **fsv-forms**, **fsv-views** or **fsv-formui** in this range.

## Ledger: `b7d96a7..6797301`

Range `b7d96a7..6797301`, 3 commits. Rust graded at `rust-port` `6c5124e`
plus the working tree at triage time. That tree already has untracked
`fsv-api` `reviewed_write.rs`, `watchers.rs` (`positive_id`, `self_id`),
`readback.rs`, `ticket_time.rs`, `problem_known_error.rs`,
`planning_text.rs`, and `Api::me()` (`service.rs:68`). Every new
command below builds on `reviewed_write::{WritePlan, execute}`.

Counts: **3 ported, 0 partial, 0 missing.**

| commit | date | subject | status | crates |
|---|---|---|---|---|
| `07c85d0` | 11-12 04:50 | feat(tickets): add source-bound canned replies and reviewed scenarios | ported | fsv-api (`canned.rs`, `scenarios.rs`, `conversations.rs` canned fields), fsv-cli (`cmd/native.rs`), fsv-completion |
| `2a03036` | 11-12 04:57 | feat(views): add guarded private ticket-view lifecycle | ported | fsv-api (`private_views.rs`), fsv-cli (`cmd/native.rs`), fsv-completion |
| `6797301` | 11-12 05:04 | feat(problems): add form-backed create and reviewed note and task workflows | ported | fsv-api (`formmeta.rs` problem module, `problem_workflows.rs`), fsv-forms, fsv-cli (`form` on problems, `problems create`, `write-child`), fsv-completion |

Python uncommitted after `6797301` (**untriaged, uncommitted**):
`src/fsv/list_preferences.py`, `src/fsv/list_preferences_cli.py`,
`tests/test_list_preferences.py`, plus edits to `src/fsv/cli.py` (registers
on every resource) and `docs/reviewed-parity.md`. They add
`{tickets,changes,problems} columns list|set` and `{…} export`. These are
native list-column preferences and a CSV export request, both on
`reviewed_write`. Rust has nothing for them. Grade them once they are
committed.

New Python modules with no Rust counterpart: `canned.py`, `canned_cli.py`,
`scenarios.py`, `private_views.py`, `private_view_cli.py`,
`problem_workflows.py`, `problem_cli.py` (and the uncommitted
`list_preferences.py`, `list_preferences_cli.py`).

### `07c85d0` feat(tickets): add source-bound canned replies and reviewed scenarios — ported

**Status:** fsv-api is ported: `canned.rs`, `scenarios.rs` and the canned
fields on `ConversationPlan`, checked against
`port/golden/gen_fsv_api_workflows.py`. fsv-cli (`cmd/native.rs`, goldens on
fixture `canned` in `tests/upstream_cmds.rs`) and fsv-completion are ported
at Python `92002d7`. The notes below are as triaged.

Rust has no `canned_response*` or `scenario_automations` route, and
`ConversationPlan` (`crates/fsv-api/src/conversations.rs:142`) has no canned
fields. Canned replies always pass `sender`, so they need the sender half of
`a5a8381` first.

- **fsv-api** new `canned.rs`:
  - Reads: `folders` GET `canned_response_folders`; `responses(folder)` GET
    `canned_response_folders/{positive_id}/canned_responses`; `show(id)` GET
    `canned_responses/{positive_id}`. Raw JSON, no pagination claim.
  - `content(raw, pointer)` (RFC 6901, objects only): no leading `/` →
    `"content pointer must start with /; inspect canned show first"`; a `~`
    not followed by `0`/`1` → `"invalid JSON pointer escape"`; unescape `~1`
    → `/` then `~0` → `~`; non-object or missing key → `"content pointer does
    not identify a source field"`; non-string or blank → `"selected canned
    content must be nonempty text/HTML"`; regex `\{\{|\{%|\$\{|\[\[` →
    `"unresolved canned placeholders require the web editor"`.
  - `reply_plan(api, store, target, response_id, pointer, sender)`:
    `conversations::build_plan(content, format=Html, sender)` then set
    `canned_id`, `canned_pointer`, `canned_hash = plan_hash(raw)`.
  - `freshen(plan, api)`: re-`show`; hash differs or `content` ≠
    `payload.body` → `"canned source changed; review again"`.
  - `scenarios` GET `scenario_automations`; `scenario(id)`: rows are
    `raw["scenario_automations"]` or the raw list, else `"scenario inventory
    is not a list; inspect scenarios list"`; exactly one row with
    `str(id)` equal, else `"scenario identity is missing or ambiguous in this
    inventory"`. No detail endpoint.
- **fsv-api** `conversations.rs`: `ConversationPlan` gains
  `canned_id: Option<i64>`, `canned_pointer`, `canned_hash`. `hash()` adds
  `"canned": {"id","pointer","hash"}` **only when `canned_id` is set**, so
  legacy hashes stay the same. In `send`, after the journal started/done
  check and before the status-context check, call `canned::freshen`. The
  wire body has no canned key.
- **fsv-api** new `scenarios.rs` on `reviewed_write`: `build_plan(target,
  scenario_id)`: `positive_id(parse_id(target, TICKETS))`, payload
  `{"ids": [target], "scenario_id": id}` (sorted keys), PUT
  `tickets/execute_scenario`, action `ticket_scenario`, context
  `plan_hash({"scenario": canned::scenario(..), "ticket":
  properties::record(..)})` (`properties.rs:54`). `send` → `execute` with
  rebuild, and verify returns `{"ticket_id", "identity_verified": str(row.id)
  == str(target), "verified": false, "warning": "Exact ticket was read back;
  scenario side effects (including mail/workflows) are unverified. Do not
  resend."}`.
- **fsv-cli**: `fsv tickets canned folders|list --folder N|show ID|reply
  TICKET --response N --content-pointer P --sender S [--dry-run] [--yes]
  [--expect-plan]` and `fsv tickets scenarios list|show ID|apply TICKET
  --scenario N [--dry-run] [--yes] [--expect-plan]`. Every command has
  `--output/-o {json}` and `--json` (`b7d96a7`). `canned reply --dry-run` =
  `{"status":"dry_run", **review, "source": {"id","hash"}, "insertion":
  "Literal selected HTML only; no attachments, signatures or placeholder
  expansion."}`. `scenarios apply --dry-run` = `{"status":"dry_run",
  **review, "definition": scenario, "warning": "Native automation may send
  mail or change records. No complete side-effect verification contract is
  captured."}` (overrides the review warning). The gate is `"review with
  --dry-run, then pass --yes and the full --expect-plan hash"` (exit 2). Canned
  reply sends with `public_confirmed=true`. `scenarios show` errors are a
  BadParameter. Group help: "Read canned responses and insert selected
  literal HTML into a reviewed reply." / "Read/review native scenarios.
  Execution side effects remain unverified."
- **fsv-completion**: two new groups, seven commands.
- Python tests: `tests/test_canned.py` (all). Pinned reply body:
  `{"body": "<div>Acme response</div>", "full_text": same,
  "conversation_from_summary": false, "cc_emails": [], "translate_reply":
  false, "freddy_message_id": null, "from_email": "support@example.com"}`.
  The scenario write is `{"scenario_id": 703, "ids": [123]}`.
  `::test_legacy_conversation_hash_is_unchanged` pins the old hash.

### `2a03036` feat(views): add guarded private ticket-view lifecycle — ported

**Status:** fsv-api `private_views.rs` is ported and checked against
`gen_fsv_api_workflows.py`. fsv-cli (`cmd/native.rs`, fixture
`private_views`) and fsv-completion are ported at Python `92002d7`.

Nothing in Rust (`ticket_filters` appears only as `filters_path` in
`crates/fsv-core/src/resource.rs:63`).

- **fsv-api** new `private_views.rs` on `reviewed_write`:
  - `detail(view_id)`: GET `ticket_filters/{id}`, unwrap `ticket_filter`;
    id mismatch → `"exact saved-view identity is unavailable"`.
  - `private_visibility(row, user)`: `user_id` ≠ self, `default` not
    exactly `false`, or `system` (default false) not exactly `false` →
    `"only self-owned, explicitly non-default private views are supported"`.
    The label comes from `visibility.label` (id from `visibility.id`) when
    `visibility` is an object, else from `visibility_name`. A casefolded,
    trimmed label outside `{me, only me, private}` → `"named private
    visibility metadata is unavailable; numeric visibility alone is not
    proof"`. Returns `positive_id(value)`.
  - `build_plan(source_id, action, name)`: action not create/edit/delete →
    `"unsupported saved-view action"`. Delete with a name → `"delete does
    not accept a name"`; DELETE `ticket_filters/{id}`, payload `null`.
    For create/edit, a blank name → `"create/edit requires a nonempty --name"`;
    non-null `advanced_query_hash` or non-list `query_hash` → `"only the
    captured ordinary query-hash subset is supported"`; `order_by` ≠
    `created_at` or `order_type` ≠ `desc` → `"only captured created-at
    descending views are supported"`. Payload `{name, workspace_id,
    visibility, query_hash, advanced_query_hash: null, order_by,
    order_type}` (sorted, `ensure_ascii=False`). Create POSTs
    `ticket_filters` and edit PUTs `ticket_filters/{id}`. The action is
    `private_view_{action}`, target = source id, context
    `plan_hash({"self","source": row,"source_id"})`. The plan also stores
    `source_id` (not part of the hash).
  - `send`: create verify takes `ticket_filter.id` from the response
    (missing → error, which `execute` turns into "Independent readback
    failed"); same as the source → `"create returned the source identity"`.
    Delete verify: `detail` must fail with HTTP **404** (an HTML 404 body
    too; other errors propagate). If it still reads, `verified: false`.
    Otherwise `collect_pages("ticket_filters", "ticket_filters",
    {page:1, per_page:30})` (`service.rs:375`) must be complete and lack the
    id. Create/edit verify re-reads `detail` and compares each payload key
    (visibility via `private_visibility` again) as
    matched/mismatched/unavailable.
- **fsv-cli**: `fsv tickets private-views show ID` and `private-views write
  SOURCE_VIEW create|edit|delete [--name] [--dry-run] [--yes]
  [--expect-plan]`, with the JSON-only output options and the usual hash
  gate. Group help: "Private ticket views: clone, rename or delete an exact
  self-owned non-default view."
- **fsv-completion**: new group, two commands.
- Python tests: `tests/test_private_views.py` (all). Writes are
  `["POST","PUT","DELETE"]`, the clone is id 702, and the payload has no
  `user_id`/`default`.

### `6797301` feat(problems): add form-backed create and reviewed note and task workflows — ported

**Status:** fsv-api `problem_workflows.rs` and the formmeta `problem` module
are ported and checked against `gen_fsv_api_workflows.py`. fsv-flows
`meta::MODULES` lists `problem` too. fsv-cli is ported at Python `92002d7`:
`cmd/form.rs` uses `formmeta::module_name` (`form dump|fields|explain
problems`), `problems create` and `write-child` are in `cmd/native.rs`
(fixture `problem_flows`), and `read_cmds.rs` `form_explain_unsupported` /
`exit_codes.rs` pin the three-module message. fsv-completion is re-dumped.

Rust formmeta knows only change/ticket (`crates/fsv-api/src/formmeta.rs:28`,
message `:87`). `form explain ID` hardcodes changes
(`crates/fsv-cli/src/cmd/form.rs:199-201`). `fsv-forms` `check` and
`module_body` are private (`crates/fsv-forms/src/payload.rs:436`, `:453`).
Rust has no `problems create`.

- **fsv-api** `formmeta.rs`: `MODULES` gains `("problem",
  "problem_form_fields", "problem_fields")`. The message becomes
  `"unsupported form {text!r}; supported: changes, tickets, problems"`.
  Update the test at `:1357`, which currently expects problems to fail.
- **fsv-forms** `payload.rs`: expose `check(state, allow_incomplete, skip)`
  and `module_body(state, keys, order, status)` (at least `pub(crate)` plus a
  pub wrapper) for problem create.
- **fsv-api** new `problem_workflows.rs` on `reviewed_write`:
  - `record` (GET `problems/{id}`, unwrap `problem`, `"exact problem
    identity is unavailable"`). `inventory(target, notes|tasks)` uses
    `collect_pages(.., {page:1,per_page:30}, what="problem {kind}")`.
    Incomplete → `"complete problem {kind} inventory is unavailable"`,
    duplicates → `"duplicate problem child identities"`, sorted by id.
  - `create_plan(answers, requester)`: `load_meta("problem","new",
    fresh=true)`. `RULES_UNAVAILABLE_KEY` → `"problem rules are unavailable;
    no static-form write"`. Requester without email → `"resolved requester
    needs an email"`. Not exactly one requester field → `"unique problem
    requester field is unavailable"`. A requester among the answers →
    `"requester answers are not allowed; use --requester"`. Then
    `state.set(requester)`, `apply_answers` (errors joined `"; "`),
    `unavailable_dependencies("new")` → `"problem rules reference
    unavailable fields or context"`, rule messages → `"problem form has rule
    messages; use the web form"`, and `check(skip=[requester])`.
    `module_body(order=(subject, description, status, priority, impact))`
    drops `requester`. Extra top keys → `"problem form needs uncaptured
    top-level create fields"`; a missing one → `"subject, description,
    status, priority and impact must be selected from the form"`. Payload
    `{**top, custom_fields, email}`, POST `problems`, action
    `problem_create`, target 0, context `plan_hash({"fields":
    meta.form_fields, "rules", "requester": str(person)})`, and
    `inputs_json = {"answers","requester"}` sorted.
  - `child_plan(target, note|task, text)`: `"unsupported problem child
    kind"` / `"nonempty text is required"`. A note is `{"body": text}`. A task
    is `{"title", "owner_id": self_id, "status": _choice("status","Open"),
    "notify_before": _choice("notify_before","Never"), "planned_effort":
    "1m"}`, with metadata GET `problem_task_fields` (list under
    `problem_task_fields|task_fields|fields`, else `"problem task metadata
    is unavailable"`). `_choice` takes the unique field by `name` (`"unique
    runtime {name} metadata is unavailable"`) and a non-deleted choice whose
    `value|label|name` casefold-matches with an int id ≥ 0 (`"active runtime
    {name} choice is unavailable or ambiguous"`). Any other required field
    → `"problem task has an uncaptured required field"`. POST
    `problems/{id}/notes|tasks`, context `plan_hash({"parent","before",
    [fields, self]})`, `inputs_json = {"text","kind"}` (**unsorted**, Python
    order).
  - `send`: rebuild from `inputs_json`. Verify reads the response key
    `problem|note|task` for `id` (`positive_id`). Create reads `record`;
    children re-read `inventory`. Each payload key needs strict type+value
    equality. Result `{"{key}_id", "fields", "verified"}`.
- ~~**fsv-cli** `form.rs`: `explain ID` uses the module's resource
  (`REGISTRY[mod+"s"]`, its `api_path` and `item_key`), not CHANGES; the
  `isdigit` shortcut is gone.~~ Done (`form_explain_ticket_record`,
  `form_explain_change_display_id`, `form_explain_problem_record`).
- **fsv-cli**: `fsv problems create --requester R [--from YAML] [--set
  LABEL=VALUE…] [--dry-run] [--yes] [--expect-plan]` ("Create a problem
  through fresh form fields and business rules; no files."). It uses
  `fsv_flows::drafts::resolve_draft` (`drafts.rs:536`); attachments, planning
  or a draft requester → `"use answers plus --requester; files, planning and
  special draft keys are unsupported"`. Also `fsv problems write-child
  PROBLEM note|task TEXT …` ("Review a body-only note or basic self-owned
  one-minute task; note visibility is not proven."). Both take the
  JSON-only output options and the usual hash gate, and FormError → exit 2.
- **fsv-completion**: two new commands. `form dump|explain` module choices
  now include problems if completed.
- Python tests: `tests/test_problem_workflows.py` (all). The create payload
  is `{"subject": "Acme problem", "description": "<div>Acme details</div>",
  "status": base, "priority": base+1, "impact": base+2, "custom_fields":
  {}, "email": "person@example.com"}` with `problem_id == 456`. The task is
  `{"title": "Acme", "owner_id": 901, "status": base+10, "notify_before":
  base+11, "planned_effort": "1m"}`. `form dump problems` →
  `"module": "problem"`.

## Ordering for `b7d96a7..6797301`

1. **fsv-forms**: make `check` / `module_body` reachable (`6797301`).
2. **fsv-api** (after the `reviewed_write` / `watchers` base above lands):
   1. `formmeta` problem module, then `problem_workflows.rs` (`6797301`).
   2. `private_views.rs` (`2a03036`). It needs the 404 status from the
      fsv-core error (`error.rs:107`).
   3. `conversations.rs` sender (`a5a8381`), then the canned identity and
      `freshen`, `canned.rs`, `scenarios.rs` (`07c85d0`).
3. **fsv-cli**: `form explain` per module plus the golden; `problems create`
   and `write-child`; `tickets private-views`; `tickets canned` and
   `tickets scenarios`. All of them need the `ReviewOutput` options. Re-dump
   `cli_tree.json` from Python at `6797301`.
4. **fsv-completion**: re-dump after the tree.

No fsv-tui, fsv-app, fsv-flows, fsv-views or fsv-formui work in this range.

## Ledger: `6797301..dbe553c`

Range `6797301..dbe553c`, 3 commits. Rust graded at `rust-port` `1abd9b7`
plus the working tree at triage time. That tree has uncommitted `fsv-api`
work in progress from another agent: `reviewed_write.rs`, `watchers.rs`
(`positive_id`, `self_id`) and `Api::me()`. The new writers below build on
`reviewed_write::{WritePlan, execute}` and the picker on
`watchers::positive_id`. Python had no uncommitted source changes at
triage time.

Counts: **2 ported, 1 partial, 0 missing.**

| commit | date | subject | status | crates |
|---|---|---|---|---|
| `6b7073a` | 11-12 05:11 | feat(lists): add reviewed native columns and scoped export requests | ported | fsv-api (`list_preferences.rs`), fsv-cli (`cmd/native.rs`), fsv-completion |
| `53c678e` | 11-12 05:17 | feat(assets): add read-only CLI and TUI candidate pickers | partial | fsv-api `asset_picker.rs`, fsv-tui and fsv-app `LiveBackend::asset_candidates`, fsv-cli `changes asset-picker`, fsv-completion done; open: fsv-api `candidates` validates ids after the duplicate check |
| `dbe553c` | 11-12 05:23 | test(legacy): cover mutation callers through offline HTTP transports | ported | fsv-api `tests/legacy_write_guards.rs` |

New Python modules with no Rust counterpart: `list_preferences.py`,
`list_preferences_cli.py`, `asset_picker.py`, `asset_picker_cli.py`.

### `6b7073a` feat(lists): add reviewed native columns and scoped export requests — ported

**Status:** fsv-api `list_preferences.rs` is ported and checked against
`gen_fsv_api_workflows.py` for tickets, changes and problems. fsv-cli
(`cmd/native.rs`, fixture `list_prefs` on all three modules) and
fsv-completion are ported at Python `92002d7`.

Rust reads `tickets/headers` only for display (`crates/fsv-views/src/source.rs:61`).
It has no `{module}/headers` POST and no `{module}/export`. The pieces it can
reuse exist: `fsv_api::views::load_views` (`crates/fsv-api/src/views.rs:76`),
`fsv_api::site::list::find_view` (`site/list.rs:305`), `SavedView::to_json`,
and `Resource::{filters_path, item_key}` (`crates/fsv-core/src/resource.rs`;
`change-filters`, `ticket_filters`, `problem-filters`).

- **fsv-api** new `list_preferences.rs` on `reviewed_write`. `ListPlan` =
  `WritePlan` + `inputs_json` (not hashed).
  - `headers(module)`: a module not in the registry →
    `"unsupported list module"`. GET `{module}/headers`, list `headers`. An
    empty list, a non-object row or a row without `name` → `"runtime
    list-header metadata is unavailable"`; duplicate names → `"ambiguous
    runtime list-header keys"`.
  - `columns_plan(module, columns, frozen)`: empty, duplicate, unknown, or
    containing `display_id` → `"choose unique exact runtime column names;
    display_id is anchored automatically"`. No `ticket_subject` →
    `"captured preference contract requires ticket_subject"`. Frozen
    duplicates or a frozen column that isn't selected → `"frozen columns
    must be unique selected columns"`. Any frozen column unless module is
    `changes` and frozen == `["requester"]` → `"only change requester
    freezing plus the automatic display_id anchor is captured"`. Payload
    `{"columns": {name: 1-based position…, "display_id": <ticket_subject's
    position>}, "frozen_columns": [*frozen, "display_id"]}` (sorted). It
    POSTs `{module}/headers` with action `list_columns`, target 0, and
    context `plan_hash(rows)`. `inputs_json = {"module","columns","frozen"}`
    is **unsorted**.
  - `_EXPORT_KEYS` = `subject→ticket_subject, priority_name→priority,
    status_name→status, group_name→group, responder_name→assigned_to,
    owner→assigned_to, display_id→display_id, requester_name→requester,
    requester_info→requester, department_name→department_name`. Tickets
    can't use `owner` or `department_name`; changes and problems can't use
    `responder_name` or `requester_name`.
  - `export_source(module, view)`: `load_views`, where any missing view →
    `"saved-view inventory is unavailable"`. Then `find_view`, then GET
    `{filters_path}/{id}`, unwrapping `{item_key}_filter`. An id mismatch →
    `"exact saved-view settings are unavailable"`. A non-list `query_hash`
    or non-null `advanced_query_hash` → `"only ordinary saved-view queries
    have a captured export contract"`.
  - `export_plan(module, view, fields)`: empty, duplicate or disallowed
    fields → `"select unique captured export field keys"`. Each key needs a
    non-blank header label for its mapped name, else `"runtime export-field
    label is unavailable for {key}"`. Payload `{"format": "csv",
    "date_filter": 30, "export_fields": {key: label}, "custom_fields": {},
    "data_hash": query_hash}`, then:
    - tickets add `ticket_state_filter: "created_at"`,
      `advanced_data_hash: null`, and `selected_filter_view: {id, name,
      is_additional_filters_applied: false}`;
    - changes and problems add `state_filter: "created_at"`;
    - changes also add `advanced_data_hash: null`.

    It POSTs `{module}/export` with action `list_export`, target 0, and
    context `plan_hash({"headers","view": row,"selection":
    selected.to_json()})`. `inputs_json = {"module","view": id,"fields"}`.
  - `send`: rebuild from inputs.
    - Export verify re-reads `export_source` and returns `{"verified":
      false, "source_readback": str(row.id) == view, "job": "unavailable",
      "download": "unavailable", "warning": "Export requested once;
      delivery/job/download effects are unverified. Do not resend."}`.
    - Columns verify re-reads `headers`. Each payload column is
      `unavailable` unless the row has `selected`, `position` and `frozen`,
      and is `matched` only for `selected is true`, an equal position, and
      `frozen is (key in frozen_columns)`. `unverified_omissions` lists the
      rows outside the payload whose `selected` is not `false`. `verified`
      needs no omissions and every field matched.
- **fsv-cli** (every resource: tickets, changes, problems):
  - `fsv {res} columns list` prints the headers.
  - `fsv {res} columns set --column C… [--freeze C…] [--dry-run] [--yes]
    [--expect-plan]`.
  - `fsv {res} export --view V --field K… [--dry-run] [--yes]
    [--expect-plan]` ("Review a native CSV/30-day export request; may email
    data, job/download contract is unknown.").

  All take the JSON-only `--output/-o`/`--json` (`b7d96a7`) and the usual
  `"review with --dry-run, then pass --yes and the full --expect-plan
  hash"` gate (exit 2). Group help: "Inspect/set native list columns
  (shared account preferences may be affected)." Check for name clashes
  with existing `ls --output csv` and column options in `generated.rs`.
- **fsv-completion**: a `columns` group and the `export` command on all
  three resources.
- Python tests: `tests/test_list_preferences.py` (all, parametrized over
  modules).
  - The columns payload is `{"columns": {"ticket_subject": 1, "requester":
    2, "status": 3, "display_id": 1}, "frozen_columns": ["display_id"]}`.
  - Export labels are `"Acme subject"`, `"Acme state"`, `"Acme ID"`, with
    view `{"id": "701", "name": "Acme queue"}`.
  - On 403/429/500/disconnect there is exactly one write, and a resend says
    "was sent".
  - Header drift blocks the write. When the preference fields are missing,
    `ticket_subject` is `"unavailable"`.

### `53c678e` feat(assets): add read-only CLI and TUI candidate pickers — partial (fsv-api ordering open)

**Status:** fsv-cli `cmd::reviewed::asset_picker` (fixture `asset_picker`)
and fsv-completion are ported at Python `92002d7`. Open in fsv-api:
`asset_picker.rs` `candidates` checks duplicates before every display id is
validated (PARITY fsv-cli divergence; ignored `asset_picker_duplicate_then_bad`).

`fsv_api::create::search_assets_for_change` exists
(`crates/fsv-api/src/create.rs:1594`, GET `assets-to-associate`) and is used
by `links` (`crates/fsv-cli/src/cmd/links.rs:257`). Nothing wraps it as a
read-only picker. In the Rust TUI, `i` is unbound
(then `app.rs:2474` dispatched `v`; now `crates/fsv-tui/src/app/key_handler.rs:159` binds `i`), and the Backend has no
asset search.

- **fsv-api** new `asset_picker.rs`:
  - `candidates(target, query, page)`: `positive_id(parse_id(target,
    CHANGES))`. A page that isn't a positive int → `"page must be
    positive"`. Calls `search_assets_for_change(.., per_page=30)` and takes
    `assets`. A non-list, or a non-object row → `"asset candidate response
    is unavailable"`. Duplicate `positive_id(display_id)` → `"duplicate
    asset display identities; selection is ambiguous"`. Returns
    `{"change_id", "page", "scope": "single search page; not a complete
    inventory", "assets", "mutation": "none", "warning": "Selection only.
    Asset/CI add-remove variants remain unsupported by this picker."}`.
  - `select(result, display_id)`: needs exactly one row by display id, else
    `"exact asset display ID is not unique on this search page"`.
- **fsv-cli**: `fsv changes asset-picker CHANGE [--search S] [--page N]
  [--select DISPLAY_ID]` (JSON-only output options; help "Search/select a
  change asset candidate without associating it (read-only)."). With
  `--select` it adds `"selected"`. Errors → exit 2. GET only.
- **fsv-tui**:
  - Help line `i  browse change asset candidates (read-only, no
    association)`, after the `v` line (`crates/fsv-tui/src/modals.rs:36`).
  - Key `i`: off a change → warning `"Select a change to browse asset
    candidates"`. Otherwise open a value prompt `"Read-only asset search —
    selection does not associate"`, where cancel sends no request.
  - Then run a background search: page 1 only, using `display_id` or `id`.
    With no rows, show the outcome `"Asset candidates — read-only"` / `"No
    candidates on search page 1. No association was attempted."`. Otherwise
    show a choice picker `"Asset candidates — page 1 only; read-only"` with
    labels `"[{display_id}] {name or -} — {ci_type_name or -}"`.
  - Picking shows the outcome `"Selected asset — no association"` with JSON
    `{"selected", "scope", "mutation": "none"}` (indent 2). Cancelling shows
    nothing. An error shows the failure `"Asset search unavailable: {exc}"`.
  - Add a Backend hook `asset_candidates(change, query)` that defaults to
    unavailable.
  - The `i` key works in read-only mode: it is not under the `a` action
    menu.
  - **Done** in `crates/fsv-tui/src/app/assets.rs` (`i` in `app.rs`, help
    line in `modals.rs`). The hook is `Backend::asset_candidates(&self, req:
    &AssetCandidatesRequest) -> BResult<AssetCandidates>` (`backend.rs`). It
    defaults to `"asset candidates is not available in this build"`.
    `AssetCandidates::select` ports `asset_picker.select`. Tests are in
    `tests/keys.rs` (`i_*`, `asset_select_*`). The 36-row help clip test in
    `tests/actions.rs` was re-pinned against a Python Textual screenshot:
    the new line wraps, and the clip now ends at `Esc  cancel filter input`.
- **Done** in fsv-api: `crates/fsv-api/src/asset_picker.rs`
  (`candidates`, `select`, `Candidates::to_json`), tests checked against
  Python on the same rows.
- **fsv-app** `tui.rs`: done. `LiveBackend::asset_candidates` over
  `asset_picker::candidates`.
- **fsv-completion**: new `changes asset-picker`.
- Python tests: `tests/test_asset_picker.py`.
  - `::test_cli_exact_display_id_and_search_params`: the GET params are
    exactly `{"entity": "change", "entity_id": "123", "include_meta":
    "true", "page": "2", "per_page": "30", "search_term": "Globex"}`, with
    `selected.id == 9000007001`.
  - `::test_ambiguous_identity_blocks_selection`.
  - `::test_pilot_readonly_search_selection_and_cancel`: modes `select`,
    `cancel-search`, `cancel-picker` and `empty`. The picker copy must not
    say "saved view", and every request is a GET.

### `dbe553c` test(legacy): cover mutation callers through offline HTTP transports — ported

**Status:** `crates/fsv-api/tests/legacy_write_guards.rs` ports all three
tests over the seven cases with a real `fsv_core::Client`. No fsv-core gap:
`Client::request` journals every write (the audit row is `ok: false` on
failure), and `should_retry` retries only GET/HEAD/OPTIONS, so a 429 never
resends a legacy PUT or POST. A failing CSRF page fetch refuses before any
mutation.

No source change. The legacy helpers are already ported:

- `fsv_api::service::add_note` (`service.rs:1200`) and `update_item`
  (`:1192`);
- `fsv_api::create::associate_assets` (`create.rs:1611`) and
  `dissociate_assets` (`:1620`, reads `changes/{id}/assets` first);
- `update_task` (`:1747`) and `delete_task` (`:1737`), both over
  `task_put` / `send_once`;
- `clone_tasks` (`:396`).

The existing Rust tests use `FakeApi`, not a real `Client`, so the transport
contract is not pinned.

- **fsv-api** new integration test, e.g.
  `crates/fsv-api/tests/legacy_write_guards.rs`. Drive a real
  `fsv_core::Client` over `fsv_core::client::mock::MockTransport`, built as
  in `crates/fsv-api/src/downloads.rs:163-200`, against a temp `Ctx` and
  journal. Use these cases (method, path, body):
  - note: POST `changes/123/notes` `{"body": "Acme evidence", "private":
    true}`
  - status: PUT `changes/123` `{"status": 47}`
  - asset-add: PUT `changes/123/assets/associate` `{"item_ids": [701]}`
  - asset-remove: PUT `changes/123/assets/detach` `{"cmdb_request_id":
    801}`, after a GET of `changes/123/assets` returning
    `{"associated_assets": [{"id": 801, "config_item": {"display_id":
    701}}]}`
  - task-update: PUT `changes/123/tasks/701` `{"title": "Acme revised"}`.
    Python reads nothing for this one, and the GET handler fails on any
    other read.
  - task-delete: PUT `changes/123/tasks/update` `{"id": "701", "action":
    "delete"}`
  - clone-task: GET `changes/123/tasks` (two tasks), then POST
    `changes/456/tasks` twice. The first body has `change_task.title ==
    "Acme task"`.

  Port the three tests over those cases:
  - `test_each_legacy_caller_never_automatically_replays`: 403, 429, 500
    and disconnect each give an error and exactly **one** mutation. That
    mutation has the exact method, path and body and an `X-CSRF-Token`
    header. The journal gets a write audit row with `ok: false` for that
    method and no `plan:*` rows. Disable retry sleeps with `.sleeper(|_| {})`
    so 429 doesn't wait.
  - `test_csrf_preparation_failure_prevents_each_legacy_mutation`: a failing
    CSRF fetch gives an error and zero mutations.
  - `test_legacy_success_contracts_are_not_confused_with_independent_readback`:
    on success there is one mutation (two for clone) with the exact
    method, path and body, and no `plan:*` journal rows.

  Check whether `task_put`'s `send_once` path writes the transport audit
  row as Python does. If it doesn't, that is a real gap and lives in
  **fsv-core** `client/mod.rs`. The same applies to whether the 429 retry
  policy in `Client::int_request` (`client/mod.rs:675`) could resend a
  legacy PUT or POST.
- Overlaps `crates/fsv-core/src/client/tests.rs` (`:388`, `:416`, `:972`,
  `:1028`, `:1091`), which pin CSRF and no-replay at the client level, not
  per legacy caller.
- Python tests: `tests/test_legacy_write_guards.py` (all three, ×7 cases).

## Ordering for `6797301..dbe553c`

1. **fsv-api tests** (independent, can start now): `legacy_write_guards.rs`
   (`dbe553c`). Fix any fsv-core audit or replay gap it exposes.
2. **fsv-api** (after the `reviewed_write` / `watchers` work in progress
   lands): `asset_picker.rs` (`53c678e`, read-only, small), then
   `list_preferences.rs` (`6b7073a`).
3. **fsv-tui**: the `i` key, help line, prompt, picker and outcome screens,
   plus the `asset_candidates` Backend hook (`53c678e`). Then **fsv-app**
   `LiveBackend::asset_candidates`.
4. **fsv-cli**: `changes asset-picker`; `{tickets,changes,problems} columns
   list|set` and `export`, all with the JSON-only output options. Re-dump
   `cli_tree.json` from Python at `dbe553c`.
5. **fsv-completion**: re-dump after the tree.

No fsv-core (unless step 1 finds a gap), fsv-flows, fsv-forms, fsv-views or
fsv-formui work in this range.

## Base pin

`port/UPSTREAM_BASE` holds the last Python commit whose range is fully
triaged (`92d76ff`). Advance it only when every row up to the new SHA is
`ported`, `partial` with a named follow-up, or `wontfix` with a PARITY
entry. Open follow-ups: fsv-api `asset_picker.rs` validation order
(`53c678e`); fsv-cli `planning-text --clear` (`e638d85`) and the clone CLI
cases (`3432bea`). Next untriaged: `2956fce` (fsv-completion owner).
(fsv-core's status-0 `ApiError` display now matches Python.)

## Ledger: `dbe553c..92002d7`

Triaged against Python `92002d7` (5 commits). The fsv-api, fsv-views,
fsv-flows and fsv-app parts are ported in the working tree; expected values
come from `port/golden/gen_fsv_api_workflows.py` (sections
`planning_text`, `problem_*`) and `gen_fsv_app_tui_reviewed.py`.

Counts: **5 ported, 0 partial, 0 missing.**

| commit | date | subject | status | crates |
|---|---|---|---|---|
| `10c3281` | 11-12 05:35 | fix(planning): verify normalized native section content | ported | fsv-api `planning_text` (`section_content`, `readback::same_text`) |
| `ed6e644` | 11-12 05:40 | fix(problems): display native analysis sections in get output | ported | fsv-api `site::detail::{planning_rows, change_planning_defs}`, fsv-views `detail::load_planning(…, module)` |
| `44e3e83` | 11-12 05:44 | fix(problems): verify submitted custom fields after server normalization | ported | fsv-api `problem_workflows`, `readback::{submitted_value_matches, decimal}` |
| `820af12` | 11-12 05:50 | fix(cli): render planning validation as clean usage errors | ported | fsv-api `create.rs`, fsv-flows `change.rs`, fsv-core status-0 `ApiError`, fsv-cli `planning-text` errors and `--section` completers |
| `92002d7` | 11-12 06:17 | Guard native planning text creates against existing content | ported | fsv-api `planning_text`, fsv-app `tui`, fsv-tui, fsv-cli goldens |

### `10c3281` fix(planning): verify normalized native section content — ported

Readback reads the section through `section_content` (first string under
`description`/`content`/`text`/…) and compares with `same_text`
(HTML-to-text, whitespace folded), so a server that wraps the text in
`<p>` no longer reports a failed verify.

### `ed6e644` fix(problems): display native analysis sections in get output — ported

`problems get` shows the problem analysis sections (`PROBLEM_SECTIONS`) read
from `problems/{id}/planning-fields` (`problem_planning_fields` rows), the
same way changes show planning. Rows that are not a list of objects give
"planning rows unavailable". fsv-cli reaches this through
`fsv_views::detail::load_detail`; no CLI change needed.

### `44e3e83` fix(problems): verify submitted custom fields after server normalization — ported

The problem create readback checks only the submitted `custom_fields` keys
and accepts canonical scalar normalization (for example `"1.50"` vs `1.5`)
through `readback::submitted_value_matches`; nulls, booleans, missing keys
and list order stay strict. The description is compared with `same_text`. `decimal()` now follows Python
`Decimal()`: strips surrounding whitespace and removes underscores.

### `820af12` fix(cli): render planning validation as clean usage errors — ported

Done: `create.rs` raises a usage error (`Error::value`, exit 2) with the bare
message for `--duplicate must be one of …`, "ambiguous planning field …" and
"planning field not found …", and lists every choice (no cap of 12, also in
fsv-flows `resolve_planning_definition`). `testdata/create_oracle.json` is
regenerated.

Also done:
- **fsv-core**: `fsv_core::ApiError` heads status 0 with `Error` (`Error:
  msg`, `Error creating changes: msg`), checked against Python `92002d7`;
  `create_oracle.json` regenerated.
- **fsv-cli**: `planning-text` catches `APIError`/`SessionError` and exits
  with `exit_code_for` (unknown section: exit 2). The `--section` hints:
  changes use `complete_planning_field_names`, problems `complete_section`.
- **fsv-cli tests** re-captured from Python `92002d7`: `read_cmds.rs`
  `download_change_named_planning`, `download_change_unknown_planning`;
  `write_cmds.rs` `update_planning_unknown`; `upstream_cmds.rs`
  `change_planning_text_unknown_section`, `…_api_error`.

### `92002d7` Guard native planning text creates against existing content — ported

Done: `planning_text::build_plan` returns `PlanningTextPlan { plan, before }`
and refuses when the section already has text ("planning section already has
content; native edit wire is not captured; use the web editor (no write
attempted)") or when attachments are present or unknown. The plan hash is
`plan_hash({"wire_plan": …, "before": …})`; the review adds
`operation: "create"`, `before`, `after`, `plan_hash`. `send` takes
`&PlanningTextPlan` and goes through `reviewed_write::execute_as`.
`editable_content` gives the TUI's starting text. fsv-app `LiveBackend`
carries `before` through `ReviewPlan::raw`. fsv-tui `load_planning_editor`
(`app/reviewed.rs:426`) starts from it through `site::planning_editable_content`
(a local copy of the same checks without the `site` feature); expected editor text
and failures come from Python `FsvApp._load_planning_editor` at `92002d7`
(`load_planning_editor_starts_from_editable_content`).

Also done: **fsv-cli**. `planning.json` gives the created sections no text
and `upstream_cmds.rs` pins the new review keys plus the refusal on a section
with text (`…_existing_refused`).

## Ledger: `92002d7..3432bea`

Triaged against Python `3432bea` (2 commits; `src` is identical at
`e638d85` and `3432bea`). Every fixture outside fsv-cli and fsv-completion
is regenerated from a `git archive 3432bea` export and stamped with it.

Counts: **2 ported, 0 partial, 0 missing.**

| commit | date | subject | status | crates |
|---|---|---|---|---|
| `e638d85` | 11-12 08:15 | Support reviewed native planning text edits and clears | ported | fsv-api `planning_text`, fsv-app `tui`, fsv-tui test docs, fsv-cli `planning-text --clear` |
| `3432bea` | 11-12 08:26 | Test clone partial failures and attachment transport errors offline | ported | fsv-api `create` tests, `FnApi::with_fetch`, fsv-cli `clone_*` tests |

### `e638d85` Support reviewed native planning text edits and clears — ported

Done in fsv-api: `planning_text::build_plan_for(c, module, target, section,
description: Option<&str>, clear, schema)`. `build_plan` stays as the
create/edit wrapper. Stored text plans a `PUT {module}/{id}/planning-fields/KEY`
(operation `edit`) and `clear` plans a bodyless `DELETE` (operation `clear`,
`after: ""`). An empty section stays `POST …?id=KEY`. Refused with no write:
`--clear` plus `--description`; a blank description ("…; use --clear to
delete"); an empty stored row that has an id; clearing an empty section;
contradictory text aliases or `empty` flag (`editable_content`). The clear
readback matches only an empty section with no row id. Expected values come
from `gen_fsv_api_workflows.py` (`planning_text.edit_clear`: wire, race,
transport, envelope, delete_readback, clear_refused, aliases, inputs), and the
tests live in `planning_text.rs`. They mirror `test_planning_edit_clear.py`
(the non-CLI tests) and the edit cases of `test_planning_state.py`.

fsv-app: `LiveBackend` plans an edit of stored text (`tests_reviewed.rs`
`planning_text_edits_stored_text_with_a_put`, golden `changes_edit` from
`gen_fsv_app_tui_reviewed.py`). fsv-tui needed no code change: the editor
already opens the stored text, and the review now comes back from the backend
(`test_tui_native_existing_content_preloads_and_reviews_edit`, cited on
`a_planning_text_picks_a_section_edits_reviews_and_sends_once`). The TUI
never clears, as in Python.

Open (fsv-cli owner, `planning_cli.py`):
- `planning-text --description` becomes optional (help "text/HTML; reviewed
  verbatim; mutually exclusive with --clear"). Add a `--clear` bool flag
  (help "delete existing section text; refuses files or unknown state").
- Call `fsv_api::planning_text::build_plan_for(c, res.name, id, section,
  a.description.as_deref(), a.clear, schema)`. Its `Error::value` refusals
  exit 2, as the other planning-text usage errors do.
- Update the `cli_tree` and completion trees for `--clear`.
- Port `test_cli_edit_clear_review_hash_and_send`,
  `test_cli_missing_or_conflicting_operation_refused` and
  `test_planning_state.py::test_cli_existing_content_review_or_unreviewed_refusal`
  (an existing section: `--dry-run` exits 0 with operation `edit`; `--yes`
  with a wrong `--expect-plan` exits 2 with "full --expect-plan hash").
- Re-pin `tests/upstream_cmds.rs` from Python `3432bea`. These fail against
  the new library (fsv-cli tree at HEAD): `change_planning_text_empty_refused`
  (message gains "; use --clear to delete") and
  `change_planning_text_existing_refused` (an existing section is now a
  reviewed edit, exit 0, not a refusal).

### `3432bea` Test clone partial failures and attachment transport errors offline — partial

Tests only. `port/golden/gen_fsv_api_failure_matrix.py` drives the Python
handlers of `test_attachment_failure_matrix.py` and
`test_clone_failure_matrix.py` through the library and writes
`fsv-api/src/testdata/failure_matrix_oracle.json`. `src/create_failure_tests.rs`
replays them on `FnApi` and its new `with_fetch` download hook. It compares
results, error text, status and `exit_code`, every write with its query string,
reads, downloads, and the upload receipt line. That line is factored out as
`create::upload_receipt`.
- Attachments: all seven Python tests (duplicate policies, replacement
  failures at download, upload or save, a missing upload id, receipt
  diagnostics, an invalid `--duplicate`, a failed download that keeps the
  existing file with force, and a missing URL that creates no directory).
- Clone, library side: the base create, then tasks, assets and planning each
  attempted once at 403/409/429/500 (exit 4 for 409, else 5); the
  `clone_side_effects` counts; and no steps when the create returns no id.

No behavior mismatch was found.

Open (fsv-cli owner, `changes clone`, from `test_clone_failure_matrix.py`):
- `test_partial_clone_preserves_new_identity_and_attempts_each_step_once`:
  the stdout JSON keeps `id: 456`, `human_display_id`, and
  `fsv_clone_steps` (`"ok"` / `"failed: …"`). stderr says "do not rerun
  clone", and the exit is `exit_code_for(first failure)`.
- `test_clone_dry_run_counts_side_effects_without_creating`
  (`--dry-run --keep-task-owners`): stderr "owners/past dates kept", "1
  asset(s)", "1 planning field(s)", "0 attachment(s)"; no writes.
- `test_clone_missing_returned_id_does_not_guess_side_effect_target`: exit
  0, stdout `{}`, stderr "response had no id", one write, read
  `changes/123` only.
- `test_clone_confirmation_failure_prevents_even_base_create`: `--no-input`
  without `--yes` exits 2 mentioning `--yes`; no writes.

## Ledger: `3432bea..2956fce`

Python `src` is identical at `3432bea`, `92d76ff` and `2956fce`; both
commits add tests only. `92d76ff` comes before `2956fce`, so the base pin
stops at `92d76ff`. The new fixture is generated from a `git archive
92d76ff` export; the other fixtures keep their `3432bea` stamp (same `src`).

Counts: **2 ported, 0 partial, 0 untriaged.**

| commit | date | subject | status | crates |
|---|---|---|---|---|
| `92d76ff` | 11-12 08:31 | Exercise query ambiguity and lifecycle rendering matrices offline | ported | fsv-api `query`, `state_flow` (tests), `query::resolve_group` |
| `2956fce` | 11-12 08:36 | Cover cached completion branches and thin subprocess protocol | ported | fsv-completion `dynamic/matrix.rs` (20 scenarios, 121 cases from Python 2956fce) |

### `92d76ff` Exercise query ambiguity and lifecycle rendering matrices offline — ported

`port/golden/gen_fsv_api_query_flow_matrix.py` writes
`fsv-api/src/testdata/query_flow_oracle.json`.

- **`test_query_matrix.py`** → `src/query_matrix_tests.rs`, 11 tests:
  - the exact wire items for 12 filters;
  - the messages for 17 invalid filters;
  - inclusive, merged date bounds for 5 fields with `>`/`<` and `>=`/`<=`;
  - an exact date followed by a repeated bound;
  - field precedence, scope prefixes and ambiguity;
  - person lookups for changes (`agent_id`) and tickets (`responder_id`),
    with the autocomplete calls;
  - agent lookup errors;
  - group lookups;
  - a raw hash, plain and percent-encoded, combined with a filter;
  - malformed raw hashes;
  - empty filters, which need no schema.

  The test needed fsv-api `query::resolve_group` (Python
  `query._resolve_group`, over `Directory::agent_groups`), which is new. For
  a malformed raw hash that fails to decode, Python's `invalid query-hash
  JSON: ` prefix is compared and the decoder's wording after it is not. That
  is the existing PARITY entry for `--query-hash`.
- **`test_state_flow_render.py`** → `src/state_flow_matrix_tests.rs`, 8
  tests:
  - a workspace path with a wrapped or bare response;
  - only a workspace 404 falls back to the plain path (403/404/429/500);
  - the full flow table and approval gates, with its reads;
  - context windows for both compact renderers;
  - names are escaped, not read as markup;
  - nothing is printed when data is unavailable;
  - the full flow reports a missing or empty lifecycle;
  - an approvals denial is optional, but a denial of the flow itself
    propagates.

  The generator records Python's `console.print` arguments: markup strings
  and table title and cells. They are compared with
  `fsv_api::state_flow::Printed`. In the `denied` diagram case, the generator
  fails the flow read, where the Python test patches `get_state_flow`. Both
  print nothing.

No behavior mismatch was found. No case is CLI-only.

`port/fsv_coverage.py` now follows `#[path = "…"]` module declarations and
credits a `#[path]` test file to the file that declares it. Without that,
`query::resolve_group` showed as fn-only, because `resolve_group` is a common
name with three fns.

CLI side of `e638d85` and `3432bea` landed in `c94f2f5`: `planning-text --description` optional, `--clear`, `build_plan_for`; 19 new planning goldens and 7 clone failure tests, all identical to Python `3432bea` (`emit_upstream.py`). `2956fce`'s completion matrix is replayed in `crates/fsv-completion/src/dynamic/matrix.rs`.

## Ledger: `2956fce..94925f4` (library side)

Triaged against Python `94925f4` (7 commits). Each commit was exported with
`git archive` at its own SHA; the fixtures are regenerated from the
`94925f4` export (`/tmp/fsv-regen.sh`, `FSV_PY_COMMIT=94925f4…`). Apart
from the new oracles, every fixture changed only in its stamp. This section
covers every crate except fsv-cli and fsv-completion. The CLI parts are
listed per commit for the fsv-cli owner.

Counts: **7 ported (library), 0 partial, 0 missing.** CLI handoff is open for
`9ab0eaa`, `41783e2`, `f402e61`, `cc41912` and `fdbb3e9`.

| commit | date | subject | status | crates |
|---|---|---|---|---|
| `9ab0eaa` | 11-12 09:15 | Add reviewed ticket resolution closure and reopen workflows | ported | fsv-api `ticket_lifecycle`, `readback::resolution_text`; fsv-views `detail` |
| `41783e2` | 11-12 09:25 | Support reviewed replies and notes with resolved or closed status | ported | fsv-api `conversations` |
| `f402e61` | 11-12 10:04 | feat: add reviewed irreversible ticket merge | ported | fsv-api `ticket_merge` |
| `cc41912` | 11-12 10:11 | feat: add reviewed ticket assignment and pickup | ported | fsv-api `ticket_assignment`, `properties` |
| `25bf9d7` | 11-12 10:22 | fix: include requested item attachments in SR reads | ported | fsv-api `downloads`; fsv-cli `sr.rs` in `1cbf7cc` |
| `fdbb3e9` | 11-12 10:26 | fix: distinguish note identity and normalized readback | ported | fsv-api `notes` |
| `94925f4` | 11-12 10:30 | fix: exclude server managed risk status from clones | ported | fsv-api `create` |

No fsv-app or fsv-tui wiring is needed. At these commits, the Python TUI uses
only `cli_sr._attachment_rows`, which reaches it through
`fsv_api::downloads::attachment_rows`.

### `9ab0eaa` resolution, closure and reopen — ported (library)

- `fsv_api::ticket_lifecycle` is new. `context` and `build_plan` are covered,
  along with `LifecyclePlan` (`payload`/`answers`/`hash`/`review`) and
  `matches`. `send` makes one PUT of the sorted payload, then verifies the
  read-back. Any failure inside the attempt becomes the UNKNOWN refusal,
  with no resend.
- `readback::resolution_text` is new, and fsv-views `detail::resolution`
  uses it.
- Generator: `gen_fsv_api_ticket_lifecycle.py` →
  `ticket_lifecycle_oracle.json` (27 cases, `matches`, `resolution_text`).
- Python's "unknown required field" branch can't be reached through the real
  `load_meta`, because every raw field parses into the spec. The case
  `closure_required_extra_field_and_unavailable` pins what Python actually
  does.

### `41783e2` replies and notes with resolved or closed status — ported (library)

- `conversations::status_fields` is new. For Resolved and Closed it goes
  through `ticket_lifecycle::context`; any other status uses the old status
  context.
  - It passes no resolver. Python passes `ClientResolver`, but no answers are
    involved.
- `build_plan_mail` and `send` now use `status_fields`.
- A closing note records the journal "done" entry only after a verified
  read-back.
- The recipient key is `to_emails` for closing notes.
- The ticket status check needs an int status and Python equality.
- Generator: `gen_fsv_api_closure_conversation.py` →
  `closure_conversation_oracle.json` (20 cases, in
  `conversations/closure_tests.rs`).

### `f402e61` irreversible ticket merge — ported (library)

- `fsv_api::ticket_merge` is new: `build_plan`, `review`, `send` (through
  `reviewed_write::execute`), and the `ticket_id`/`history`/`closed`
  helpers.
- Duplicate conversation IDs are deduplicated by
  `service::load_conversations`, as in Python.
- Generator: `gen_fsv_api_ticket_merge.py` → `ticket_merge_oracle.json`
  (16 cases, plus a direct `closed`/`history` check).

### `cc41912` assignment and pickup — ported (library)

- `fsv_api::ticket_assignment` is new: `AssignmentArgs`, `AssignmentPlan`,
  `build_plan` and `send`.
- In `properties::evaluate`, a null agent or group lookup now encodes as JSON
  `null` (unassign).
- Generator: `gen_fsv_api_ticket_assignment.py` →
  `ticket_assignment_oracle.json`. It has 16 cases, and a `helpers` section
  for `_positive`, `_resolve` and `_member`.

### `25bf9d7` requested-item attachments — ported

- `downloads::attachment_rows` now appends requested-item general
  attachments with source "requested item", deduplicated by id.
- Generator: `gen_fsv_api_sr_attachments.py` → `sr_attachments_oracle.json`.
- The fsv-cli side (`cmd/sr.rs`, which has its own `attachment_rows`)
  landed in `1cbf7cc`.

### `fdbb3e9` note identity and normalized read-back — ported (library)

- `notes::send` read-back now matches `notify_emails` against `to_emails`
  when only the latter comes back.
- The body is compared with `readback::same_text`; every other key is
  compared exactly.
- Generator: `gen_fsv_api_note_readback.py` → `note_readback_oracle.json`
  (6 cases).

### `94925f4` server-managed risk status — ported

- `create::READ_ONLY_FIELDS` gains `attached_risk_survey_response_status`.
- `o_create.py` `CHANGE` carries the field, and `create_oracle.json` shows it
  stripped.

### CLI handoff (fsv-cli owner)

- `ticket_lifecycle_cli.py`: `tickets resolve|close|reopen|status` with
  `--note`, `--set`, `--dry-run`, `--yes`, `--expect-plan` → `ticket_lifecycle`.
- `ticket_merge_cli.py`: `tickets merge` with `--note-primary`,
  `--note-secondary`, the private flags and `--json` → `ticket_merge`.
- `ticket_assignment_cli.py`: `tickets assign` with `--agent`, `--group`,
  `--unassign`, and pickup → `ticket_assignment`.
- The `cli.py` registrations for the three command groups.
- `ticket_notes` `--set-status` help: "add and set status (Pending, Resolved
  or Closed; runtime requirements apply)".
- The `fdbb3e9` add-note message: "Note {id} submitted on ticket {t};
  exact-ID read-back: {identity_verified}; all fields verified: {verified}".
- The test `write_cmds.rs::reply_set_status_not_pending` is stale against
  `41783e2`. `--set-status Resolved` now reads `tickets/5` for the lifecycle
  context; the fixture has no route for it, so the test exits 1 with "HTTP 404
  fetching ticket".

`port/fsv_coverage.py` now maps `ticket_lifecycle`, `ticket_merge` and
`ticket_assignment` to fsv-api.

Next batch: `9910ddc`, `b48889a` (the CLI side is in `b09db91`), `17ada1b`,
`c46d3b0`, `5d7d777`.

## Ledger: `94925f4..39e4853` (library side)

Triaged against Python `39e4853` (11 commits). Each commit was exported with
`git archive` at its own SHA and its generator was first run there; the
fixtures are regenerated from the `39e4853` export (`/tmp/fsv-regen.sh`,
`FSV_PY_COMMIT=39e4853…`). Apart from the new oracles and the strict-query
changes in `query_flow_oracle.json` and `coverage_oracle.json`, every
fixture changed only in its stamp (`create_oracle.json` also has new
wall-clock backup-name stamps). This section covers every crate except
fsv-cli and fsv-completion; the CLI parts are listed for the fsv-cli owner.

Counts: **11 ported (library), 0 partial, 0 missing.** CLI handoff is open for
`9910ddc`, `bd6f6bd`, `17ada1b`, `c46d3b0`, `5d7d777`, `1c186e0` and `ba6aef9`.

| commit | date | subject | status | crates |
|---|---|---|---|---|
| `9910ddc` | 11-12 10:35 | feat: add reviewed ticket note edits and deletion | ported | fsv-api `ticket_note_mutation` |
| `b48889a` | 11-12 10:42 | fix: refuse unsupported strict date filter operators | ported | fsv-api `query`; CLI side in `b09db91` |
| `17ada1b` | 11-12 10:50 | feat: add reviewed problem incident associations | ported | fsv-api `problem_incidents` |
| `c46d3b0` | 11-12 11:00 | feat: add reviewed ticket task lifecycle | ported | fsv-api `ticket_tasks` |
| `5d7d777` | 11-12 11:04 | feat: add reviewed problem status lifecycle and closure preflight | ported | fsv-api `problem_status`, `reviewed_write::execute_rejecting` |
| `1c186e0` | 11-12 11:20 | feat: add reviewed two-step attachments and owner-verified deletion | ported | fsv-api `attachments`; fsv-core `journal::sha256_hex` made public |
| `ec7a6ef` | 11-12 11:34 | fix: render attachment-only planning and conversation lines once | ported | fsv-views `pages::attachment_text` |
| `bd6f6bd` | 11-12 11:39 | fix: enforce strict filter grammar and refuse unproven query operators | ported | fsv-api `query` (`parse_where`, `where_operator` now public) |
| `6e598b2` | 11-12 12:02 | fix: build ticket task create from the task metadata the tenant returns | ported | fsv-api `ticket_tasks` |
| `ba6aef9` | 11-12 12:10 | fix: read the attachment size limit the tenant publishes | ported | fsv-api `attachments`, `service::Api::page`; fsv-app `ClientSource`, fsv-views `AssociationReader` forward `page` |
| `39e4853` | 11-12 12:15 | fix: name the task field and its offered choices when a --set value is not offered | ported | fsv-api `ticket_tasks` |

No fsv-app or fsv-tui wiring is needed. At these commits the Python TUI uses
none of the new reviewed-write modules. Its change detail renders through
`pages.main_lines`, which fsv-app already calls as
`fsv_views::pages::main_lines`, so `ec7a6ef` reaches the TUI as is. The
strict query grammar reaches it through `fsv_api::query`.

### `9910ddc` note edits and deletion — ported (library)

- `fsv_api::ticket_note_mutation` is new: `EDIT`, `DELETE`, `build_plan`,
  `review` and `send`. Edit is one PUT `conversations/{id}` of the sorted
  `{"body", "conversation_from_summary": false}`; delete is a bodyless
  DELETE. Both bind the exact private note (complete history, one row,
  source 2, a bool `private`) and verify by read-back.
- Generator: `gen_fsv_api_ticket_note_mutation.py` →
  `ticket_note_mutation_oracle.json` (28 cases).

### `b48889a` and `bd6f6bd` strict filters — ported (library)

- `query::parse_where` is Python's strict grammar: controls, an empty field
  or value, an unterminated quote and a value starting with an operator
  character are refused, and a malformed operator is never repaired.
- `where_operator` has no equality fallback. On date fields `>`/`<` are
  refused, `>=`/`<=` are inclusive bounds, and anything else is refused.
- Both are now `pub`, so fsv-cli can delegate to them as Python's `cli.py`
  now does.
- Generator: `gen_fsv_api_query_flow_matrix.py` gains a `strict` section
  (23 parse, 9 operator, 1 schema, 3 builder cases), replayed in
  `query_matrix_tests.rs`.

### `17ada1b` problem incident associations — ported (library)

- `fsv_api::problem_incidents` is new: `LINK`, `UNLINK`, `build_plan` and
  `send`. Link is PUT `problems/{p}/tickets/associate`, unlink DELETE
  `problems/{p}/tickets/{t}`. The context binds the complete association
  inventory, which also verifies the write.
- Generator: `gen_fsv_api_problem_incidents.py` →
  `problem_incidents_oracle.json` (21 cases).

### `c46d3b0`, `6e598b2` and `39e4853` ticket tasks — ported (library)

- `fsv_api::ticket_tasks` is new: `TaskPlan`, `TaskArgs`, `inventory`,
  `fields_from`, `custom_values`, `required_for_create`, `reminder`,
  `CONTEXT_FIELDS`, `build_plan` and `send`.
  - Add is POST `tickets/{t}/tasks`, status is PUT `…/tasks/{id}`, and
    delete is PUT `…/tasks/update`.
- `6e598b2`:
  - `notify_before` is sent only when the metadata defines it, and is the
    id of the unique active "Never" choice (string ids accepted).
  - `required_for_create` reads the top-level flags and
    `field_options.required_for_create`, including `"true"`.
  - Every unfilled required field is listed in one refusal.
  - `workspace_id` is neither asked for nor sent.
  - Native (`default: false`) `cf_` fields are sent unprefixed.
- `39e4853`: an unoffered `--set` choice names the field and its offered
  choices.
- Generator: `gen_fsv_api_ticket_tasks.py` → `ticket_tasks_oracle.json`
  (36 cases).

### `5d7d777` problem status — ported (library)

- `fsv_api::problem_status` is new: `ACTION`, `StatusPlan`,
  `closure_context`, `build_plan`, `rejection` and `send`.
- `reviewed_write::execute_rejecting` takes Python's `rejection=` callback
  for a definite 4xx. `execute_as` delegates to it with none.
- Generator: `gen_fsv_api_problem_status.py` → `problem_status_oracle.json`
  (31 cases).

### `1c186e0` and `ba6aef9` attachments — ported (library)

- `fsv_api::attachments` is new. It covers hash-bound `LocalFile`s,
  `AttachedPlan` (`hash`/`review`) and `wrap`.
- Plans are built by `conversation_plan` (ticket note or reply),
  `problem_note_plan`, or `wrap` around a caller-built create plan.
- `send` makes one `POST attachments` per file, then one owning write, then
  `readback`. Any failure inside that becomes the "do not replay" refusal
  with the uploaded IDs.
- Deletion is `deletion_owner`, `delete_plan` and `delete`; the deletion is
  verified by the owner's remaining attachment IDs.
- `ba6aef9`:
  - `size_limit` returns `None` when the metadata has no byte key.
  - `page_limit` reads `window.MAX_ATTACHMENT_LIMIT` (MiB, exact `Decimal`
    arithmetic) from `GET /a/dashboard` through the new `Api::page`.
  - `resolve_limit` applies `max_bytes` overrides.
  - The limit applies to the total of all files.
  - The review gains `attachment_size_limit`.
  - A plan with no limit cannot be sent.
- Generator: `gen_fsv_api_attachments.py` → `attachments_oracle.json`
  (61 cases, both tenants). The Rust test recreates the same files in a
  tempdir.
- PARITY divergences: content types come from `mime_guess`, and a published
  limit beyond `i64` is refused.

### `ec7a6ef` attachment-only lines — ported

- `fsv_views::pages::attachment_text` is used by `main_lines` (description
  and planning) and by `conversation_lines`.
- Generator: `gen_fsv_views_attachment_rendering.py` →
  `tests/fixtures/attachment_rendering.json`. It also checks
  `html_to_text_str` on the raw bodies.

### CLI handoff (fsv-cli owner)

- **`LazyApi` must forward `Api::page`** (`Api::page(self.get()?.as_ref(),
  path)`). Without it, every attached send whose metadata has no byte key
  is refused with "could not read the tenant's published attachment size
  limit".
- `ticket_note_mutation_cli.py`: `tickets note-edit ID CONV_ID --body` and
  `tickets note-delete ID CONV_ID`, with `--dry-run`, `--yes`,
  `--expect-plan`, `-o` and `--json` → `ticket_note_mutation`.
- `bd6f6bd` `cli.py`:
  - `_parse_where` and `_where_operator` delegate to `query` (error text
    unchanged, exit 2). Use `fsv_api::query::{parse_where, where_operator}`.
  - A relative date period accepts only `=`/`>=`; anything else is "relative
    date periods support only = or >=; no inverse period wire is proven".
  - fsv-cli's own `filters.rs` grammar still passes its tests but no longer
    matches Python.
- `problem_cli.py`:
  - `problems link-incident` and `problems unlink-incident` →
    `problem_incidents`.
  - `problems status --to` → `problem_status`.
  - `problems note --body --attach [--max-bytes N]` →
    `attachments::problem_note_plan`.
- `ticket_tasks_cli.py`: `tickets tasks add|status|delete|list` →
  `ticket_tasks`. The old ticket `tasks` command becomes the hidden
  `tasks-legacy`.
- `attachments_cli.py`:
  - `attachments delete ID --owner tickets/ID|problems/ID|changes/ID
    [--child N | --section KEY]` → `attachments::{delete_plan, delete}`.
  - `finish` exits 4 when the read-back is not verified.
- `--attach` on `tickets note`, `tickets reply` (requires `--sender`) and
  `tickets create`. Create builds `WritePlan("ticket_create_attachments", 0,
  …, "POST", "tickets", json.dumps(payload, sort_keys=True), context)` and
  `attachments::wrap(base, paths, &raw, c, max_bytes)`.
- `ba6aef9`: `--max-bytes N` (`min=1`) on `tickets create`, `tickets reply`,
  `tickets note` and `problems note`. The help text is "explicit total
  attachment size limit in bytes; replaces the tenant-published limit
  (required to send when the tenant publishes none)".
- The `test_completion_gen` changes for the new commands.

`port/fsv_coverage.py` now maps `ticket_note_mutation`, `problem_incidents`,
`ticket_tasks`, `problem_status` and `attachments` to fsv-api.

## Ledger: `39e4853..7f58023` (library side)

Triaged against Python `7f58023`. `8f156c0` and `d1c158b` were exported at
their own SHAs; fixtures that only needed the snapshot rules were already
stamped `8f156c0`. `problem_status`, `ticket_lifecycle` and the detail-line
fixture were regenerated from a `git archive 7f58023` export
(`FSV_PY_COMMIT=7f580232e8e4ce1e5dae059115fca1bb697ef0c3`). Write payloads
were not retuned. This section covers every crate except fsv-cli and
fsv-completion.

Counts: **3 ported, 0 partial.** `port/UPSTREAM_BASE` is `7f58023`.
Commits after it (through `e7c536b`) are a later batch, not triaged here.

| commit | date | subject | status | crates |
|---|---|---|---|---|
| `8f156c0` | 11-12 12:29 | fix: stabilize reviewed read snapshots without changing write payloads | ported | fsv-core `reviewed_snapshot`, `journal::snapshot_hash`, `pystr` URL checks; fsv-api pre-write read hashes |
| `d1c158b` | 11-12 12:34 | fix: read native article inventory for problem closure preflight | ported | fsv-api `problem_workflows::inventory` |
| `7f58023` | 11-12 13:52 | Revert detail dedupe: planning text lines and attachments are both real content | ported | fsv-views `pages` |

### `8f156c0` stable read snapshots — ported

`fsv_core::reviewed_snapshot::stable_snapshot` strips volatile timestamps,
keeps attachment inventories to the stable field set, and drops signing
query params from `http`/`https` links inside text. `custom_fields` is
kept verbatim. `fsv_core::journal::snapshot_hash` is `plan_hash` of that
value. A link `urlsplit` rejects is `Err` (Python `ValueError`).

`fsv_core::pystr::{urlsplit, urlunsplit}` now apply Python 3.12's bracketed
netloc / IPv6 / NFKC netloc checks and `urlunsplit`'s `uses_netloc` table.
Plan hashes of the write body stay `plan_hash`. The pre-write read in these
modules is `snapshot_hash` (or `stable_snapshot` where the value is stored,
not only hashed): `attachments`, `canned`, `conversations`,
`list_preferences`, `planning_text`, `private_views`, `problem_incidents`,
`problem_known_error`, `problem_status`, `problem_workflows`, `properties`,
`scenarios`, `ticket_assignment`, `ticket_lifecycle`, `ticket_merge`,
`ticket_note_mutation`, `ticket_tasks`, `ticket_time`, `watchers`.

Churn goldens: `ticket_note_mutation` rotate cases, `attachments`
`delete_signed_*`, `ticket_lifecycle` `read_churn`. A re-read that only
re-signs links and bumps `updated_at` still sends. A real body change still
refuses.

### Public API for the fsv-cli owner

No new fsv-api method. Plan builders already used by the CLI now hash the
stable snapshot internally. Call these only if a CLI path hashes a read
itself:

- `fsv_core::reviewed_snapshot::stable_snapshot(&Value) -> Result<Value, String>`
- `fsv_core::journal::snapshot_hash(Option<&Value>) -> Result<Option<String>, String>`

Both are public. fsv-api's wrappers stay `pub(crate)`.

### `d1c158b` article inventory — ported

`problem_workflows::inventory` still requests `problems/{id}/{kind}` and
still reports "problem {kind}". For `kind == "solutions"` the page key is
`articles`. A `solutions` envelope, a missing list, a non-list, or a short
page is "complete problem solutions inventory is unavailable". Closure still
requires a non-empty list; `article_type` filtering is not in this commit.

### `7f58023` detail lines — ported

`pages::attachment_text` is gone. Description, planning text and
conversation body render in full, and the 📎 rows render as well. An empty
conversation body is `(empty)` even when files are attached. PARITY's
`ec7a6ef` row is replaced.

### CLI handoff (do not edit fsv-cli here)

- Route fixtures `tests/fixtures/routes/problem_status.json` and
  `problem_status_rejected.json` still return a `solutions` key.
  `tests/golden/upstream/problem_status.json` names the close cases that
  read that route. They are not `#[test]` fns in HEAD `upstream_cmds.rs`.
  When emitted, these fail closed until the fixture uses `articles`:
  `problems_status_close_dry_run`, `problems_status_close_send`,
  `problems_status_close_rejected_by_server`,
  `problems_status_close_missing_analysis_refused`,
  `problems_status_close_required_field_refused`.
- Any golden that pins `plan_hash` for a command whose pre-write read now
  goes through `snapshot_hash` would move if that read had a signed URL or
  a volatile timestamp. Scratch tree (`git archive HEAD` plus this library
  diff, fsv-cli left at HEAD): `cargo test -p fsv-cli --offline` passed,
  0 failed. No compiled fsv-cli test pins a moved plan hash.
- `7f58023` changes detail markup. CLI or TUI goldens that expected an
  attachment-named line to be omitted now expect the line and the 📎 row.

`port/fsv_coverage.py` maps `reviewed_snapshot` to fsv-core.

## Ledger: `36408b2` (library side)

Triaged against a `git archive 36408b2` export
(`FSV_PY_COMMIT=36408b25fe8f9488d664efefbd104ba5b5f02b25`).
`port/golden/gen_fsv_api_integrated_assets.py` stamps
`crates/fsv-api/src/testdata/integrated_assets_oracle.json`. The TUI
asset block in `reviewed_expected.json` was regenerated from the same
export; the rest of that fixture is unchanged apart from the stamp.
This section covers every crate except fsv-cli and fsv-completion.

Counts: **0 ported, 1 partial.** `port/UPSTREAM_BASE` is `36408b2`.
The partial is the fsv-cli half, named below.

| commit | date | subject | status | crates |
|---|---|---|---|---|
| `36408b2` | 11-12 14:31 | Use integrated inventory for reviewed CI and service associations | partial | fsv-api `integrated_assets`, `asset_picker`, `Api::absolute_get`; fsv-flows `with_devices`; fsv-app `LiveWriter::associated_devices`. fsv-cli follow-up below |

### `36408b2` integrated inventory — partial

`fsv_api::integrated_assets` ports `search`, `inventory`, `build_plan`,
`send`, and `verify_devices`. Search is `GET https://{domain}/itam/core/{family}/api/`
with `order`, `page`, `page_size=50`, `q`. Association reads are
`GET {module}/{id}/associated-cis`. Add sends decimal-string
`device_ids` or `businessapp_ids`. Remove sends the current
`itam_cmdb_request_id`. `sr` is the tickets wire. `search_assets_for_change`
stays; only the picker stopped calling it.

`asset_picker::candidates` checks the change id, then searches devices.
Each row is `display_id`, `name`, `ci_type_name`, and `inventory`.

Create-time devices are not a field on `CreateOptions` or `TicketOptions`
(fsv-cli builds those as struct literals). `fsv_flows::change::with_devices`
and `fsv_flows::ticket::with_devices` insert `device_ids` and rehash.
After a successful create, a truthy `device_ids` adds `device_verification`.
`ChangeWriter::associated_devices` fails closed unless the writer reads
inventory. `LiveWriter` reads change devices. Ticket create reads through
`integrated_assets::inventory`.

`Api::absolute_get` defaults to `absolute request is unavailable: {url}`.
`Client`, `FakeApi`, and `FnApi` implement it.

### Public API for the fsv-cli owner

- `fsv_api::integrated_assets::{target_id, search, inventory, build_plan, send, verify_devices, verification}`
- `fsv_flows::change::with_devices(plan, &[i64]) -> Result<Plan>` and
  `fsv_flows::ticket::with_devices`. Call after `build_create_plan` where
  Python passes `devices=`.
- `Api::absolute_get(&self, url, params) -> Result<Value>`. `LazyApi` must
  forward it to `Client::request("GET", url, params)` or asset search and
  the ITAM commands fail closed.

### CLI handoff (do not edit fsv-cli here)

`integrated_assets_cli.register(app, "sr", factory)` adds `sr assets`
`list` / hidden `ls` / `add` / `remove`. `search`, hidden `legacy`, and
the `ci-add` / `ci-remove` / `ci-list` aliases are registered only when
the module is `changes`, and this commit does not call `register` for
changes. JSON create output includes `device_verification` when the
result has it.

Scratch tree (`git archive HEAD` plus this library diff, fsv-cli left at
HEAD): `cargo test -p fsv-cli --offline` is 545 passed, 13 failed. All 13
are asset-picker tests whose fixture still serves
`GET /api/_/assets-to-associate`. The library now requests
`GET /itam/core/device/api/` and the failure is `HTTP 404 listing device`.
Do not edit fsv-cli. Failing tests:

`asset_picker_api_error_is_uncaught`, `asset_picker_bad_display_id`,
`asset_picker_display_id_target`, `asset_picker_duplicate_display_ids`,
`asset_picker_duplicate_then_bad`, `asset_picker_empty_page`,
`asset_picker_empty_select`, `asset_picker_page`,
`asset_picker_row_not_object`, `asset_picker_rows_not_list`,
`asset_picker_search_page_2`, `asset_picker_select`,
`asset_picker_select_missing`.

These still pass because they refuse before the search:
`asset_picker_bad_target`, `asset_picker_page_must_be_positive`,
`asset_picker_ticket_target`, `asset_picker_zero_target`.

`port/fsv_coverage.py` maps `integrated_assets` to fsv-api. Audit
(`/tmp/cov.md`, live Python tree): 1777 rows, 1552 covered, 45 missing,
16 fn-only. Every `fsv.integrated_assets` def is covered. Missing from
this commit: `integrated_assets_cli.finish`, `register`, `register_search`.

## Ledger: `9070409` (library side)

Triaged against a `git archive 9070409` export
(`FSV_PY_COMMIT=9070409b2f42c8616cd3c3c3ab43e80eeeae2a61`).
`port/golden/gen_fsv_api_sr_approvals.py` stamps
`crates/fsv-api/src/testdata/sr_approvals_oracle.json`.

Counts: **0 ported, 1 partial.** `port/UPSTREAM_BASE` is `9070409`.
The partial is the fsv-cli command group, named below.

| commit | date | subject | status | crates |
|---|---|---|---|---|
| `9070409` | 11-12 14:36 | Add reviewed service request approval lifecycle | partial | fsv-api `sr_approvals`, `approval_groups`; `Client::send_once` Content-Type. fsv-cli follow-up below |

### `9070409` SR approvals — partial

`fsv_api::sr_approvals` ports `inventory`, `recipients`, `build_plan`, and
`send`. Inventory reads `tickets/{id}/approval-chains` through
`approval_groups`. Request is `POST tickets/{id}/approval-groups`. Approve
and reject are `PUT .../approvals/{id}` with `approval_status` 1 or 2.
Cancel and remind are bodyless PUTs and still send
`Content-Type: application/json`. `Client::send_once` always sets that
header (`reviewed_write.execute` at this commit).

`get_change_approval_groups` stays the change wrapper. A non-positive id
is now `record ID must be positive`. `approval_groups` accepts `changes`
or `tickets`.

### Public API for the fsv-cli owner

- `fsv_api::sr_approvals::{inventory, recipients, build_plan, send, ApprovalPlan}`
- `fsv_api::approvals::approval_groups(c, target, module)`

### CLI handoff (do not edit fsv-cli here)

`sr_approvals_cli.register` adds `sr approvals list|request|approve|reject|cancel|remind`.
`list` prints `{"approval_groups": ...}`. `request` takes `--name`,
`--message`, repeatable `--approver`, `--rule`. The other actions take
the approval id, or the group id for `cancel`, plus `--remark` (required
on reject). Dry-run and send go through `integrated_assets_cli.finish`.

`port/fsv_coverage.py` maps `sr_approvals` to fsv-api.

## Ledger: `be41032` (library side)

Triaged against a `git archive be41032` export
(`FSV_PY_COMMIT=be410329d2f89944bd445e414738754681d2c19a`).
`port/golden/gen_fsv_api_change_task_update.py` stamps
`crates/fsv-api/src/testdata/change_task_update_oracle.json`.

Counts: **0 ported, 1 partial.** `port/UPSTREAM_BASE` is `be41032`.
The partial is `changes tasks-update` in fsv-cli, named below.

| commit | date | subject | status | crates |
|---|---|---|---|---|
| `be41032` | 11-12 14:40 | Review complete captured change task update fields | partial | fsv-api `change_task_update`. fsv-cli follow-up below |

### `be41032` change task update — partial

`fsv_api::change_task_update` ports `inventory`, `reminder`, `build_plan`,
and `send`. The PUT is `changes/{id}/tasks/{task}` and contains only the
supplied fields. `owner`/`group` of `none` write a null id. Reminder
seconds come from the runtime `notify_before` choice. A `Completed` status
refuses when a `required_for_closure` field is blank. Readback matches
dates by instant, notes by stripped HTML, and `owner_id` against `agent_id`
when the task stores the alias.

### Public API for the fsv-cli owner

`fsv_api::change_task_update::{inventory, reminder, build_plan, send, TaskUpdatePlan}`.

`build_plan(c, target, task, inputs)` takes the captured map. Keys: `title`,
`status`, `owner`, `group`, `note`, `planned_start`, `planned_end`,
`planned_effort`, `notify_before`, `values`. `send` rebuilds from
`inputs_json`.

### CLI handoff (do not edit fsv-cli here)

`changes tasks-update` should call `build_plan` / `send` through
`integrated_assets_cli.finish` and `guard`. Map flags into `inputs`:
`--title`, `--status`, `--owner`/`--agent`, `--group`, `--note`,
`--planned-start`, `--planned-end`, `--planned-effort`, `--notify-before`.
`--system` and `--environment` become `values["cf_system"]` and
`values["cf_environment"]`. `--edit` and `--due-date` raise
`ValueError("--edit/--due-date are not captured task-update contracts; use explicit planned dates and reviewed flags")`.
Dry-run, `--yes`, `--expect-plan`, and `--read-only` stay on `finish`.

`port/fsv_coverage.py` maps `change_task_update` to fsv-api.

## Ledger: `0719e33` (library side)

Triaged against a `git archive 0719e33` export
(`FSV_PY_COMMIT=0719e3325c1252974c61081872b91b2967b760e5`).
`port/golden/gen_fsv_api_solutions.py` stamps
`crates/fsv-api/src/testdata/solutions_oracle.json`.
`port/golden/gen_fsv_api_problem_status.py` was regenerated at the same commit
because closure now requires `article_type` 1 or 2. Closed plan hashes moved.
Nothing in fsv-cli pins the old hashes.

Counts: **0 ported, 1 partial.** `port/UPSTREAM_BASE` is `0719e33`.
The partial is the fsv-cli command surface, named below.

| commit | date | subject | status | crates |
|---|---|---|---|---|
| `0719e33` | 11-12 14:49 | Create draft solutions and review problem article links | partial | fsv-api `solutions`, `problem_status::closure_context`. fsv-cli follow-up below |

### `0719e33` solutions — partial

`fsv_api::solutions` ports draft create, link, and unlink. Create is
`POST solutions/articles` with `status` 1, `source` `problem`, and
`article_type` 1 (permanent) or 2 (workaround). Category and folder come
from complete metadata; a unique default is used when the caller omits
them. The plan target of a create is the folder id. Link and unlink are
`PUT problems/{id}/solutions/{article}` with `{"type":"link"}` or
`{"type":"unlink"}`. Readback uses the `articles` inventory.

`closure_context` accepts a nondeleted article whose `article_type` is the
integer 1 or 2. Any other link, including a missing type, still counts as
missing.

### Public API for the fsv-cli owner

`fsv_api::solutions::{metadata, folder, article, create_plan, send_create, link_plan, send_link, SolutionPlan}`.

### CLI handoff (do not edit fsv-cli here)

`solutions_cli.register` adds top-level `solutions create` (`--title`,
`--body`, `--type` workaround|permanent, optional `--category` and
`--folder`, plus dry-run / `--yes` / `--expect-plan` / `--read-only`).
`solutions_cli.register_links` adds `problems link-solution` and
`problems unlink-solution` (problem id, article id). Both go through
`integrated_assets_cli.finish` and `guard`. Create does not link or publish.

`port/fsv_coverage.py` maps `solutions` to fsv-api.

## Ledger: `d870fa7` (library side)

Triaged against a `git archive d870fa7` export
(`FSV_PY_COMMIT=d870fa754623adf838f7381e8b6a5cdf836e1e1c`).
`port/golden/gen_fsv_api_approval_recipients.py` stamps
`crates/fsv-api/src/testdata/approval_recipients_oracle.json`.
The individual-approver request hash
`7f34fb9adce9ce2a92cbf6f9ff6c4239eb8cac53e6c4a70b2d1dc23496111c07`
is unchanged. `review()` now always includes `recipient_sources`
(`[]` when the context is empty).

Counts: **0 ported, 1 partial.** `port/UPSTREAM_BASE` is `d870fa7`.
The partial is the fsv-cli command surface, named below.

| commit | date | subject | status | crates |
|---|---|---|---|---|
| `d870fa7` | 11-12 14:58 | Expand reviewed group and CAB selections to eligible users | partial | fsv-api `approval_recipients`, `approvals`. fsv-cli follow-up below |

### `d870fa7` approval recipients — partial

`fsv_api::approval_recipients` expands a group or CAB to its eligible
`approval_users`. Group members are paged (`per_page` 30, up to 100
pages) and must report one stable count. CAB members are one
`changes/{id}/cab-members` read. Bootstrap `agent_ids` are not
recipients. The union of user ids is what `notified_to` carries.
`resolve` returns that list plus `recipient_context_json`. Empty
context stays `""` and does not enter the plan hash. A non-empty
context adds `recipient_sources` to the hash. `validate` re-reads
membership and refuses when it moved. `send` calls `validate` inside
the same prepare path that rewrites every failure as
`could not prepare a complete approval inventory/session; no approval POST was attempted`.
`resolve_approvers` looks up emails through requester autocomplete.

### Public API for the fsv-cli owner

`fsv_api::approval_recipients::{members, resolve, validate}`.
`approvals::build_plan` stays six arguments. `build_plan_with` adds
`recipient_context_json`. `ApprovalPlan.recipient_context_json` is the
stored membership. `review()` always has `recipient_sources`.

### CLI handoff (do not edit fsv-cli here)

`changes approval-request` must accept optional `--approver`, plus
`--approver-group` and `--cab` / `--approver-cab`. If none of the three
is passed, ValueError `provide --approver, --approver-group or --cab explicitly`.
Expand every eligible `approval_users` row (not bootstrap `agent_ids`),
dedupe into `notified_to`, put the membership in `recipient_sources`
and the hash, and call `validate` before send. `ReviewedWriteError`
exits 2. `cmd/review.rs` still requires `--approver` and does not pass
groups or cabs.

Scratch goldens that now fail, left unedited:
`approval_request_no_input` and `approval_request_send_unverified`
(review JSON gained `recipient_sources: []`; plan hash
`c920ebf2e43cd003d49aeff9cb60ab8811aa6820896ef977e0d8bebca81758a1`
did not move), and `approval_request_dry_run` (email lookup now
calls requester autocomplete, so the agents fixture misses and
the duplicate-id check never runs).

`port/fsv_coverage.py` maps `approval_recipients` to fsv-api.

## Ledger: `780c002` (library side)

Triaged against a `git archive 780c002` export
(`FSV_PY_COMMIT=780c002ec61d25dbc465c54027312fc8bcef6d19`).
`port/golden/gen_fsv_api_change_tasks_required.py` stamps
`crates/fsv-api/src/testdata/change_tasks_required_oracle.json`.
A plan built without `fields` keeps its hash. `fields=[]` binds
metadata and moves the hash. The review contract text changed for
every plan.

Counts: **0 ported, 1 partial.** `port/UPSTREAM_BASE` is `780c002`.
The partial is the fsv-cli command surface, named below.

| commit | date | subject | status | crates |
|---|---|---|---|---|
| `780c002` | 11-12 15:13 | Preflight runtime required change task fields and expose custom values | partial | fsv-api `tasks`. fsv-cli follow-up below |

### `780c002` change task required fields — partial

`validate_required` refuses a create whose runtime
`required_for_create` fields are blank, skipping `workspace_id`.
Custom values go on the payload as `custom_fields`. When the field
list contains `notify_before`, the seconds come from the runtime
choice. The hash gains `metadata` only when `fields_json` is set.
`send` also refuses when a legacy plan (the same plan with
`fields_json` cleared) is already `started` or `done`. A metadata
change on send is swallowed into the prepare failure. Description
readback uses `strip_html`. Custom fields use
`submitted_value_matches`.

### Public API for the fsv-cli owner

`tasks::build_plan` is unchanged. Set `TaskArgs.custom_fields` and
`TaskArgs.fields`, or call `build_plan_ex`. `runtime_fields(c)` reads
`change_task_fields`. `validate_required(fields, payload)`.

### CLI handoff (do not edit fsv-cli here)

`changes tasks-add` must take repeatable `--set NAME=VALUE`,
`--system`, and `--environment` / `--env`. Load `runtime_fields`
before the real plan. Map those flags through `ticket_tasks::custom_values`.
Pass the field list into the plan so the hash binds metadata. The
review contract is now `Accepted native create/task envelope. Runtime required_for_create custom fields and dates validated; blank optional fields omitted.`

Scratch goldens that pin the old contract, left unedited:
`task_add_dry_run`, `task_add_no_input`, `task_add_confirm_eof`,
`task_add_hash_mismatch`, `task_add_send_unverified`,
`task_add_full_dry_run`. Their plan hashes are unchanged because the
CLI still builds without `fields`.

## Ledger: `ad40a51` (library side)

Triaged against a `git archive ad40a51` export
(`FSV_PY_COMMIT=ad40a517bc4cbdf16b0eb26acf87725f824c529c`).
`gen_fsv_api_solutions.py` and `gen_fsv_api_integrated_assets.py`
were regenerated at that commit. Create `device_ids`, association
`device_ids` / `businessapp_ids`, `itam_cmdb_request_id`, and
solution `folder_id` are JSON integers. fsv-cli does not pin the
old hashes.

Counts: **1 ported, 0 partial.** `port/UPSTREAM_BASE` is `ad40a51`.

| commit | date | subject | status | crates |
|---|---|---|---|---|
| `ad40a51` | 11-12 15:50 | Serialize reviewed inventory and solution identities as JSON integers | ported | fsv-api `solutions`, `integrated_assets`; fsv-flows `change::with_devices` |

`with_devices` writes integer `device_ids`. Association add writes the
id list as integers. Removal writes `positive_id` of
`itam_cmdb_request_id`. Solution create writes integer `folder_id`.

## Ledger: `e173437` (library side)

Triaged against a `git archive e173437` export
(`FSV_PY_COMMIT=e1734374f1ab283a7077a238cfd4bcaf3561ac0c`).
`gen_fsv_api_approval_recipients.py` was regenerated. CAB reads
prefer `approval_agents`. A second `approval_users` list is
accepted only when its ids match. The flatten plan hash is
unchanged.

Counts: **1 ported, 0 partial.** `port/UPSTREAM_BASE` is `e173437`.

| commit | date | subject | status | crates |
|---|---|---|---|---|
| `e173437` | 11-12 15:54 | Accept native CAB approval_agents recipient envelopes | ported | fsv-api `approval_recipients` |

## Ledger: `b310885` (library side)

Triaged against a `git archive b310885` export
(`FSV_PY_COMMIT=b31088581522be3b5c9a26a32093ae9a1e6191fa`).
`port/golden/gen_fsv_api_change_task_update.py` was regenerated.
Plan hashes are unchanged. Send results gained `fields_verified`.
fsv-cli does not pin these hashes.

Counts: **1 ported, 0 partial.** `port/UPSTREAM_BASE` is `b310885`.

| commit | date | subject | status | crates |
|---|---|---|---|---|
| `b310885` | 11-12 15:59 | Normalize task effort readbacks and report omitted reminders as unavailable | ported | fsv-api `tasks::same_effort`, `change_task_update` |

`same_effort` treats `1h` and `1h 0m` as the same duration, and `90m` as
`1h 30m`. Create-path `compare_fields` uses it for `planned_effort`.
Description on that path stays stripped HTML.

Update-path readback uses `same_text` for description and `same_effort`
for `planned_effort`. `verified` stays true when `notify_before` is
`unavailable`. `fields_verified` stays false in that case. The warning
is `Reminder absent from GET remains unverified; do not resend.` Other
missing fields still fail verification.

No new CLI flags. A `changes tasks-update` caller that prints `send`'s
result gets `fields_verified` and that warning. The `be41032` CLI
follow-up is unchanged.

## Ledger: `11e76ed` (library side)

Triaged against a `git archive 11e76ed` export
(`FSV_PY_COMMIT=11e76ed502217750f9713371113eddf406b8aa9c`).
`port/golden/oracles/fsv_oracle/o_catalog.py` was regenerated into
`crates/fsv-api/src/testdata/catalog_oracle.json`. Plans with a
requester and another recipient now bind `requester_id`. The
recipients plan hash moved from `ab09e59e…` to `917c8820…`.
fsv-cli does not pin the old hash.

Counts: **1 ported, 0 partial.** `port/UPSTREAM_BASE` is `11e76ed`.

| commit | date | subject | status | crates |
|---|---|---|---|---|
| `11e76ed` | 11-12 16:06 | Verify SR recipients from exact ticket user identities and nested projections | ported | fsv-api `sr_recipient_readback`, `catalog` |

`recipient_fields` accepts an email string, a nested `{id, email}`, and
the ticket's `requester_id` / `requested_for_id`. Every representation
has to agree. CC stays a case-insensitive list. `duplicate_check` still
blocks a journal `started` event recorded under the hash from before
`requester_id` was bound.

No new CLI flags. `build_sr_plan` / `execute` / `duplicate_check` pick
this up for any caller that already passes a requester with
`requested_for` or CC.

## Ledger: `ffcab5b` (library side)

Triaged against `git show ffcab5b`
(`ffcab5b1b47a3dc7f445b3cc8fd5c6e1c4d7b8eb`). The change is in
`Client._request_inner`: a blank body is `None` before JSON parsing.
`fsv-core` `request_inner` returns JSON null for that body. Non-empty
malformed JSON and HTTP 400/500 still fail. No plan hash moved.

Counts: **1 ported, 0 partial.** `port/UPSTREAM_BASE` is `ffcab5b`.

| commit | date | subject | status | crates |
|---|---|---|---|---|
| `ffcab5b` | 11-12 16:15 | Parse bodyless solution unlink success before independent readback | ported | fsv-core `client` |

A 200 or 204 labeled `application/json` with an empty or whitespace
body is null, same as a JSON `null` body. Unlink readback still decides
whether the article is gone. No CLI flag change.

## Ledger: `e7c536b` (library side)

Triaged against a `git archive e7c536b` export
(`FSV_PY_COMMIT=e7c536b57dcec600a09200ccc130ed0212a3775a`).
`port/golden/gen_fsv_api_agent_lookup.py` stamps
`crates/fsv-api/src/testdata/agent_lookup_oracle.json`.

Counts: **0 ported, 1 partial.** `port/UPSTREAM_BASE` is `e7c536b`.
The partial is fsv-cli `lookup.rs` `resolve_agent` and the task-add
help text, named below.

| commit | date | subject | status | crates |
|---|---|---|---|---|
| `e7c536b` | 11-12 16:57 | Resolve exact agent emails from native autocomplete | partial | fsv-api `agent_lookup`, `tasks::resolve_owner`, `query::resolve_agent`, `change_task_update`. fsv-cli follow-up below |

`exact_agent_id` returns a user id only for one exact active row.
Email lookup reads `id`, `email`, and `details` (the native `id` is
the email). A second email on the row that disagrees is not a match.
Full-name lookup reads `value` and `name`. `resolve_owner` accepts an
id, an exact email, or a full name. Lookup failure is `TaskError`.
`changes tasks-update` turns that `TaskError` into `ValueError`.
`query._resolve_agent` uses the same email match and still falls
through to `pick_lookup` for a non-email.

### CLI handoff (do not edit fsv-cli here)

`lookup.rs` `resolve_agent` should, for an email, call
`agent_lookup::exact_agent_id` and return the user id as text. A
`ValueError` from that lookup is the command error. Non-email values
keep `pick_lookup` on `value`, `id`, `email`, and `details`. The
`changes tasks-add --owner` help text should say an explicit user id,
exact email, or full name.

Still not triaged: none. Range through `e7c536b` is triaged.

## Ledger: `4626e12` (library side)

Triaged against a `git archive 4626e12` export
(`FSV_PY_COMMIT=4626e1203103bb75bb764625345b6c0c0a4652e7`).
`port/golden/gen_fsv_api_ticket_tasks.py` stamps
`crates/fsv-api/src/testdata/ticket_tasks_oracle.json`. Existing
add/status/delete plan hashes are unchanged.

Counts: **0 ported, 1 partial.** `port/UPSTREAM_BASE` is `4626e12`.
The partial is the fsv-cli ticket-task command, named below.

| commit | date | subject | status | crates |
|---|---|---|---|---|
| `4626e12` | 11-12 17:51 | Add metadata-gated ticket task owner and group reviews | partial | fsv-api `ticket_tasks`. fsv-cli follow-up below |

`build_plan` is unchanged: status is always sent, and owner and group
stay unset. `build_plan_assigned` adds an optional owner, an optional
group, and a status that can be omitted. `TaskArgs` has no new fields.
An `update` writes `PUT tickets/{id}/tasks/{task}` and includes
`owner_id` or `group_id` only when that task field is the one active
copy. An email owner is `exact_agent_id`; any other owner is
`positive_id`. A group is `resolve_group`, and a blank group is
`group must be a positive ID or exact name`. Title, reminder, required
fields, and `custom_fields` stay create-only unless `values` is set.
An empty update is `provide at least one task update field`. `send`
rebuilds from inputs and leaves status out when the stored status is
JSON null.

### CLI handoff (do not edit fsv-cli here)

`tickets tasks add` should take optional `--owner` and `--group` and
call `ticket_tasks::build_plan_assigned`. Add `tickets tasks update
TARGET TASK` with optional `--title`, `--owner`, and `--group`. That
command calls `build_plan_assigned` with action `update`, status
omitted, and no values. Dry-run, `--yes`, and `--expect-plan` stay on
the existing reviewed-write helper. Do not add fields to `TaskArgs`.

Still not triaged: `dbe1149` and later.

## Ledger: `dbe1149` (library side)

Triaged against `git show dbe1149`
(`dbe1149d812a468e061bb0c1e6675a01ef790ddd`). The write itself is the
existing `integrated_assets` plan. This commit is the TUI review around
it. `docs/reviewed-parity.md` has no library behavior.

Counts: **1 ported, 0 partial.** `port/UPSTREAM_BASE` is `dbe1149`.

| commit | date | subject | status | crates |
|---|---|---|---|---|
| `dbe1149` | 11-12 18:04 | Add reviewed TUI device and service association actions | ported | fsv-tui `assets`, fsv-app `LiveBackend` |

`i` on a ticket, problem, or change opens an association menu. Read-only
mode only searches devices and does not prepare a plan. Add searches one
integrated page and reviews the chosen id. Remove lists the associated
rows and reviews one id. Confirm sends once through
`integrated_assets.send`. Cancel and an empty association list do not
write.

Still not triaged: `9c8c91d` and later.

## Ledger: `9c8c91d` (library side)

Triaged against a `git archive 9c8c91d` export
(`FSV_PY_COMMIT=9c8c91d8a10c52bd57a015e6591a8bea11c16b59`).
`crates/fsv-flows/tests/golden/gen_flows.py` stamps
`crates/fsv-flows/tests/golden/flows.json`. Create hashes are unchanged.
Update hashes now wrap the legacy request hash, the record snapshot,
the metadata fingerprint, and the tenant. The generator pins an empty
`FSV_CONFIG_DIR` before importing `fsv`, and it snapshots form fields
once so later plans share that fingerprint.

Counts: **0 ported, 1 partial.** `port/UPSTREAM_BASE` is `9c8c91d`.
The partial is the fsv-cli change update and task-delete commands,
named below. `docs/change-tasks.md` and `docs/reviewed-parity.md` have
no library behavior.

| commit | date | subject | status | crates |
|---|---|---|---|---|
| `9c8c91d` | 11-12 18:44 | Audit TUI sends with real transport and close legacy write gaps | partial | fsv-flows `change`, fsv-api `change_task_delete`, fsv-tui, fsv-app. fsv-cli follow-up below |

An update plan stores `legacy_hash`, `record_hash`, and `tenant`.
`change::execute` keeps its signature and sends property updates through
`send_form_update`. Files or planning on that plan are refused before
any PUT. An empty tenant or a changed payload is
`Change tenant or exact reviewed payload changed; review again`.
A missing hash, payload, or record snapshot is
`Complete hash-bound property plan required`. A journal `started` or
`done` on the wrapped hash or the legacy hash is
`Change update was sent or outcome unknown; do not replay`.
A definite 4xx records `failed` and
`write rejected (HTTP {status}); no retry`. Any other write error
leaves `started` and
`write outcome unknown; inspect target and journal before retry`.
Readback requires the target id, `display_id`, or `CHN-` suffix, the
same record snapshot, and the same metadata fingerprint. Description
matches with `same_text` only. Keys ending `_date` match with
`same_instant` only. Other keys use `submitted_value_matches`.

`change_task_delete::build_plan` writes
`PUT changes/{id}/tasks/update` with `{"action":"delete","id":"<task>"}`.
Verification is the previous inventory minus that one task. Clearing
every task stays unverified. `ticket_tasks::build_plan` is unchanged.
An omitted update status stays on `build_plan_assigned`. `TaskArgs`
has no new fields.

TUI status, notes, replies, task updates, and task deletes no longer
use the legacy sender. Ticket status reviews through
`prepare_ticket_property`. Change status reviews a live edit form.
Problem notes use `problem_workflows`. Task mutations use
`change_task_update` and `change_task_delete`. `do_write` refuses with
`Legacy TUI send path disabled`.

### CLI handoff (do not edit fsv-cli here)

`changes update` JSON should add `verified`, `fields`, and `warning`.
When the update is not verified, exit 4 with
`Change update readback unverified; do not resend` for JSON and human
output. Human output prints later steps only when verified.

`changes tasks-delete` should build `change_task_delete::build_plan`,
require `--yes` and `--expect-plan` equal to `plan.hash`, then send.
Add `--expect-plan` and `--json`. An unverified send exits 4. Do not
call the legacy `delete_task`.

HEAD scratch of `fsv-cli` against this library still fails the same 23
tests. No new name. The update-hash move is not pinned by those goldens.

Still not triaged: `922817f` and later.

## Ledger: `922817f` (library side)

Triaged against a `git archive 922817f` export
(`FSV_PY_COMMIT=922817f958cd142404f48efc2933e508beacd2bc`).
`port/golden/gen_fsv_api_closure_conversation.py` stamps
`crates/fsv-api/src/testdata/closure_conversation_oracle.json`.
Only `ambiguous_to_emails` moved: popping `to_emails` still leaves
`notify_emails`, and that alias now verifies.

Counts: **1 ported, 0 partial.** `port/UPSTREAM_BASE` is `922817f`.

| commit | date | subject | status | crates |
|---|---|---|---|---|
| `922817f` | 11-12 18:57 | Fix conversation recipient readback verification | ported | fsv-api `conversations` |

`notify_emails` and `to_emails` are one observed field. CC and BCC stay
distinct. Comparison is the set of parsed, casefolded addresses, so
order, case, display names, and duplicates still match. A missing key
on both aliases is `unavailable`. A non-list, a non-string, or an
invalid address is `mismatched`. The new Python CLI test
`test_forward_cli_with_native_to_emails_readback` calls this send.
HEAD scratch of `fsv-cli` still fails the same 23 tests. No new name.

Still not triaged: `98acb71` and later.

## Ledger: `98acb71` through `7b7f5d2` (library side)

Triaged against `git show` of each commit. None of these touch `src/fsv`
behavior. README, `docs/index.md`, and `docs/commands.md` have no
library port.

Counts: **3 ported, 1 partial.** `port/UPSTREAM_BASE` is `7b7f5d2`.
The partial is the workspace version, named below.

| commit | date | subject | status | crates |
|---|---|---|---|---|
| `98acb71` | 11-12 19:25 | Refresh README for release | ported | none |
| `0d7d929` | 11-12 19:25 | Add documentation index | ported | none |
| `365f1a4` | 11-12 19:25 | Add command cheat sheet | ported | none |
| `7b7f5d2` | 11-12 19:25 | Bump release version | partial | workspace version follow-up |

Python `pyproject.toml` moves from `2026.05.29` to `2026.11.12`.
The Rust workspace stays `2026.5.29`. `Cargo.lock` is not edited here.

### Version follow-up

Bump the workspace `version` to `2026.11.12` together with `Cargo.lock`
when that file may change. `fsv --version` will keep printing
`2026.5.29` until then.

Still not triaged: `6ead7c9` and later.

## Ledger: `6ead7c9` (library side)

Triaged against a `git archive 6ead7c9` export
(`FSV_PY_COMMIT=6ead7c9904717101a61dba9a518b674f559286b6`).
`port/golden/gen_fsv_api_attachments.py` stamps
`crates/fsv-api/src/testdata/attachments_oracle.json`. Existing cases
are unchanged. New cases: `two_step_change_acme`,
`two_step_change_globex`, and `page_change`.

Counts: **0 ported, 1 partial.** `port/UPSTREAM_BASE` is `6ead7c9`.
The partial is the change-create CLI, named below. `docs/commands.md`
and `docs/reviewed-parity.md` have no library behavior.

| commit | date | subject | status | crates |
|---|---|---|---|---|
| `6ead7c9` | 11-12 21:55 | Review initial change attachment transactions and document inline capture boundary | partial | fsv-api `attachments`. fsv-cli follow-up below |

`attachments::readback` treats a `changes` create like a ticket create.
The response envelope is `change`, and the owner row is
`GET changes/{id}`. A change create verifies from that row. Ticket,
problem, and conversation readback stay as they were.

### CLI handoff (do not edit fsv-cli here)

`changes create` should take `--max-bytes` and pass it into the
before-create attachment plan. When the draft has attachments and
`--attach-flow before-create`, build `attachments::wrap` around the
change create plan and finish it with the reviewed-write helper.
Refuse that combination when the draft also has planning, devices, or
an interactive form:
`initial attachments cannot combine with planning, devices or interactive forms`.
A before-create run that cannot prove the published byte limit exits 2
with `no upload` and sends nothing.
HEAD scratch of `fsv-cli` still fails the same 23 tests. No new name.

Still not triaged: `550fe8c` and later.

## Ledger: `550fe8c` (library side)

Triaged against `git show 550fe8c`
(`550fe8ca81282965db3a26e918b6e49e96d969cf`). No plan hash of an
existing whole-ticket forward moved. `docs/commands.md` and
`docs/reviewed-parity.md` have no library behavior.

Counts: **0 ported, 1 partial.** `port/UPSTREAM_BASE` is `550fe8c`.
The partial is the `tickets forward --conversation` flag, named below.
The committed CLI does not call the new builder, so the previous
scratch list is unchanged.

| commit | date | subject | status | crates |
|---|---|---|---|---|
| `550fe8c` | 11-12 22:02 | Add reviewed exact-conversation forwarding | partial | fsv-api `conversations`. fsv-cli follow-up below |

`forward_source` snapshots the one conversation whose id matches, and
only from a complete history. A missing, partial, or ambiguous history
is `exact forward source unavailable in complete ticket history`.
`build_plan_forward_source` puts `conversation_id` on the payload and
stores that snapshot as `record_hash`. `ConversationArgs` has no new
field. `send` recomputes the snapshot before the POST and refuses with
`forward source changed; review again` when it differs. A selector on
a reply or note is `conversation selector is only supported for forward`.

### CLI handoff (do not edit fsv-cli here)

`tickets forward` should take `--conversation` and call
`conversations::build_plan_forward_source` with that id. Dry-run,
`--yes`, and `--expect-plan` stay on the existing helper.

Still not triaged: `e116be9` and later.
