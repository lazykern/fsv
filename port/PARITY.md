# Parity ledger

Each section: Python module → Rust location, status, ported tests, golden
fixtures, known gaps. Statuses: `ported`, `partial`, `missing`.

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
- `keepalive()` is ported and still has no caller.
- `fsv cache clear views` deletes the cache under `CACHE_DIR/filters`.
  Python deletes `CONFIG_DIR/filters` and leaves the views cache in place.
  No test asserts the wrong directory. The command's help says it clears
  the cache.
- Commands that crash with a Python traceback (no domain on `url` and
  `changes associations`, `form explain --id` with a bad id, `completion
  show powershell`) exit 1 with the same message and no traceback. The
  exit code stays.
- The TUI command palette (`ctrl+p`) is not ported. It is Textual's, not
  an fsv screen. Filter history still uses `ctrl+p` / `ctrl+n`.
- A failed search toasts the error. In Python the worker's uncaught
  exception exits the whole app.
- Keys typed in an open form do not reach the main TUI's key handler.
- The description preview keeps focus, so F3 toggles it off again. In
  Textual, hiding the focused editor moves focus away and F3 stops working.
- The status bar is a row above the key hints. Textual draws the hints on
  top of the status bar.
- TUI workers do not drop stale generations, and there is no change-emit
  lock. Python's other staleness checks are kept. A slow reply can still
  land after a newer load.
- `detail_panel` does not wrap a long planning line at 60 columns. Rich
  wraps it. The Rust block leaves wrapping to the console.
- An unquoted YAML key `true:` loads as `"true"`. Python gives `"True"`.
  The loader in `fsv-core` cannot tell a bool key from a quoted one after
  parse. `fsv-flows` golden draft cases remap that one key.
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
- `ClientResolver.lookup` custom lookups (Python ccc3a64) are ported to both
  `fsv_api::formmeta::ClientResolver` and `fsv_flows::meta::ClientResolver`.
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
| live wiring behind `views` reads, `formmeta.load_meta`, `create` writes, form session plan/save/send, TUI backend and pages | `fsv_app::{source, meta, writer, form, tui}` | partial. `cargo test -p fsv-app --offline` was 86 passed. Request paths, bodies and plan hashes captured from Python. `run_tui` and `run_form` need a terminal and have no tests. |
| `change_cli._send`, `cli_sr._send`, `ticket_flow.send`, `_report_partial` for a form-reviewed plan; `tui.actions.failure_detail` attributes | `Submitted::send_cli`, `send_checked`, `send_confirmed`, `SendError::{report, outcome}`, `tui::forms::send_submitted` | ported. Golden: `port/golden/gen_fsv_app_send.py` (metadata changed after review: diff lines, exit 4 without a terminal, re-review and confirm in one, exit 2 when the new form needs answers; partial failure lines and exit 5; SR equivalents). |

- Form review rows can show lookup ids where Python shows names. `fsv_flows` drafts hold JSON answers.
- TUI details load without form properties: `views.load_properties` / `load_edit_rules` are not ported anywhere, so Python's "form rules could not be applied" note is absent.
- Service-request drafts save through `fsv_flows::sr::save_sr_draft` from the reviewed plan, with no separate requested-for. The file bytes match Python's `render_sr_template` for the same plan.
- `run_tui` takes a schema loader argument because `schema::load` and saved-view loading live in `fsv-cli`, which `fsv-app` cannot depend on.
- Not yet wired in `LiveBackend` (trait defaults say unavailable): change tasks and task edit/delete, associations, downloads, catalog, completion, clipboard and open-URL, filter history.
- Directory lookups that hit an expired session surface as a query error, because `fsv_api::QueryError` does not carry the session flag.
- `FormSession.send` (TUI, `FormBackend::send`) still refuses a changed plan with exit 4, as Python does. The CLI re-confirm lives in `Submitted::send_cli`. Until fsv-cli calls it in place of `session.send`, the form paths of `changes create/update` and `sr create` exit 4 where Python shows the diff and asks again, and a partial change failure exits 1 with the one-line message where Python prints `_report_partial` and exits 5.
- A service-request form send that fails after upload or at place-request does not print `cli_sr._send`'s `uploaded attachment ids (not attached to any request): ...` line or its `{step} failed: HTTP N; ...` exit, and the TUI failure detail has no `uploaded`/`error` lines. `fsv_api::catalog::execute` flattens `SendFailure` into `Error::other`, so the step, uploads and cause are gone, and the exit code is 1 instead of the cause's. The fix belongs in fsv-api (a structured `SendFailure`). `fsv-cli`'s flow-path `sr_send` has the same gap.
- In tests, the upload progress line (`  uploading a.txt... ok (5 B)`) skips `term::capture` because it goes straight to stderr. Its text matches Python.

