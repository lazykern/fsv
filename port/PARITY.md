# Parity ledger

Each section: Python module → Rust location, status, ported tests, golden
fixtures, known gaps. Statuses: `ported`, `partial`, `missing`.

Function-level status for every Python def lives in `port/function_map.json`
(checked by `crates/fsv-cli/tests/function_map.rs`). `matched` entries are name
matches that still need a line-by-line review; `open` entries are the parity queue.

## Divergences

Fixes of bad Python behavior, plus a few platform differences. The
end-of-port review reads this list. Wire formats (requests, plan hashes,
journal kinds, cache filenames, session bytes, draft keys) are not on it:
those stay identical.

- Files are written with LF on every platform. Python on Windows writes CRLF.
- YAML parse errors say `YAML error: …` but the line wording is not PyYAML's.
- Human terminal output matches content, order, wording and columns. Box
  glyphs, Rich number/URL highlighting and `:emoji:` substitution may differ.
- Click prompt abort prints `Aborted!`. `_err_exc` prints `Aborted.`
- No Windows Credential Manager backend. Windows defaults to `file`, as Python does.
- `keepalive()` is ported and still has no caller, as in Python. It is behind fsv-core's `keepalive` feature (`cargo test -p fsv-core --features keepalive` runs its test).
- `fsv cache clear views` deletes the cache under `CACHE_DIR/filters`.
  Python deletes `CONFIG_DIR/filters` and leaves the views cache in place.
  No test asserts the wrong directory. The command's help says it clears
  the cache.
- Commands that crash with a Python traceback (no domain on `url` and
  `changes associations`, `form explain --id` with a bad id, `completion
  show powershell`), and an API error that escapes a command (`canned show`
  on a 404, `problems columns list` with no route) exit 1 with the same
  message and no traceback. The exit code stays.
- Boolean flag pairs (`--billable/--non-billable`, `--pager/--no-pager`,
  `--watch/--unwatch`, `--private-primary/--public-primary`, ...) complete as
  two flags. Python's generated scripts put the pair in one word, which the
  shell offers as a flag that does not exist. Fish gets one line per flag.
- Help pages use clap's layout (`Usage: fsv [OPTIONS] [COMMAND]`, no Rich
  boxes). Command and option help text, defaults, choices and examples are the
  Python strings; `tests/tree.rs` checks them.
- Concurrent GETs (the parallel loads in `get`, `ls`, `tabs`) can be issued
  in a different order than Python issues them. Writes keep their order.
  `tests/differential.rs` compares GETs as a set.
- The TUI command palette (`ctrl+p`) is not ported. It is Textual's, not
  an fsv screen. Filter history still uses `ctrl+p` / `ctrl+n`.
- A failed search toasts the error. In Python the worker's uncaught
  exception exits the whole app.
- Keys typed in an open form do not reach the main TUI's key handler.
- The description preview keeps focus, so F3 toggles it off again. In
  Textual, hiding the focused editor moves focus away and F3 stops working.
- The status bar is a row above the key hints. Textual draws the hints on
  top of the status bar.
- TUI workers keep Python's staleness guards (`pending_detail_key`,
  `list_epoch`, the search sequence). An `exclusive=True` worker also
  drops an older result from the same Textual group once a newer exclusive
  worker in that group has started. The in-flight request still finishes;
  a thread cannot be cancelled. Non-exclusive `write` workers both apply.
  An exclusive inventory send invalidates earlier `write` results. There
  is no change-emit lock.
- `detail_panel` does not wrap a long planning line at 60 columns. Rich
  wraps it. The Rust block leaves wrapping to the console.
- A plain YAML bool or null key (`true:`, `FALSE:`, `null:`, `~:`) loads
  as Python's `str(k)` (`"True"`, `"False"`, `"None"`); a quoted `"true":`
  stays `"true"`. Every Python caller (`draft_from_data` answers, planning
  labels and unknown-key errors, `matching_hints`) applies `str()`, so draft
  keys and errors match (`fsv_core::yaml` `typed_keys_read_as_python_str`,
  `fsv_flows::drafts` `plain_bool_and_null_keys_are_python_str`). Left:
  a hints `when` key that is no field reports `hints: unknown field 'True'`;
  Python's `{k!r}` prints `True`. Nested mappings are string-keyed too
  (Python keeps `False` inside a value dict); no Python caller dumps them.
- Change create/update (`fsv_flows::change::execute`) journals `failed`
  only when the server definitely refused the change body: HTTP 4xx other
  than 403, 408 and 429, the rule incident create, SR place-request and the
  reviewed writes use. 5xx, 403, 408, 429, session and transport errors
  leave `started`, so `check_duplicate` refuses a resend (exit 4, "no
  recorded outcome") until `--allow-duplicate`. A failed upload before
  create still journals `failed`: no change was posted. Python has the bug
  (`change_flow.py:637-640` marks every `APIError` / `RuntimeError` /
  `SessionError` failed), so a 502/504 after the server created the change
  lets the next run create a duplicate. Flows and fsv-api share
  `fsv_api::service::definite_rejection`.
- Change execute refuses with exit 5, "could not persist the change journal;
  nothing was sent", when the `started` event fails to write or does not
  read back as the newest row. Python sends anyway. The one write journal,
  `fsv_api::service::Journal`, has `try_record_event` (default: record, then
  `Ok`); `DiskJournal` returns the append error from
  `fsv_core::journal::try_record_plan`. The journal bytes do not change.
  `Journal::recent_plan` defaults to `fsv_core::journal::recent_plan_rows`
  over `read()` with Python's `hours` cutoff (a `started` counts only with no
  later outcome).
- Read-only notes in a change plan follow the form's visible-field order.
  Python iterates a set, so the order is not stable.
- `create.clone_planning_fields` treats a planning row with
  `"attachments": null` as no attachments. Python raises `TypeError`.
- A schema cache whose `saved_at` is NaN or Infinity is treated as missing
  and refetched before returning. Python reads it and treats it as stale.
  `serde_json` cannot parse those values.
- `resolve_view_name` skips non-object view rows. Python raises
  `AttributeError`. `schema::fetch` treats a non-list, non-empty fields
  payload as empty.
- `fsv_forms::FormSpec::from_form_fields` returns Python's exception text
  for malformed fields. A string, list or dict `position` fails at
  construction with Python's later `unsupported operand type(s) for +`
  text. Python builds the spec and fails on first use. A numeric
  `lookup_link` becomes a string; a falsy non-string becomes no link.
