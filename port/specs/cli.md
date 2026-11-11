# fsv CLI layer — behavioral spec for the Rust port (`fsv-cli`)

Source of truth: `/Users/phusit.somb/lab/work/fsv/src/fsv/{cli.py, change_cli.py, ticket_cli.py, cli_sr.py, form_cli.py, __init__.py}`
plus the `register()` command factories in `change_tasks.py`, `change_approvals.py`,
`change_approval_actions.py`, `ticket_notes.py`. Flag/default/help inventory: `port/cli-surface.txt`.
All `file:line` refs are to the Python tree (typer 0.26.7 with vendored click 8.2).
"stdout"/"stderr" = Rich `console` / `err` (`err` = `Console(stderr=True, style="dim")`).
`[x]...[/]` = Rich markup (ANSI on a TTY, stripped otherwise; Rich soft-wraps at 80 cols when not a TTY —
ARCHITECTURE lists no-hard-wrap as a deliberate divergence).

---------------------------------------------------------------------------------------------------

## 1. Process-level behavior (typer/click quirks)

### 1.1 Entry, global state
- `fsv = "fsv:main"` → `fsv/__init__.py:8 main()` → `cli.app()` (standalone mode).
- Root callback `cli.py:225 main(no_input, verbose, version)`:
  - `--version/-V` (is_eager): prints `fsv {importlib.metadata.version("fsv")}` (currently `fsv 2026.5.29`,
    fallback `fsv 0.1.0`) to **stdout**, exit 0. Only valid before the subcommand (`fsv changes ls -V` → usage error).
  - Sets globals `NO_INPUT`, `VERBOSE`, then `client.set_verbose(verbose)`.
  - If no subcommand but some root flag was given (`fsv -v`, `fsv --no-input`): prints root help to stdout, **exit 0**.
- `no_args_is_help=True` on the root and every group: bare `fsv`, `fsv changes`, `fsv changes assets`, `fsv auth`, …
  print help to **stdout** and **exit 2** (click 8.2 `NoArgsIsHelpError`). `-h/--help` → exit 0.
  (`context_settings={"help_option_names": ["-h","--help"]}` is set on the root only, but click inherits it.)
- `--install-completion` / `--show-completion` (typer built-ins, root only): `cli.py:179 _patch_typer_shell_detection`
  makes shell detection fall back to `$SHELL` basename (`sh`→`bash`, accepted: bash zsh fish powershell pwsh) and makes
  the script come from `completion_gen.build_script(shell, prog)` (fallback to typer's own script if that raises).
- Env `_FSV_COMPLETE` set → completion mode: Rich disabled, `console`/`err` are null sinks (`render.py:9`);
  typer's completion protocol answers. Dynamic completers otherwise run via `python -m fsv._complete` (owned by `fsv-completion`).

### 1.2 Root-flag reordering (`cli.py:50 _reorder_root_flags`)
Monkeypatch (`cli.py:140`): **every** typer group is `FSVGroup` and every command `FSVCommand`.
`FSVGroup.parse_args` rewrites args before click parses them:
- Walk tokens; after `--` everything is passthrough. Once a token matching a sub-command name of the *current* group has
  been seen (`command_started`), any later literal `-v` or `--verbose` token is removed and prepended to the arg list.
- Option values are skipped using the current command's param arity (`_option_arity`: 0 for flags or tokens containing `=`,
  else `nargs`); so `--body -v` keeps `-v` as the value. Arity uses the command reached so far (group or leaf).
- Not handled: bundled shorts (`-vy`), `--verbose=x`. `--no-input` and `-V` are **not** moved (`fsv changes ls --no-input`
  → "No such option" exit 2 because `ls` has no `--no-input`).
- Because subgroups also run it, the rewrite is idempotent (nothing left to move below the root).
- Help: `_FSVHelpMixin.format_help` appends a clone of the root `-v/--verbose` option under a **"Global options"** panel in
  every sub-command's `--help` (not `--no-input`).
- Rust: pre-process `std::env::args()` the same way before clap; mark `-v/--verbose` `global = true` is NOT equivalent
  (clap global would also accept it before `--` passthrough and in option-value position).

### 1.3 Legacy nested routing (`cli.py:126 ChangeLegacyGroup`)
Used by `changes assets` and `changes associations` groups (`invoke_without_command=True`, `no_args_is_help=True`).
If `args[0]` does not start with `-`, is not a known sub-command, and `parse_id(args[0], CHANGES)` succeeds
(`CHN-<n>`, `chn-<n>`, or bare digits), set `ctx.meta["fsv_legacy_nested"]=True` and prepend `legacy` (a hidden command).
The group callback then prints to stderr (no highlight):
`[yellow]deprecated[/]: use \`fsv changes assets <ls|search|add|remove|pick|categories> ...\``
(associations: `... <ls|search|add|remove|pick> ...`). An option first (`fsv changes assets -q x CHN-1`) is not routed → usage error.

### 1.4 Usage errors, enums, options
- All click usage errors (unknown option/command, missing arg, bad int, bad Choice, `typer.BadParameter`) print
  to **stderr**: `Usage: fsv changes ls [OPTIONS]` / `Try 'fsv changes ls -h' for help.` / a Rich box titled `Error`
  with e.g. `Invalid value for '--output' / '-o': 'xml' is not one of 'table', 'json', 'csv', 'tsv'.`,
  `No such command 'nope'. Did you mean 'notes'?`, `Missing argument 'ID'.`,
  `Invalid value for 'TASK_ID': 'abc' is not a valid integer.` → **exit 2**.
- Enums (case-sensitive choices): `OutputFormat` table|json|csv|tsv (`-o/--output` on resource read commands, `search`,
  `changes create`, `tickets create`); `SearchSort` relevance|created|modified (`--sort`); `SortOrder` asc|desc (`--order-type`).
  `sr *` and `form *` `--output` is a **free string**: `"json"` → JSON, anything else → table (no validation).
  `--json` on read commands = alias for `-o json` (wins over `-o`); on write commands it means "emit structured result".
- Multi options (repeatable, `multi=True`): `--where/-w`, `--set`, `--file/-f`, `--attach/-a` (changes) / `--attach` (sr),
  `--planning` (changes download), `--add`, `--remove`, `--approver` (required, ≥1), `--notify-email`,
  `config defaults set --where` (required). Variadic positionals (`nargs=-1`, required ≥1): `assets add/remove ASSET...`,
  `associations add/remove TICKET...`.
- `--x/--no-x`: `--pager/--no-pager` (default on); `--backup-replaced/--no-backup-replaced` (tri-state, default `None`).
- Hidden: commands `changes assets legacy`, `changes associations legacy`; options `changes create --optional/-O`, `--all`
  (accepted, ignored).
- Metavars: typer shows `ID_` for `id_` args without metavar (activity/tasks/approvals/notes/add-note/conversations/reply,
  tickets approvals/associations); `ID`, `CHANGE_ID`, `TASK_ID`, `ASSET`, `TICKET`, `QUERY`, `APPROVAL_ID`, `GROUP_ID`,
  `ITEM`, `SR-ID`, `DIR` where set.

### 1.5 Abort / Ctrl-C / uncaught exceptions (typer `core.py:180-242`)
| event | output | exit |
|---|---|---|
| `typer.Abort` (`confirm(..., abort=True)` answered no, or EOF at a prompt) | stderr `Aborted.` (EOF also prints a newline first) | 1 |
| `KeyboardInterrupt` not caught by a handler | **nothing** | **130** |
| `typer.Exit(n)` / `SystemExit(n)` | — | n |
| any other uncaught exception | Rich traceback panel on stderr + `ExcType: message` | 1 |
`_err_exc` has an Abort/KeyboardInterrupt branch (`Aborted.` exit 1) but no caller passes those in practice (dead).
Known traceback paths (parity = keep exit 1; message differs): see §5 "bugs".

### 1.6 Prompts (click semantics; port to `fsv_core::term`)
- `typer.prompt(text, default=d)` renders `text [d]: ` (`show_default=False` → `text: `); `type=int` re-prompts with
  `Error: 'x' is not a valid integer.`
- `typer.confirm(text, default=False)` → `text [y/N]: `; `default=True` → `[Y/n]`; `abort=True` → Abort (exit 1 `Aborted.`).
- Prompts/confirms are written to **stdout** by click (except our own preceding lines printed via `err`).

### 1.7 Shared helpers (`cli.py`)
| helper | behavior |
|---|---|
| `_err(msg, code=2)` `:269` | stderr `[red]error[/red]: {msg}` (**msg NOT markup-escaped**), `raise typer.Exit(code)` |
| `_err_exc(exc)` `:275` | msg = whitespace-collapsed `str(exc)` or type name; if `VERBOSE` and `APIError` append `"\n"+exc.detail()`; escape markup; `_err(msg, errors.exit_code_for(exc))` |
| `exit_code_for` (errors.py) | `SessionError`→3; `APIError` 409, or 400/422 whose body matches `/already (exists\|been taken\|associated)\|duplicate/i` →4, other APIError→5; `ValueError`(and subclasses e.g. `AssociationError`)→2; else follow `__cause__`/`__context__` if APIError/SessionError; else 1 |
| `_flow_exit(e)` `:288` | `FlowExit(code, message)`: if message: stderr `[red]error[/red]: {escaped}` when code≠0 else plain escaped message; exit `code` |
| `_api(fn)` `:296` | run fn; `SessionError/APIError/ValueError` → `_err_exc` |
| `_client()` `:303` | `session.session_age_hours() is None` → stderr `[red]error[/red]: no session; run \`fsv auth login\``, **exit 3**; `get_client()` raising `SessionError` → `error: {e}` exit 3, `RuntimeError` (e.g. no domain) → exit 1 |
| `_cid(id, res)` `:363` | `resources.parse_id`; ValueError → exit 2 (`invalid id: X` / `prefix XYZ not valid for changes (expect CHN)`; tickets expect `INC or SR`) |
| `_no_input(local)` `:166` | `NO_INPUT or local` (root flag merged with per-command `--no-input`) |
| `_interactive()` `:578` | `not _no_input() and stdin.isatty()` |
| `_confirm_writes(risks, subject, yes, no_input)` `:718` | no risks or `--yes` → proceed; no-input or non-TTY stdin → `_err("{subject}: r1; r2. Pass --yes to confirm.")` exit 2; else stderr `[bold]{subject}[/bold] will:` + `  - {risk}` lines, `confirm("Proceed?", default=False, abort=True)` |
| `_emit_dry_run(res,cid,body,action="update",risks)` `:759` | `emit_json({"dry_run":true,"action":…,"id":format_id,"body":…[,"requires_confirmation":[…]]})` |
| `_edit_body(data, hint, no_input)` `:316` | no-input → exit 2 `editor disabled by --no-input; use --dry-run/--dry or provide non-interactive flags`; `editor.edit_json`; `EditorAbort` → `_err_exc` (exit 1) |
| `_emit_fmt(raw, flat_rows, fmt, json_out)` `:403` | json → `emit_json(raw)`; csv/tsv → `render.emit_delimited(flat_rows, sep)`; table → returns False (caller draws table) |
| `_pick_lookup(kind, value, results, exact_keys)` `:588` | none → `no {kind} match: {v}`; exactly one exact (safety.norm: NFC+casefold+ws-collapse) match → it; interactive: stderr `{problem}; candidates:` + `  i. label` (≤20), `prompt("Pick a number (0 to cancel)", type=int, default=0)`, out-of-range → `no {kind} chosen`; non-interactive → `_err("{problem}: a; b; … (+N more). Use the exact name/email or a numeric ID.")` (≤8 listed). problem = `multiple exact {kind} matches for 'v'` / `no exact {kind} match for 'v'` |