## fsv-api (started)

| Python | Rust | status |
|---|---|---|
| `query.py` filter → `query_hash`, date merge, one-shot percent-decode | `fsv_api::query` | ported (the four edge-case tests). Schema loading and the HTTP directory wait on `fsv-core`. `query._resolve_group` has no caller; `cli._resolve_group` is the one that is used. |
| `task_status.py` | `fsv_api::task_status` | ported (tenant ids, fail-closed choices, no default id). `change_tasks.build_plan` still to come. |
| `schema.py` choice labels, priority/impact/risk, v2 merge | `fsv_api::schema` | ported for the pure helpers. Cache TTL constant is present; the on-disk schema cache still waits on the client. |
| `views.py` dates, html, span, relative | `fsv_api::site::format` | ported. `span(23h 59.67m)` is `24h` (Python does not carry that into `1d`). List items keep the blank line `</li>` inserts. |
| `views.py` saved views, columns, cells, headers, tabs | `fsv_api::site` list | ported, no HTTP. `find_view` / `list_meta` / later-page headers match the Python tests. |
| `views.py` lifecycle, planning bind, flat properties | `fsv_api::site` detail | ported as pure JSON in, struct out. `match_planning_row` keeps the Python error sentences. |
| `views.py` record bits (source, approval, conversations, late badge) | `fsv_api::site::record` | ported. |
| `views.collect_pages` | `fsv_api::site::collect_pages` | ported as a walker the caller feeds. Page-limit, short-page `count`, and foreign `next_page_url` match `test_read_completeness.py`. Incomplete meta inserts `continuation` before `partial`, matching Python's update-then-assign order. |
| coverage-audit helpers: `query` private helpers, `formmeta._add` / `_cookie_workspace` / `ClientResolver._bootstrap`, `schema._cached_path` / `_fetch_schema`, `task_status.active_choices`, the three `_id`s, plan `payload`s, `views` list/format/detail/record helpers | same modules | ported. `cargo test -p fsv-api --offline` was 222 passed. Golden: `port/golden/gen_fsv_api_coverage.py` → `src/testdata/coverage_oracle.json`. |

Fixed to match Python while adding those tests: `_parse_where` needs a one-character field (`>=x` is field `>`, op `=`) and rejects an embedded newline. Field, choice and lookup matching use `casefold` and Python `or` truthiness (an id `0` or a `null` label no longer matches `"0"` / `"None"`). `_resolve_requester` falls back to the id for a blank `value`. `active_choices` takes the first truthy `value`/`label`/`name` and casefolds labels. `_file_names` gives `str(id)` when every name is falsy (`{"id": 0}` → `"0"`).

Gaps:
- `fsv_core::Client` has no way to list its cookie jar, so `Api::workspace_cookies` returns nothing for the live client and `_cookie_workspace` only works through test fakes. This needs a jar accessor in fsv-core.
- `ticket_conversation.build_plan` `forward`, `bcc` and `sender` are not ported. `conversations::build_plan` rejects `forward`, and `ConversationPlan::path` maps every non-reply kind to `notes`.
- `SendFailure` is still flattened into `Error::other` (see fsv-app). A structured error needs an `fsv_core::Error` variant or a new `catalog::execute` error type that the callers adopt.
- Python modules added on 2026-11-12 are not ported: `bulk_priority`, `reviewed_write`, `watchers`, `planning_text.definitions`, `readback`, `ticket_time._duration`.

## fsv-views

| Python | Rust | status |
|---|---|---|
| `pages.py`, `render.py` | `fsv_views::pages`, `fsv_views::render` | ported. `cargo test -p fsv-views --offline` was 99 passed. Expected strings came from running the Python functions. |
| `views.py` loaders (`load_tab`, `load_detail`, conversations, requested items) | `fsv_views::detail`, `fsv_views::source` | ported, covered from `coverage`. |
| `views.ticket_headers` | `fsv_views::source::ticket_headers` | ported. `fsv-app` (`tui.rs` `ticket_headers`) still has its own inline copy. |
| `render._requested_item_field_defs` | `render::field_defs` | ported. |
| `render.strip_html` | `fsv_core::output::strip_html` | ported in fsv-core. |