- The stale-schema warning goes to stderr with `eprintln!` on the CLI, as in
  Python. The TUI loads with `fsv_api::schema::load_in_quiet`, so the line
  never draws over the screen (Python's TUI captures stderr instead).
- The form's F3 description preview shows only the top rows of long text.
  Textual's 14-row preview scrolls.
- Form submit sends the reviewed plan, as Python's `FormSession` does. The
  builder's plan (`change::Plan` or `catalog::SRPlan`) rides in
  `fsv_formui::Plan::detail`; save and send downcast it. Send then runs the
  duplicate check and the metadata freshen, as in Python: an unchanged
  fingerprint keeps the plan, a changed one rebuilds and refuses on a diff.
  A plan without that detail (not built by this form) is refused with exit
  2, "the plan was not built by this form; review it again". Python cannot
  reach that case.
- `ClientResolver.lookup` custom lookups (Python ccc3a64) are in
  `fsv_api::formmeta::ClientResolver` (`fsv_flows::meta` re-exports it).
  A `custom_lookup` / `custom_multi_lookup` field with a link always asks
  `lookup_choices(link, query)` first, before the user, agent or group
  autocomplete, and skips rows with no id or a truthy `deleted`. Outputs and
  error texts match a Python run. No divergence.
- A digit that `str.isdigit` accepts but `int()` rejects (`²`): Python's
  `formmeta._ws_id` and `task_status.active_choices` raise
  `invalid literal for int()`. Rust skips that workspace id, and
  `active_choices` fails with `invalid active task-status identity`. Other
  Unicode decimal digits (`١٣`) parse as in Python.

## fsv-app

| Python | Rust | status |
|---|---|---|
| live wiring behind `views` reads, `formmeta.load_meta`, `create` writes, form session plan/save/send, TUI backend and pages | `fsv_app::{source, meta, writer, form, tui}` | partial. `cargo test -p fsv-app --offline` was 101 passed. Request paths, bodies and plan hashes captured from Python. `run_tui` and `run_form` need a terminal and have no tests. |
| `tui.app._prepare_ticket_conversation` / `_send_ticket_conversation`, `_load_ticket_property_fields`, `_prepare_ticket_property` / `_send_ticket_property`, `_prepare_known_error` / `_send_known_error`, `_load_planning_sections` / `_load_planning_editor` / `_prepare_planning_text` / `_send_planning_text`, `_load_asset_candidates` | `LiveBackend::{ticket_conversation_plan, send_ticket_conversation, ticket_property_fields, ticket_property_plan, send_ticket_property, known_error_plan, send_known_error, planning_definitions, planning_row, planning_text_plan, send_planning_text, asset_candidates}` | ported. Golden: `port/golden/gen_fsv_app_tui_reviewed.py` → `src/tui/testdata/reviewed_expected.json` (reply with CC + Pending, BCC + sender reply, private note with notify, sender/BCC refusals and accepted plans, the property picker filter, answers plans and sends for subject / priority / custom text / custom dropdown / beneficiary plus the status and planned-date refusals, a tag-plan send, known-error mark, problem and change planning text, one asset search page). Plans, reviews (warnings included), hashes, request lines, bodies and results match. Planning text plans carry `before` in `ReviewPlan::raw` and rebuild a `PlanningTextPlan` (Python `92002d7`); a section with stored text plans a PUT edit (`changes_edit`, Python `e638d85`). |
| `change_cli._send`, `cli_sr._send`, `ticket_flow.send`, `_report_partial` for a form-reviewed plan; `tui.actions.failure_detail` attributes | `Submitted::send_cli`, `send_checked`, `send_confirmed`, `SendError::{report, outcome}`, `tui::forms::send_submitted` | ported. Golden: `port/golden/gen_fsv_app_send.py` (metadata changed after review: diff lines, exit 4 without a terminal, re-review and confirm in one, exit 2 when the new form needs answers; partial failure lines and exit 5; SR equivalents). |

- Form-picked lookups reach the flows' JSON drafts as `Value::lookup_answer` objects, which `Value::from_answer` reads back as the `LookupValue`, so review rows, saved drafts and kept hidden/unknown lines show names as Python does. Golden: `port/golden/gen_fsv_app_form_lookup.py` → `src/form/testdata/form_lookup.json`.
- Service-request drafts save through `fsv_flows::sr::save_sr_draft` from the reviewed plan, with no separate requested-for. The file bytes match Python's `render_sr_template` for the same plan.
- `run_tui` takes a schema loader argument because `schema::load` and saved-view loading live in `fsv-cli`, which `fsv-app` cannot depend on.
- Directory lookups that hit an expired session surface as a query error, because `fsv_api::QueryError` does not carry the session flag.
- `FormSession.send` (TUI, `FormBackend::send`) still refuses a changed plan with exit 4, as Python does. The CLI re-confirm lives in `Submitted::send_cli`, which fsv-cli's form paths (`changes create/update`, `sr create`, `tickets create`) call.
- A service-request form send goes through `catalog::execute_detailed`. A failure is `SendError::Sr(SendFailure)`: the CLI prints `uploaded attachment ids (not attached to any request): ...` when files went up, then exits with the cause's code and `{step} failed: ...`; the TUI failure detail has Python's `uploaded` / `error` lines (golden: `gen_fsv_app_send.py` `sr_*` failure scenarios). `fsv-cli`'s flow-path `sr_send` does not use it yet.
- In tests, the upload progress line (`  uploading a.txt... ok (5 B)`) skips `term::capture` because it goes straight to stderr. Its text matches Python.
- Reviewed conversation plans stay in `LiveBackend` by hash, because `ReviewPlan.raw` is only the payload (the TUI reads `private` from it). A send for a hash this backend did not plan is refused: "the conversation plan was not built by this session; review it again". Python's TUI cannot reach that case.
- Ticket reply/note Sender and BCC go through `conversations::build_plan_mail` with `MailArgs`; the old local refusal is gone. Python's refusal texts and order come from fsv-api.
- Ticket property answers go through `properties::build_plan_with` / `send_with` with a `ClientResolver` for the beneficiary. `ReviewPlan.raw` carries the `AnsweredPropertyPlan` fields (`answers_json`, `metadata_hash` included). Flag-built plans in `raw` without `answers_json` send as `{}` answers, so their hash stays the `PropertyPlan` one.
- Known-error and planning-text plans keep the `WritePlan` fields in `ReviewPlan.raw` (`kind: "reviewed_write"`). A send whose `raw` is not that kind, or whose action belongs to the other hook, is refused with "not a plan for this action"; Python's TUI cannot reach that case.
- `LiveBackend::with_change_schema` swaps the change-schema loader for planning definitions. It defaults to `planning_text::live_change_schema` (a forced fetch, as `schema.load(CHANGES, force=True)`).

## fsv-api (started)

| Python | Rust | status |
|---|---|---|
| `query.py` filter → `query_hash`, date merge, one-shot percent-decode, `_resolve_group` | `fsv_api::query` (`resolve_group` over `Directory::agent_groups`) | ported. `src/query_matrix_tests.rs` mirrors `test_query_matrix.py` (Python `92d76ff`) against `port/golden/gen_fsv_api_query_flow_matrix.py`; a malformed raw hash keeps Python's `invalid query-hash JSON: ` prefix with serde_json's wording after it (as `--query-hash` below). Schema loading is the caller's. `query._resolve_group` has no Python caller; `cli._resolve_group` (fsv-cli `lookup::resolve_group`) is the one commands use. |
| `state_flow.py` reads and renderers | `fsv_api::state_flow` (`Printed`: the `console.print` arguments) | ported. `src/state_flow_matrix_tests.rs` mirrors `test_state_flow_render.py` (Python `92d76ff`): exact markup strings and table cells recorded from a Python console, and each read. |
| `task_status.py` | `fsv_api::task_status` | ported (tenant ids, fail-closed choices, no default id). `change_tasks.build_plan` still to come. |
| `schema.py` choice labels, priority/impact/risk, v2 merge | `fsv_api::schema` | ported for the pure helpers. Cache TTL constant is present; the on-disk schema cache still waits on the client. |
| `views.py` dates, html, span, relative | `fsv_api::site::format` | ported. `span(23h 59.67m)` is `24h` (Python does not carry that into `1d`). List items keep the blank line `</li>` inserts. |
| `views.py` saved views, columns, cells, headers, tabs | `fsv_api::site` list | ported, no HTTP. `find_view` / `list_meta` / later-page headers match the Python tests. |
| `views.py` lifecycle, planning bind, flat properties | `fsv_api::site` detail | ported as pure JSON in, struct out. `match_planning_row` keeps the Python error sentences. |
| `views.py` record bits (source, approval, conversations, late badge) | `fsv_api::site::record` | ported. |
| `views.collect_pages` | `fsv_api::site::collect_pages` | ported as a walker the caller feeds. Page-limit, short-page `count`, and foreign `next_page_url` match `test_read_completeness.py`. Incomplete meta inserts `continuation` before `partial`, matching Python's update-then-assign order. |
| coverage-audit helpers: `query` private helpers, `formmeta._add` / `_cookie_workspace` / `ClientResolver._bootstrap`, `schema._cached_path` / `_fetch_schema`, `task_status.active_choices`, the three `_id`s, plan `payload`s, `views` list/format/detail/record helpers | same modules | ported. `cargo test -p fsv-api --offline` was 222 passed. Golden: `port/golden/gen_fsv_api_coverage.py` → `src/testdata/coverage_oracle.json`. |
| `readback.py` | `fsv_api::readback` | ported (`test_readback_normalization` duration/timestamp cases). |
| `reviewed_write.py` | `fsv_api::reviewed_write` | ported (`WritePlan`, `execute`, `refused`). |
| `watchers.py`, `problem_known_error.py`, `planning_text.py`, `ticket_time.py`, `bulk_priority.py` | same-stem modules | ported, no CLI. Hashes and reviews checked against `port/golden/gen_fsv_api_reviewed.py` → `src/testdata/reviewed_oracle.json`. |
| `ticket_properties.build_plan(answers=…)`, beneficiary, rule-context reset | `properties::build_plan_with` / `send_with` → `AnsweredPropertyPlan` | ported. `PropertyPlan` (no answers) keeps its fields and hash. The review `after` keeps the payload's key order without `br_validation_excludes` (pinned as `after_keys` from Python); serde_json map removals use `shift_remove` throughout, since `remove` is `swap_remove` under `preserve_order`. |
| `ticket_notes._EMAIL` / `_valid_recipient` (also `change_approvals._EMAIL`, the `catalog`, `ticket_flow` and `cli_sr` plain-email checks) | `fsv_api::recipients::{EMAIL, valid_recipient, plain_email}` | ported once; notes, conversations, approvals, catalog and `fsv_flows::ticket` call it. `fsv-cli/src/cmd/sr.rs` still has a local `plain_email`. |
| `ticket_conversation` bcc / sender / forward, CC re-check | `conversations::build_plan_mail` + `MailArgs` | ported; `build_plan` is the no-BCC/no-sender wrapper. |
| `catalog.SendFailure` + `cli_sr.place` conversion | `catalog::SendFailure`, `Executed`, `execute_detailed` | ported; `execute` still flattens to `Error::other` with the same text. |
| `change_approvals._run` echo before an error | `approvals::run_action_detailed` → `RunActionError { echo, error }` | ported; `run_action` unchanged. |
| `cli._merge_same_conditions` | `fsv_api::merge_same_conditions` | ported (the fsv-cli copy in `resource.rs` can switch to it). |
| `Client.me`, `Client.v2_get` | `Api::me`, `Api::v2_get` | ported for `fsv_core::Client` and fsv-cli's `LazyApi`. The fsv-flows and fsv-views test fakes keep the erroring default: none of their tests reach `me` / `v2_get`. |
| `asset_picker.py` | `fsv_api::asset_picker::{candidates, select, Candidates::to_json}` | ported at `36408b2`. `candidates` searches `GET https://{domain}/itam/core/device/api/` (`integrated_assets.search`) and each row carries `inventory`. `search_assets_for_change` remains for other create paths. |
| `canned.py` (folders, responses, show, `content` JSON pointer, `reply_plan`, `freshen`, scenarios inventory) and the canned fields on `ConversationPlan` (hashed only when set) | `fsv_api::canned`, `conversations::ConversationPlan::{canned_id, canned_pointer, canned_hash}` | ported, no CLI. `conversations::send` re-reads the canned source before sending. |
| `scenarios.py` | `fsv_api::scenarios::{build_plan, send}` | ported, no CLI. One PUT `tickets/execute_scenario`; `verified` is always false. |
| `private_views.py` | `fsv_api::private_views::{build_plan, send, ViewPlan}` | ported, no CLI. Delete is verified by a 404 and a complete re-list. |
| `list_preferences.py` | `fsv_api::list_preferences::{headers, columns_plan, export_plan, export_source, send, ListPlan}` | ported, no CLI. |
| `problem_workflows.py`, `formmeta` `problem` module | `fsv_api::problem_workflows::{create_plan, child_plan, send, ProblemPlan}`, `formmeta::MODULES` | ported, no CLI. The create verify uses `readback::submitted_value_matches` for `custom_fields` and `same_text` for `description` (Python `44e3e83`). |
| `readback.same_text`, `submitted_value_matches` (`Decimal` text, underscores stripped) | `fsv_api::readback` | ported, plus `py_equal` / `strict_equal` for Python `==` and `type(a) is type(b) and a == b`. |
| `planning_text.PlanningTextPlan`, `editable_content`, `section_content`, `build_plan(…, clear=)` (Python `10c3281`, `92002d7`, `e638d85`) | `fsv_api::planning_text::{build_plan, build_plan_for}` | ported. An empty section is a create (`POST …/planning-fields?id=KEY`), stored text an edit (`PUT …/planning-fields/KEY`), `clear` a bodyless `DELETE …/planning-fields/KEY`; the review's `operation` says which. Contradictory aliases, an empty row with an id, and clearing an empty section are refused. The hash binds the `before` text. `reviewed_write::execute_as` runs a plan whose hash binds more than the wire. |
| `ticket_note_mutation.py` (Python `9910ddc`) | `fsv_api::ticket_note_mutation::{build_plan, review, send}` | ported, no CLI. Edit is one PUT `conversations/{id}`, delete a bodyless DELETE; both bind the exact private note. Golden: `gen_fsv_api_ticket_note_mutation.py`. |
| `problem_incidents.py` (Python `17ada1b`) | `fsv_api::problem_incidents::{build_plan, send}` | ported, no CLI. Link is PUT `problems/{p}/tickets/associate`, unlink DELETE `problems/{p}/tickets/{t}`, verified by the complete association inventory. Golden: `gen_fsv_api_problem_incidents.py`. |
| `ticket_tasks.py` (Python `c46d3b0`, `6e598b2`, `39e4853`) | `fsv_api::ticket_tasks::{build_plan, send, inventory, fields_from, custom_values, required_for_create, reminder, TaskPlan, TaskArgs}` | ported, no CLI. `notify_before` is sent only when the metadata defines it; `cf_` keys of native fields are sent unprefixed; `workspace_id` comes from the parent ticket. Golden: `gen_fsv_api_ticket_tasks.py`. |
| `change_task_update.py` (Python `be41032`) | `fsv_api::change_task_update::{inventory, reminder, build_plan, send, TaskUpdatePlan}` | ported, no CLI. PUT sends only supplied fields. `--owner none` / `--group none` clear the id. Reminder seconds come from runtime `notify_before` choices. Completed status checks `required_for_closure`. Golden: `gen_fsv_api_change_task_update.py`. |
| `change_tasks.py` required fields (Python `780c002`) | `fsv_api::tasks::{validate_required, runtime_fields, build_plan_ex}` | ported, no CLI. `required_for_create` is checked before POST. Custom values ride as `custom_fields`. Metadata is in the hash only when fields were supplied. Golden: `gen_fsv_api_change_tasks_required.py`. |
| `problem_status.py`, `reviewed_write.execute(..., rejection=)` (Python `5d7d777`, `0719e33`) | `fsv_api::problem_status::{build_plan, send, closure_context, rejection, StatusPlan}`, `reviewed_write::execute_rejecting` | ported, no CLI. Closure counts a nondeleted workaround (`article_type` 2) or permanent (`1`) link. Golden: `gen_fsv_api_problem_status.py`. |
| `solutions.py` (Python `0719e33`, integer `folder_id` at `ad40a51`) | `fsv_api::solutions::{metadata, folder, article, create_plan, send_create, link_plan, send_link, SolutionPlan}` | ported, no CLI. Create is a draft `POST solutions/articles`. `folder_id` is a JSON integer. Link and unlink are `PUT problems/{id}/solutions/{article}`. Golden: `gen_fsv_api_solutions.py`. |
| `approval_recipients.py` (Python `d870fa7`, CAB `approval_agents` at `e173437`) | `fsv_api::approval_recipients::{members, resolve, validate}` plus `approvals::build_plan_with` | ported, no CLI. Group and CAB selections expand to eligible users. CAB reads prefer `approval_agents` and refuse a conflicting `approval_users` list. Empty context keeps the old request hash. Golden: `gen_fsv_api_approval_recipients.py`. |
| `attachments.py` (Python `1c186e0`, `ba6aef9`) | `fsv_api::attachments` (`size_limit`, `page_limit`, `resolve_limit`, `local_files`, `wrap`, `conversation_plan`, `problem_note_plan`, `send`, `readback`, `deletion_owner`, `delete_plan`, `delete`) | ported, no CLI. Uploads are one `POST attachments` per file, then one owning write; a failure in between is the "do not replay" refusal with the uploaded IDs. The page limit is read with the new `Api::page` (`GET /a/dashboard`, no redirects). Golden: `gen_fsv_api_attachments.py`. |

The five workflow modules and the planning changes are checked against `port/golden/gen_fsv_api_workflows.py` → `src/testdata/workflows_oracle.json`: request logs, plans, results, error texts and journal events (`src/workflows_tests.rs`). API error texts carry Python's request context there, so those compare by HTTP status. `tests/legacy_write_guards.rs` drives a real `fsv_core::Client` over `MockTransport` through the seven legacy callers (Python `dbe553c`): one mutation on 403/429/500/disconnect with a failed audit row, none when the CSRF fetch fails. No fsv-core gap.

Fixed to match Python while adding those tests: `_parse_where` needs a one-character field (`>=x` is field `>`, op `=`) and rejects an embedded newline. Field, choice and lookup matching use `casefold` and Python `or` truthiness (an id `0` or a `null` label no longer matches `"0"` / `"None"`). `_resolve_requester` falls back to the id for a blank `value`. `active_choices` takes the first truthy `value`/`label`/`name` and casefolds labels. `_file_names` gives `str(id)` when every name is falsy (`{"id": 0}` → `"0"`).

Gaps:
- `Api::me` / `Api::v2_get` have erroring defaults. fsv-cli `LazyApi`, fsv-flows' testing `Api` and the fsv-views golden fake should forward them; until then watchers / ticket time through those impls fail with "signed-in agent identity is unavailable".
- `cli._resolve_where` (labels, aliases, `Me`) is an fsv-cli function; `query.resolve_where`'s "field not found: 'x'" already matches `query.py`.
- `1dc7ab6`'s exact-bytes/headers HTTP tests have no equivalent; fsv-api tests run on `FakeApi` / `FnApi`, which see the method, path and body but not headers. `tests/legacy_write_guards.rs` is the exception: it sees headers.
- `create` planning-field resolution and `--duplicate` errors are `Error::value` (Python `UsageError`, `820af12`): the bare message, exit 2. Other `APIError(0, …)` sites keep `ApiError` status 0, which `fsv_core::ApiError` renders as Python does: `Error: …` (`Error <what>: …` with a request).

Divergences:
- `readback::duration_seconds` accepts Unicode decimal digits in the hours and in the second digit of minutes and seconds, matching Python's `\d`. The first minute and second digit stays ASCII `[0-5]`, so `१२:३०` is refused and `१२:05` is 12 hours. Hours that overflow `i64` are an error rather than a big int. `ticket_time::duration`, `watchers::positive_id`, catalog item ids, CLI `int()` of a digit string, change-asset ids (`raw.isdigit()`), `forms.workspace_id`, merge conversation ids, and the TUI asset picker take Unicode decimal digits as Python does (`"१२"` is 12). A superscript digit is not a decimal id. An id past `i64` is refused where Python returns the big int. A task group or owner id typed in the TUI uses `str.isdecimal()` the same way.
- `conversations::parseaddr` uses `email_addr::parseaddr`, a port of Python 3.12's strict `email.utils.parseaddr` (`AddrlistClass`, parenthesis and domain-literal checks, one address only). Golden: `port/golden/gen_fsv_api_parseaddr.py` → `src/testdata/parseaddr_oracle.json` (76 inputs: quoted local parts, comments, several `<>`, groups, routes, malformed values).
- `attachments::LocalFile` content types come from `mime_guess` (as `create::upload_file`), not Python's platform `mimetypes` table. The golden covers `.txt`, `.csv`, `.png`, `.json` and no extension, which agree.
- `attachments::page_limit` refuses a published limit whose byte value does not fit an `i64` (over about 8.8e12 MiB) as "ambiguous or invalid"; Python accepts the big int.
- `Api::page` defaults to an error. `fsv_core::Client`, fsv-app `ClientSource` and fsv-views `AssociationReader` implement it; fsv-cli `LazyApi` must forward it, or every attached send without a metadata byte limit is refused with "could not read the tenant's published attachment size limit".
- `merge_same_conditions` turns a truthy non-iterable `value` (a number or `true`) into a one-item list; Python raises `TypeError` there.
- `SendFailure::into_error` names the Python type of the cause from the `fsv_core::Error` variant (`Other`/`Context` → `RuntimeError`, `Io` → `OSError`, `Exit` → `FlowExit`), and takes the exit code from `Error::exit_code`, except that an `Exit` cause gives 1, as Python's `exit_code_for` does for a `FlowExit` (golden `send_failure` in `reviewed_oracle.json`).
## fsv-views

| Python | Rust | status |
|---|---|---|
| `pages.py`, `render.py` | `fsv_views::pages`, `fsv_views::render` | ported. `cargo test -p fsv-views --offline` was 99 passed. Expected strings came from running the Python functions. |
| `views.py` loaders (`load_tab`, `load_detail`, conversations, requested items) | `fsv_views::detail`, `fsv_views::source` | ported, covered from `coverage`. Reads go through `&dyn fsv_api::service::Api` (`fsv_app::ClientSource` implements it over the shared client); `FetchError::from(fsv_core::Error)` is `optional_get`'s `except` ladder. `load_detail` / `load_tab` take the `Source` adapter (any `Api`) until fsv-cli passes `&src`. |
| `views.ticket_headers` | `fsv_views::source::ticket_headers` | ported. `fsv-app`'s `LiveBackend::ticket_headers` calls it. |
| `views.load_planning(module=…)`: problem analysis sections in `get` output (Python `ed6e644`) | `fsv_views::detail::load_planning(src, record, schema, module)`, `fsv_api::site::{planning_rows, change_planning_defs}` | ported. Rows that are not a list of objects give `planning rows unavailable`; text comes from `planning_text::section_content`; files from `attachments`, else `files`. Expected values from running Python. |
| `pages.main_lines` / `conversation_lines` after `7f58023` | `fsv_views::pages::{main_lines, conversation_lines}` | ported. `7f58023` reverts `ec7a6ef`: description, planning text and conversation body render in full, and the 📎 rows render as well. An empty conversation body is `(empty)` even when files are attached. Golden: `port/golden/gen_fsv_views_attachment_rendering.py` → `tests/fixtures/attachment_rendering.json`. |
| `render._requested_item_field_defs` | `render::field_defs` | ported. |
| `render.strip_html` | `fsv_core::output::strip_html` | ported in fsv-core. |

Golden values for the last three come from `port/golden/gen_fsv_views_extra.py`.

Known test limits, not behavior changes: relative times use the real clock, so tests check placement rather than a pinned "2 days ago". `_LazyModule` and `_NullConsole` are import-time shims with no Rust counterpart. `print_blocks` and `print_detail` write to the terminal; tests cover `blocks_text` and `detail_blocks`.

## fsv-completion

| Python | Rust | status |
|---|---|---|
| `_complete._completions`, `main` | `dynamic::complete`, `run` | ported. `cargo test -p fsv-completion --offline` was 40 passed. |
| `completion` completers, including `complete_format`, `complete_store`, `complete_shell`, `complete_sort_order`, `complete_config_key`, `complete_config_value`, `complete_cache_target`, `complete_search_sort`, `complete_duplicate_mode` | one Rust fn each in `dynamic` | ported. |
| `completion.complete_update_choice`, `complete_help_topic` | `fsv_completion::{complete_update_choice, complete_help_topic}` | ported. `complete_update_choice` is dispatched by `_complete` (see Divergences). `complete_help_topic` has no arm in `_complete`, in Python or Rust. |
| `completion._config_load`, `_completion_network` | `fsv_completion::{config_load, completion_network}` | ported with the directory and env value passed in. `fsv-app`'s TUI backend and `fsv-cli` (`cmd::completion::network_enabled`) call it; fsv-cli's own copy, which treated `network: 1.0` as off, is gone. |
| `completion_gen` walk and emitters | `model`, `emit` | ported. `_extract_params`, `_filter_params_by_flags`, `_is_bool_annotation` (clap action plus `value_flag`), `_collect_bash`/`_collect_zsh` (one `collect`) have their own fns. |

Not ported, with reasons:
- `completion._schema_path` and `completion_gen._bash_escape` are dead code; nothing calls them.
- `completion_gen._completer_name` and `_resource_from_closure` read Python closures. The `OptionHint` table in `fsv-cli` records the completer and resource.
- `_complete._MockCtx` only carries `--kind`. Rust passes the value to `complete_lookup_query`.
- `completion_gen._Command.__init__` is the `Node` struct literal.
- `cli._field_matches` and `query._field_matches` are substring matches, not this crate's prefix match. They live in `fsv-cli` (`lookup::field_matches`) and `fsv-api`.

Divergences:
- `completion_network` returns off when `completion` is not an object (null, a string) or `config.json` is not an object. Python raises `AttributeError`. `_complete` swallows it, but `completion doctor` and the TUI crash.
- `complete_update_choice` skips choices that are not objects. Python raises `AttributeError`.
- `--status <TAB>` and `--priority <TAB>` on `update` and `create` complete. Python's scripts call `_complete … complete_update_choice <resource> <token>`, but `_complete` has no arm for that completer, so Python prints nothing. Rust scripts append `-- <field>` (`status` or `priority`, the option id) and `_complete` answers with `complete_update_choice(resource, field)`. Without a field it still prints nothing. Only these script lines differ from Python's: `fsv-cli/tests/fixtures/completion.{bash,zsh,fish}` need the `-- status` / `-- priority` suffix.

Golden values: `port/golden/gen_fsv_completion_extra.py` writes `tests/fixtures/extra_expected.json`.

## fsv-core

| Python | Rust | status |
|---|---|---|
| `session._b64e`, `_b64d` | `session::{b64e, b64d}` | ported. `b64d` follows CPython's non-strict decoder: it skips non-alphabet characters, stops at completing padding, and gives the same error text. |
| `session._passphrase`, `_argon_encrypt` | `session::{passphrase, argon_encrypt}` | ported. The prompt is injected; the binary passes `rpassword`. |
| `session._read_secret` | `paste::read_secret` (`collect_secret`, `read_piped`) | ported. Key-loop expectations come from Python reading a pty. |
| `session._http_json`, `_page_ws_url`, `_cdp_call`, `_direct_browser_cookies` | `browser` | ported. Tested against a local `TcpListener` DevTools fake running the same scenarios Python ran: ping, event and foreign-id frames, error fallback to `getAllCookies`, a non-object result, and close. |
| `journal._command`, `errors.APIError.__init__` | `journal::command`, `ApiError::{new, with_request}` | ported. |
| Rich `Console.print` soft wrap (`render.console` / `render.err`) | `term::Console::{print, print_plain, print_soft}` | ported. `print` and `print_plain` word-wrap at the console width (`$COLUMNS`, else the terminal, else 80) with Rich's default justify and `fold` overflow: no padding, trailing spaces kept unless past the width, long words folded, tabs expanded to 8-cell stops. `print_soft` is `soft_wrap=True` (only `cli_sr` "Saved to …" uses it). `render` stays unwrapped for `fsv-views`. 204 cases (widths 20/60/80, stdout/stderr, color on/off) match Rich 15.0.0 byte for byte. |
| `safety.write_risks` | `safety::write_risks` | ported from Python 273ffde: every status change on a non-change resource is a risk, `<what> status -> <label> (status semantics and workflows require confirmation)`. `status_closes` is kept (Python keeps it too) but no longer decides. |
| httpx query encoding (`Client._request(params=…)`) | `client::wire_url` | ported. Byte-identical to `httpx.Client().build_request(...).url` (httpx 0.28.1): `urlencode` with `quote_plus`: space is `+`, and everything except letters, digits and `_.-~` is percent-encoded (`@` → `%40`, `,` → `%2C`, `/` → `%2F`). `true`/`false` lowercase, `None` sent as `key=` (the old Rust code skipped it), list values repeated. Any `params` map, even an empty one, replaces the URL's own query, as httpx does; no fsv caller passes both. The `--verbose` log target keeps Python's own `_query_string` encoding. |
| `client._int_request` 403 replay | `Client::int_request` | already matched 273ffde: CSRF is fetched first and a write is never replayed. |
| `formmeta._cookie_workspace` jar walk (`client._client.cookies.jar`) | `Client::cookies`, `CookieJar::ordered` | ported. Iterates like `http.cookiejar`: domain bucket, then path bucket, then cookie, each in first-creation order; a replaced cookie keeps its slot; `get`, `header_for` and the `fw-session-id` pick follow the same order. Divergence: a tenant cookie that arrives both with and without `Domain` is one cookie in Rust (tenant cookies are host-only), two in Python. |
| `cargo test -p fsv-core --offline` | | 175 passed. |

Not ported, with reasons:
- `session._ws_read_exact`, `_ws_send_text`, `_ws_recv_text`: `tungstenite` does the websocket framing. Their behavior is covered through `cdp_call`.
- `client.Client.close`: the reqwest pool closes when the last `Arc<Client>` drops. Python calls `close()` in `finally` blocks, and Rust scopes do the same job.
- `session.login_interactive` is a one-line wrapper that reads the real stdin. `login_interactive_with` is tested.

Divergences:
- A bad base64 field in an argon session file is a session error, `invalid encrypted session file` (exit 3). Python raises `binascii.Error`, a `ValueError` (exit 2).
- Ctrl-C at the hidden cookie prompt is read as a byte and gives `Error::Aborted`. Python leaves ISIG on and gets `KeyboardInterrupt`.
- The CDP client answers server pings with a pong and checks `Sec-WebSocket-Accept`. Python ignores pings and only checks for `101` in the status line.

Golden values: `port/golden/gen_fsv_core_session.py` writes `tests/fixtures/session_expected.json`. `strip_html.json` comes from `gen_fsv_views_extra.py`. `port/golden/gen_fsv_core_term_wrap.py` writes `tests/fixtures/term_wrap.json`. The httpx URLs in `client::tests::wire_url_matches_httpx` and the `write_risks` texts in `safety::tests` were printed by the Python worktree.

Known gaps: Rich emits one SGR run per markup span; Rust merges adjacent runs with the same style, so colored bytes can differ when two spans share a style. Plain text is identical. With `TERM=dumb` on a real terminal Rich ignores `$COLUMNS` unless `$LINES` is also set; Rust always honors `$COLUMNS`.

## fsv-forms

| Python | Rust | status |
|---|---|---|
| `forms/*` | `fsv_forms` | ported. `cargo test -p fsv-forms --offline` was 107 passed. |
| `errors.*Error.__init__` | `FormError` variants; message fns `unknown_field_message`, `ambiguous_message`, `payload_message`; `FormError::parse_opts` | ported. Messages are pinned by `errors/py_tests.rs`. `FormRuleError` has no fn of its own because the variant stores the message as given. |
| `rules.RulesEngine._match` | `RulesEngine::match_condition` | ported (`match_group_and_match_condition_match_python`). |
| `state._offset`, `tz._off` | `DateTimeValue::offset_text`, `pydt::offset_text` | ported (`off_and_offset_match_python`). |
| `state.LookupResolver.lookup` | `LookupResolver::lookup` | trait. `lookup_resolver_calls_match_python` pins the entity, query and field passed for each lookup kind, plus the outcome (`port/golden/gen_fsv_forms_resolver.py`). |

No divergences from this pass.

## fsv-cli

| Python | Rust | status |
|---|---|---|
| `cli.py` command tree at Python `92002d7` (152 nodes: root, 22 groups, 129 leaves) | `fsv_cli::generated`, `app` | ported. `tests/tree.rs` checks every node's name, hidden flag, options, positionals and help text against a Python dump. `tests/golden/not_ported.py` lists Python paths the dumpers (`dump_cli_tree.py`, `dump_completion.py`) skip; it is empty. |
| `watchers_cli`, `ticket_time_cli`, `planning_cli` (`planning-text`, `mark-known-error`), `bulk_priority_cli`, `tickets forward`, `tickets reply --bcc/--sender`, `<res> update --reviewed` with `--set` answers, `ReviewOutput` (`--output/-o json`, `--json`) | `cmd::reviewed`, `cmd::tickets::forward`/`reply_reviewed`, `cmd::resource::ticket_properties`, `types::ReviewOutput` | ported. `tests/upstream_cmds.rs` goldens emitted from Python `92002d7` (`tests/golden/cmp.py --emit`, fixtures `watchers`, `ticket_time`, `planning`, `ticket_mail`, `ticket_reviewed`). `planning-text` catches `APIError`/`SessionError` (`str(exc)`, `exit_code_for`) and refuses an existing section on create, as `10c3281`…`92002d7` do. |
| `change_cli` `asset-picker` (`53c678e`, wire moved at `36408b2`) | `cmd::reviewed::asset_picker` over `fsv_api::asset_picker` | library ported. CLI goldens still stub `GET assets-to-associate` and the old row shape, so those tests fail against this library until the CLI owner retargets the fixture. An `APIError` stays uncaught (exit 1), as in Python. |
| `canned_cli` (`tickets canned folders/list/show/reply`, `tickets scenarios list/show/apply`, `07c85d0`), `private_view_cli` (`tickets private-views show/write`, `2a03036`), `list_preferences_cli` (`<res> columns list/set`, `<res> export`, `6b7073a`), `problem_cli` (`problems create`, `write-child`, `6797301`) | `cmd::native` over `fsv_api::{canned, scenarios, private_views, list_preferences, problem_workflows}` | ported. Goldens on fixtures `canned`, `private_views`, `list_prefs` (all three modules), `problem_flows`: dry runs, sends with readback (verified and unverified), the review gate and every refusal. The `show`/`list` reads have no `try` in Python, so their failures (including fsv-api refusals, which carry exit 2) exit 1. |
| `form dump/fields/explain` on `problems` (`6797301`) | `cmd::form::module` over `formmeta::module_name` | ported; goldens on `problem_flows`, plus `form_dump_problems` (snapshot minus `taken_at`). |
| `cli_sr._send` failure report (`{step} failed: …`, `uploaded attachment ids (not attached to any request): …`) | `cmd::sr::sr_send` over `fsv_api::catalog::execute_detailed` | ported; `upstream_cmds.rs` `sr_send_failure_lists_uploads` (place-request 500 after two uploads, exit 5). |
| `cli.py` `help`, `search`, `auth`, `cache`, `journal`, `config`, `completion`, `views`, `url`, `<res> lookup` | `fsv_cli::cmd::*` | ported. |
| `cli.py` `tickets add-note` (reviewed, no `--set-status`), `changes tasks-delete --dry-run`, `changes update --due-by --dry-run` | `cmd::tickets`, `cmd::changes` | ported. |
| `<res> ls/get/activity/tasks/notes/fields`, `update`, `download`, `tickets reply/create/conversations/approvals/associations`, `changes create/update`, `changes tasks-*/approvals/state/clone/assets/associations`, `sr *`, `form *`, `tui`, `setup` | `fsv_cli::cmd::*` | ported. Golden tests in `tests/{read,write,local,where}_cmds.rs` were emitted from Python runs on the same routes (`tests/golden/cmp.py --emit`). |
| `cli._resolve_where`, `_parse_where`, `_where_operator`, `_date_condition`, `_where_schema_field`, `_load_query_hash`, `_merge_date_ranges` | `fsv_cli::filters` | ported. `ls --where` used `fsv_api::resolve_where` (the `query.py` resolver), which rejects field labels and aliases (`Agents`, `Assigned to`, `Created`), periods (`Last 6 months`), `me`, `(empty)`, plural cuts and date `!=`. It now uses the CLI resolver; `tests/where_cmds.rs` pins 37 Python runs plus 3 sent payloads. |
| `change_cli._send`, `cli_sr._send`, `ticket_flow.send`, `_report_partial` on the form path | `fsv_app::Submitted::send_cli`, `fsv_app::report_partial` | ported. `changes create/update`, `sr create` and `tickets create` send a form-reviewed plan through `send_cli` (update passes `--duplicate` and always allows a duplicate, as Python does); the flow-path `send_plan` reports partial failures with `fsv_app::report_partial`. `tests/send_cmds.rs` pins the flow path against Python: metadata changed after the review (exit 4 with `--yes`; on a terminal the new review and `Send the updated plan?`, declined and confirmed) and a failed attachment PUT after create (exit 5). The full-screen form needs a real terminal, so its `send_cli` calls are covered by fsv-app's `send_tests`, not by fsv-cli goldens. |
| `<res> lookup --refresh` (`d18bab9`), `form explain --id` on the module's own resource (`6797301`, changes and tickets) | `cmd::resource::lookup`, `cmd::form::explain` | ported; `read_cmds.rs` `lookup_refresh_refetches_schema`, `lookup_uses_cached_schema`, `form_explain_ticket_record`, `form_explain_change_display_id`. |
| `views.current_user` | `cmd::resource::current_user` | ported, fetched once per run as Python caches it on the client (`requester=me` made a second `agents/me` call). |
| `cli._complete_tab`, `_ctx_change_id`, `_complete_task_id`, `_asset_candidates`, `_complete_asset_category`, `_complete_asset_search_for_change`, `_complete_asset_id_for_change`, `_complete_associated_asset_for_change`, `_complete_task_status`, `_task_field_name_matches`, `_task_field_choice_pairs`, `_task_observed_value_pairs`, `_complete_task_custom_field`, `_complete_ticket_for_change` | none | n/a. Typer `autocompletion=` callbacks. fsv's generated scripts call `fsv._complete`, which dispatches only `fsv.completion` names, so Python never runs them. |
| `gen_tree.py` (generator for `generated.rs`) | `tests/golden/gen_tree.py` | restored. `python3 crates/fsv-cli/tests/golden/gen_tree.py` rewrites `generated.rs` from `cli_tree.json` and `completion_tree.json` (`--check` exits 1 if stale); the former hand edits (shared structs, help patches, the `backup_replaced` value flag, hints) are rules in the script. |

CLI divergences:
- A missing required argument or option prints Click's panel (`Missing option '--requester'.`, `app::missing_parameter` with the generated `CLICK_USAGE`). Other usage errors use clap's text. Exit code 2 matches Click. This includes a non-`json` `--output` on the reviewed commands (Click's rich choice panel; both exit 2 before any request).
- Typer `BadParameter` raised by a command body (`watchers list` errors) is reproduced as Rich's error panel (`cmd::reviewed::bad_parameter`), sized from the terminal width.
- Uncaught exceptions (traceback, exit 1 in Python) print `error: <message>` with exit 1.
- Completion scripts call `FSV_THIN_COMPLETE=1 fsv _complete` in place of `python -m fsv._complete`.
- `--backup-replaced` carries a `value_flag` override so completion treats it as taking a value, as Python's generator does.
- `--query-hash` with bad JSON prints `error: invalid --query-hash JSON: ` as Python does, then serde_json's message (`[{` gives `EOF while parsing an object at line 1 column 2`) where Python prints `json.JSONDecodeError`'s (`Expecting property name enclosed in double quotes: line 1 column 3 (char 2)`). Exit 2 in both.
- `fsv tui` with no terminal exits 1 with an error. Python hangs waiting for Textual's driver.
- `--show-completion` / `--install-completion` with an undetectable shell print `error: could not detect shell; ...` and exit 1. Typer prints `Shell  not supported.` or `Shell None is not supported.`, also exit 1. Shell detection reads only `$SHELL`; Typer asks shellingham about the parent process first.
- `form explain --id x` (not an ID) prints `error: invalid id: x` and exits 1; Python raises an uncaught `ValueError` (traceback, exit 1).
- Test-only: the routed mock (`src/mock.rs`, `tests/golden/run_py.py`) supports `"once": true` routes, so a second fetch of the same URL can answer differently.
- Test-only: debug builds treat `FSV_TEST_TTY=1` as an interactive stdin (`util::stdin_tty`) so golden tests can script prompts through a pipe; `tests/golden/run_py.py` patches `sys.stdin.isatty` the same way. Release builds ignore it.

## fsv-formui

| Python | Rust | status |
|---|---|---|
| `formui/session.py`, `lookups.py`, `files.py`, `when.py`, `fuzzy.py`, `widgets.py`, `screens.py`, `app.py` | `fsv_formui` (`session`, `lookups`, `files`, `kit`, `widgets`, `screen`, `app`) | ported. `cargo test -p fsv-formui --offline` was 140 passed (76 unit, 21 session, 31 UI, 12 widgets). Session expectations come from running Python on the Acme fixture; dialog results from a Textual pilot of the Python screens. |

- Textual button handlers (`_save_btn`, `_cancel_btn`, `_review_btn`, `_close_btn`, `ConfirmDialog._pressed`, …) are methods of the same name that Enter/Space on the focused button calls.
- `run_form` itself needs a real terminal (raw mode, alternate screen) and has no test. The loop it runs is tested on a `TestBackend` with scripted events.
- `when._no_dst_doubt` is inlined in `fsv_forms::when::to_text` (the `localize(..).is_ok_and(..)` check). fsv-formui pins it through `DateEditor::set_value` for ambiguous, skipped and plain Berlin wall times.
- Textual focus and property plumbing has no separate fn: `focus_editor` of each editor and of `FieldRow` is `FormScreen::focus_key`. `ComboBox.input` / `is_open`, `DateEditor.input`, `FileEditor.input` and `DescriptionEditor.area` are public fields.

- Plan building, draft saving, sending, `render_sr_template` and requester resolution sit behind `FormBackend` and `SessionInit`. `fsv-flows` supplies them. Until then the form cannot submit.
- Callers must set `SessionInit.paths` (`HOME`, cwd). The library no longer reads env or process state.
- Review tables are plain text. The cyan label column is not reproduced.
- F3 description preview shows the Markdown as typed, not rendered.
- Mouse support and Textual's command palette are not ported. The form is keyboard-only. No mouse capture means no click on buttons, fields or the review's acknowledgement box. `CalendarGrid.on_click` (header arrows change the month, a day cell picks it) and `ComboBox._clicked` (pick a list row, refused while a remote search is stale) have no Rust counterpart. Every one of these has a key path: Tab/arrows plus Enter/Space on buttons, PgUp/PgDn and Enter in the calendar, ↑↓ and Enter in the list. Enter has the same stale-search refusal.
- The SR `SAFETY` banner line is a constant in `screen/review.rs`. No Rust crate held that text.
- Copy depends on OSC 52 in the terminal.

## fsv-flows

| Python | Rust | status |
|---|---|---|
| `drafts.py`, `change_flow.py`, `ticket_flow.py` | `fsv_flows` | ported. `cargo test -p fsv-flows --offline` was 88 passed. Create (10), update (5), incident (4) and draft parses match `tests/golden/flows.json` from `gen_flows.py`. Plan JSON carries `risks` and `requires_confirmation` (Python `7903e2a`); the golden tests also check its key order. |
| `formmeta.py` cache and workspace discovery | `fsv_flows::meta` | ported as re-exports of `fsv_api::formmeta` (one `FormMeta`, `ClientResolver`, `Resolver`, `MetaSource`). Callers pass metadata through `MetaSource`. `MODULES` lists `problem` as in Python. `resolve_planning_definition` lists every choice (Python `820af12` dropped the cap of 12). |
| change execute HTTP | `change::ChangeWriter` | partial. Upload, POST, PUT, attach and planning sit behind the trait. Tests use a recording fake; the live client is `fsv_app::writer::LiveWriter`. |

`interactive()` and `editor_edit` are smoke-only. Flows take `&dyn fsv_api::service::Journal` (`DiskJournal` wraps `fsv_core::journal`); tests use an in-memory journal. Change execute journal safety (see Divergences) is pinned by `execute_unknown_outcome_leaves_started` (500/502/503/504/429/408/403 on POST and PUT), `execute_definite_rejection_records_failed` (400/404/409/422), `execute_failed_upload_before_create_records_failed` and `execute_refuses_when_started_is_not_persisted` (a journal whose write errors and one that drops writes).

## fsv-py

| Python behavior | Rust | status |
|---|---|---|
| `json.dumps(...)` all option combos used by fsv | `fsv_py::Dumps`, `canonical`, `compact`, `pretty` | ported (unit tests vs CPython output) |
| `float.__repr__` | `fsv_py::float_repr` | ported |
| `str()` / `repr()` of JSON values | `fsv_py::py_str`, `py_repr`, `py_repr_str` | ported |

## fsv-tui

| Python | Rust | status |
|---|---|---|
| `tui/app.py`, `actions.py`, `modals.py`, `tui/__init__.launch` | `fsv_tui` (`app`, `app/reviewed.rs`, `modals`, `runtime::{launch, run}`) | partial. `cargo test -p fsv-tui --offline` was 199 passed (119 unit, 7 in `tests/actions.rs`, 66 in `tests/keys.rs`, 7 in `tests/tui.rs`). Key bindings, modals, status/note/approval/download/form flows, the `a` menu, ticket property edit (`e` on a ticket), known-error mark, native planning text, ticket reply/note options (Ctrl+O) and the read-only asset candidate picker (`i`, `app/assets.rs`; mirrors `test_asset_picker.py::test_pilot_readonly_search_selection_and_cancel`) are covered against a recording `Backend`. The new flows mirror `test_tui_ticket_options.py`, `test_tui_ticket_properties.py`, `test_planning_text.py` and `test_problem_known_error.py`; all 23 of those Python tests pass. Gaps below. |

- `FsvApp._maybe` is a Textual `query_one` shim, so it has no counterpart. `FsvApp._set_filter_error` has no caller in Python (dead), so it is not ported; `filter_error` is set directly.
- `launch` needs a terminal. Its loop is `fsv_tui::run(terminal, app, next_event)`, tested with `TestBackend` and scripted events.
- The key-hint row advertises `gw`, which does nothing. Copied from Python; Python does the same.
- Help dialog is clipped on 36-row terminals and does not scroll.
- `s` on a change confirms with `change_flow.Plan.to_json` (review, warnings, risks, payload), the same object Python dumps into the status confirm.
- Plain ticket replies (`r`, Ctrl+S) go through `ticket_conversation_plan` / `send_ticket_conversation`, as in Python. `Backend::add_reply`, `actions::apply_reply` and the "Confirm reply" write path were unreachable (`can_reply` is tickets-only) and are deleted; `fsv_api::service::add_reply` stays for the CLI.
- Read-only mode is also enforced at the backend: `fsv_app::run_tui` wraps `LiveBackend` in `fsv_tui::backend::ReadOnly`, whose write methods fail with `read-only mode: writes are disabled (…)` before any request and whose `forms_available()` is false. Python gates read-only only in the UI; nothing changes for the user. The `Backend` network methods have no defaults; test doubles opt into stubs with `backend_stubs!`.
- Python `exclusive=True` cancels the previous worker in that group. Rust drops the older result once a newer exclusive worker in the group has started (`spawn_group`). The request itself still finishes. `_load_asset_inventory` is group `asset-inventory`. `_send_inventory_association` is the exclusive member of group `write`. `_change_task_choices` is exclusive group `task-create-choices` and lists runtime `change_task_fields` labels. An omitted task status or reminder is that form's first choice.
- Asset picker: Python calls `asset_picker.select` inside the picker callback, outside the worker's `try`, so a failure there would be an unhandled exception. Rust shows the failure screen `"Asset search unavailable: {exc}"` instead. It is unreachable in practice, because `candidates` already rejects duplicate display IDs. `AssetCandidates::select` and `backend::positive_id` port `asset_picker.select` and `watchers.positive_id` locally; fsv-api now has `asset_picker::select` too.
- Asset picker failures are pushed as a plain `FailureScreen`, as in Python: they do not set the `!` "last write failure".

### fsv-tui → fsv-app wiring (new `Backend` methods, all default to "… is not available in this build")

`LiveBackend` in `fsv-app/src/tui.rs` should implement:

| `Backend` method | Python call | fsv-api today |
|---|---|---|
| `ticket_property_fields(ticket_id) -> Vec<PropertyField>` | `formmeta.load_meta(c, "ticket", "edit", fresh=True, module_id=ticket_id)`, then keep `spec.fields` where `(f.sent or f.field_type == "default_requested_for") and not f.synthetic and f.is_input() and f.kind != unsupported and (f.is_custom or f.api_name in {subject, description, requester, requested_for, group, agent, status, priority, urgency, impact, source, category, ticket_type})`. `name = api_name`, `label`. For `kind == dropdown`, `choices = Some([(ch.value, ch.label) for visible ch])`, else `None`. | `fsv_api::formmeta::load_meta(c, store, "ticket", "edit", true, &id)` + `FormMeta::spec()`. |
| `ticket_property_plan(ticket_id, name, value) -> ReviewPlan` | `ticket_properties.build_plan(c, ticket_id, answers={name: value})` | wired: `properties::build_plan_with` + `ClientResolver`. |
| `send_ticket_property(plan) -> Value` | `ticket_properties.send(plan, c, reviewed_hash=plan.hash)` wired: `properties::send_with`. |
| `known_error_plan(problem_id) -> ReviewPlan` | `problem_known_error.build_plan(c, id)`: a `WritePlan("problem_known_error_mark", cid, domain, "PUT", "problems/{cid}", '{"known_error": true}', plan_hash(record))` | wired: `problem_known_error::build_plan`. |
| `send_known_error(plan) -> Value` | `problem_known_error.send(plan, c, reviewed_hash=plan.hash)` | wired: `problem_known_error::send`. |
| `planning_definitions(module) -> Vec<Value>` | `planning_text.definitions(c, module)`. `changes`: `create.planning_definitions(schema.load(CHANGES, force=True)["fields"])`. `problems`: the three fixed `{name, label}` rows. | wired: `planning_text::definitions`. |
| `planning_row(module, id, section) -> Option<Value>` | `create.match_planning_row({"name": section}, planning_text.inventory(c, module, id))` | wired: `planning_text::inventory` + `create::match_planning_row`. The TUI does the "preservation is unavailable" check itself. |
| `planning_text_plan(module, id, section, description) -> ReviewPlan` | `planning_text.build_plan(c, module, id, section, description)` (create into an empty section, edit of stored text) | wired: `planning_text::build_plan`. The TUI never clears. |
| `send_planning_text(plan) -> Value` | `planning_text.send(plan, c, reviewed_hash=plan.hash)` | wired: `planning_text::send`. |
| `ticket_conversation_plan(ticket_id, body, &ConversationRequest) -> ReviewPlan` | `recipients(c, cc)`, `recipients(c, notify)`, `recipients(c, bcc)` (validation only), then `build_plan(c, id, body, kind="reply"/"note", public, cc, notify, bcc, sender, set_status)` | wired: `conversations::{recipients, build_plan_mail}` + `MailArgs`; `ReviewPlan.raw = plan.payload()`. |
| `send_ticket_conversation(plan, public_confirmed) -> Value` | `ticket_conversation.send(plan, c, reviewed_hash=plan.hash, public_confirmed=...)` | wired: `conversations::send`. |
| `asset_candidates(&AssetCandidatesRequest { change, query, page }) -> AssetCandidates` | `asset_picker.candidates(c, change, query, page=page)` (TUI: page 1, `change = display_id or id`). One `GET /itam/core/device/api/` (`36408b2`). Map the result dict onto `AssetCandidates { change_id, page, scope, assets, mutation, warning }`; `assets` rows stay as returned, including `inventory`. Its errors (`"page must be positive"`, `"asset candidate response is unavailable"`, `"integrated inventory unavailable"`, `"duplicate asset display identities; selection is ambiguous"`, `positive_id`'s) become `Failure`s, shown as `"Asset search unavailable: {e}"`. | wired: `fsv_api::asset_picker::candidates`. GET only. |

`LiveBackend` implements every method in this table. See the fsv-app section.

Each send result is shown pretty-printed with `json.dumps(indent=2)`. A truthy `verified` reloads the record.

- Ticket reply/note option values ("Comma-separated plain emails (empty clears)", "Configured support email") accept an empty Enter, which clears the field. Python's `ValueScreen` ignores an empty Enter there, so a set CC/notify/BCC/sender cannot be cleared despite the prompt.