Machine-output emitters (byte parity required; keep each as-is):
| emitter | format |
|---|---|
| `render.emit_json` | `json.dump(strip_internal(obj), indent=2, default=str)` (**ensure_ascii=True**) + `\n`; drops keys `_resource,_search_result,_agent,_group` recursively |
| `render.emit_delimited` | `csv.DictWriter` (header = keys of first row, `\r\n` line endings, QUOTE_MINIMAL, `None`→empty, other values `str()` i.e. Python repr for dict/list/bool); **empty rows → prints nothing** |
| `change_cli`/`ticket_cli`/`cli_sr` plans & results | `print(json.dumps(x, indent=2, default=str, ensure_ascii=False))` |
| `tickets create --json` result | `print(json.dumps({"created":…,"plan_hash":…}, default=str))` — **single line, ensure_ascii=True** |
| `tasks-add`, `approval-*` | `typer.echo(json.dumps(x, indent=2))` (ensure_ascii=True, no default) |
| `tickets add-note` | `typer.echo(json.dumps(x, ensure_ascii=False, indent=2))` |
| `journal ls/tail --json` | one `json.dumps(row, ensure_ascii=False)` per line |
| `changes associations` review | `err.print_json(data=plan.review())` (Rich JSON, indent 2, ensure_ascii=False, dim, stderr) |
| `global search -o csv/tsv` | own `csv.DictWriter`; empty → writes a bare `\r\n` header line (differs from emit_delimited) |

`--expect-plan` semantics differ by command family (keep exactly):
- **prefix** (`forms.plan.plan_matches`: optional `sha256:` prefix stripped, case-insensitive, ≥12 hex chars, startswith):
  `changes create`, `changes update` (form path), `tickets create`, `sr create`. Mismatch → `FlowExit(4, "plan changed: expected E, computed H. Review again with --dry-run.")`; plan incomplete → `FlowExit(2, "--expect-plan given but the plan is incomplete: …")`.
- **full exact string** equality: `changes tasks-add`, `approval-request`, `approval-respond`, `approval-cancel`,
  `changes associations add/remove/pick`, `tickets add-note`. `--yes` there additionally *requires* it.

---------------------------------------------------------------------------------------------------

## 2. Commands

Legend: **Out** = stdout, **Err** = stderr. Exit codes default: success 0; `_cid` failure 2; `_client()` 3/1; `_api` errors 2/3/4/5.

### 2.1 Top-level

| command | handler | behavior |
|---|---|---|
| `fsv help [topic]` | `help_topic` cli.py:2830 | no topic: table "help topics" (Topic, Use=`fsv help NAME`) + `Use \`fsv COMMAND --help\` for command reference.`; topic casefolded must be in `HELP_TOPICS` (cli.py:2820, verbatim strings: auth, workflow, fields, scripting, comments) → print string; else exit 2 `unknown help topic 'x'; choices: auth, workflow, fields, scripting, comments` |
| `fsv search QUERY` | `global_search_cmd` :4671 → `global_search` :2531 | `_client()`; `c.fulltext_search("all", q, page, sort)`; keep results with truthy `result_type`. `--npages N`/`--all`: loop pages from `--page`, Err `  fetched {n}...` per page, stop when batch<30 or N pages fetched (no upper bound for --all). json → `emit_json(items)` raw; csv/tsv rows `{type,id,title,status,group}` from `_search_extract` (type label map `_SEARCH_TYPE_LABEL`, html-stripped title); table cols Type/ID/Title(≤70)/Status/Group + Err `{n} rows`. Wrapper catches `Exception` → `_err_exc` |
| `fsv setup` | `setup` :4874 | §3.1 |
| `fsv tui [--read-only]` | `tui` :4914 | `tui.launch(read_only)`; read-only = flag OR config `tui.read_only` ∈ on/true/1/yes (`tui/actions.py:20 resolve_read_only`) |

### 2.2 `auth`, `completion`, `config`, `cache`, `journal` — see §3 for flows

| command | handler |
|---|---|
| `auth login` | `auth_login` :2703 |
| `auth logout` | `auth_logout` :2774 |
| `auth status` | `auth_status` :2791 |
| `completion show [shell]` | `completion_show` :2950 |
| `completion install [shell]` | `completion_install` :2962 |
| `completion refresh` | `completion_refresh` :3003 → `cache_refresh()` |
| `completion doctor` | `completion_doctor` :3009 |
| `config list/get/set` | `config_list` :3047, `config_get` :3057, `config_set` :3070 |
| `config defaults show/set/clear` | `defaults_show` :3096, `defaults_set` :3112, `defaults_clear` :3124 |
| `cache status/refresh/clear` | `cache_status` :3146, `cache_refresh` :3197, `cache_clear` :3203 |
| `journal ls/tail/path` | `journal_ls` :3260, `journal_tail` :3272, `journal_path_cmd` :3282 |

### 2.3 Resource factory `_make_subapp(res)` (cli.py:3308) — `changes`, `tickets`, `problems`

Groups built for each `REGISTRY` resource (`changes` CHN, `tickets` INC/SR (`has_notes=False`), `problems` PRB).
Help strings use `singular = res.name[:-1]`. Command presence:

| command | changes | tickets | problems | handler → impl |
|---|---|---|---|---|
| ls | ✓ | ✓ | ✓ | `ls` :3332 → `list_resource` :1154 |
| get | ✓ | ✓ | ✓ | `get` :3380 → `get_resource` :1256 |
| activity | ✓ | ✓ | ✓ | :3395 → `activity_resource` :1347 |
| tasks | ✓ | ✓ | ✓ | :3406 → `tasks_resource` :1371 |
| fields | ✓ | ✓ | ✓ | :3884 → `fields_resource` :492 |
| lookup | ✓ | ✓ | ✓ | :3907 → `lookup_resource` :538 |
| views | ✓ | ✓ | ✓ (all have `filters_path`) | :3927 → `views_resource` :2433 |
| search | ✓ | ✓ | ✓ | :3937 → `search_resource` :2496 → `_fulltext_search` :2458 |
| url | ✓ | ✓ | ✓ | :3952 → `url_resource` :2580 |
| update | ✓ (own) | ✓ | ✓ | changes :3984; others :4322 → `update_resource` :2585 |
| download | ✓ (own) | ✓ `--conversations` | ✓ `--notes` | changes :4178; others `download_generic` :4345 |
| notes / add-note | ✓ | — | ✓ | :4598 → `notes_resource` :2248; :4616 → `note_resource` :2632 |
| conversations / reply | — | ✓ | — | :4631 → `conversations_resource` :2274; :4647 → `reply_resource` :2648 |
| tickets add-note | — | ✓ | — | `ticket_notes.register` :201 (reviewed flow) |
| create | ✓ | ✓ | — | changes :4535 → `change_cli.create_command`; tickets :4265 → `ticket_cli.create_command` |
| approvals | ✓ | ✓ | — | changes :3687; tickets :4299 → `ticket_approvals_resource` :2301 |
| associations | group | ✓ list | — | changes group :3772; tickets :4309 → `ticket_associations_resource` :2345 |
| assets (group) | ✓ | — | — | :3427 |
| tasks-add, tasks-update, tasks-delete, state, clone, approval-request/-respond/-cancel | ✓ | — | — | see 2.5 |

Wrapping: `fields`, `lookup` catch `Exception` → `_err_exc` (RuntimeError → exit 1 with message). `ls/get/activity/tasks/views/search/url/notes/conversations/approvals(tickets)/associations(tickets)/update(generic)/add-note/reply` have no wrapper beyond `_api`/`_cid`/`_client`.

#### `ls` (`list_resource`)
1. Default filters: `config.get_default_where(res.name)` unless `--no-defaults`; defaults whose field (`_where_field`, text before the first operator) also appears in explicit `--where` are dropped. Effective where = defaults + explicit.
2. `_client()`; `_build_query_hash` (`_api`): `--query-hash` (URL-decoded JSON, must be array: `invalid --query-hash JSON: …` / `--query-hash must be a JSON array`) + each `--where` via `_resolve_where` (§4 `query`) → `_merge_date_ranges` → `_merge_same_conditions` → `json.dumps(q, separators=(",",":"))`; `--or` → `{"any": q}`.
3. `--debug`: table "query" (field, condition, operator, type, value) + `api: internal /api/_/ with query_hash ({n} conditions, AND|OR)`; return (before `--or` validation).
4. `--or` and res≠changes → exit 2 `OR grouping not supported for {res}; use AND filters`.
5. View: `_pick_view` → `_saved_views` (`_load_views`: `int_get(res.filters_path)`, list under first key ending `_filters`; non-Session errors → `[]`). `--view` → `views.find_view(saved, name)`; ValueError → if no saved views pass the text through as id, else exit 2 with message. No `--view` and no where/query-hash → `views.default_view(saved)`.
6. Order: `--order-by` else view settings (`views.view_settings`) `order_by/order_type` else `created_at`/`desc`.
7. tickets: `views.ticket_headers(c)`.
8. Load: single page `service.list_page(res, page, per_page, filter_name=view.id, order_by, order_type, query_hash, or_grouping)`; `--all`/`--npages N`: per_page forced to 100, pages `page..page+(N or 100)`, Err `  fetched {n} so far...`, stop on short batch or `not more` (`views.list_meta`).
9. json → `emit_json(items)` (raw records). csv/tsv → rows from `views.list_row` over `views.list_columns(res, sch, headers)` (needs `views.current_user`, `views.user_tz`). table → `pages.list_header(ListPage)` + `[dim] · filtered: {where joined by ' ' or 'query hash'}[/]` when filtered, then `pages.list_table`; paged via `$PAGER` (default `less -R`, shell=True) when `--pager`, stdout TTY and `len*2+2+4 > terminal lines`; Err `{n} rows`.