Golden values for the last three come from `port/golden/gen_fsv_views_extra.py`.

Known test limits, not behavior changes: relative times use the real clock, so tests check placement rather than a pinned "2 days ago". `_LazyModule` and `_NullConsole` are import-time shims with no Rust counterpart. `print_blocks` and `print_detail` write to the terminal; tests cover `blocks_text` and `detail_blocks`.

## fsv-completion

| Python | Rust | status |
|---|---|---|
| `_complete._completions`, `main` | `dynamic::complete`, `run` | ported. `cargo test -p fsv-completion --offline` was 38 passed. |
| `completion` completers, including `complete_format`, `complete_store`, `complete_shell`, `complete_sort_order`, `complete_config_key`, `complete_config_value`, `complete_cache_target`, `complete_search_sort`, `complete_duplicate_mode` | one Rust fn each in `dynamic` | ported. |
| `completion.complete_update_choice`, `complete_help_topic` | `fsv_completion::{complete_update_choice, complete_help_topic}` | ported. Neither has an arm in `_complete`, in Python or Rust, so the generated scripts print nothing for `--status`/`--priority`. |
| `completion._config_load`, `_completion_network` | `fsv_completion::{config_load, completion_network}` | ported with the directory and env value passed in. `fsv-cli` (`cmd::completion::network_enabled`) and `fsv-app` still keep their own copies; theirs treat `network: 1.0` as off, Python treats it as on. |
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

Golden values: `port/golden/gen_fsv_completion_extra.py` writes `tests/fixtures/extra_expected.json`.

## fsv-core

| Python | Rust | status |
|---|---|---|
| `session._b64e`, `_b64d` | `session::{b64e, b64d}` | ported. `b64d` follows CPython's non-strict decoder: it skips non-alphabet characters, stops at completing padding, and gives the same error text. |
| `session._passphrase`, `_argon_encrypt` | `session::{passphrase, argon_encrypt}` | ported. The prompt is injected; the binary passes `rpassword`. |
| `session._read_secret` | `paste::read_secret` (`collect_secret`, `read_piped`) | ported. Key-loop expectations come from Python reading a pty. |
| `session._http_json`, `_page_ws_url`, `_cdp_call`, `_direct_browser_cookies` | `browser` | ported. Tested against a local `TcpListener` DevTools fake running the same scenarios Python ran: ping, event and foreign-id frames, error fallback to `getAllCookies`, a non-object result, and close. |
| `journal._command`, `errors.APIError.__init__` | `journal::command`, `ApiError::{new, with_request}` | ported. |
| `cargo test -p fsv-core --offline` | | was 169 passed. |

Not ported, with reasons:
- `session._ws_read_exact`, `_ws_send_text`, `_ws_recv_text`: `tungstenite` does the websocket framing. Their behavior is covered through `cdp_call`.
- `client.Client.close`: the reqwest pool closes when the last `Arc<Client>` drops. Python calls `close()` in `finally` blocks, and Rust scopes do the same job.
- `session.login_interactive` is a one-line wrapper that reads the real stdin. `login_interactive_with` is tested.

Divergences:
- A bad base64 field in an argon session file is a session error, `invalid encrypted session file` (exit 3). Python raises `binascii.Error`, a `ValueError` (exit 2).
- Ctrl-C at the hidden cookie prompt is read as a byte and gives `Error::Aborted`. Python leaves ISIG on and gets `KeyboardInterrupt`.
- The CDP client answers server pings with a pong and checks `Sec-WebSocket-Accept`. Python ignores pings and only checks for `101` in the status line.

