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
| live wiring behind `views` reads, `formmeta.load_meta`, `create` writes, form session plan/save/send, TUI backend and pages | `fsv_app::{source, meta, writer, form, tui}` | partial. `cargo test -p fsv-app --offline` was 94 passed. Request paths, bodies and plan hashes captured from Python. `run_tui` and `run_form` need a terminal and have no tests. |
| `tui.app._prepare_ticket_conversation` / `_send_ticket_conversation`, `_load_ticket_property_fields`, `_send_ticket_property` | `LiveBackend::{ticket_conversation_plan, send_ticket_conversation, ticket_property_fields, send_ticket_property}` | partial. Golden: `port/golden/gen_fsv_app_tui_reviewed.py` → `src/tui/testdata/reviewed_expected.json` (reply with CC + Pending, private note with notify, sender/BCC refusals, the property picker filter, a tag-plan send). `ticket_property_plan` is refused (see below). |
| `change_cli._send`, `cli_sr._send`, `ticket_flow.send`, `_report_partial` for a form-reviewed plan; `tui.actions.failure_detail` attributes | `Submitted::send_cli`, `send_checked`, `send_confirmed`, `SendError::{report, outcome}`, `tui::forms::send_submitted` | ported. Golden: `port/golden/gen_fsv_app_send.py` (metadata changed after review: diff lines, exit 4 without a terminal, re-review and confirm in one, exit 2 when the new form needs answers; partial failure lines and exit 5; SR equivalents). |