#### `get` (`get_resource`)
- changes + `--stats` + json: `service.get_change_evidence(cid)` → `emit_json({**change, "planning_fields": by_name, "change_planning_fields": list, "planning_field_definitions": service.get_change_planning_field_definitions(sch), "main_attachments", "description_attachment_urls"})`. `--stats` otherwise ignored.
- Else `schema.load`; `service.get_item(res, cid, includes="requester,stats,phone,feedback,ticket_status,responder" for tickets)`; `views.load_detail(..., all_conversations)`; `--all-tabs` loads every tab except details; `--tab T` → `views.resolve_tab` (ValueError → exit 2) then `views.load_tab`.
- json: `emit_json({**record, ["requested_items"], "view": detail.to_json()})`. csv/tsv rows `{section: summary|properties, label, value}`. table → `pages.print_detail(console, d, tab, all_tabs)`.

#### `activity`, `tasks`
- `views.load_tab(c, res, {"id": cid}, "activity"|"tasks")`; `_read_warnings(td)`: Err `td.missing.note`, exit 1 if no data and no parts; Err each `td.notes`.
- activity: first `--limit` items; flat `{day, time, actor, content}` (day = Today/Yesterday/YYYY-MM-DD in local tz, time HH:MM or `--:--`); table form prints `\n[bold]{day}[/]` headers, `[dim]HH:MM[/]  [cyan]actor[/]  text`, sub-contents `      [dim]·[/] text`.
- tasks: flat `{id (human_display_id|id), status (status_name|status), group_agent, due (task_due_status|due_date|-), [system_env "sys / env" changes only], title[:60]}`; table TSK/Status/Group/Agent/Due/[System/Env]/Title.

#### `fields`, `lookup`
- fields: `--default`+`--custom` → exit 2 `use --default or --custom, not both`; `schema.load(force=--refresh)`; `--choices F` → `_find_schema_field` (`scope:` prefix `default:`/`custom:`; exact name/label wins, name preferred; ambiguous → exit 2 `ambiguous field 'x':\n  name  label  type  scope` ≤10; not found → `field not found: F; run \`fsv {res} fields\``); json emits raw choices, csv flat `{id,value,detail}`; else filter by search substring (name/label casefold), default/custom/required; json raw fields; table name/label/type/scope/req(`*`)/choices count.
- lookup kind casefold: `requester(s)` → `c.autocomplete("requesters", q, {"all_users":"true"})`; `agent(s)` → `autocomplete("agents", q)`; `group(s)` → `int_get("bootstrap/agents_groups").groups` name substring; else schema field choices filtered by substring (`unknown lookup kind or field: K` exit 2). Flat `{id: user_id|id, value, detail: email|details}`.

#### `views`
`_saved_views`; rows `{**SavedView.to_json(), "opens_by_default"}`; flat `{name,id,type:system|custom,default:yes|""}`; empty → Err `no saved views for {res} (the account does not list them)`; table (`▸` marker, View, Type, id) + Err `fsv {res} ls --view NAME  (exact name or a unique prefix)`.

#### `search` (per resource)
`service.search_items(q, entity=res.name, sort, page)` → `(results, totals)`; paging loop like global (Err `  fetched {n} / {total}...`, stop batch<30 or N). json = results with `_`-prefixed keys removed; flat/table `{id, subject[:80], status, priority(priority_label), group(_group)}`; Err `{n} rows (total: {total})`.

#### `url`
Out `https://{config.require_domain()}/a/{api_path}/{cid}` (no `_client()`; no domain → uncaught RuntimeError traceback exit 1).

#### generic `update` (tickets, problems) — `update_resource`
`schema.load`; body: `--status`/`--priority` via `_resolve_update_choice` (digits → int; schema choice label → id; priority fallback `schema.PRIORITY`; else exit 2 `unknown status: 'x'; choices: …` (≤12) or `; use numeric ID`); `--agent` → `responder_id` (tickets) / `agent_id` via `_resolve_agent`; `--group` → `group_id` via `_resolve_group`; each `--set F=V` via `_resolve_set_field` (no `=` → `--set requires FIELD=VALUE format, got: 'x'`; unknown field message is hard-coded `field not found: 'F'; run \`fsv changes fields\`` even for tickets). Empty → exit 2 `nothing to update; pass at least one field (--status, --priority, --agent, --group, --set FIELD=VALUE)`. Risks `_risks_for_item_write` (fetches current item when status/agent/responder/group present; `safety.write_risks`). `--dry-run` → `_emit_dry_run` (with `requires_confirmation`). Else `_confirm_writes` → `service.update_item` → json `emit_json(item)` or Out `[green]updated[/] {format_id}`.

#### `add-note` (changes, problems) / `reply` (tickets)
No client before confirmation. Risks `safety.note_risks(public)` / `reply_risks()` (`ticket reply: sent to the requester by email` always). `--dry-run` → `_emit_dry_run(…, {"body", "private": not public}, "add_note")` / `{"body"}` `"reply"`. `_confirm_writes(risks, "note on CHN-5" / "reply on INC-5")`. Then `service.add_note(public=)` → Out `[green]added[/] {public|private} note {id}`; `service.add_reply` → `[green]added[/] reply {id}`.

#### `notes` / `conversations`
`_history_rows`: `views.collect_pages(c, "{api}/{cid}/{notes|conversations}", key, {page, per_page, include: "user" | "user,phone,feedback"}, max_pages=min(N or (50 if --all else 1), 50))`; missing with zero pages → Err note, exit 1; partial → Err note. Flat notes `{id, created_at[:19], author, visibility private|public, body}`; conversations `{…, kind: private note|incoming|public reply, body}`.

#### generic `download` (tickets/problems) — `download_generic` :4345
Selectors `--attachments/--main-attachments`, `--conversations` (tickets) | `--notes` (problems), `--description-attachments`, `--all`; none → main only. Out dir `--dir/--out` or `./{INC|PRB}-{cid}` (tickets always `INC-` prefix). `int_get("{api}/{cid}", include=requester,stats)`; main → `create.download_attachment(att, out, force, c)`; threads → `collect_pages(... per_page 30)`, incomplete → result `{"source":key,"status":"incomplete","complete":false,"note":…,"meta":…}` + Err note; description → regex `(?:https?://[^"'<>\s]+)?/helpdesk/attachments/(\d+)(?:\?[^"'<>\s]+)?` on html-unescaped description, relative → `https://{DOMAIN}…`, dedup, name `attachment-{id}`. Each result gets `source` and `status` (`skipped`|`downloaded`). json `emit_json(results)`; table "Downloaded attachments for PFX-id" (Status, Source, File, Size) + Out `[green]output[/] {dir}`. Only Session/APIError caught.

#### tickets `approvals` / `associations`
- approvals: `service.get_ticket_approvals`; APIError 404 → exit 2 `ticket approvals not available for this ticket` (+ `; INC tickets often do not expose approvals, try SR or another approval-capable ticket` when id prefix is INC); other → `_err_exc`. Flat `{level, approver, status, decided[:19], type, comment}`.
- associations: `int_get("tickets/{id}/tabs")` → `associated_modules` of tab `associations`; then (only if module listed or no list) `collect_pages` of `changes?change_type=change`, `changes?change_type=change_cause`, `problems`. json `{"changes","change_causes","problems"}`; flat `{type,id,subject,status,priority}`; empty → Out `no associations`; tables Changes / Caused by changes / Problems.

### 2.4 `tickets create` (`cli.py:4265` → `ticket_cli.py:29 create_command`)
`as_json = --json or -o json` (csv/tsv = table). Wrapper: FlowExit → `_flow_exit`; Session/APIError → `_err_exc`; **any other Exception (incl. FormError) → `_err_exc` → exit 1**.
1. `tty = change_flow.interactive() (stdin isatty) and not no_input` (no_input merged with root).
2. `form` = no `--from/--set/--preset/--edit` and tty and `change_cli.form_tty()`; `template_only` = none of those and not form and `--dry-run`; none at all → FlowExit 2 `pass --from or --set, or open the interactive form in a terminal`; `--edit` without tty → FlowExit 2 `--edit needs a terminal; use --from or --set`.
3. `formmeta.load_meta(client, "ticket", "new", fresh=dry or yes or not tty)`; `drafts.resolve_draft(from, preset, sets, Draft())` (ValueError → FlowExit 2); requester via `change_cli._Requester` (flag > draft > config `defaults.requester`; never prompts here).
4. build = `ticket_flow.build_create_plan`; requester problem inserted first and hash cleared.
5. template_only → Out `flow.template_for(plan, draft, title="fsv incident draft: fill in, then use tickets create --dry-run")`.
6. form → `formui.session.ticket_session` + `change_cli._run_form`; edit → `flow.edit_loop` (EditorAbort → FlowExit 1).
7. `--dry-run` → `print_plan`: json `plan.to_json()`; else review rendered to stdout + `Payload (POST /api/_/tickets)` + payload JSON + `plan_hash: H\nSend with --yes --expect-plan H[:16]`, or `No payload: fix the problems above.`; not ok → FlowExit 2; then `flow.expect`.
8. tty & no `--yes` (no form): review to Err; not ok/blocking → FlowExit 2 `fix the form or acknowledge blocking messages before creating`; `confirm("Create this incident?")` no → FlowExit 1 `cancelled; nothing was sent`. Non-form non-tty: no `--yes` → FlowExit 2 `review with --dry-run, then use --yes --expect-plan HASH`; non-tty and no `--expect-plan` → FlowExit 2 `non-interactive create requires --expect-plan HASH`.
9. not ok/blocking → FlowExit 2 `cannot create: …`; `flow.expect`; `ticket_flow.send(plan, client, rebuild, expected, allow_duplicate)`.
10. Out json single-line `{"created","plan_hash"}` or `[green]created[/green] incident {id}`.