Golden values: `port/golden/gen_fsv_core_session.py` writes `tests/fixtures/session_expected.json`. `strip_html.json` comes from `gen_fsv_views_extra.py`.

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
| `cli.py` command tree (111 nodes: root, 13 groups, 97 leaves) | `fsv_cli::generated`, `app` | ported. `tests/tree.rs` checks every node's name, hidden flag, options, positionals and help text against a Python dump. |
| `cli.py` `help`, `search`, `auth`, `cache`, `journal`, `config`, `completion`, `views`, `url`, `<res> lookup` | `fsv_cli::cmd::*` | ported. |
| `cli.py` `tickets add-note` (reviewed, no `--set-status`), `changes tasks-delete --dry-run`, `changes update --due-by --dry-run` | `cmd::tickets`, `cmd::changes` | ported. |
| `<res> ls/get/activity/tasks/notes/fields`, `update`, `download`, `tickets reply/create/conversations/approvals/associations`, `changes create/update` (form path), `changes tasks-add/-update/approvals/state/clone/assets/associations`, `sr *`, `form *`, `tui`, `setup` | `fsv_cli::pending` | **not ported.** Each runs its local checks, prints `error: X is not ported to Rust yet`, exits 1. Blocked on schema, views, formmeta, catalog and change-flow wiring in `fsv-api`/`fsv-flows`/`fsv-views`. |
| `gen_tree.py` (generator for `generated.rs`) | none | missing from disk. `generated.rs` was hand-edited after generation: module renamed to `generated` (`gen` is reserved in edition 2024), struct doc comments removed, hint fields fixed. Regenerating needs the script rewritten. |

CLI divergences:
- Usage errors use clap's text. Exit code 2 matches Click.
- Completion scripts call `FSV_THIN_COMPLETE=1 fsv _complete` in place of `python -m fsv._complete`.
- `FSV_COMPLETION_NETWORK=1` gates live completers. It is a new env switch, read only in `cmd/completion.rs`.
- `--backup-replaced` carries a `value_flag` override so completion treats it as taking a value, as Python's generator does.

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
| `drafts.py`, `change_flow.py`, `ticket_flow.py` | `fsv_flows` | ported. `cargo test -p fsv-flows --offline` was 82 passed. Create (10), update (5), incident (4) and draft parses match `tests/golden/flows.json` from `gen_flows.py`. |
| `formmeta.py` cache and workspace discovery | `fsv_flows::meta` | partial. Only the pure helpers the flows call. Callers pass metadata through `MetaSource`. |
| change execute HTTP | `change::ChangeWriter` | partial. Upload, POST, PUT, attach and planning sit behind the trait. Tests use a recording fake. No live client yet. |

`interactive()` and `editor_edit` are smoke-only. The `~/a.pdf` draft case is skipped unless home is `/Users/phusit.somb`, because the golden file stores the expanded path. `DiskJournal` wraps `fsv_core::journal`; tests use an in-memory journal.

## fsv-py

| Python behavior | Rust | status |
|---|---|---|
| `json.dumps(...)` all option combos used by fsv | `fsv_py::Dumps`, `canonical`, `compact`, `pretty` | ported (unit tests vs CPython output) |
| `float.__repr__` | `fsv_py::float_repr` | ported |
| `str()` / `repr()` of JSON values | `fsv_py::py_str`, `py_repr`, `py_repr_str` | ported |

## fsv-tui

| Python | Rust | status |
|---|---|---|
| `tui/app.py`, `actions.py`, `modals.py`, `tui/__init__.launch` | `fsv_tui` (`app`, `app/reviewed.rs`, `modals`, `runtime::{launch, run}`) | partial. `cargo test -p fsv-tui --offline` was 190 passed (115 unit, 7 in `tests/actions.rs`, 61 in `tests/keys.rs`, 7 in `tests/tui.rs`). Key bindings, modals, status/note/approval/download/form flows, the `a` menu, ticket property edit (`e` on a ticket), known-error mark, native planning text, and ticket reply/note options (Ctrl+O) are covered against a recording `Backend`. The new flows mirror `test_tui_ticket_options.py`, `test_tui_ticket_properties.py`, `test_planning_text.py` and `test_problem_known_error.py`; all 23 of those Python tests pass. Gaps below. |

- `FsvApp._maybe` is a Textual `query_one` shim, so it has no counterpart. `FsvApp._set_filter_error` has no caller in Python (dead), so it is not ported; `filter_error` is set directly.
- `launch` needs a terminal. Its loop is `fsv_tui::run(terminal, app, next_event)`, tested with `TestBackend` and scripted events.
- The key-hint row advertises `gw`, which does nothing. Copied from Python; Python does the same.
- Help dialog is clipped on 36-row terminals and does not scroll.
- Plain ticket replies (`r`, Ctrl+S) go through `ticket_conversation_plan` / `send_ticket_conversation`, as in Python. `Backend::add_reply` and the "Confirm reply" path are now unreachable, because `can_reply` is tickets-only. They are kept so `LiveBackend` keeps compiling.
- Python marks these workers `exclusive=True`, so a second press cancels the first. Rust runs both, like the other TUI workers (see the stale-generation divergence).

