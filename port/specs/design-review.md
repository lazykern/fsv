# Design review — before fsv-api / fsv-flows / fsv-views / fsv-cli / fsv-formui / fsv-tui

Reviewer: advisor agent, 2026-11-12. Scope: `port/ARCHITECTURE.md` decisions plus
the Python import graph (verified with an AST walk over every `import`, including
function-local ones). fsv-core and fsv-forms were still empty stubs at review time,
so every change below is cheap now.

## Verdicts

| # | Topic | Verdict | One-line reason |
|---|---|---|---|
| 1 | Blocking reqwest + worker threads | **keep** (with client shape changes) | Python never aborts in-flight HTTP; TUI drops stale results by epoch/seq. Blocking is exact parity. Keepalive is never called. |
| 2 | Crate cut | **change** | `views.py` is a loader module; fsv-api modules import it and it imports fsv-api modules, so fsv-api ↔ fsv-views is a cycle. Move `views` into fsv-api, split `state_flow`, and put the attachment helpers in fsv-api. |
| 3 | Process-global `RwLock` config | **change** | Edition 2024 makes `set_var` unsafe, and parallel `cargo test` plus env rebinding races. Pass an explicit `Ctx` and allow a global only at the binary edge. |
| 4 | `serde_json::Value` everywhere | **keep** for server payloads, **change** for access helpers | `#[serde(flatten)]` reorders keys and breaks hashes and re-emitted JSON. Add Python-truthiness accessors and type only the Python dataclasses. |
| 5 | Single `fsv_core::Error` | **change** | `raise X from None` still sets `__context__`, and `exit_code_for` walks it, so the swallowed ApiError must be stored. Partial failures need structured payloads. |
| 6 | Rich markup strings | **keep** the markup strings, **change** the internals | `pages.py` is a markup contract shared by CLI and TUI (~450 sites). Parse once into `Span`/`Line` in fsv-core; both front ends consume spans. |
| 7 | Session storage | **keep** Python's exact matrix | `store=keychain` written on Windows would make Python call `security`. Any Windows Credential Manager backend gets a new name later, as a divergence. |
| 8 | YAML via yaml-rust2 + own resolver | **keep**, with a spec of eight behaviors | Merge keys stay literal, duplicate keys are last-wins, explicit tags are honored, keys can be bool/None, single document only. All verified against Python. |
| 9 | Name cleanups | **change** internal names only | Journal `kind`/action strings, cache filenames, config keys, draft keys, env vars and hashed-dict keys are load-bearing; Rust module paths are free. |
| 10 | Missed risks | **change** | Journal dir hash on Windows (`canonicalize` → `\\?\`), raw redirect-following downloads, `update_cookie` race, CRLF on Windows, per-call-site `json.dumps` flags. |

---

## 1. Blocking client vs async — keep blocking

**Evidence.**
- Every TUI load is `@work(thread=True, exclusive=True, group=...)` (`tui/app.py:279, 423, 476, 927, 1177, 1396, 1426, 2297, 2339, 2387`). Textual's `exclusive` only marks a *thread* worker cancelled; it cannot interrupt a running `httpx` call.
- Staleness is handled entirely by tokens. `_list_epoch` is captured at start and compared in `_accept_list` / `_list_failed` (`tui/app.py:281, 289, 339`). `_search_seq` is captured and compared (`1179, 1194`). `formui/widgets.py:433-470` uses `exclusive=True` plus `_stale()`.
- `Client.keepalive()` (`client.py:498`) has **no callers** in the Python tree, so it imposes no requirement on the TUI design.
- `cache.refresh_async` (`cache.py:127`) spawns a daemon thread even in the CLI. `std::thread::spawn` has the same exit semantics, because the process exits without joining it.

The async alternative would cost a tokio runtime in the CLI's primary path, `async` signatures through fsv-api/fsv-flows, and a sync facade for the form engine. None of that buys parity, since Python cannot cancel either.

**Keep, with these client-shape requirements (fsv-core, now):**
- `Client: Send + Sync`, shared as `Arc<Client>`. The cookie jar and CSRF token go behind a `Mutex`; no `&mut self` methods.
- **TUI cancellation rule:** don't cancel; use generations, as Python does. Each worker group owns an `Arc<AtomicU64>`. The spawner captures the value, the worker sends `(gen, Result)` over a channel to the UI loop, and the UI drops the result if `gen` is no longer current. Optionally a worker may check the generation *before* each additional page in multi-page loops (`views.collect_pages`). That check never fires in Python, so it's an optimization, not parity.
- Timeouts: httpx `timeout=30.0` is per phase (connect/read/write/pool), but reqwest `ClientBuilder::timeout` is a **total** deadline. Use `connect_timeout(30s)` plus a per-read timeout. Never set a 30 s total deadline, or large attachment uploads and downloads will fail where Python succeeds.
- Redirects: see item 10b. This is the one client feature the current doc gets wrong.

## 2. Crate cut — change

The verified edges that break the proposed cut:

| Edge | Python | Problem |
|---|---|---|
| `change_tasks` → `views.load_tab` | `change_tasks.py:216` | fsv-api → fsv-views |
| `change_approvals` → `views.load_tab` | `change_approvals.py:173` | fsv-api → fsv-views |
| `change_associations` → `views.collect_pages` | `change_associations.py:56` | fsv-api → fsv-views |
| `ticket_notes` → `views.load_conversations` | `ticket_notes.py:22` | fsv-api → fsv-views |
| `views` → `change_associations.AssociationReader`, `create.match_planning_row`, `service.get_item`, `formmeta.*` | `views.py:758, 801, 878, 1207, 1467` | fsv-views → fsv-api (the reverse), so this is a **cycle** |
| `views` → `render._requested_item_rows` | `views.py:1431` | fsv-api → fsv-views once views moves |
| `render.detail_panel` → `state_flow.render_state_diagram` | `render.py:318` | `state_flow` mixes an HTTP fetch with Rich rendering |
| `tui/actions` → `cli_sr._attachment_rows`, `cli_sr._download` | `tui/actions.py:172, 202` | TUI → CLI |
| `formui/session` → `cli_sr.render_sr_template` | `formui/session.py:695` | formui → CLI |
| `completion_gen` → `cli` | `completion_gen.py` (local) | fsv-completion → fsv-cli while fsv-cli → fsv-completion |

`views.py` has zero Rich imports and zero markup (count: 0). It is HTTP loaders plus value formatting (`load_tab`, `load_detail`, `collect_pages`, `ListPage`, `Detail`, `fmt_when`, …), so it is API-layer code.

**Required moves (smallest set):**
1. **`views.py` → fsv-api** in full, as `fsv_api::site` (see item 9 on the name). The intra-crate `views ↔ change_associations ↔ create` cycle is then fine.
2. **`render._requested_item_rows`, `_requested_item_field_defs`, `_field_value_text`** (pure text) → `fsv_api::site`. `render.py`'s Rich parts stay in fsv-views.
3. **Split `state_flow`**: `get_state_flow` and `_try_get_approvals` → `fsv_api::state_flow`; `render_flow`, `render_compact`, `render_state_diagram` (Rich Console/Table) → `fsv_views::state_flow`.
4. **New `fsv_api::attachments`**: `cli_sr._attachment_rows`, `cli_sr._download`, `create._attachment_url`, plus `catalog.is_file_value`, `catalog.safe_target`, `catalog.domain_url` (all pure or HTTP). `catalog` in fsv-flows re-uses them.
5. **`render_sr_template`** → `fsv_flows::catalog` (already planned; confirmed it depends only on `catalog`, `drafts`, `yamlio`).
6. **fsv-completion takes `&clap::Command`** as an argument (fsv-cli passes `Cli::command()`). It must not depend on fsv-cli. The completion data helpers (`_cached_groups`, `_remote_user_values`, `_completion_network`) stay in fsv-api as planned. `_completion_network` reads `config.json` directly (`completion.py:26`), so give it a cheap fsv-core reader.

**formmeta → fsv-api: confirmed.** It imports only `cache, config, errors, forms, safety` (no flows), and `views` (now fsv-api) needs `ClientResolver`, `existing_from_record`, `name_existing`, `workspace_from_fields`, `load_meta`, `_cached`. Putting it in fsv-flows would recreate the cycle. fsv-api therefore depends on fsv-forms.

**Does fsv-views need fsv-flows? No.** `pages.py` imports only `views` and `task_status`; `render.py` imports `resources` and `state_flow`. After the moves:

```
fsv-py ← fsv-forms ← fsv-core ← fsv-api ← fsv-flows ← fsv-formui ← fsv-tui ← fsv-cli
                                  ↑  ↖───── fsv-views ←──────────────┘      ↑
                                  └──────── fsv-completion ─────────────────┘
```

Here fsv-core → fsv-forms is optional; it is only needed if you take option A in item 5. fsv-views depends on fsv-api only. fsv-tui depends on fsv-views, fsv-formui, fsv-flows and fsv-api.

Other locals checked and fine under the new cut: `catalog → create`, `change_flow → create`, `create → change_approvals/associations`, `formui.screens → catalog`, `tui/app → catalog, create, change_tasks, ticket_notes, task_status, completion`.

## 3. Global state — change to an explicit `Ctx`

**Why the global breaks:**
- `cargo test` runs a crate's tests on parallel threads in one process. Python tests do "set env + `config.reload()`" per test. Ported literally, every test would mutate one process-wide `RwLock` and the process environment.
- Edition 2024 (the workspace edition) makes `std::env::set_var` `unsafe`, because concurrent `getenv` is UB on some platforms.
- A test mutex works only if *every* test in the binary takes it, including tests that read config indirectly through `journal::record` or `cache::load`. That is unenforceable, and one forgotten test gives flaky failures.

**Evidence that an explicit context is cheap:** the domain modules hardly touch config. I counted 0–2 `config.*` reads per module, and only `DOMAIN`, `API_INT`, `CACHE_DIR`, `cache_domain_key` and `load_config`. Nearly every function already takes `c`/`client` explicitly. The hidden readers are `journal`, `cache`, `session` and `formmeta._cache_path`.

**Rule (put this in ARCHITECTURE.md):**

```rust
// fsv-core
pub struct Paths { pub config_dir: PathBuf, pub cache_dir: PathBuf, pub state_home: PathBuf }
impl Paths {
    pub fn from_env() -> Paths;            // the ONLY place FSV_CONFIG_DIR/FSV_CACHE_DIR/XDG_STATE_HOME/HOME are read
    pub fn under(root: &Path) -> Paths;    // tests
}
pub struct Tenant { pub domain: String, pub api_v2: String, pub api_int: String, pub login_url: String }
pub struct Flags { pub verbose: bool, pub no_input: bool }
pub struct Ctx { pub paths: Paths, pub tenant: Option<Tenant>, pub flags: Flags, pub command_path: Option<String> }

impl Ctx { pub fn require_tenant(&self) -> Result<&Tenant, Error>; }  // = config.require_domain
pub struct Client { ctx: Arc<Ctx>, /* … */ }
impl Client { pub fn ctx(&self) -> &Ctx; }

journal::record(ctx: &Ctx, …); journal::read(ctx: &Ctx, …);
cache::load(path) / cache::save(path, …)        // paths computed by callers from ctx.paths
session::load_cookies(ctx: &Ctx) -> Result<…>
config::load(paths: &Paths) / config::save(paths: &Paths, …)
```

- Library crates (fsv-api, fsv-flows, fsv-views, fsv-formui) **never** read env or a global. They take `&Client` or `&Ctx`.
- A single `fsv_core::runtime::init(Ctx)` (a `OnceLock<Arc<Ctx>>`) is allowed in fsv-cli `main`, and fsv-tui and fsv-completion may read it. `set_domain`/`auth login --domain` builds a **new** `Ctx` and a new `Client`, mirroring Python's `reset_client()`, so the shared `Ctx` is never mutated in place. If something truly must change after init, hold `ArcSwap`/`RwLock<Arc<Ctx>>` in fsv-cli only.
- Tests: `Ctx { paths: Paths::under(tmp.path()), … }` and no `set_var`. Add a CI grep that fails on `env::var(` outside `Paths::from_env` and fsv-cli.

## 4. `serde_json::Value` vs structs — keep Value for server data; add accessors; type the dataclasses

**Keep `Value` for server records** (tickets, changes, tasks, approvals, schema fields, catalog items):
- `#[serde(flatten)]` + `preserve_order` serializes the typed fields first and the flattened map after. That changes key order in `-o json` output, cache `data`, and anything fed to `plan_hash` (sorted, so order is irrelevant there, but the *values* must round-trip exactly).
- Python code copies server records and adds keys (`{**a, "source": "request"}` in `cli_sr.py:755`), which needs an open map anyway.
- serde_json with `preserve_order` already matches Python's duplicate-key behavior (last value wins, first position kept).

**The real duplication risk is Python truthiness, not dict access.** There are about 170 `x.get(k) or {}` / `or []` / `or ""` patterns (views 52, create 29, service 19, catalog 15, formmeta 13, …). These treat `0`, `""`, `[]`, `{}`, `false` and `null` as absent. Hand-written Rust `.get(k).and_then(Value::as_object)` gets `0`/`""`/`false` wrong in a different way at each site. **Required** (fsv-py or fsv-core, before fsv-api starts):

```rust
pub fn truthy(v: &Value) -> bool;                       // Python bool()
pub trait ValueExt {
    fn get_truthy(&self, k: &str) -> Option<&Value>;    // x.get(k) if truthy
    fn obj_or_empty(&self, k: &str) -> &Map<String, Value>;   // x.get(k) or {}
    fn list_or_empty(&self, k: &str) -> &[Value];       // x.get(k) or []
    fn py_str(&self, k: &str) -> String;                 // str(x.get(k)) incl. "None"/"True"/float repr
}
```

**Type these (they are Python dataclasses or file formats, not server records):**
- fsv-api `site` (ex-`views`): `Missing`, `SavedView`, `Column`, `ListPage`, `Tab`, `LifecycleState`, `Lifecycle`, `PlanningEntry`, `Prop`, `TabData`, `Detail`. These hold `Value` for the record itself.
- fsv-api `formmeta`: the meta/resolver types. fsv-flows: `catalog::ItemMeta`, `SRPlan`, the `change_flow`/`ticket_flow` plan structs, and the plan structs in `change_tasks`/`approvals`/`approval_actions`/`associations`/`ticket_notes`.
- fsv-core file formats, as structs with **field order equal to Python's dict order**: cache envelope (`_kind, saved_at, checked_at, changed_at, hash, etag, latency_ms, fetch_count, data: Value`), journal HTTP entry and plan entry (`journal.py:117-130, 180-198`), the session file (`saved_at, domain, cookies`), and the argon envelope.
- `config.json` stays `Value`: unknown keys must survive, and it is written `sort_keys=True`.

Numbers: Python ints are unbounded, and serde_json without `arbitrary_precision` turns integers above u64 into f64. Freshservice ids fit, so accept this and note it in PARITY.md. Don't enable `arbitrary_precision`, because it changes `Number` semantics underneath fsv-py.

## 5. Errors — change: keep one enum, but store the context and structured failures

**What a flat enum would lose (verified):**
- `exit_code_for` walks `exc.__cause__ or exc.__context__`. `raise TaskError(...) from None` suppresses display but **keeps `__context__`**. I ran it: a `TaskError` raised inside `except APIError(409)` exits **4**, and inside `except APIError(500)` exits **5**. The same holds for `NoteError`, `ApprovalError`, `TaskError` (RuntimeError family, around 20 `from None` sites) and for any exception raised while handling an `APIError`/`SessionError`.
- Order matters: `ValueError` is checked **before** the context. So `AssociationError(ValueError)` exits **2** even when the swallowed error was an `APIError`. `FormError` is not a `ValueError`, so `exit_code_for` gives 1 (or the context's code); CLI sites special-case it to 2 (`form_cli.py:68`, `cli_sr.py:70`, `cli.py:4039, 4583`, `change_cli.py:427`).
- `PartialFailure` carries `created_id`, `uploaded`, `steps` and `error`. `SendFailure` carries `step`, `uploaded` and `error`, and `cli_sr.py:571` computes the exit code from **`e.error`**. The CLI prints the created id and uploaded attachment ids. That is user-visible recovery information, so it can't be flattened to a string.
- `_err_exc` prints `" ".join(str(exc).split()) or type(exc).__name__`. With `--verbose` it appends `APIError.detail()`.

**Required shape (fsv-core):**

```rust
pub enum Error {
    Api(ApiError),                                   // 409 / dup-regex → 4, else 5
    Session(String),                                 // 3
    Usage { message: String, context: Option<Box<Error>> },   // ValueError family → 2, context ignored for the code
    Exit { code: i32, message: String },             // FlowExit
    Failed { kind: &'static str, message: String, context: Option<Box<Error>> },
                                                     // RuntimeError/Exception family (TaskError, NoteError, ApprovalError,
                                                     // FormError, EditorAbort…): code = context's code if Api/Session, else 1
    Partial(Box<PartialFailure>),                    // { created_id: Value, uploaded: Vec<Value>, steps: Vec<Value>, error: Error }
    Send(Box<SendFailure>),                          // { step, uploaded, error: Error }
    Io(std::io::Error),
    Aborted,
}
impl Error {
    pub fn exit_code(&self) -> i32;                  // exact port of exit_code_for incl. recursion
    pub fn with_context(self, swallowed: Error) -> Error;   // the `raise X from None` inside `except` idiom
}
```

- `Display` never includes `context` (that is what `from None` means). Only `exit_code()` and `--verbose` see it.
- Convention for fsv-api/fsv-flows: a typed domain error is fine internally, but every place Python does `except APIError: raise X(...) from None` must call `.with_context(api_err)`. Port tests must assert exit codes for 409 and 500 contexts.
- FormError structure (field label, kind, ambiguous candidates) stays in `fsv_forms::FormError`. fsv-forms cannot see fsv-core, and the orphan rule forbids writing `impl From<FormError> for fsv_core::Error` in fsv-api. Either (A) let fsv-core depend on fsv-forms (pure, no cycle) and add `Error::Form(FormError)`, or (B) convert to `Failed { kind: "FormError", … }` in fsv-api with an explicit function. **Pick A.** CLI sites that Python maps to 2 match on `Error::Form` and are then explicit.

## 6. Terminal markup — keep markup strings, parse once into spans

**Evidence:** `pages.py`'s docstring makes the contract explicit: *"Every function returns Rich markup … so the CLI prints them and the TUI drops them into its widgets unchanged."* The TUI consumes `pages.lifecycle_markup`, `reporter_markup`, `tab_bar`, `main_lines`, `notes_lines`, `side_lines` and `tab_lines` (`tui/app.py:1497-1583`). The counts of markup-bearing lines are much higher than the brief's ~100:

| file | markup lines |
|---|---|
| cli.py | 130 |
| tui/app.py | 149 |
| pages.py | 74 |
| cli_sr.py | 37 |
| form_cli.py | 22 |
| formui/screens.py | 16 |
| change_cli.py | 15 |
| render.py | 13 |
| review.py | 6 |
| formui/widgets.py | 5 |

That is about 450 in total. Converting every call site to a `Span` builder roughly doubles the port effort for those files and makes side-by-side review against Python harder. Keeping the strings is cheaper.

**Required change:** fsv-core owns **one** markup parser, and nothing else re-parses markup:

```rust
pub struct Style { fg, bg, bold, dim, italic, underline, reverse, link: Option<String> }
pub struct Span { pub text: String, pub style: Style }
pub type Line = Vec<Span>;
pub fn parse(markup: &str) -> Vec<Line>;   // Rich tag grammar: [style], [/style], [/], [link=…], \[ escapes
pub fn escape(s: &str) -> String;          // byte-exact rich.markup.escape (only escapes '[' that starts a tag-like run, and trailing '\')
pub fn plain(markup: &str) -> String;      // = what non-TTY / NO_COLOR prints
```

- The CLI renders spans to ANSI or plain text. fsv-tui maps `Span` → `ratatui::text::Span` in a ~30-line adapter inside fsv-tui, so fsv-core does not depend on ratatui.
- `term::Table` takes cells as markup, measures `plain()` width with `unicode-width`, and folds like Rich's `overflow="fold"`.
- Be lenient where Rich is strict: Rich raises `MarkupError` on a stray `[/red]`. Render it literally instead and log in debug.
- Golden tests for `escape` and `plain` against Python. A wrong `escape` is a correctness bug, because server text containing `[x]` disappears.

**Parity users actually see:** byte-exact output for `--json`/`-o json|csv|tsv`, YAML drafts and print-plan, which scripts consume. For human output, the content, order, wording and column set must match. Box glyphs, colors, wrapping, Rich auto-highlighting (numbers/URLs colored on a TTY) and Rich `:emoji:` substitution are allowed to differ. List those last two in PARITY.md divergences. Non-TTY tables at width 80 are what Python tests assert substrings against, so port those assertions as `contains`, not byte equality.

## 7. Session storage — keep Python's exact matrix

- `current_backend()` (`session.py:47-56`) accepts exactly `file|argon|keychain` from `<config>/store`. Unknown values are ignored, and the fallback is `session.json` exists → `file`, else darwin → `keychain`, else `file`.
- Mapping Windows Credential Manager onto `keychain` breaks interop: Python on the same Windows machine reads `store=keychain` and shells out to `security`, which fails, so every Python command errors with "no session".
- A new name (`wincred`) is ignored by Python, which then reports "no session". That's acceptable only as an explicit, documented divergence after parity ships. **Do not add it in this wave.**
- Port exactly: `security -i` with the command on stdin (the secret never appears in argv), base64 payload with legacy plain-JSON decode (`_keychain_decode`), service `fsv`, account `fsv-session`, `FSV_NO_KEYCHAIN`, the `_keychain_delete()` on file/argon save, and the `store` file write on every save.
- Windows users default to plaintext `file`, which is Python's behavior too. `fsv auth` help could point Windows users to `argon`. That's documentation, not behavior.

## 8. YAML — keep yaml-rust2 + own resolver, to this spec

Verified with `fsv.yamlio.loads` on Python:

| Input | Python result | Rust must |
|---|---|---|
| `other: {<<: *b, y: 2}` | `{'<<': {'x': '1'}, 'y': '2'}`. The merge resolver is deleted along with the others, so **no merge**. | treat `<<` as a plain string key (yaml-rust2 does not merge; keep it that way) |
| `a: 1\nb: 2\na: 3` | `{'a': '3', 'b': '2'}` | last value wins, first position kept (`IndexMap::insert`), no error |
| `a: !!int "3"`, `b: !!bool yes`, `c: !!str true` | `3`, `True`, `'true'` | honor explicit tags: `!!str`, `!!int`, `!!float`, `!!bool` (YAML 1.1 bool words incl. yes/no/on/off), `!!null`. Reject other tags with a ValueError, as SafeLoader does for `!!python/*`. |
| `true: x`, `~: y`, `1: z` | keys `True`, `None`, `'1'` | the parser yields typed keys. `drafts.draft_from_data` then does `str(k)`, so answers get `"True"`/`"None"` (use `fsv_py::py_str`, not `"true"`) |
| `a: &x [1,2]\nb: *x` | both `['1','2']` | aliases expand (copy) |
| two documents | `ValueError("YAML error: expected a single document …")` | error, exit 2 |
| empty text | `None` → empty Draft | `Null` |
| `a: >` folded, plain multi-line | `'one two\n'`, `'plain continued'` | standard folding; yaml-rust2 matches |

- Implicit resolution applies **only to plain scalars**: null = `~|null|Null|NULL|<empty>`, bool = `true|True|TRUE|false|False|FALSE`, everything else is a string. Quoted scalars are always strings.
- Error message text will differ from PyYAML (line/column wording). It goes to stderr with exit 2. Record it as a divergence.
- PyYAML rejects C0 control characters other than tab, LF and CR ("unacceptable character"), while yaml-rust2 may accept them. Low impact; record it.
- Features the drafts writer emits (`drafts.py`, `cli_sr.render_sr_template`, `yamlio.scalar/flow_list/block`): plain scalars, JSON-quoted strings, flow lists, `|-` blocks, `#` comments and empty values. Users hand-edit these files, so the table above is the real contract. Commit a corpus of about 40 inputs run through Python under `port/golden/yaml/`.

## 9. Names

**Load-bearing (never change; constants in one place):**
- Journal plan actions, giving `kind = "plan:<action>"`: `create` (changes **and** SRs share it; `catalog.py:420`), `ticket_create`, `ticket_note`, `change_task_create`, `change_approval_request`, `change_approval_cancel`, `change_approval_respond`, `change_ticket_association`. Duplicate detection reads journals written by the other implementation.
- Journal HTTP kinds `"<verb>:<resource>"`, `note`, `reply`, `upload`. Step whitelist: `create, update, attachments, upload, place-request, after`; anything else becomes `other`. Events `started|failed|done`; `error: "details omitted"`.
- Cache paths: `schema/<dom>--<name>.json`, `filters/<dom>--<name>.json`, `forms/<dom>--<name>.json`, `groups--<dom>.json`, `lookup/<md5>.json`, `*.lock`. Envelope `_kind` values. `cache_domain_key` fallback `default`.
- Resource names `tickets|changes|problems`, which appear in filenames and `config.json` `defaults.<res>.where`.
- Config keys: `domain`, `defaults.<res>.where`, `completion.network` (values `on|true|"true"|"1"`). The `store` values `file|argon|keychain`. Session keys `saved_at, domain, cookies`. Argon envelope keys `backend, kdf, params, …`. Keychain `fsv`/`fsv-session`. `REQUIRED_COOKIES`.
- Draft YAML keys: `preset, requester, answers, description_md, description_html, attachments, planning, ack`; planning sub-keys `files, description_md, description_html`.
- Keys inside hashed dicts, e.g. `{"request": payload, "target": …}` in `catalog.py`. Renaming a struct field that feeds `plan_hash` silently changes hashes and defeats duplicate detection.
- JSON output keys and enum strings: attachment rows `source: "request"`, `status: downloaded|exists|rejected|error`, and so on.
- Env vars: `FSV_CONFIG_DIR, FSV_CACHE_DIR, XDG_STATE_HOME, FSV_NO_KEYCHAIN, FSV_COMPLETION_NETWORK, _FSV_COMPLETE, NO_COLOR, EDITOR/VISUAL`.

**Free to clean up:**
- Module tree `fsv_api::change::{tasks, approvals, approval_actions, associations}` and `fsv_api::ticket::notes`. Rename `views` → `fsv_api::site`, because "views" collides with Freshservice *saved views* (`SavedView`, `load_views`) that live in the same module.
- Types `TaskError`/`NoteError`/… become `Failed { kind }` per item 5.
- Rename `filters_path`/`ticket_filters` consistently, as long as the on-disk names stay.
- Hidden legacy groups (`changes assets legacy`, etc.) stay hidden clap subcommands with identical flags, implemented as thin aliases over the current handlers.

Keep one `const ACTION_*` / `KIND_*` module in fsv-core so the strings exist exactly once.

## 10. Risks the questions missed

**a. Journal directory hash on Windows and macOS.** The path is `sha256(os.fsencode(CONFIG_DIR.resolve()))` (`journal.py:31`).
- Rust `std::fs::canonicalize` returns `\\?\C:\Users\…` on Windows, and it **fails** when the directory doesn't exist, whereas Python's `resolve()` is non-strict.
- A different hash puts Rust in a different journal. That silently disables "this task was sent or its outcome is unknown; do not resubmit" (`change_tasks.py:283`) and the 24 h duplicate check across Python and Rust, which risks duplicate production writes.
- **Fix:** implement `py_resolve()` (non-strict, symlinks resolved on macOS `/var`→`/private/var`, no verbatim prefix; e.g. the `dunce` crate) and golden-test it on both OSes.

**b. Redirect-following downloads bypass the client.**
- `c._client.get(url, follow_redirects=True)` appears at `cli_sr.py:794`, `create.py:352, 377, 718, 1084` and `cli.py:1456`. It uses the raw httpx client (no retry, journal or `_check_response`), and the CSRF probe uses `follow_redirects=True` at `client.py:471`.
- reqwest's redirect policy is per-`Client`, not per-request, and automatic following skips your hand-rolled `Set-Cookie` handling on intermediate hops.
- **Fix in fsv-core now:** a manual redirect loop (max 20, like httpx) with a per-request `follow: bool`. Apply the cookie jar at every hop, and send Freshservice cookies **only** to the tenant host; attachment redirects go to S3/CDN. Expose `Client::fetch_raw(url, follow) -> Response` for the download sites.

**c. `session.update_cookie` race.** It does an unlocked read-modify-write of `session.json`/keychain on every rotated cookie (`session.py:240`), with `_SESSION_CACHE` as a module global. Rust TUI workers run truly in parallel, so guard it with a process `Mutex` inside fsv-core `session`.

**d. Windows newlines.**
- Python `atomic_write_text` opens `"w"` without `newline=""` (`config.py:49`), so on Windows `config.json`, `session.json`, cache envelopes and drafts are written with **CRLF**, and Python's stdout translates `\n` → `\r\n`.
- Both implementations read either form, so interop is safe. But "byte-exact YAML drafts / `--json`" is only true on macOS.
- **Decide now:** write LF on all platforms and list it in PARITY.md divergences. Generate goldens on macOS only.

**e. `json.dumps` flags differ per call site.**
- `config.json`: `indent=2, sort_keys, ensure_ascii=True`.
- Cache: `indent=2, ensure_ascii=False`.
- Journal: compact default separators, `ensure_ascii=False, default=str`.
- Session payload: default separators `", "`/`": "` with `ensure_ascii=True`.
- `plan_hash`: `sort_keys, (",",":"), ensure_ascii=False, default=str` (`default=str` stringifies non-JSON values).

Each writer must name its `fsv_py::Dumps` flags explicitly. A shared "write JSON file" helper with one flag set would quietly change bytes.

**f. "Aborted" wording.** `_err_exc` prints `Aborted.` (`cli.py:279`), while click prompt aborts print `Aborted!`. ARCHITECTURE.md lists only `Aborted!`; keep both, per path.

**g. Empty-message fallback.** `str(exc) or type(exc).__name__` means Python class names (`TaskError`, `KeyError`, …) can reach stderr. That's another reason for `Failed { kind }` in item 5.

---

**Single change to insist on before fsv-api starts:** the crate-cut fix in item 2. Move `views.py` (plus `render._requested_item_rows` and the fetch half of `state_flow`) into fsv-api, and create `fsv_api::attachments`. Without it, the first fsv-api module (`change_tasks`, `change_approvals`, `change_associations` or `ticket_notes`) cannot compile against the planned crate graph. fsv-core should land the `Ctx`, error and redirect shapes (items 3, 5 and 10b) in the same pass, since nothing is written there yet.