### 2.5 `changes`-only commands

#### `changes create` (`cli.py:4535` → `change_cli.py:206 create_command`)
`--description -` reads stdin. Legacy flags map: `subject, description, status, priority, agent, group`. Wrapper: FlowExit → `_flow_exit`; FormError → exit 2 `str(e)` (unescaped); Session/APIError → `_err_exc`; other → `_err_exc`.
1. `tty = interactive() and not no_input`; `attach_flow = change_flow.resolve_attach_flow(opt)` (option > config `changes.attach_flow` > `after-create`; invalid → FlowExit 2 `attach flow 'x': expected one of after-create, before-create`).
2. has_input = from/sets/preset/legacy/attach. No input and no `--edit`: tty+form_tty → interactive form; elif `--dry-run` → template only; elif tty → edit=True; else FlowExit 2 `nothing to create: pass --from FILE, --set LABEL=VALUE or --preset NAME (the interactive form and --edit need a terminal); see \`fsv changes create --help\``. `--edit` without tty → FlowExit 2 `--edit needs a terminal (no TTY or --no-input); use --from FILE`.
3. `load_meta(client, "change", "new", fresh=dry or yes or not tty)`; extra Draft: attachments (expanduser), `flow.legacy_answers(meta, subject, priority, agent, group)` (missing field → FlowExit 2 `this form has no X field`), status → status field api_name, description → `description_html`; `resolve_draft(from, preset, sets, extra)` (ValueError → FlowExit 2).
4. `_Requester(client, --requester, tty)`: flag > draft `requester:` > prompted > config `defaults.requester`. If tty and not form/edit/dry/template and no value → `prompt("Requester (email)", default="", show_default=False)`. Resolution `formmeta.resolve_requester(client, v, _pick if tty)`; `_pick` lists ≤20 to Err (`several requesters match; candidates:`), prompt `Pick a number (0 to cancel)` → FlowExit 2 `no requester chosen`.
5. build = `flow.build_create_plan(meta, d, resolver=ClientResolver, requester, ack_all, allow_incomplete=--no-validate)`; a requester problem replaces errors starting `requester is not set`.
6. template_only → Out `flow.template_for(plan, draft, title="fsv change draft: fill in, then \`fsv changes create --from FILE --dry-run\`")` (no trailing extra newline).
7. form → `formui.session.change_session(client, meta, requester (only if --requester; problem → Err \`[yellow]--requester[/yellow]: …\`), dry_run)`; `_run_form`: `save_draft` → FlowExit 0 `saved P; continue later with --from P (nothing was sent)`; not submit → FlowExit 1 `cancelled; nothing was sent`. edit → `flow.edit_loop(build, draft, title="New change")`; EditorAbort → FlowExit 1.
8. `--dry-run` → `_print_plan`: json `plan.to_json()` (stdout); else review to stdout, then `[dim]{review.NO_PAYLOAD}[/dim]` or `[bold]Payload[/bold] (POST /api/_/changes)` + payload JSON, `plan_hash: H`, `[dim]send exactly this with: --yes --expect-plan H[:16][/dim]`; not ok → FlowExit 2; `flow.expect`.
9. form session → `flow.expect` then `_send`. tty & no `--yes` → `_confirm_loop` (review to Err; menu prompt `[y] create  [e] edit  [s] save draft  [n] cancel` or `Problems must be fixed first: [e] edit  [s] save draft  [n] cancel`, default `n`, first char; `y` with blocking unacked → Err `[magenta]blocking rule message:[/] m` + `confirm("Acknowledge and continue?")`; `e` → edit_loop (EditorAbort → Err `[yellow]{e}[/yellow]; keeping the previous draft`); `s` → `prompt("Save draft to", default="change-draft.yaml")`, existing → `confirm("{p} exists; overwrite?")`, `flow.save_draft` → Err `[green]saved[/green] P; continue later with --from P`, FlowExit 0; `n` → FlowExit 1 `cancelled; nothing was sent`). Non-tty & no `--yes` → review to Err, FlowExit 2 `not a terminal: review with --dry-run, then send with --yes --expect-plan HASH`; not ok → FlowExit 2 `cannot create: …`; blocking → FlowExit 2 `blocking rule message(s) need --ack: …`; `flow.expect`.
10. `_send` (change_cli.py:168): `flow.check_duplicate` (journal hit in 24h → FlowExit 4 `this exact plan was already sent at TS (created ID)…`/`was started at TS with no recorded outcome…`; `--allow-duplicate` skips); `flow.freshen(plan, client, rebuild)` → if diff: Err `[yellow]form metadata changed since the review; the plan is now different:[/yellow]` + lines; new not ok → FlowExit 2 `the updated form needs more answers: …`; if expect/yes/non-tty → FlowExit 4 `plan changed to H; review it again with --dry-run`; else `confirm("Send the updated plan?")`. `flow.expect`; `flow.execute(plan, client, duplicate, attach_flow)`. `PartialFailure` → Err `[red]error[/red]: change {id} was written, but a later step failed: {err}`, `uploaded attachment ids: …`, step lines, FlowExit 5 `partial failure; change id {id}`.
11. Out json `{"created","plan_hash","steps","uploaded"}` (indent 2, ensure_ascii False) or `[green]created[/] {format_id}  {portal_url}/{id}` + `  [green]ok[/] {step}` for steps[1:].

#### `changes update` (`cli.py:3984`) — mode dispatch
`no_input` merged. `open_form` = not no_input and *no* flag among status/priority/agent/group/edit/set/planning/description/files/attach/due_by/from/preset, and `interactive() and form_tty()`.
- **Form engine path** if `(--edit or --set or --from or --preset or open_form)` and no `--due-by` and no `--planning` → `change_cli.update_command(...)` (status/priority/agent/group passed as legacy answers, `status_label=None`, subject `CHN-id`, `form=open_form`). FlowExit → `_flow_exit`; FormError → exit 2.
  - `update_command` (change_cli.py:357): form requested but no tty/form → FlowExit 2 `nothing to update: the interactive form needs a terminal; pass --set LABEL=VALUE, --from FILE, --status/--priority/--agent/--group or --edit`; `--edit` no tty → FlowExit 2 `--edit needs a terminal (no TTY or --no-input); use --from FILE or --set`; `int_get("changes/{cid}")`; `load_meta(client,"change","edit", fresh=dry/yes/!tty, module_id=display_id|id|cid)`; status with no status field → FlowExit 2 `this form has no status field`; `flow.build_update_plan(meta, d, record, cid, …)`; form → `change_session(mode="edit", target, record, title="Edit CHN-id")`; edit → edit_loop. `--dry-run` → `_print_plan` (payload header ` (PUT changes/{cid})`). ok & no hash → Out `[yellow]no changes[/] CHN-id`. tty & no yes → `_confirm_loop` verb `update`, then no hash → `no changes`. Non-interactive: not ok → FlowExit 2 `cannot update: …`; `plan.risks` and no `--yes` → FlowExit 2 `CHN-id: r1; r2. Pass --yes to confirm.`; blocking → FlowExit 2 `blocking rule message(s) need --ack: …`; `flow.expect`. `_send(..., allow_duplicate=True, duplicate=--duplicate)`. Out json `{"updated","plan_hash","steps"}` or `[green]updated[/] CHN-id` + ok steps.
- Otherwise: `modes = due_by? + planning? + edit + quick_flags(attach|set|status|priority|agent|group)` > 1 → exit 2 `choose one update mode: --due-by, --planning, --edit, or field/attachment flags`. no_input and `--duplicate prompt` with files/attach → exit 2 `--duplicate prompt disabled by --no-input; use --duplicate skip|replace|append`.
  - `--due-by`: dry → `_emit_dry_run(..., {"due_by"}, "set_due_by")`; `create.set_due_by(cid, v)` → Out `[green]updated[/] #{cid} resolution due → {v}` (no confirmation).
  - `--planning`: `create.resolve_planning_definition(label, schema fields)` → `planning_server_key`; `--description -` reads stdin; dry → `_emit_dry_run(…, {"planning_field","description","files","duplicate"}, "update_planning_field")`; `create.update_planning_field(cid, key, description, file_paths, duplicate, backup_replaced, backup_name, label)`; json → `emit_json(updated)`; `_fsv_noop` → `[yellow]no changes[/] planning field 'k'` + ` (a, b already attached)`; else `[green]updated[/] planning field 'k' (description, N file(s))`.
  - quick path (status/priority/agent/group and/or `--attach`): body via `_resolve_update_choice`/`_resolve_agent`/`_resolve_group` (key `agent_id`); risks; not dry → `_confirm_writes`; attach: dry → body `attachments: {"preserve_existing": true, "upload_files": [...], "duplicate"}`, else `create.attach_files_to_change(cid, files, c, duplicate, backup_replaced, backup_name)`; dry → `_emit_dry_run`; body → `create.update_change` → json/`[green]updated[/] {format_id}`; attach noop → `[yellow]no changes[/] CHN-id ( … already attached)`; attach only → json result or `[green]updated[/] CHN-id`.
  - (The quick-path `--set` loop and the `--edit` JSON-editor block at :4090 are unreachable: `--set`/`--edit` always take the form path or hit the mode error.)
  - nothing → exit 2 `nothing to update; use --set, --from, --attach, --planning, --edit, or pass --status/--priority/--agent/--group (in a terminal, no flags opens the edit form)`.
  - Wrapper: Session/APIError/other Exception → `_err_exc`.

#### `changes download` (:4178)
`--all` = planning+main+description; nothing selected → all planning. Out dir `--dir` or `./CHN-{cid}`. `service.get_change_evidence`; planning selection: all planning fields with attachments and/or each `--planning` value (`resolve_planning_definition` + `match_planning_row` → row name or server key), dedup; field without attachments → `{"source":"planning:K","status":"no attachments"}`; downloads via `create.download_attachment`, `source` = `planning:K` / `attachments` / `description`. json `emit_json(results)`; table + `[green]output[/] CHN-id`.