### fsv-tui → fsv-app wiring (new `Backend` methods, all default to "… is not available in this build")

`LiveBackend` in `fsv-app/src/tui.rs` should implement:

| `Backend` method | Python call | fsv-api today |
|---|---|---|
| `ticket_property_fields(ticket_id) -> Vec<PropertyField>` | `formmeta.load_meta(c, "ticket", "edit", fresh=True, module_id=ticket_id)`, then keep `spec.fields` where `(f.sent or f.field_type == "default_requested_for") and not f.synthetic and f.is_input() and f.kind != unsupported and (f.is_custom or f.api_name in {subject, description, requester, requested_for, group, agent, status, priority, urgency, impact, source, category, ticket_type})`. `name = api_name`, `label`. For `kind == dropdown`, `choices = Some([(ch.value, ch.label) for visible ch])`, else `None`. | `fsv_api::formmeta::load_meta(c, store, "ticket", "edit", true, &id)` + `FormMeta::spec()`. |
| `ticket_property_plan(ticket_id, name, value) -> ReviewPlan` | `ticket_properties.build_plan(c, ticket_id, answers={name: value})` | **missing**: `fsv_api::properties::build_plan` has no `answers` argument (only tags and planned dates). Python also resolves `requested_for` by email and hashes `answers_json` + `meta.fingerprint.hash`. Port those first. `review = plan.review()`, `hash = plan.hash()`. |
| `send_ticket_property(plan) -> Value` | `ticket_properties.send(plan, c, reviewed_hash=plan.hash)` | `fsv_api::properties::send(&plan, c, journal, store, Some(&hash))`. Keep the `PropertyPlan` in `ReviewPlan.raw` or rebuild it from there. |
| `known_error_plan(problem_id) -> ReviewPlan` | `problem_known_error.build_plan(c, id)`: a `WritePlan("problem_known_error_mark", cid, domain, "PUT", "problems/{cid}", '{"known_error": true}', plan_hash(record))` | **missing**: no `problem_known_error` or `reviewed_write` module in fsv-api. |
| `send_known_error(plan) -> Value` | `problem_known_error.send(plan, c, reviewed_hash=plan.hash)` | **missing** (same). |
| `planning_definitions(module) -> Vec<Value>` | `planning_text.definitions(c, module)`. `changes`: `create.planning_definitions(schema.load(CHANGES, force=True)["fields"])`. `problems`: the three fixed `{name, label}` rows. | **missing**: no `planning_text` module. |
| `planning_row(module, id, section) -> Option<Value>` | `create.match_planning_row({"name": section}, planning_text.inventory(c, module, id))` | `fsv_api::site::match_planning_row` exists. `inventory` (`GET {module}/{id}/planning-fields` with completeness checks) is **missing**. The TUI does the "preservation is unavailable" check itself. |
| `planning_text_plan(module, id, section, description) -> ReviewPlan` | `planning_text.build_plan(c, module, id, section, description)` | **missing**. |
| `send_planning_text(plan) -> Value` | `planning_text.send(plan, c, reviewed_hash=plan.hash)` | **missing**. |
| `ticket_conversation_plan(ticket_id, body, &ConversationRequest) -> ReviewPlan` | `recipients(c, cc)`, `recipients(c, notify)`, `recipients(c, bcc)` (validation only), then `build_plan(c, id, body, kind="reply"/"note", public, cc, notify, bcc, sender, set_status)` | `fsv_api::conversations::{recipients, build_plan}`. **Gap**: `ConversationArgs` has no `bcc`/`sender` (nor `forward`, which the TUI never uses). Python's `bcc_emails` / `from_email` payload keys and their validation need porting. Set `ReviewPlan.raw = plan.payload()`: the TUI reads `private` from it. Set `review = plan.review()`, `hash = plan.hash()`. |
| `send_ticket_conversation(plan, public_confirmed) -> Value` | `ticket_conversation.send(plan, c, reviewed_hash=plan.hash, public_confirmed=...)` | `fsv_api::conversations::send(&plan, c, journal, store, Some(&hash), public_confirmed)` |

Each send result is shown pretty-printed with `json.dumps(indent=2)`. A truthy `verified` reloads the record.

- Ticket reply/note option values ("Comma-separated plain emails (empty clears)", "Configured support email") accept an empty Enter, which clears the field. Python's `ValueScreen` ignores an empty Enter there, so a set CC/notify/BCC/sender cannot be cleared despite the prompt.