- Form review rows can show lookup ids where Python shows names. `fsv_flows` drafts hold JSON answers.
- TUI details load without form properties: `views.load_properties` / `load_edit_rules` are not ported anywhere, so Python's "form rules could not be applied" note is absent.
- Service-request drafts save through `fsv_flows::sr::save_sr_draft` from the reviewed plan, with no separate requested-for. The file bytes match Python's `render_sr_template` for the same plan.
- `run_tui` takes a schema loader argument because `schema::load` and saved-view loading live in `fsv-cli`, which `fsv-app` cannot depend on.
- Not yet wired in `LiveBackend` (trait defaults say unavailable): change tasks and task edit/delete, associations, downloads, catalog, completion, clipboard and open-URL, filter history.
- Directory lookups that hit an expired session surface as a query error, because `fsv_api::QueryError` does not carry the session flag.
- `FormSession.send` (TUI, `FormBackend::send`) still refuses a changed plan with exit 4, as Python does. The CLI re-confirm lives in `Submitted::send_cli`, which fsv-cli's form paths (`changes create/update`, `sr create`, `tickets create`) call.
- A service-request form send that fails after upload or at place-request does not print `cli_sr._send`'s `uploaded attachment ids (not attached to any request): ...` line or its `{step} failed: HTTP N; ...` exit, and the TUI failure detail has no `uploaded`/`error` lines. `fsv_api::catalog::execute` flattens `SendFailure` into `Error::other`, so the step, uploads and cause are gone, and the exit code is 1 instead of the cause's. The fix belongs in fsv-api (a structured `SendFailure`). `fsv-cli`'s flow-path `sr_send` has the same gap.
- In tests, the upload progress line (`  uploading a.txt... ok (5 B)`) skips `term::capture` because it goes straight to stderr. Its text matches Python.
- Reviewed conversation plans stay in `LiveBackend` by hash, because `ReviewPlan.raw` is only the payload (the TUI reads `private` from it). A send for a hash this backend did not plan is refused: "the conversation plan was not built by this session; review it again". Python's TUI cannot reach that case.
- Ticket reply/note Sender and BCC: `fsv_api::conversations::ConversationArgs` has no `bcc`/`sender`. The backend still looks BCC up with `recipients` and runs Python's checks in Python's order, with the same texts: "sender must be a plain configured support email address", "note sender/BCC wire is not proven", "combined BCC/CC/status wire is not proven", "BCC requires a configured --sender and bare recipient emails". A request that passes them, which Python sends with `from_email` / `bcc_emails`, is refused with "replies with a sender or BCC are not available in this build; clear Sender and BCC to send". Waits on fsv-api `a5a8381`.
- The reply/note review `warning` is fsv-api's older text ("A reply is public and may notify recipients; …"). Python now says "Replies and forwards send mail; notes may trigger tenant workflows. Explicit sender addresses are user-asserted, not verified mailbox configuration." The hash does not include it.
- A conversation send does not re-resolve the CC recipients before the POST. Python's `send` calls `recipients` again ("CC recipient identity changed; review again"), and for BCC addresses too. Both are fsv-api gaps.
- `e` on a ticket lists the fields (`ticket_property_fields` matches Python's filter and requests), but picking one and entering a value is refused with "reviewed ticket property edits from the form are not available in this build; use the web form". `fsv_api::properties::build_plan` has no `answers` (`f761a01`, plus `3057492` / `774f4f1` for the beneficiary). The older tag and planned-date flags do not cover any of it: the picker never lists tags or planned dates, and Python's `answers` path refuses them too ("Start: update wire is not proven"). `send_ticket_property` is wired: it takes a `property_review(&PropertyPlan)` and calls `properties::send`. Once `answers` lands, the plan hash must also include `metadata_hash`.

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
| `readback.py` | `fsv_api::readback` | ported (`test_readback_normalization` duration/timestamp cases). |
| `reviewed_write.py` | `fsv_api::reviewed_write` | ported (`WritePlan`, `execute`, `refused`). |
| `watchers.py`, `problem_known_error.py`, `planning_text.py`, `ticket_time.py`, `bulk_priority.py` | same-stem modules | ported, no CLI. Hashes and reviews checked against `port/golden/gen_fsv_api_reviewed.py` → `src/testdata/reviewed_oracle.json`. |
| `ticket_properties.build_plan(answers=…)`, beneficiary, rule-context reset | `properties::build_plan_with` / `send_with` → `AnsweredPropertyPlan` | ported. `PropertyPlan` (no answers) keeps its fields and hash. |
| `ticket_conversation` bcc / sender / forward, CC re-check | `conversations::build_plan_mail` + `MailArgs` | ported; `build_plan` is the no-BCC/no-sender wrapper. |
| `catalog.SendFailure` + `cli_sr.place` conversion | `catalog::SendFailure`, `Executed`, `execute_detailed` | ported; `execute` still flattens to `Error::other` with the same text. |
| `change_approvals._run` echo before an error | `approvals::run_action_detailed` → `RunActionError { echo, error }` | ported; `run_action` unchanged. |
| `cli._merge_same_conditions` | `fsv_api::merge_same_conditions` | ported (the fsv-cli copy in `resource.rs` can switch to it). |
| `Client.me`, `Client.v2_get` | `Api::me`, `Api::v2_get` | ported for `fsv_core::Client`; defaulted to an error on other impls. |

Fixed to match Python while adding those tests: `_parse_where` needs a one-character field (`>=x` is field `>`, op `=`) and rejects an embedded newline. Field, choice and lookup matching use `casefold` and Python `or` truthiness (an id `0` or a `null` label no longer matches `"0"` / `"None"`). `_resolve_requester` falls back to the id for a blank `value`. `active_choices` takes the first truthy `value`/`label`/`name` and casefolds labels. `_file_names` gives `str(id)` when every name is falsy (`{"id": 0}` → `"0"`).

Gaps:
- `fsv_core::Client` has no way to list its cookie jar, so `Api::workspace_cookies` returns nothing for the live client and `_cookie_workspace` only works through test fakes. This needs a jar accessor in fsv-core.
- `Api::me` / `Api::v2_get` have erroring defaults. fsv-cli `LazyApi`, fsv-flows' testing `Api` and the fsv-views golden fake should forward them; until then watchers / ticket time through those impls fail with "signed-in agent identity is unavailable".
- `07c85d0` (canned replies: `canned_id` and friends on `ConversationPlan`, `canned.py`, `scenarios.py`) is not ported.
- `cli._resolve_where` (labels, aliases, `Me`) is an fsv-cli function; `query.resolve_where`'s "field not found: 'x'" already matches `query.py`.
- `1dc7ab6`'s exact-bytes/headers HTTP tests have no equivalent; fsv-api tests run on `FakeApi` / `FnApi`, which see the method, path and body but not headers.

Divergences:
- `readback::duration_seconds`, `ticket_time::duration` and `watchers::positive_id` accept ASCII digits only; Python's `\d` / `str.isdigit` also accept other Unicode digits. Hours that overflow `i64` are an error rather than a big int.
- `conversations::parseaddr` is a simplified `email.utils.parseaddr`: the address inside the last `<…>`, else the trimmed value. Quoted local parts and comments are not parsed.
- `merge_same_conditions` turns a truthy non-iterable `value` (a number or `true`) into a one-item list; Python raises `TypeError` there.
- `SendFailure::into_error` names the Python type of the cause from the `fsv_core::Error` variant (`Other`/`Context` → `RuntimeError`, `Io` → `OSError`, `Exit` → `FlowExit`), and takes the exit code from `Error::exit_code`, which for an `Exit` cause is its own code (Python's `exit_code_for` gives 1).
- `Client::send_once` no longer sends `Content-Type` on a bodiless request (a `DELETE` with no payload), matching `reviewed_write.execute`, which passes headers without a body.

## fsv-views

| Python | Rust | status |
|---|---|---|
| `pages.py`, `render.py` | `fsv_views::pages`, `fsv_views::render` | ported. `cargo test -p fsv-views --offline` was 99 passed. Expected strings came from running the Python functions. |
| `views.py` loaders (`load_tab`, `load_detail`, conversations, requested items) | `fsv_views::detail`, `fsv_views::source` | ported, covered from `coverage`. |
| `views.ticket_headers` | `fsv_views::source::ticket_headers` | ported. `fsv-app`'s `LiveBackend::ticket_headers` calls it. |
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
| `completion._config_load`, `_completion_network` | `fsv_completion::{config_load, completion_network}` | ported with the directory and env value passed in. `fsv-app`'s TUI backend calls it. `fsv-cli` (`cmd::completion::network_enabled`) still keeps its own copy, which treats `network: 1.0` as off; Python treats it as on. |
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
| `cli.py` command tree (111 nodes: root, 13 groups, 97 leaves) | `fsv_cli::generated`, `app` | ported. `tests/tree.rs` checks every node's name, hidden flag, options, positionals and help text against a Python dump. |
| `cli.py` `help`, `search`, `auth`, `cache`, `journal`, `config`, `completion`, `views`, `url`, `<res> lookup` | `fsv_cli::cmd::*` | ported. |
| `cli.py` `tickets add-note` (reviewed, no `--set-status`), `changes tasks-delete --dry-run`, `changes update --due-by --dry-run` | `cmd::tickets`, `cmd::changes` | ported. |
| `<res> ls/get/activity/tasks/notes/fields`, `update`, `download`, `tickets reply/create/conversations/approvals/associations`, `changes create/update`, `changes tasks-*/approvals/state/clone/assets/associations`, `sr *`, `form *`, `tui`, `setup` | `fsv_cli::cmd::*` | ported. Golden tests in `tests/{read,write,local,where}_cmds.rs` were emitted from Python runs on the same routes (`tests/golden/cmp.py --emit`). |
| `cli._resolve_where`, `_parse_where`, `_where_operator`, `_date_condition`, `_where_schema_field`, `_load_query_hash`, `_merge_date_ranges` | `fsv_cli::filters` | ported. `ls --where` used `fsv_api::resolve_where` (the `query.py` resolver), which rejects field labels and aliases (`Agents`, `Assigned to`, `Created`), periods (`Last 6 months`), `me`, `(empty)`, plural cuts and date `!=`. It now uses the CLI resolver; `tests/where_cmds.rs` pins 37 Python runs plus 3 sent payloads. |
| `change_cli._send`, `cli_sr._send`, `ticket_flow.send`, `_report_partial` on the form path | `fsv_app::Submitted::send_cli`, `fsv_app::report_partial` | ported. `changes create/update`, `sr create` and `tickets create` send a form-reviewed plan through `send_cli` (update passes `--duplicate` and always allows a duplicate, as Python does); the flow-path `send_plan` reports partial failures with `fsv_app::report_partial`. `tests/send_cmds.rs` pins the flow path against Python: metadata changed after the review (exit 4 with `--yes`; on a terminal the new review and `Send the updated plan?`, declined and confirmed) and a failed attachment PUT after create (exit 5). The full-screen form needs a real terminal, so its `send_cli` calls are covered by fsv-app's `send_tests`, not by fsv-cli goldens. |
| `<res> lookup --refresh` (`d18bab9`), `form explain --id` on the module's own resource (`6797301`, changes and tickets) | `cmd::resource::lookup`, `cmd::form::explain` | ported; `read_cmds.rs` `lookup_refresh_refetches_schema`, `lookup_uses_cached_schema`, `form_explain_ticket_record`, `form_explain_change_display_id`. |
| `views.current_user` | `cmd::resource::current_user` | ported, fetched once per run as Python caches it on the client (`requester=me` made a second `agents/me` call). |
| `cli._complete_tab`, `_ctx_change_id`, `_complete_task_id`, `_asset_candidates`, `_complete_asset_category`, `_complete_asset_search_for_change`, `_complete_asset_id_for_change`, `_complete_associated_asset_for_change`, `_complete_task_status`, `_task_field_name_matches`, `_task_field_choice_pairs`, `_task_observed_value_pairs`, `_complete_task_custom_field`, `_complete_ticket_for_change` | none | n/a. Typer `autocompletion=` callbacks. fsv's generated scripts call `fsv._complete`, which dispatches only `fsv.completion` names, so Python never runs them. |
| `gen_tree.py` (generator for `generated.rs`) | none | missing from disk. `generated.rs` was hand-edited after generation: module renamed to `generated` (`gen` is reserved in edition 2024), struct doc comments removed, hint fields fixed. Regenerating needs the script rewritten. |

CLI divergences:
- Usage errors use clap's text. Exit code 2 matches Click.
- Completion scripts call `FSV_THIN_COMPLETE=1 fsv _complete` in place of `python -m fsv._complete`.
- `FSV_COMPLETION_NETWORK=1` gates live completers. It is a new env switch, read only in `cmd/completion.rs`.
- `--backup-replaced` carries a `value_flag` override so completion treats it as taking a value, as Python's generator does.
- `--query-hash` with bad JSON prints `error: invalid --query-hash JSON: ` as Python does, then serde_json's message (`[{` gives `EOF while parsing an object at line 1 column 2`) where Python prints `json.JSONDecodeError`'s (`Expecting property name enclosed in double quotes: line 1 column 3 (char 2)`). Exit 2 in both.
- `fsv tui` with no terminal exits 1 with an error. Python hangs waiting for Textual's driver.
- `--show-completion` / `--install-completion` with an undetectable shell print `error: could not detect shell; ...` and exit 1. Typer prints `Shell  not supported.` or `Shell None is not supported.`, also exit 1. Shell detection reads only `$SHELL`; Typer asks shellingham about the parent process first.
- `form dump/diff/explain/fields` accept `changes` and `tickets`. Upstream 6797301 added `problems` to `formmeta.MODULES`; that is part of the triaged upstream range, so the unsupported-form message still lists two modules.
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
| `drafts.py`, `change_flow.py`, `ticket_flow.py` | `fsv_flows` | ported. `cargo test -p fsv-flows --offline` was 82 passed. Create (10), update (5), incident (4) and draft parses match `tests/golden/flows.json` from `gen_flows.py`. Plan JSON carries `risks` and `requires_confirmation` (Python `7903e2a`); the golden tests also check its key order. |
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
| `tui/app.py`, `actions.py`, `modals.py`, `tui/__init__.launch` | `fsv_tui` (`app`, `app/reviewed.rs`, `modals`, `runtime::{launch, run}`) | partial. `cargo test -p fsv-tui --offline` was 199 passed (119 unit, 7 in `tests/actions.rs`, 66 in `tests/keys.rs`, 7 in `tests/tui.rs`). Key bindings, modals, status/note/approval/download/form flows, the `a` menu, ticket property edit (`e` on a ticket), known-error mark, native planning text, ticket reply/note options (Ctrl+O) and the read-only asset candidate picker (`i`, `app/assets.rs`; mirrors `test_asset_picker.py::test_pilot_readonly_search_selection_and_cancel`) are covered against a recording `Backend`. The new flows mirror `test_tui_ticket_options.py`, `test_tui_ticket_properties.py`, `test_planning_text.py` and `test_problem_known_error.py`; all 23 of those Python tests pass. Gaps below. |

- `FsvApp._maybe` is a Textual `query_one` shim, so it has no counterpart. `FsvApp._set_filter_error` has no caller in Python (dead), so it is not ported; `filter_error` is set directly.
- `launch` needs a terminal. Its loop is `fsv_tui::run(terminal, app, next_event)`, tested with `TestBackend` and scripted events.
- The key-hint row advertises `gw`, which does nothing. Copied from Python; Python does the same.
- Help dialog is clipped on 36-row terminals and does not scroll.
- Plain ticket replies (`r`, Ctrl+S) go through `ticket_conversation_plan` / `send_ticket_conversation`, as in Python. `Backend::add_reply` and the "Confirm reply" path are now unreachable, because `can_reply` is tickets-only. They are kept so `LiveBackend` keeps compiling.
- Python marks these workers `exclusive=True`, so a second press cancels the first. Rust runs both, like the other TUI workers (see the stale-generation divergence). The same holds for `_load_asset_inventory` (`group="asset-inventory"`).
- Asset picker: Python calls `asset_picker.select` inside the picker callback, outside the worker's `try`, so a failure there would be an unhandled exception. Rust shows the failure screen `"Asset search unavailable: {exc}"` instead. It is unreachable in practice, because `candidates` already rejects duplicate display IDs. `AssetCandidates::select` and `backend::positive_id` port `asset_picker.select` and `watchers.positive_id` locally until fsv-api has `asset_picker`.
- Asset picker failures are pushed as a plain `FailureScreen`, as in Python: they do not set the `!` "last write failure".

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
| `asset_candidates(&AssetCandidatesRequest { change, query, page }) -> AssetCandidates` | `asset_picker.candidates(c, change, query, page=page)` (TUI: page 1, `change = display_id or id`). Map the result dict onto `AssetCandidates { change_id, page, scope, assets, mutation, warning }`; `assets` rows stay as returned. Its errors (`"page must be positive"`, `"asset candidate response is unavailable"`, `"duplicate asset display identities; selection is ambiguous"`, `positive_id`'s) become `Failure`s, shown as `"Asset search unavailable: {e}"`. | **missing**: no `asset_picker` module; `fsv_api::create::search_assets_for_change` exists. GET only, so the method must work in read-only mode. |

`LiveBackend` now implements `ticket_conversation_plan`, `send_ticket_conversation`, `ticket_property_fields` and `send_ticket_property`. `ticket_property_plan` refuses every field until fsv-api has `answers`. The known-error and planning-text hooks still use the defaults. See the fsv-app section.

Each send result is shown pretty-printed with `json.dumps(indent=2)`. A truthy `verified` reloads the record.

- Ticket reply/note option values ("Comma-separated plain emails (empty clears)", "Configured support email") accept an empty Enter, which clears the field. Python's `ValueScreen` ignores an empty Enter there, so a set CC/notify/BCC/sender cannot be cleared despite the prompt.