#### `changes clone` (:4435)
`create.change_clone_data(cid)` (no `_client()` pre-check). `--dry-run`: `emit_json(data)`; if any `--with-*`: `create.clone_side_effects(cid)` → Err `[bold]side effects if run without --dry-run:[/bold]` + `  - N task(s) created (M with an owner; owners/past dates kept|dropped)`, `  - tenant workflows may add their own tasks to the new change`, `  - N asset(s) associated`, `  - N planning field(s) copied, M attachment(s) re-uploaded`. Else `_confirm_writes(safety.clone_risks(with_tasks), "clone of CHN-id")`; body = data (`--no-edit`) or `_edit_body(data, "Cloning change #{cid}")`; `create.submit_change(body)`; Out `[green]created[/] {label}  {portal_url}/{id}` (json: Err `created {label}`); no id → Err warning `change created but the response had no id; see \`fsv journal tail\``. Steps (each Err `  {msg}` or `[red]step failed[/red]: name: e`): tasks `create.clone_tasks(cid, new, keep_owners, dropped)` → `cloned N task(s)` + Err `  note: …` for dropped + tenant-workflow note; assets `clone_assets`; planning `clone_planning_fields`. json `emit_json({**created, ["fsv_clone_steps": {name: "ok"|"failed: e"}]})`. Failures → Err `[red]error[/red]: {label} WAS created but N step(s) failed; do not rerun clone …`, exit `exit_code_for(first failure)`.

#### `changes tasks-update` (:3577)
`TASK_ID` int. `--edit`: `create.get_task_for_edit(cid, tid)` → `_edit_body(…, "Editing task {tid} on change #{cid}")`, `status` normalized via `_task_status_value` (`task_status.resolve(v, _task_field_defs())[0]`; defs from `int_get("change_task_fields", include_deleted_choices=true)` keys `change_task_fields|task_fields|fields`, errors → `[]`). Flags: status, title, `group_id` (`_resolve_group`), `owner_id` (`--agent` via `_resolve_agent`), `due_date`, `planned_start_date`, `planned_end_date`, `custom_fields.system/environment` via `_task_choice_value` (digits or label map; ValueError `system must be numeric ID or one of: …`). Empty → exit 2 `nothing to update; use --edit or pass task fields`. Risks `write_risks(kind="task", status_label = --status text or "status {v}")` (fetches current task when needed). Dry → `emit_json({"action":"update_task","change_id","task_id","body"[,"requires_confirmation"]})`. `_confirm_writes(risks, "task {tid} on #{cid}")` → `create.update_task` → json or Out `[green]updated[/] task {tid} on #{cid}`. Session/API/ValueError → `_err_exc`.

#### `changes tasks-delete` (:3664)
Dry → `emit_json({"action":"delete_task","change_id","task_id"})`. No `--yes`: no-input/non-TTY → exit 2 `pass --yes to delete task`; else `confirm("Delete task {tid} from CHN-id?", abort=True)`. `create.delete_task` → Out `[green]deleted[/] task {tid} from #{cid}`.

#### `changes approvals` (:3687)
`--groups`: `create.get_change_approval_groups(cid)`; flat `{group_id, status (approval_status), member_ids "a, b"}`; table `Group ID / Raw status / Member IDs`. Default: `create.get_change_approvals(cid)`; flat `{level, approver, status ("peer approved" for peer_responded), decided[:10], message}`. Catches Session/API/`ApprovalError` (RuntimeError → exit 1).

#### `changes state` (:3744)
`int_get("changes/{cid}")`; json `{"change", "flow": state_flow.get_state_flow(state_flow_id, c, workspace_id)}`; csv/tsv `emit_delimited([change])`; table `state_flow.render_flow(change, console)`. Missing `state_flow_id` → exit 2 `no state_flow_id on this change`.

#### `changes tasks-add` (`change_tasks.py:341`)
Not merged with root `--no-input`. Errors (ValueError/TaskError) → Err plain `str(e)`, **exit 2** (no `error:` prefix); Session/APIError not caught by the wrapper.
1. `--read-only` and not dry → `read-only mode: no task was sent`.
2. Local validation: `status_label(status)`, `build_plan(id, title, status_id=1, **opts)`; then real `build_plan(id, title, owner_id=resolve_owner(owner), group_id=resolve_group(group), status, status_id=resolve_status(status), note, notify_before, planned_start, planned_end, planned_effort)`.
3. Dry → Out `json.dumps(send(plan, reviewed_hash=None, dry_run=True), indent=2)`.
4. Err review JSON. `--expect-plan` ≠ hash → `task review hash does not match; review the exact task again`; `--yes` requires equal hash (`--yes requires the full --expect-plan from a prior task review`); no-input → `sending requires --yes and --expect-plan in no-input mode`; else `confirm("Create this exact reviewed task?")` no → Out `Cancelled; no task was sent.` exit 0.
5. `send(plan, reviewed_hash)` → json, or Out `Task {task_id} submitted on change {cid}; exact-ID/state read-back: {verified}` + Err warning.

#### `changes approval-request` (`change_approvals.py:390`)
Same skeleton. read-only → `read-only mode: no approval request was sent`; `build_plan(id, name, message, approver_ids=[1], rule)` then `resolve_approvers(approver)`; mismatch `approval review hash does not match; review the exact request again`; yes msg `--yes requires the full --expect-plan from a prior approval review`; confirm `Send this exact approval request and irreversible emails (no verified revoke)?`; cancel `Cancelled; no approval request was sent.`; Out `Approval requested on change {cid}; approval record IDs: {ids}; complete inventory verification: {v}; group name verified: {g}`.

#### `changes approval-respond` / `approval-cancel` (`change_approval_actions.py:293/317`, shared `_run` :259)
respond: `--approve` == `--reject` → `choose exactly one of --approve or --reject`; `build_plan(change, approval_id, op, remark)`. cancel: `build_plan(change, group_id, "cancel")`. `_run`: read-only → `read-only mode: no approval action was sent`; dry → Out JSON; mismatch `approval action review hash does not match; review the exact action again`; yes msg `…prior approval action review`; no-input msg as above; review JSON (cancel adds `review["group"] = _cancel_group(get_client(), plan)`) to Err — **printed after the hash checks** (unlike tasks-add); confirm `Send this exact approval action (notifies people; may move workflow; cancel irreversible)?` (skipped with `--yes`); Out `Approval {group|record} {id} submitted on change {cid}; observed raw status: {json}; verification: {basis}` + Err warning.

#### `changes assets` group (:3427; all via `assets_resource` :1684)
| sub | call |
|---|---|
| `legacy ID [--search/-q Q] [--add …] [--remove …] [--pick] [--list-categories] [--category C] [-p] [-n] [--dry-run] [-y] [-o] [--json]` | all options; >1 action → exit 2 `choose only one action: --search, --add, --remove, --pick, or --list-categories` |
| `ls ID` | list associated |
| `search ID QUERY` | search |
| `add ID ASSET...` | add |
| `remove ID ASSET...` | remove |
| `pick ID` | pick |
| `categories` | `asset_categories_resource` |
Order inside `assets_resource`: `_client()` → `--category` resolved first (`_resolve_asset_category`: `all`/`all assets` → All assets; exact casefold; unique substring; else `unknown asset category: 'x'. Try --list-categories` / `multiple asset categories match 'x': …`) → `--list-categories` → `_cid`.
- Categories: `_fetch_asset_categories` scrapes `GET https://{DOMAIN}/cmdb/items` (HTML, follow redirects) `<select name="ci_type_id">` options → `{name, filter: ci_type_id|"all_assets", ci_type_id}` deduped by casefold label; process-level cache; no select → exit 2 `could not load asset categories from /cmdb/items`. json = list; table "Asset categories" (Name, CI Type ID).
- add: `_resolve_change_asset_ids` (digits pass through; else `create.search_assets_for_change(cid, v, per_page=50)` filtered by category; exact display_id/name, single result, or errors `no asset match[ in category 'C']: v`, `multiple assets match 'v': name [id], …`); payload `{"action":"associate_assets","change_id","asset_ids"}`.
- remove: resolves against `create.get_change_assets` (`asset not associated on CHN-id[ in category 'C']: 'v'`); payload `{"action":"dissociate_assets","change_id","asset_display_ids"}`.
- pick: requires TTY & not no-input (`interactive picker requires TTY`); no category → prompt_toolkit `input_dialog("Asset category", "Category for asset search. Leave empty for All assets.", default "All assets")`; cancel → exit 0; search dialog `Search assets for CHN-id[\nCategory: C]`; `checkboxlist_dialog("Associate assets", "Select asset(s) for CHN-id", …)`; empty selection → Out `cancelled`.
- write: dry → `emit_json(payload)`; no `--yes`: non-interactive → exit 2 `pass --yes to associate assets` / `pass --yes to remove assets`; else `confirm("Associate N asset(s) with CHN-id?"|"Remove N asset(s) from CHN-id?", abort=True)`; `create.associate_assets` / `dissociate_assets`; json `{**payload,"ok":true}` or Out `[green]associated[/] N asset(s) with CHN-id` / `[green]removed[/] N asset(s) from CHN-id`.
- search: `create.search_assets_for_change(cid, q, page, per_page)`; with category json adds `assets` (filtered) and `meta.filtered_count`, `meta.category`. ls: `get_change_assets` filtered. Rows `_asset_display` `{id,name,type,used_by,location,state,serial}` (state/serial via `_prefixed_value` `asset_state_<ci_type_id>`/`serial_number_…`); table "Assets".
- Each nested command catches Session/API/ValueError → `_err_exc`.

#### `changes associations` group (:3772; via `change_associations_resource` :1805)
| sub | args |
|---|---|
| `legacy ID [--search/-q] [--add …] [--remove …] [--pick] [--dry-run] [-y] [--expect-plan] [--read-only] [-o] [--json]` | >1 action → exit 2 `choose only one action: --search, --add, or --remove` |
| `ls ID`, `search ID QUERY`, `add ID TICKET...`, `remove ID TICKET...`, `pick ID` | — |
No `_client()` call (no session pre-check; uses `get_client()` inside `create.*`).
- search: `create.search_change_tickets(q)` → flat `{id, subject, status}`; table "Ticket search".
- write (`change_associations.build_plan/send`): `--read-only` & not dry → exit 2 `read-only mode: no association request was sent`; remove with ≠1 ticket → exit 2 `unlink accepts one ticket per reviewed request; review each removal separately`; ids from pick (`_pick_change_tickets`, prompt_toolkit dialogs `Associate tickets`), remove (`_resolve_associated_change_ticket_id`: inventory must be complete `initiating-ticket inventory unavailable/incomplete; cannot resolve removal`; `ticket not associated on change #ID: 'v'`), add (`_resolve_change_ticket_id`: parse_id TICKETS else search; `no ticket match: v`, `multiple ticket matches for 'v': …`). Empty → Out `cancelled`. `build_plan(cid, ids, unlink=bool(remove))`; dry → `emit_json(send(plan, None, dry_run=True))`; Err Rich JSON review; mismatch → `association review hash does not match; review again`; `--yes` needs equal → `--yes requires the full --expect-plan from a prior association review`; non-interactive → `sending requires --yes and --expect-plan in no-input mode`; `confirm("Send this exact reviewed association request?")` no → Out `cancelled`. `send(plan, reviewed_hash, read_only)` → json or Out `Association {status}; complete inventory verified: {verified}`.
- ls: `create.get_change_associations(cid)` → Err each warning (markup off); json = whole dict; flat `{type: tickets|problems|releases, id, subject, status}`; tables per kind; none → `no associations` if `complete` else `association inventory unavailable/incomplete; not an empty list`.

### 2.6 `form` (`form_cli.py`) — `_client` = `cli._client`; `_fail` = Err `[red]error[/red]: {escaped}` + exit; `_guard` catches Session/API/FormError/ValueError (FormError → 2, others `exit_code_for`)
| command | handler | behavior |
|---|---|---|
| `form explain [module=changes]` | `explain` :86 | `formmeta.module_name` (ValueError → 2); `--id` → `parse_id(CHANGES)` (unguarded) → `int_get("changes/{cid}")`; `load_meta(c, mod, "edit"|"new", module_id)`; Err dim `rules_note`; `resolve_draft(from, preset, set)`; `FormState(spec, engine, mode, resolver, existing=name_existing(existing_from_record…))`; `apply_answers`; rows for `spec.inputs()` visible or rule-touched (all with `--all`): `{field,label,visible,required,disabled,value,populated_by,why}`; json `{"rows","messages":[{text,blocking,rule}],"conflicts","errors","metadata"}` (exit 0 even with errors); table + messages/conflicts/`[red]answer error[/]` lines; errors → exit 2 |
| `form fields [module]` | `fields` :161 | `load_meta(new)`; `--choices F` → `find_field`; nested level with no pool → exit 2 `{label}: choose the upper level first (its choices depend on it)`; json `{"field","label","choices":[{label,value,id,description}]}`; default rows `{label,name,kind,custom,required_static,visible_on_load,required_on_load,help,choices,choice_descriptions}` |
| `form dump [module] [--mode new\|edit]` | `dump` :229 | bad mode → exit 2 `--mode must be new or edit`; `load_meta(fresh=True)`; Out JSON `{"fsv_form_snapshot":1,"module","form_type","workspace_id","taken_at": strftime("%Y-%m-%dT%H:%M:%S%z"),"fields","business_rules"}` |
| `form diff SNAPSHOT [--against S2]` | `diff` :305 | read errors → exit 2 `cannot read snapshot P: e` / `P is not an fsv form snapshot (make one with \`fsv form dump\`)`; else current meta (`edit` if `form_type == "edit_form"`, fresh); `diff_snapshots` :262; json list; table lines `+/-/~ kind label (ref): what` + `N difference(s)` or `no changes` |

### 2.7 `sr` (`cli_sr.py`) — `_fail`/`_guard` (FlowExit printed like `_flow_exit`; FormError 2; Session/API/ValueError → `exit_code_for`)
| command | handler | behavior |
|---|---|---|
| `sr catalog` | `catalog_cmd` :88 | `catalog.list_categories`; `--categories`: json raw list / table "Service catalog categories"; else `catalog.list_items(popular)`, `--category` → `resolve_category`, `filter_items(category_ids, search, categories)`, add `category_name`, sort by (norm category, norm name); json `[{display_id,id,name,short_description,category_id,category}]`; table + Err `no items match` / `[dim]N item(s); see one with: fsv sr fields ID[/dim]` |
| `sr fields ITEM` | `fields_cmd` :217 | `catalog.load_item_meta`; `fields_doc` :156 → json; table rendering + Err `[yellow]warning[/yellow]: …` + footer `item form as of …; N business rule(s) apply. Create with: fsv sr create ID (interactive form)` |
| `sr create ITEM` | `create_cmd` :720 → `create_command` :581 | like changes create with: root `--no-input` **not merged**; requester config key `defaults.sr_requester`; prompt `Requester (email; approvals route via their manager)` (empty → FlowExit 2 `no requester given; nothing was sent`); `resolve_sr_draft` (preset < file < --set < --attach; file paths relative to draft dir); template → Out `render_sr_template` + Err `[dim]fill this in, then: fsv sr create ID --from FILE --dry-run[/dim]`; form `sr_session` (cancel → FlowExit 1 `cancelled; nothing was uploaded or sent`); `_print_plan` adds `[bold red]⚠ {catalog.SAFETY}[/bold red]` and header `Payload (POST /api/_/catalog/service-items/{id}/place-request; <upload N: name> becomes the attachment id after upload)`; `_confirm` menu `[y] place request …`, save default `sr-draft.yaml` (writes `render_sr_template(base_dir=dest dir)`); `_send` → `catalog.duplicate_check`, `catalog.freshen` (diff msg `the catalog item or its rules changed since the review; …`, confirm `Place the updated request?`), `flow.expect`, `catalog.execute`; `SendFailure` → Err uploaded ids, FlowExit(`exit_code_for(e.error)`, `{step} failed: {err}; nothing was placed` / `the server refused it; nothing was placed` (3/4/5) / `the outcome is unknown: check \`fsv journal tail\` and the requester's requests before retrying`). Out json `{"created","result","plan_hash","uploaded","steps"}` or `  [green]ok[/green] uploaded N -> id (target)` + `[green]placed[/green] {display}  {domain}/a/tickets/{id}` |
| `sr get SR-ID` | `get_cmd` :807 | `--download` starting with `-` → `BadParameter` (usage exit 2, hint `--download`); none of items/attachments/download → both; `parse_id(TICKETS)`; `int_get("tickets/{id}")`; `service.get_requested_items`; `catalog.item_labels` per item; values `{label,name,value,raw}` (`raw` dropped in json); `_attachment_rows`; `_download` per row → statuses `downloaded|exists|rejected|error`; doc `{"id","subject"[, "items"][, "attachments"][, "downloads"]}`; table: `console.rule`, per-item tables, Attachments list, download lines, `Saved to DIR`; any rejected/error → **exit 2** (after output) |

---------------------------------------------------------------------------------------------------

## 3. Flows

### 3.1 `fsv setup` (`cli.py:4874`)
1. stdin not TTY → exit 2 `setup requires an interactive terminal`.
2. `_setup_show_status`: table "Current status" (no header): domain (or `[dim](not set)[/dim]`), auth (`[green]✓[/green] {backend}, {age:.1f}h ago` / `[red]✗[/red] not logged in`), completion (checks hard-coded `~/.config/fish/completions/fsv.fish`, `~/.fsv-complete.{bash,zsh}`), network completion (`config.completion.network` or `[dim](unset)[/dim]`), `defaults.{res}` for each resource.
3. Menu table 1 Domain, 2 Authentication, 3 Shell completion, 4 Network completion, 5 Default filters, 6 All; `prompt("Choose", default="6")` until 1-6 (Err `[red]error[/red]: choose 1-6`).
4. Steps (printed headers `\n[bold]── Domain ──[/bold]` etc.):
   - Domain: `_resolve_domain()` then Out `domain set to [bold]{DOMAIN}[/bold]`.
   - Auth: domain missing → `domain required first` + `_resolve_domain()`; `session.login_interactive()` (SessionError → `_err_exc`); `_validate_new_cookies` (no `--force` here); `_choose_store()`; `save_cookies`; same success lines as login; failure Err `[yellow]warning[/yellow]: saved but could not verify: {e}`.
   - Completion: `_shell_from_env()` (none → Err `[yellow]could not detect shell; skipping[/yellow]`); existing script → Out `already installed: P` + `confirm("Reinstall?", default=False)`; else `confirm("Install {shell} completion?", default=True)`; writes like `completion install` (Out `installed: P`, `sourced from: RC`); powershell: path None and no install (falls through to `restart shell` line only); Out `restart shell: exec $SHELL`.
   - Network: Out `current: X`, `when on, TAB fetches live requester/agent names`; `prompt("Enable? [on/off]", default="on")` → must be on/off else Err `[yellow]expected on/off; skipping[/yellow]`; writes `completion.network`; Out `completion.network = on`.
   - Defaults: Out header lines; per resource `prompt("\n{res} --where (comma-sep, enter to skip)[ [cur]]", default="", show_default=False)`; split by comma → `config.set_default_where`; Out `  defaults.{res}.where = [...]` (Python list repr).
5. Out `\n[green]✓[/green] setup complete`.

### 3.2 `auth login` (`cli.py:2703`)
Validation (exit 2): `--store` ∉ file/argon/keychain → `--store must be 'file', 'argon', or 'keychain'`; `--browser`+`--header` → `use either --browser or --header, not both`; `--browser`+no-input → `--browser is interactive; do not use --no-input`; no header, no browser, no-input → `pass --header when using --no-input`.
Domain: `--domain` → `config.set_domain` (normalize; ValueError → exit 2); elif no configured domain → `_resolve_domain(no_input)`; elif TTY & interactive → `_resolve_domain`. `_resolve_domain` (:2671): configured & stdin not TTY → Err `Domain: [bold]{d}[/]`, return; not configured & no-input → exit 2 `no Freshservice domain; pass \`fsv auth login --domain ...\` when using --no-input`; `prompt("Domain", default=DOMAIN or "acme.freshservice.com")` (note: reads stdin even when not a TTY); `config.set_domain(normalize_domain(answer))`; returns `"://" in answer`. If URL pasted, not explicit, TTY, interactive → `confirm("Extracted {d} — login?", default=True, abort=True)`.
Cookies: `--browser` → `session.login_browser()`; `--header` (`-` = all stdin, stripped) → `parse_cookie_header` + `validate`; else `login_interactive()`. SessionError → exit 3.
Unless `--force`: `_validate_new_cookies` (`Client(cookies=…).check_session()`): SessionError → exit 3 `these cookies are expired or invalid for {d}; nothing saved (use --force to save anyway)`; other → exit 1 `could not verify cookies against {d}: {e}; nothing saved (use --force to save anyway)`.
Store: none & TTY & interactive → `_choose_store` (table "Store session" #/Backend/Notes; options: keychain (darwin only, "macOS Keychain; recommended"), argon ("encrypted file; asks passphrase when used"), file ("plain JSON file; chmod 600"); default = `current_backend()` if offered else first; `prompt("Choose store", default=idx)` accepts number or name, else Err `[red]error[/red]: choose 1-N or one of: …`). `save_cookies(cookies, store)` (None → current backend) → Out `saved N cookies to {Keychain | ~/…/session.json}`.
Verify: `reset_client(); get_client().check_session()` → Out `logged in as: [bold]{first last}[/bold] <{email}>` or `logged in (identity unavailable: agent profile is not readable)`; Err `warming cache in background...`; `_refresh_cache(blocking=False)` (daemon threads; usually killed at exit). Exception → Err `[yellow]warning[/yellow]: saved cookies but could not verify login for {d}: {e}` + `If this is the wrong tenant, rerun with \`fsv auth login --domain acme.freshservice.com\`.`; exit 0.

### 3.3 `auth logout` / `auth status`
- logout: no `--yes`: TTY & interactive → `confirm("Wipe session for {DOMAIN or 'Freshservice'}?", abort=True)`; else exit 2 `pass --yes to confirm non-interactive logout`. `session.logout()` (unlink session file, keychain delete, store pref) → Out `session cleared`.
- status: no session → **exit 2** `no session; run \`fsv auth login\`` (not 3); `_client()`; `(check_session(), rate_limit_remaining())` via `_api`; Out `[bold]{name}[/bold]  <{email}>` (empty agent → `identity unavailable (403)`), `session:     valid`, `department:  {dept}  ({team})` (if any; team = custom_fields.team), `session age: {h:.1f}h` (+`  [yellow]⚠ session may be stale[/yellow]` if >8), `rate limit:  {rem}/{tot}`.

### 3.4 `completion *`
- `_completion_shell(shell)` (:2940): explicit arg used **unvalidated**; else `$SHELL` basename ∈ bash/zsh/fish/powershell/pwsh (no `sh` alias, unlike `_shell_from_env`); else exit 2 `could not detect shell; pass one of: bash, zsh, fish, powershell, pwsh`.
- show: `sys.stdout.write(build_script(shell, basename(argv[0])))` (powershell/pwsh/unknown → ValueError traceback, exit 1).
- install: fish → mkdir `~/.config/fish/completions`, write `{prog}.fish`, Out `fish completion installed in P`; bash/zsh → write `~/.{prog}-complete.{shell}`, append `\n. {path}\n` to `~/.bashrc`/`~/.zshrc` unless the path string already occurs, Out `{shell} completion installed in P`, `sourced from RC`; other → typer `_completion_shared.install(shell, prog)` → Out `{shell} completion installed in P`; always Out `restart shell: exec $SHELL`.
- doctor: table "completion doctor": shell ($SHELL or ?), fsv path (`which fsv` or argv0), network (on/off via `completion._completion_network`: env `FSV_COMPLETION_NETWORK=1` or config value ∈ on/True/"true"/"1"), `schema {res}` ok/missing (cache file exists); Out `Use TAB after a prefix, e.g. \`fsv changes ls --where requester=phu<TAB>\`.`

### 3.5 `config *`
Keys (`_CONFIG_KEYS` :3037, order matters for `list`): `completion.network`, `defaults.requester`, `changes.attach_flow`, `forms.workspace_id`, `defaults.sr_requester`, `tui.read_only`.
- list: reads via `completion._config_load()` (CONFIG_DIR captured at import; unreadable/corrupt → `{}`); Out per key `{key} = {value|(unset)}  [dim]# {desc}[/]` (value is Python `str()` of stored JSON value).
- get: unknown → exit 2 `unknown key: K; supported: ['completion.network', …]` (Python list repr); Out value or `(unset)`.
- set: same unknown check; **no value validation**; stores the raw string under `cfg[section][prop]` via `config.load_config()`/`save_config()` (json indent 2, sort_keys, trailing newline, atomic 0600; corrupt file is backed up to `config.json.corrupt-{ts}` by `load_config`); Out `{key} = {value}`.
- defaults show [res]: resource unvalidated; per resource Out `{res}: {expr}` lines or `{res}: (none)` (from `defaults.{res}.where`).
- defaults set RES -w …: RES ∉ changes/tickets/problems → exit 2 `unknown resource 'x'; choose: changes, tickets, problems`; replaces list; Out `defaults.{res}.where = ['a=b', …]`.
- defaults clear RES: pops whole `defaults.{res}`; Out `defaults.{res}.where cleared`.

### 3.6 `cache *`
- status: for each resource × (schema "schema", filters "views") the first candidate path that `cache.load` returns a doc for; age from `saved_at` or mtime; state `stale|ok`; ttl from `cache.TTL` (`{h:.0f}h` if ≥3600 else `{m:.0f}m`: schema 168h, filters 15m, groups 6h); plus groups cache; table "cache status" (cache, kind, age `{h:.1f}h`, state, ttl). Missing caches omitted.
- refresh (and `completion refresh`): `_refresh_cache(verbose=True, blocking=True)` — **refreshes everything** (help text "only stale" is wrong): tasks schema:{res} (`schema.load(force=True)`), filters:{res} (`int_get(filters_path)` → `cache.save(filters_cache_path, "filters", {"filters": …})`), groups (`bootstrap/agents_groups` → `cache.save(groups_cache_path, "groups", {"groups": …})`), each with its own `Client()`, ThreadPool(7); Err spinner `syncing...`; table "cache refresh" (resource, count) rows in completion order (`{res}` / `{res} views` / `groups`); failures silently skipped.
- clear TARGET: TARGET ∉ schema/views/filters/groups/all (or missing) → exit 2 `choose schema|views|groups|all`; schema/all → rmtree `SCHEMA_DIR` → Out `schema cache cleared`; views/filters/all → rmtree **`CONFIG_DIR/filters`** (bug: real cache is `CACHE_DIR/filters`) → Out `views cache cleared` only if it existed; groups/all → unlink groups candidates → `groups cache cleared` if any removed.

### 3.7 `journal *`
- ls: `journal.read()`, `--failed` keeps `not ok`; `--limit N` (0 = all) keeps last N (`rows[-N:]`; negative N behaves as Python slicing).
- tail: `journal.read(limit)` (`-n 0` → all).
- Output (`_journal_rows`): `--json` → one compact `ensure_ascii=False` JSON object per line; empty → Out `[dim]journal is empty[/dim]`; table columns time(ts), command, kind, target, status (`"ok "`/`"FAIL "` + status), created (created_id), plan (plan_hash[:12]).
- path: plain `print(journal.journal_path())` (`$XDG_STATE_HOME|~/.local/state/fsv/journals/<sha256(resolved CONFIG_DIR)>/<sha256(domain)|unknown>/journal.jsonl`).

---------------------------------------------------------------------------------------------------

## 4. Helpers to move out of `fsv-cli` (recommended owner crate)

| Python (cli layer) | used by | owner |
|---|---|---|
| `cli_sr._attachment_rows`, `cli_sr._download` (+ `catalog.is_file_value/safe_target/domain_url`, `create._attachment_url`) | `tui/actions.py:172,202`, `sr get` | `fsv-api` (new `attachments` module; catalog in fsv-flows re-exports) |
| `cli_sr.render_sr_template`, `resolve_sr_draft`, `edit_loop`, `fields_doc`, `_reveals`, `_attachment_rule` | `formui/session.py:695`, `sr *` | `fsv-flows` (catalog) |
| `change_cli._Requester` + `cli_sr._Requester` (merge; parameter = config key `defaults.requester` vs `defaults.sr_requester`, prompt text, "empty prompt is fatal") | create flows | `fsv-flows` (prompt via callback/`fsv_core::term`) |
| `change_cli._send` / `cli_sr._send` (duplicate check → freshen → diff exit 4 → expect → execute), `_report_partial` | create/update/sr | `fsv-flows` (UI confirm passed as callback) |
| `change_cli._confirm_loop`/`_menu`/`_ack_blocking`/`_pick`, `cli_sr._confirm`, `_run_form`, `form_tty` | CLI only | stay `fsv-cli` (terminal UI); `form_tty` → `fsv-formui::form_available` |
| `_build_query_hash`, `_parse_where`, `_where_field`, `_resolve_where`, `_date_condition`, `_where_schema_field`, `_where_operator`, `_query_item`, `_merge_date_ranges`, `_merge_same_conditions`, `_load_query_hash`, `_WHERE_*`/`_PERIOD_TOKENS` tables | `ls`; tests test_query_* | `fsv-api` (`query`) |
| `_find_schema_field`, `_schema_field_matches`, `_field_matches`, `_resolve_set_field`, `_choice_value`, `_resolve_update_choice`, `_status_label`, `_field_scope/_field_api_type`, `_REQUIRED_FK_KEYS`, `_missing_required_change_fields` | update/fields/lookup/query | `fsv-api` (`schema`) |
| `_resolve_agent`, `_resolve_group`, `_resolve_requester`, `_resolve_lookup_user`, `_pick_lookup` (picker as callback) | update, tasks-update, query | `fsv-api` (`lookup`) |
| `_risks_for_item_write`, `_current_flat` | updates | `fsv-api` (+ `safety` in fsv-core) |
| `_task_field_defs`, `_task_choice_value`, `_task_status_value`, `_task_field_choice_pairs` | tasks-update, completion | `fsv-api` (`task_status`) |
| asset helpers `_fetch_asset_categories` (+cache), `_resolve_asset_category`, `_filter_assets_by_category`, `_resolve_change_asset_id(s)`, `_prefixed_value`, `_asset_display` | assets cmds, completion | `fsv-api` (`change_associations`/assets); `_asset_display` row → `fsv-views` |
| `_resolve_change_ticket_id`, `_resolve_associated_change_ticket_id` | associations | `fsv-api` |
| `_saved_views`, `_pick_view`, `_load_views`, `_SITE_TABS`, `_history_rows`, `_activity_day/_activity_text/_local_dt`, `_search_extract`/`_SEARCH_TYPE_LABEL`, flat-row builders of tasks/notes/approvals/associations | read cmds; TUI has its own | `fsv-views` |
| `_refresh_cache` (schema/filters/groups warmers) | auth login, setup, cache refresh | `fsv-api` |
| download loops (generic + changes) and description-attachment regex | download cmds | `fsv-api` (next to `create.download_attachment`) |
| `form_cli.diff_snapshots`, snapshot doc builder | form diff/dump | `fsv-api` (`formmeta`) |
| `_shell_from_env`, `_completion_shell`, completion install (dup in `completion_install` and `_setup_completion_install`) | completion, setup | `fsv-completion` (`install(shell, prog) -> InstallReport`) |
| `completion_gen.build_script` needs the command tree (imports `cli.app`, `_ROOT_FLAGS_ANYWHERE`) | completion | `fsv-completion` takes a `clap::Command` (or neutral tree) passed in by `fsv-cli`; `ROOT_FLAGS_ANYWHERE` const lives in fsv-cli and is passed too |
| `_CONFIG_KEYS` (and the stale 3-key copy in `completion.complete_config_key`) | config cmds, completion | `fsv-core::config` (single table) |
| `_validate_new_cookies`, `_choose_store` options | auth, setup | validation → `fsv-core::session`; table/prompt stays CLI |
| `render.emit_json`/`emit_delimited` | everywhere | `fsv-core` (already planned) |

---------------------------------------------------------------------------------------------------

## 5. Inconsistencies, duplication, dead code (CLI surface must stay; internal only)

Dead code (do not port; keep tests that monkeypatch them working by porting the test intent instead):
- `cli.py`: `_emit_items` (only monkeypatched in tests), `_is_terminal_status`, `_confirm_terminal_status`, `_check_required` (thus `_missing_required_change_fields` only used by a test), `_network_completion_enabled`, `_ticket_id`, `_resolve_view_name`, `_resolve`, `TERMINAL_STATUS_WORDS`, `search_resource` (pure pass-through), the `KeyboardInterrupt/Abort` branch of `_err_exc`, the unreachable `--edit` JSON-editor block and `--set` loop in `changes update`, the `**({"expect_plan":…} if …)` kwargs dance in associations wrappers (equal to defaults).
- `change_cli._emit`, `__all__` re-export of `render_template`.

Duplication to unify internally (keep outputs byte-identical):
- Four error styles: `_err` (unescaped) / `_err_exc` (escaped) / `_fail` in `cli_sr`+`form_cli` (escaped, `highlight=False`) / plain `typer.echo(msg, err=True)` + exit 2 in register() modules (no `error:` prefix). One `CliError { code, msg, style }`.
- Two `_guard`s with different exception sets; three `_client` wrappers.
- Three plan printers (`change_cli._print_plan`, `cli_sr._print_plan`, `ticket_cli.print_plan`) with different wording (`NO_PAYLOAD` vs `No payload: fix the problems above.`; `send exactly this with:` vs `Send with`).
- Two `_Requester`s, two `_send`s, two confirm menus.
- Seven JSON emission variants (table in §1.7). Implement one writer with flags (indent, ensure_ascii, strip_internal, trailing newline) and pick per call site.
- Asset/association nested commands all funnel into one 5-action function with positional padding; split into per-action functions in Rust.
- Completion install logic duplicated in setup.
- `cache refresh` and `completion refresh` identical.

Inconsistent semantics (keep behavior, record in PARITY if fixed later):
- `--expect-plan`: prefix≥12 vs full-exact (§1.7).
- Root `--no-input` ignored by `tasks-add`, `approval-*`, `tickets add-note`, `sr create`.
- `auth status` no session → exit 2; `_client()` no session → exit 3.
- `tickets create` FormError → exit 1; `changes create/update` → 2.
- tickets/problems `update --set` unknown field message says `fsv changes fields`.
- `-p` = `--page` (ls/search/notes) vs `--priority` (update/create); `-a` = `--all` vs `--attach`; `-s` = `--status` vs `sr catalog --search`; `-n` = `--per-page` vs `journal --limit`.
- `--json` = "alias for --output json" on reads vs "emit JSON/structured result" on writes; `sr`/`form` `-o` unvalidated strings.
- Help/epilogs and `HELP_TOPICS["workflow"]` advertise `fsv changes get CHN-1234 --internal` — no such flag (usage error). Text must stay verbatim.
- `cache refresh` help "Refresh only stale caches." but refreshes all.
- `config list/get` read config through `completion._config_load` (import-time path, silent `{}` on corruption); `config set` through `config.load_config` (backs up corrupt). Rust: one reader, but must not overwrite a corrupt file on `list/get` (they never write).
- `config set` accepts any value (help says on|off|…).
- `setup` completion detection hard-codes prog `fsv`; install uses `basename(argv[0])`.
- Naming: singular/plural (`approvals` list vs `approval-request`; `tasks` vs `tasks-add`; `lookup requester|requesters`; param names `assets_`, `tickets_`, `change` vs `id_`), `views` command vs internal `filters`/`filter_name`; `cache clear filters` hidden alias of `views`. Keep CLI names; use consistent internal names.

Bugs observed (parity default = reproduce exit code; flag as candidate divergences in PARITY.md):
- `cache clear views` removes `CONFIG_DIR/filters`, not `CACHE_DIR/filters` (views cache is never cleared).
- Uncaught tracebacks (exit 1): `url` with no domain (`config.require_domain` RuntimeError); `changes associations *` with no domain (no `_client()` pre-check); `form explain --id BAD`; `completion show powershell|pwsh|<unknown>` (`build_script` ValueError); `tickets/problems download` non-API errors; Session/APIError escaping `tasks-add`/`approval-*`/`tickets add-note` wrappers.
- `auth login` without TTY and without domain/`--no-input`: `prompt("Domain")` consumes the first stdin line before `--header -` reads stdin.
- `_refresh_cache(blocking=False)` uses daemon threads; the process normally exits before they finish (background warm is effectively a no-op).
- `_err(str(e))` with server/engine text is not markup-escaped (`[...]` in messages can be swallowed by Rich). Rust must apply the same markup parser to keep identical stripped text.

---------------------------------------------------------------------------------------------------

## 6. Test map (tests/ files that drive the CLI)

| test file | commands / functions covered |
|---|---|
| test_cache_cli.py | `cache clear` |
| test_catalog_safety.py | `sr create`, `cli_sr.fields_doc` |
| test_change_approval_actions.py | `changes approval-respond`, `changes approval-cancel` |
| test_change_approval_groups.py | `changes approvals --groups` |
| test_change_approvals.py | `changes approval-request`, `changes approvals` |
| test_change_asset_categories.py | `assets_resource`, `_fetch_asset_categories`, `_resolve_asset_category` (`changes assets *`) |
| test_change_associations.py | `changes associations add`, `ls` |
| test_change_evidence_service.py | `get_resource` (`changes get --stats --json`) |
| test_change_nested_subcommands.py | `changes assets` (legacy routing + `ls`), `changes associations` (legacy + `add`), `changes tasks-update` |
| test_change_optional_fields.py | `changes tasks-add`, `changes approval-request` |
| test_change_task_wire.py | `changes tasks-delete` |
| test_change_tasks.py | `changes tasks-add` |
| test_change_workflow_helpers.py | `assets_resource` |
| test_changes_form_cli.py | `changes create`, `changes update`, `form fields/explain/dump/diff` |
| test_client_session.py | `auth login`, `auth status` |
| test_completion_autocomplete.py | `_complete_task_system`, `_task_choice_value` |
| test_completion_shell_fallback.py | `--show-completion`, `_patch_typer_shell_detection`, `_shell_from_env` |
| test_content_fields.py | `changes create --dry-run -o json`, `form explain`, `form fields` |
| test_create_flags.py | `changes create` (legacy flags), `_missing_required_change_fields` |
| test_fix_round.py | `_err_exc` |
| test_formui_change.py | `changes create`, `changes update` (interactive form path) |
| test_formui_globex.py | `changes create` |
| test_formui_sr.py | `sr create` |
| test_journal_editor.py | `journal ls/tail/path/--help`, `changes add-note` |
| test_problem_internal_api.py | `note_resource`, `update_resource` (problems) |
| test_query_hash_params.py | `list_resource` (query_hash/order params) |
| test_read_completeness.py | `{changes,problems,tickets} add-note --help`, `tickets download --all --json` |
| test_readme_command_matrix.py | every group `--help` |
| test_requested_for.py | `changes create --from … --dry-run -o json`, `changes update --set` |
| test_rules_workspace.py | `form fields` |
| test_search_routing.py | `changes views`, removed `changes filter(s)`, `changes --help`, `global_search`, `search_resource` |
| test_sr_catalog.py | `sr fields` (and catalog helpers) |
| test_sr_create.py | `sr create` |
| test_sr_get.py | `sr get`, `sr --help` |
| test_task_status_portability.py | `_task_status_value` |
| test_ticket_create.py | `tickets create` (dry-run json, `--yes` exit 2, bad `--expect-plan` exit 4) |
| test_ticket_details.py | `tickets approvals`, `ticket_approvals_resource`, `ticket_associations_resource`, `conversations_resource`, `get_resource` |
| test_ticket_notes.py | `tickets add-note` (incl. `--confirm-public`) |
| test_tui_read_only_config.py | `tui --read-only` / `tui.read_only` config |
| test_universal_config.py | `auth login` (`--no-input` root vs local, `--header`, `--domain`, `--store`) |
| test_verbose_logging.py | `-v/--verbose` anywhere (`changes ls -v`, `help -v`), help "Global options", `cli.main` |
| test_views_cli.py | `changes ls --view/--where/-o`, `changes get`, `tickets get`, `tickets ls`, `changes views` |
| test_write_safety.py | `changes update`, `changes add-note`, `changes clone`, `changes tasks-update`, `tickets update`, `tickets reply`, `_confirm_writes`, `_resolve_agent/_group/_requester`, `_asset_display` |

Not CLI-driven (lower-module tests): test_cache_config_hardening, test_choice_descriptions_polish, test_completion_gen, test_download_attachments, test_formui_{gallery,qa,units}, test_planning_*, test_qa_regressions, test_query_edge_cases, test_schema_cache, test_session_store (fakes the keychain `security` subprocess only), test_set_field, test_tui_*.
