# Spec: `fsv-formui` + `fsv-tui` (Textual → ratatui 0.30 / crossterm 0.29)

Sources: `src/fsv/formui/{app,screens,widgets,session,lookups,when,fuzzy}.py`,
`src/fsv/tui/{app,modals,actions}.py` + `app.tcss`, `docs/formui.md`, tests in
`tests/`. Line refs are `file:line` in the Python tree (Textual 8.2.7). "Toast"
means Textual `notify` (bottom-right, default timeout **5 s**, severities
information/warning/error, optional title). Markup in quotes is Rich markup:
`[dim]x[/]` etc. — render it through the markup→`Line` converter (§D6).

---------------------------------------------------------------------------

## 0. Textual semantics the port must reproduce (both crates)

| Concern | Textual behavior relied on | Port rule |
|---|---|---|
| Focus chain | Tab / Shift+Tab (`Screen` bindings) walk focusable widgets in DOM order, skipping hidden (`display=False`) and disabled ones. Focusable here: `Input`, `TextArea`, `Checkbox`, `Button`, `DataTable`, `VerticalScroll` (scroll containers), `CalendarGrid`, `OptionList` (except ComboBox's, `can_focus=False`, widgets.py:285). | `FocusRing` rebuilt each frame from the visible, enabled widget list in compose order. |
| Key dispatch | 1) app/screen **priority** bindings; 2) `Key` event to focused widget, bubbling to ancestors → screen → **app `on_key`**; a handler can `stop()`/`prevent_default()`; 3) non-priority bindings along the focus chain (innermost first: widget → container → screen → app). A binding action raising `SkipAction` falls through to the next. `Input`/`TextArea` consume printable keys. | Reproduce as an explicit chain (§D3). |
| Input | Bindings: ←/→ (+shift select), ctrl+←/→ word, home/ctrl+a, end/ctrl+e, backspace, delete/ctrl+d, ctrl+w (word left), ctrl+u (to start), ctrl+k (to end), ctrl+backspace (word right), ctrl+shift+a select all, ctrl+x/c/v cut/copy/paste, enter → `Submitted`. Programmatic `value=` **also** emits `Changed`. A `Suggester` shows a dim ghost completion after the cursor; → at end of text accepts it. `type="number"` restricts typing to a float-like pattern. Placeholder shown dim when empty. | `LineInput` widget (§D5). |
| TextArea | Soft wrap; `tab_behavior="focus"` (Tab moves focus). Bindings: arrows, ctrl+←/→, home/end(ctrl+a/e), pgup/pgdn, shift-select, f6 select line, f7 select all, backspace, ctrl+w/ctrl+backspace, delete/ctrl+d, alt+delete, ctrl+x/c/v, ctrl+u, ctrl+k, ctrl+shift+k, ctrl+z/y. `load_text` emits `Changed`. | `ratatui-textarea` wrapper. |
| OptionList | ↑/↓, home/end, pgup/pgdn, enter → `OptionSelected(index)`; highlight change → `OptionHighlighted`; mouse click selects. | `PickList` widget. |
| DataTable (row cursor) | ↑/↓, pgup/pgdn, home/end, ctrl+home/end, enter; `move_cursor` / cursor change → `RowHighlighted(row_key)`. | `VirtualTable` (§D5). |
| Checkbox | enter/space toggle; renders `▐X▌ Yes` (X when on). Programmatic `value=` emits `Changed`. | `Toggle`. |
| Button | enter/space/click → `Pressed`; disabled buttons ignore input and render dimmed; variants primary/success/warning/error color the button. | `Button`. |
| App keys | `ctrl+q` quit (priority, App default); `ctrl+c` → toast "Press **ctrl+q** to quit the app" titled "Do you want to quit?" (FormApp); FsvApp overrides `ctrl+c` → quit (priority, app.py:168). `ctrl+p` opens the command palette (both apps, enabled by default). | Keep ctrl+q / ctrl+c semantics. **Drop the command palette** (divergence, log in PARITY). |
| Workers | `@work(thread=True, exclusive=True, group=G)` cancels the previous worker *object* of G, but a running thread keeps going and still calls `call_from_thread`; staleness is guarded by explicit epoch/seq/key checks (port those too). Unhandled exception in a worker exits the app with a traceback (`exit_on_error=True`). | Generation counter per group + the Python guards; never let a worker panic kill the UI (§B12 divergence). |
| Timers | `set_timer(delay, cb)`; `.stop()` cancels. `call_after_refresh(cb)` runs after the next repaint. | `Timers` with a mockable clock; "after refresh" = queue to run before the next draw. |
| Docking | Several `dock: bottom` widgets **overlap** (Textual `_arrange_dock_widgets`: every bottom dock is placed at `height - h`). TUI `#status-bar` and `#key-hints` both dock bottom → only `#key-hints` (later) is visible. | Reproduce: draw only key hints at the bottom row; keep `status_line()` computed (tests/debug). |
| Footer | `Footer` lists visible (`show=True`) bindings of the focus chain + screen: key in `$footer-key-foreground`, description in text, background `$footer-background`. | `FooterHints` computed from the active binding set. |

---------------------------------------------------------------------------

## A. `formui` → `fsv-formui`

### A1. Public contract and callers

- `form_available()` (formui/app.py:16): `False` if env `FSV_NO_FORM` is non-empty or `TERM == "dumb"`; else `stdin.isatty() && stdout.isatty()` (errors → `False`). `change_cli.form_tty()` wraps it (patched in tests).
- `run_form(kind | session, **ctx) -> FormResult` (app.py:51): `open_session(kind, **ctx)` unless a session is passed; runs `FormApp(session)`; when the app exits without a result (ctrl+q) returns `FormResult("cancel", session.state, None, session)`.
- `FormApp` (app.py:27): title `fsv`; on mount reads `config.forms.theme` (`textual-dark` default; `textual-light` etc.; unknown → ignore), pushes `FormScreen(session)`, exits with its result.
- `FormResult { action: "submit" | "save_draft" | "cancel", state, plan, saved_path, session }` (session.py:110).
- Callers (`change_cli._run_form`, change_cli.py:155): `save_draft` → `FlowExit(0, "saved {path}; continue later with --from {path} (nothing was sent)")`; anything but submit-with-plan → `FlowExit(1, "cancelled; nothing was sent")`; submit → returns `plan`. `cli_sr` remaps exit 1 to `"cancelled; nothing was uploaded or sent"` (cli_sr.py:658). After the form, callers rebuild with `session.build_plan(ack_all or session.ack, meta)` for the freshness check and use `plan.draft`. Entry points: `changes create` (change_cli.py:286, `change_session(client, meta, requester, dry_run)`), `changes update` (415, `mode="edit", target, record, meta, dry_run, status_label, title=f"Edit {subject}"`), `tickets create` (ticket_cli.py:63, `ticket_session`), `sr create` (cli_sr.py:649, `sr_session`). The TUI embeds `FormScreen` (§B9).
- Embedding: `FormScreen(session)` is a **full screen** (not modal) that dismisses with a `FormResult`; it never sends. `session.send(plan)` is the embedder's send path.

### A2. `FormSession` (UI-agnostic; session.py:173) — port 1:1 into `fsv_formui::session`

Constants: `WIDGET_FOR_KIND` (session.py:47): text→text, textarea→textarea, richtext→richtext, number→number, checkbox→checkbox, dropdown/department/nested→choice, multi_select→multi, lookup_user/lookup_group/lookup_agent/lookup→lookup, date→date, datetime→datetime, attachment→file, notice→notice, **unknown kinds → notice**. `ENTITY_FOR_KIND` user/group/agent; other lookups use `field.lookup_entity or "lookup"`. Synthetic keys `REQUESTER="__requester__"`, `ATTACHMENTS="__attachments__"`, `PLANNING="planning:"` + label. `MAX_EFFECT_CHOICES=30`. `_plain(text, limit=600)`: `plain_text`, whitespace collapsed, cut to `limit-1` + `…`.

Construction (session.py:175-225): validates `kind ∈ {change, sr, ticket}`, `mode ∈ {new, edit}`, ticket ⇒ new (else `ValueError`). Edit mode: `existing = name_existing(existing_from_record(spec, record), spec, record, resolver)`. `FormState(spec, meta.engine(), mode, resolver, existing)`. Time zone (`_timezone`, 242): engine tz unless source is `"system time zone"` and `meta.rules["user"]` is absent → `client.check_session()["time_zone"]` → `zone(name)` → source `"Freshservice user time zone ({name})"` (any exception → keep). If the spec has a requester field, a requester was given and the requester is "special" (`kind=="sr" or mode=="new"`), `state.set(req, requester)` (FormError ignored). `draft.requester = email or display`. Edit-mode change description prefill = `plain_text(existing[desc])`. `load_notes = rules_warnings(meta.rules) + _preselect_all()`, with `requester_note` inserted first.

Items (`_build_items`, 257): `REQUESTER` lookup item first for SR, and for ticket when the spec has no requester field (help "Select the requester by name or email."); then `spec.ordered()` minus status fields on new change/ticket; then for change: `ATTACHMENTS` ("Files for the change's Attachments section; uploaded only after you confirm. Separate several with ';'.") and one `planning:<label>` file item per planning-type field (`"{label} (planning files)"`, help + "Planning field files, attached after the change is saved."); for SR/ticket with `item.allow_attachments`: `ATTACHMENTS` ("General files for the request; uploaded only after you confirm."). Each spec item: `help=_plain(f.help)`, `placeholder=_plain(f.placeholder, 80)`, `allowed_types`, `max_files`.

`view(key) -> ItemView{visible, required, disabled, disabled_reason, value, text, error, note}` (287): REQUESTER → visible, required, value=requester. ATTACHMENTS → required only for SR (`ev.attachments_required` if not None else `item.attachment_mandatory`). PLANNING → files from `draft.planning[label].files`. Spec field → visible/required from `ev`; requester value replaced by `self.requester` when special; disabled → reason `decision.describe()` or `"read-only"`; restricted → note `"choices limited by rule {rule_name or rule_id}: {first 8 visible labels joined ', ' or 'none'}"`; unavailable → error = first `unavailable_errors()` starting with label, else `"not offered now"`. Then `errors_by_key[key]` overrides error; else `"required"` when visible+required+empty and (touched or `show_all_errors`). Empty = checkbox `!= True`; others `None, "", (), []`.

`text_for(item, value)` (333): empty → `""`; date/datetime → `when.to_text(value, tz)`; choice/multi → `state.display(name, value)`; file → paths joined `"; "`; change richtext → current markdown (`_desc_md` or prefill); `LookupValue` → `str()` (`Name <email>`); tuple → `", "`-joined; else `str`.

`problems()` (359): `"Requester is required"` when special and no requester and (sr/ticket or the spec has a requester field); then `state.errors()` except `"blocked by rule…"` and `"{requester label} is required"` when special; SR required attachments missing → `"Attachments: at least one file is required"`; each visible `errors_by_key` → `"{label}: {error}"`; dedupe keeping order. `missing_keys()` = visible+required+empty items. `visible_keys()`. `input_errors()` = visible `errors_by_key` as `"{label}: {e}"`.

`help_for(key)` (389): `{label, required, help, placeholder, choices[(label, desc)] (choice/multi pool), effects, why[], kind, disabled (reason), note}`; `why` gets `"time zone: {tz_source}"` (datetime) / `"date only"` (date) + `str(e)` for `state.explain([name])` entries with a decision.

`suggestions(key, query) -> (Vec<Suggestion{label, detail, value, selected}>, note)` (414; may block on network → call from a worker):
- choice/multi: pool = visible choices with label and non-empty value (`FormError` → empty). Empty pool on a nested level → note `"choose {parent label | 'the level above'} first"`. `fuzzy_filter(query, (label, description))`; `selected` = value in current (tuple for multi); note `"no choice matches"` when none.
- lookup: entity user/lookup + blank query → `([], "type part of a name or email")` (no call). Else `lookups::search_field`; `LookupUnavailable` → for user with `@` in query offer `Suggestion("use {q}", "typed address (search unavailable)", LookupValue(None, q, q))`; note `"{err}; type the exact email"` (user) or `str(err)`. Success → `Suggestion(str(r), "", r)`, note `"no match"` when empty.

`effects(key)` (447): only choice/checkbox spec fields not disabled. Cache key `(key, repr(sorted(state.values)))` (Rust: hash of canonical sorted values). Options: checkbox ticked/unticked; choice first 30 pool entries. Skip current value. For each, fork state (`_fork`, 121: copy values, sources, `_seq`, `_cleared`, `_events`, `acknowledged`), `set`, diff parts (`_diff_parts`, 481): `show {labels with " (required)"}`, `hide …`, `make required: …` (not appeared), `make optional: …` (still visible), `limit {F} to {labels | 'nothing'}`, `offer every choice again in …`, `show the message {text!r}` (Python repr quoting — use `fsv_py::py_repr_str`). Line = `"{Ticking|Unticking|Picking {label}} will " + "; ".join(parts)`.

`set(key, raw) -> Outcome{notes, error}` (502): mark touched, drop `errors_by_key[key]`.
1. Requester (synthetic, or spec requester when special) → `_set_requester` (569): non-`LookupValue` → error `"pick a person from the list"`; sr/ticket without email → `"{person} has no email address"`; store; `draft.requester`; change/ticket also `state.set(req)` (FormError → error, requester stays set).
2. Synthetic file items → `check_files(raw, max_bytes)` → error stored; ATTACHMENTS → `draft.attachments`; PLANNING → `draft.planning[label] = {files, description_html: None}` or removed when empty. Returns no notes.
3. Spec file field → `check_files(raw, allowed, max_files, max_bytes)`; value = tuple of paths or None.
4. Change richtext → `_set_description` (588): `_desc_md = md`; edit and `md.strip() == prefill.strip()` → `draft.description_html=None`, html = existing; else `html = mdlite::to_html(md)` if non-blank, `draft.description_html = html` or (`""` edit / None new); `state.set(name, html)`.
5. Else `state.set(name, raw)` (FormError → `errors_by_key[key] = str(e)`); `_remember(name)` (559: user-sourced value → `draft.answers[name]`; cleared → `None` in edit, removed in new); notes = `_notes(f, diff)` + `_preselect(diff.restricted)`.

`_notes` (607) formats, cause = `"{label} = {display or '(empty)'}"`: `+ {labels(required)} appeared because {cause}`; `- {labels} hidden because {cause}`; `* {labels} now required because {cause}`; `{labels} no longer required`; `{F}: choices limited to {labels | nothing}`; `{F}: every choice is offered again`; `! {msg}` + `" (blocking: acknowledge it on the review)"`; `{labels} cleared ({label} changed)`. `_preselect` (632): visible, enabled, empty dropdown/department/nested fields with `flow.only_option` → set, remember, note `"{F} = {choice} (the only choice rule {rule_name or id} allows)"` + its notes.

`mark_invalid(key, error)` (545): non-empty → touched + store; empty → remove.

`build_plan(ack=None, meta=None)` (661): new change → `change_flow::build_create_plan(m, draft, resolver, requester, ack_all)`; edit change → `build_update_plan(m, draft, record, target, resolver, ack_all, status_label)`; ticket → `ticket_flow::build_create_plan`; sr → `catalog::build_sr_plan`. Pending `input_errors()` not already in `plan.errors` are appended to `plan.errors` **and** `plan.review.errors`, and `plan.hash = review.plan_hash = ""` (never submit an older value silently). Stores `self.plan` only when `meta is None`.

Drafts: `default_draft_path()` = cwd / `ticket-draft.yaml` | `sr-draft.yaml` | `change-{target}-draft.yaml` (edit) | `change-draft.yaml`. `save_draft(path, plan=None)` expands `~`; SR → `render_sr_template(plan, base_dir=parent)`; else `change_flow::save_draft(plan, draft, path)`; sets `saved_path`. `command_for(path, plan)` (703): `h = hash[:16]`, `q = shlex.quote(path)`, `ack = " --ack"` when `self.ack`: `fsv sr create {display_id} --from {q} --yes --expect-plan {h}{ack}` | `fsv tickets create …` | `fsv changes update {target} …` | `fsv changes create …`.

`send(plan, allow_duplicate=False)` (715): dry run → `FlowExit(2, "dry run: nothing is sent")`; ticket → `ticket_flow::send(plan, client, rebuild, allow_duplicate)`; sr → `catalog::duplicate_check`, `catalog::freshen` (diff → `FlowExit(4, "the item form changed since the review: …")`), `catalog::execute`; change → `change_flow::check_duplicate(plan, allow_duplicate || edit)`, `freshen` (diff → `FlowExit(4, "form metadata changed since the review: …")`), `execute`.

Constructors (743-811): `_default_requester(client, key)` reads `config.defaults.{key}` → `resolve_requester` → note `"Requester {v} from config defaults.{key}"`; any error → `(None, "config defaults.{key} = {v!r} not used: {msg}")`. `change_session` (edit without `custom_fields` in record → `client.int_get("changes/{target}")["change"]`; `load_meta(client, "change", mode, module_id=display_id|id|target)` for edit; default requester `defaults.requester` for new; title `"New Change"` / `"Edit change {human_display_id or target}"`). `sr_session` (`catalog::load_item_meta(client, item_ref)`, `defaults.sr_requester`, title `"New service request: {item.name or meta.name}"`). `ticket_session` (`load_meta(client, "ticket", "new")`, `defaults.requester`, title `"New Incident"`). `open_session` aliases: change/changes, sr/service_request, ticket/tickets/incident; else `ValueError("unknown form kind {kind!r}; expected change, ticket or sr")`.

Files (session.py:133-170): `check_files(raw, allowed, max_files, max_bytes)`: split list or string on `\n`→`;`; abspath+expanduser of non-blank parts; checks in order: count (`"at most {n} file(s)"`), then per path not found / `"{name}: not a file"` / `"{name}: allowed file types are {sorted .ext list}"` / `"{name}: the file is empty"` / `"{name}: {size/1e6:.1f} MB is over the {max/1e6:.0f} MB limit"`. `human_size`: `"{n:,} B"` else `"{n:.1f} KB|MB|GB"` (1024 base). `_max_bytes` = `int(float(forms.max_file_mb) * 1_000_000)`.

### A3. `lookups` (lookups.py) — `fsv_formui::lookups`

- `LookupUnavailable(what, status)`: message `"{what} is not available for your role (HTTP {status} | not offered)"`.
- `search_requesters(client, q, limit=20)`: blank → `[]`; `client.autocomplete("requesters", q, {"qf": "details"})` = `GET /search/autocomplete/requesters?q=&qf=details`; 403/404 → Unavailable("requester search"); rows with id, not `deleted` → `person(row)`.
- `person(row, id_key)`: name = `value|name|details before '<'`; email from `email|primary_email` containing `@`, else `details` `"Name <email>"` or bare email.
- `AgentsGroups` (72): lazily `client.int_get("bootstrap/agents_groups")` **once** (mutex; 403/404 error is cached and re-raised). `groups()` id+name; `agents()` skips `"(Deactivated)"`, id key `user_id` if present else `id`; `filter(entity, query, limit=50)` fuzzy over `(display, email)`.
- `search_field` (122): user → requesters; group/agent → bootstrap filter when group or the bootstrap has an `agents` list; group Unavailable re-raised; agent fallback (blank → `[]`) `resolver.lookup("agent", q, field)` (403/404 → Unavailable("agent search")); others blank → `[]`, `resolver.lookup(entity, q, field)` (403/404 → Unavailable("{label} search")).
- Rust: `AgentsGroups` behind `Arc<Mutex<…>>` (shared by worker threads).

### A4. `fuzzy` and `when` (owned by `fsv-forms`; contract used here)

- `fold`: NFKD, drop combining marks, casefold, collapse whitespace. `score`: 0 prefix, 1 word prefix (split on space after replacing `- / (`), 2 substring, 3 in description, 4 letters in order (spaces removed), else none; blank query → 0. `fuzzy_filter` stable sort by (score, index). Use `unicode-normalization`; casefold ≈ `to_lowercase` + `ß→ss` etc. — golden-test against Python.
- `when.parse_when(text, tz, tz_source, date_only, today) -> Parsed{value, preview, error}` never fails; `to_text`, `preview`, `tz_phrase`, `date_text`, `offset_text`, `month_weeks(year, month, firstweekday=0)` (full Monday-first weeks incl. adjacent-month days, = `calendar.monthdatescalendar`), `add_months` (clamps day), `TIME_SUGGESTIONS = 09:00 10:00 13:00 14:00 18:00 19:00 20:00 22:00`. DST gap/overlap errors come from `forms.tz.localize` ("does not exist", "happens twice … +02:00 … +01:00").

### A5. Widgets (widgets.py) — behavior, keys, look

Messages: `FieldEdited{key, value}` (commit), `FieldInvalid{key, error}` (unparseable text; empty error withdraws). Editors never touch the engine except `DateEditor.commit` calling `session.mark_invalid` directly (widgets.py:669) so a review opened before the message is processed still sees the error. Common: `key`, `sync(view)`, `focus_editor()`, `dirty()` (uncommitted text).

Debounce constants: `DEBOUNCE_TEXT = 0.3 s`, `DEBOUNCE_LOOKUP = 0.25 s` (test asserts 0.25), combo close-on-blur delay 0.15 s.

| Widget | Layout / look | Commit + sync rules | Keys |
|---|---|---|---|
| `TextEditor` (80; also `number`) | 1-line `Input` (Textual default: tall border `$border-blurred`, `$border` when focused, height 3). Placeholder `item.placeholder` or `"a number"` (number). Number type restricts input. | `Changed` → (re)start 0.3 s timer → `commit`: value ≠ committed → `FieldEdited(key, str)`. Blur → commit. Enter → commit + focus next. `sync`: when not focused and value ≠ view.text → committed = value = view.text. Disabled when view.disabled. | Input bindings; Enter. |
| `ParagraphEditor` (118) | `TextArea` height 5, soft wrap, no line numbers, Tab moves focus. | Same debounce/blur commit (Enter inserts newline). `sync` via `load_text`. | TextArea bindings. |
| `DescriptionEditor` (151) | Vertical: `TextArea#desc-text` h10; `Markdown#desc-preview` (hidden; when shown: max-h 14, round `$primary` border, padding 0 1); bar `"Markdown: **bold**, *italic*, - lists, [link](https://…) · F3 preview"` muted. | Debounce 0.3 s / descendant blur → `FieldEdited(key, markdown)`. | **F3** (shown in footer "Preview"): toggle preview; on: render text or `_(empty)_`, hide TextArea; off: show + focus TextArea. Hazard: in Textual hiding the focused TextArea moves focus away, so F3 can no longer toggle back — port keeps focus on the editor container in preview mode (log as a fix). |
| `CheckEditor` (216) | `Checkbox("Yes")`. | Toggle → `FieldEdited(key, bool)`; programmatic sync suppressed by `_syncing` one-shot flag. | enter/space. |
| `ComboBox` (241) — choice, multi, lookup | Vertical: (multi) `.combo-chosen` line (accent, padding 0 1): `"✓ {text}"` or `[dim]nothing selected[/]`; `Input` placeholder `item.placeholder` or `"type to search"` (lookup) / `"type to filter, Enter toggles"` (multi) / `"type to filter, Enter picks"`; `OptionList` (hidden unless `-open` and non-empty; round `$accent` border, max-height 9 incl. border, not focusable); `.combo-note` (muted, padding 0 1, hidden when empty). Option text: multi prefix `"[x] "` (bold) / `"[ ] "`, label, `"  {detail}"` dim. | See state machine below. | down/up ±1, pagedown/pageup ±8, escape close — each only when the inner Input is focused (else SkipAction → falls through; escape when closed → screen Cancel). Enter = `Input.Submitted`. Click on option picks. |
| `DateEditor` (588) | Horizontal[`Input` 1fr, `Button("Calendar")` w12 min10]; `.date-preview` (success color, `-bad` → error color, padding 0 1). Placeholder `"YYYY-MM-DD, 26 Nov 2026, tomorrow"` (date) / `"YYYY-MM-DD HH:MM, tomorrow 19:00, fri 9am"`. | `Changed` → live preview (`parse_when` with session tz) showing error or preview. Commit on Enter or Input blur: text == committed → if an invalid text was pending, clear it (`mark_invalid(key,"")` + `FieldInvalid(key,"")`); parse error → keep text as `_invalid`, `session.mark_invalid(key, err)` + `FieldInvalid`; ok → committed = text, `FieldEdited(key, value)` (None when empty). `set_value(v)` from calendar: text = `to_text(v, tz)`, preview, committed, `FieldEdited`. `sync`: disable input+button; keep text while `_invalid == input`; else when unfocused and text ≠ view.text → replace + preview. | **F2** (footer "Calendar") or button → `OpenCalendar`. The button is in the Tab chain. |
| `FileEditor` (740) | `Input` with path suggester; placeholder `"path/to/file.pdf; another.png (→ completes)"` + `"  [{types joined ', '} · max {n}]"` when rules exist; `.file-list` (muted, padding 0 1): per file `"✓ {basename}  {human_size}"` or `"✗ {basename}  missing"`, then `[dim]uploaded only after you confirm the review[/]`. | Commit on Enter or Input blur (no debounce): `FieldEdited(key, text)`. `sync` like TextEditor + disabled + list from `view.value`. | → at end accepts ghost completion. |
| `CalendarGrid` (515) | 22×9, focus bg `$boost`. Line 0: `"‹" + "{Mon} {YYYY}".center(18) + "›"` bold (CPython `str.center`: left pad = `marg//2 + (marg & width & 1)`). Line 1 `"Mo Tu We Th Fr Sa Su"` dim. Weeks: `"{d:2}"` cells joined by one space. Styles (first match): cursor reverse+bold, other month dim, today underline. | `Picked(date)`. | ←/→ ±1 day, ↑/↓ ±7, PgUp/PgDn ±1 month (footer "Prev month"/"Next month"), Home today ("Today"), Enter pick. Click: row 0 x<11 prev month else next; row ≥2 → `weeks[y-2][x//3]` then pick. |
| `FieldRow` (821) | `#row-{safe key}` (non `[A-Za-z0-9_-]` → `_`), padding 0 1, margin-bottom 1, focus-within bg `$boost`. Label bold: `label + " [bold red]*[/]"(required) + "  [dim](read-only)[/]"(disabled)`. Editor. `.field-help` muted: first sentence (`help.split(". ")[0]`, cut to 159 + `…` when ≥160), hidden when no help. `.field-note` warning: disabled reason if disabled else view.note. `.field-error` error color. Notice items: only a round `$secondary` bordered box with `plain_text(content or label)`. | `sync()`: display = visible; update label/note/error (hide empty); disable Text/Paragraph/Check editors; `editor.sync(view)`. | — |

ComboBox state machine (fields: `options`, `shown_text` = text reflecting the stored value, `options_query` = query the shown remote options answer, `searches` counter):
- `query()` = `""` when single and input == shown_text (a picked value lists everything), else input text.
- Focus on input → `open()`: no-op when disabled; add `-open` + `Opened` (screen scrolls the combo into view); remote → schedule search (immediate for group/agent, else debounced); local → `refresh_options()` = `session.suggestions(key, query())` synchronously.
- Typed (only when input focused; single and text == shown_text and closed → ignore): open if closed; remote: if `options_query ≠ query()` clear options + note `"searching…"` then debounce 0.25 s; local: refresh.
- `_search()`: q = query(); searches+=1; note `"searching…"`; worker (exclusive per combo): `suggestions(key, q)`; exception → `([], first 200 chars of whitespace-collapsed message)`; on UI thread `_found(q, …)`: drop if `q ≠ query()` or closed; else `options_query = q`, show.
- `show_options`: replace list; hide list when empty; highlight: multi keeps old index if valid, else 0; note = highlighted detail (local only) ` · ` note, dim.
- Enter: closed → open. Single with blank text → `shown_text=""`, `FieldEdited(key, None)`, close. Remote stale (`options_query ≠ query()`) → note `"still searching… (Enter again when the results are shown)"`. Else pick highlighted.
- Click: stale → note `"still searching…"`; else pick + refocus input.
- `pick(i)`: multi → toggle value in `view(key).value` list → `FieldEdited(key, list or None)`, clear input, stay open (sync refreshes local options). Single → `shown_text = text_for(value) or label`, input = shown_text, `FieldEdited`, close.
- Blur → after 0.15 s close if input not focused. `close()`: remove `-open`, clear options/options_query/note; single + unfocused + text ≠ shown_text → revert text.
- `sync`: input.disabled; multi → chosen line, refresh if open & local; single → when unfocused and text ≠ view.text: shown_text = input = view.text; elif open & local → refresh.
- `dirty()`: non-blank text and (multi or text ≠ shown_text). No `commit()` (typed person text is never committed by Ctrl+S/F4/Esc).

`complete_path(value)` (718): last `;` segment, left-stripped; blank → None; expanduser; list dir (`"."` default; error → None); names starting with base, hidden ones only when base starts with `.`; common prefix; single directory hit → `+ "/"`; equal to base → None; returns `value + suffix` (sorted listing).

### A6. Screens (screens.py)

**FormScreen** (449) — full screen, dismisses `FormResult`.
Layout (top→bottom): `#form-title` (h1, bg `$primary`, `$text`, bold, padding 0 1): `title + " (edit)"(edit) + "  [dim]dry run[/]"(dry run)`. `#form-notes` (auto, max 4 lines, padding 0 1, `$accent`). `#form-body` Horizontal: `#form-fields` VerticalScroll 2fr (all FieldRows in item order, then `#form-actions`: Buttons `Review & submit` (#btn-review, primary), `Save draft`, `Cancel` (error), margin-right 1, padding 0 1, margin-bottom 1) | `#form-help` VerticalScroll 1fr, min 30 max 64 cols, left border solid `$primary`, padding 0 1, `#help-text`. `#form-status` (h1, padding 0 1, bg `$panel`, muted). `Footer`.
Narrow (`app width < 100`, re-checked on resize): body vertical, fields 100%×1fr, help 100%×9 rows with top border instead. F1 toggles `-hide-help` (help hidden).
Bindings: **ctrl+s** Review (priority — works inside TextArea), **F4** Save draft, **Esc** Cancel, **F1** Help panel; plus footer entries of the focused widget (F2 Calendar, F3 Preview, PgUp/PgDn/Home in calendar).
Mount: narrow class, `refresh_rows()`, `show_notes(load_notes)`, after refresh focus the first visible, non-disabled item that has an editor.
`refresh_rows()` (526): sync every row; `#btn-review.disabled = problems non-empty`; status text: missing → `"{n} required field(s) missing: {first 4 labels}{ +k}"`; else first problem; else `"Ready: Ctrl+S to review and submit"`; always `+ "  ·  Ctrl+S review · F4 save draft · Esc cancel"`; re-render help (forced) if a help key is shown.
`show_notes(notes)`: last 4 into `#form-notes`, toast with all notes (6 s), clear `#form-notes` 8 s later (`NOTE_SECONDS`, restart per call).
Help (`show_help`, 554; on descendant focus of a row, skipped when same key unless forced): `[b]{label}[/b]` + `" [bold red]* required[/]"` | `" [dim]optional[/]"`; help; `[dim]e.g. {placeholder}[/]`; `[yellow]read-only: {reason}[/]`; `[yellow]{note}[/]`; blank + `[b]Choices[/b]` + up to 40 `" • {label}"` + `" [dim]— {desc}[/]"`, then `" [dim]… {n} in all; type to filter[/]"`; blank + `[b]Rule effects[/b]` + `" {effect}"` lines; blank + dim `why` lines; blank + dim kind hint: choice `"Type to filter · ↑↓ move · Enter picks · Esc closes"`, multi `"… Enter toggles · Esc closes"`, lookup `"Type to search · ↑↓ move · Enter picks"`, date `"Type a date or press F2 for the calendar"`, datetime `"Type a date and time or press F2 for the calendar"`, file `"Paths separated by ';' · → completes the path"`, richtext `"Markdown, sent as HTML · F3 preview"`. All values escaped.
Events: `FieldEdited` → `session.set` → `refresh_rows` → `show_notes(out.notes)`; `FieldInvalid` → `mark_invalid` → refresh; `OpenCalendar` → push `DatePickerScreen(label, view.value, date_only, tz, tz_source)`; non-None result → `editor.set_value`; refocus input.
Actions:
- Review: commit the focused row's editor (`commit()` if it has one), then after refresh: `show_all_errors = true`, refresh, `plan = build_plan()`, push `ReviewScreen`. (Ctrl+S always opens the review even with problems; the disabled button only blocks clicks.) Result: submit → `FormResult(submit, state, session.plan)`; save_draft → with `saved_path`; cancel → cancel; back → focus first `missing_keys()` or first visible `errors_by_key` key.
- F4: commit focused; `SavePathDialog(default_draft_path(), save=session.save_draft)` → saved → `FormResult(save_draft, plan, saved_path)`.
- Esc: `dirty` = any editor `dirty()` (error ⇒ dirty); commit focused; untouched and not dirty → dismiss cancel; else `ConfirmDialog("Discard this form? Nothing has been sent.", "Discard", "Keep editing")` → yes → cancel.

**ReviewScreen** (312) — full screen, dismisses `submit | back | save_draft | cancel`.
Layout: `#rv-title` (h1, primary bg, bold) `"Review: {title}"`; `#rv-banner` (warning, bold, padding 0 1, hidden when empty); `#rv-scroll` VerticalScroll 1fr padding 0 1 with `#rv-body` (review render) + `#rv-payload` (hidden until F7; muted); `Checkbox#rv-ack "I acknowledge the blocking rule message(s)"` only when `plan.review.blocking_unacked` or `session.ack`; `#rv-actions` Buttons: Submit (success), `Back to edit` (primary), `Save draft`, `Copy as command`, `Cancel` (error); Footer. Mount: render, focus Submit if enabled else Back.
Body = `Review.render(console)` at width `max(40, screen width − 4)` (re-render on resize) — identical to the CLI review (fsv-flows). Payload: `"Payload\n" + json.dumps(payload, indent=2, default=str, ensure_ascii=False)` when payload and `plan.ok`, else `"No payload yet — fix the problems above"`.
Banner lines: SR → `"⚠ " + catalog::SAFETY` (`"This sends a real service request to the catalog item's fulfilment team."`); dry run → `"Dry run: Finish shows the plan in the terminal; nothing is sent."`; edit with ok plan and empty hash → `"No changes: nothing would be sent."`; blocking unacked → `"A blocking rule message needs your acknowledgement (tick the box below)."`.
Submit label `"Finish (dry run)"` | `"Place request"` (sr) | `"Update"` (edit) | `"Create"`; disabled unless `plan.ok && plan.hash && !blocking_unacked`. Copy-as-command disabled without hash.
Bindings: **Esc** Back to edit, **ctrl+s** Submit, **F4** Save draft, **F6** Copy as command, **F7** Show payload. Ack toggle → `session.ack = v`, `plan = session.build_plan()`, re-render. Submit when disabled → toast `"fix the problems first (Back to edit)"` warning 4 s. F4 → `SavePathDialog(default, save=|p| session.save_draft(p, plan))` → `save_draft`. F6 → no hash → toast `"no complete plan yet"` warning 4 s; else `SavePathDialog(default, "Save the draft for the command to", save=…)` → push `TextDialog("Equivalent command", session.command_for(relpath(saved), plan), "draft saved to {saved}; the command refuses to send if the plan changes")`; the review stays open.

**Dialogs** (all modal, centered, `background: surface`, `border: thick $primary`, padding 1 2; the screen behind is dimmed by Textual's modal overlay):

| Dialog | Size | Content | Keys / result |
|---|---|---|---|
| `ConfirmDialog(q, yes, no)` (41) | w60, ≤95% | question; Buttons yes (error) / no (primary), focus **no** | Esc → false; `y` → true; buttons. |
| `SavePathDialog(default, title="Save draft to", save=None)` (75) | w76 | `[b]{title}[/b]`, `Input#dlg-path` = default (focused), `.dlg-note` (warning), Save (primary)/Cancel | Enter/Save validate in order: blank → `"type a file path"`; abspath(expanduser); is dir → `"{path} is a folder: add a file name"`; parent missing → `"cannot save: the folder {folder} does not exist"`; exists and not yet confirmed for this path → `"{path} exists: press Save again to overwrite"`; `save(path)` I/O error → reset confirm, `"cannot save to {path}: {strerror or e}"` and stay; success → dismiss saved path (or path when no `save`). Esc/Cancel → None. |
| `TextDialog(title, text, note)` (154) | w90% | `[b]title[/b]`; `#dlg-command` text on `$panel` padding 1; optional dim note; Copy (primary, focused)/Close | Copy → clipboard (Textual uses OSC 52) + toast `"copied to the clipboard (terminal clipboard support permitting)"` 4 s. Esc/Close. |
| `DatePickerScreen(label, value, date_only, tz, tz_source)` (195) | w60, ≤98% | `[b]label[/b]`; row: `CalendarGrid#dp-cal` + (datetime) `#dp-times` (1fr, h9, margin-left 2): `Input#dp-time` (placeholder `HH:MM`) above `OptionList#dp-suggest` (TIME_SUGGESTIONS); `.dp-preview` (success, margin-top 1); hint `[dim]arrows move · PgUp/PgDn month · Home today · Enter picks[/]`; OK (primary)/Cancel | Initial: aware datetime → `astimezone(tz)` (keeps the instant) → day + `"%H:%M"` (`":%S"` when seconds); date → day, `09:00`; none → today in tz, `09:00`. Focus calendar. Preview after every key/time change: value → `when.preview(v, tz_source)` else error or `"type a time like 19:00"`. Grid Enter → date only: dismiss date; else focus time input. Suggestion picked → set time, focus OK. Time Enter / OK → `value()` (date only: cursor; else `parse_when("{iso} {time}", tz, tz_source)`) → dismiss when valid. Esc/Cancel → None. |

### A7. Theme (FormApp = Textual theme; default `textual-dark`, `forms.theme` may pick `textual-light`)

| var | textual-dark | textual-light |
|---|---|---|
| primary | #0178D4 | #004578 |
| secondary | #004578 | #0178D4 |
| accent / warning | #FEA62B | #FEA62B |
| error | #B93C5B | #B93C5B |
| success | #4EBF71 | #4EBF71 |
| foreground | #E0E0E0 | #1F1F1F |
| background | #121212 | #E0E0E0 |
| surface | #1E1E1E | #D8D8D8 |
| panel | #242F38 | #D0D0D0 |
| boost | #FFFFFF0A (≈ blend 4% white) | transparent |
| text / text-muted | auto 87% / auto 60% (contrast color at that alpha over the bg) | same |
| block-cursor bg | #0178D4 | #004578 |
| footer key fg / footer bg | #FFA62B / #242F38 | #0178D4 / #D0D0D0 |
| input border focused / blurred | $border = primary / #191919 | primary / #D0D1D0 |
| input selection bg | #368AE966 | #23568B66 |

Blend alpha colors onto the parent background when converting to `Color::Rgb`. Other theme names: map the Textual built-ins we care about; unknown → default (as Python).

### A8. Error paths (formui)

Lookup failures → dim combo note, never a crash; worker exceptions in combo search → note; invalid typed dates → kept text + inline error + blocks submit (plan hash cleared); file errors inline; `FormError` from `set` inline; save errors inside the dialog; dry-run send → `FlowExit(2)`; freshness change → `FlowExit(4)` (CLI path asks again via caller, `send()` refuses). Constructors may raise (meta load / record fetch) — callers turn them into exits / TUI failure screens.

---------------------------------------------------------------------------

## B. `tui` → `fsv-tui`

### B1. Launch, read-only

`fsv tui [--read-only]` → `launch(read_only)` (tui/__init__.py) → `FsvApp(read_only).run()`. `actions.resolve_read_only(flag)` (actions.py:20): flag wins; else `config.tui.read_only`: string → `strip().lower() ∈ {on,true,1,yes}`; non-dict `tui` or load error → false; else `bool(val)`. Read-only effects: key hints omit write keys and say `r reload` instead of `r reload/reply`; `s`/`n` → toast `"Read-only mode: write actions are disabled"` (warning); `r` in the detail pane reloads (reply not possible); `a` → `"Read-only mode: actions are disabled"`; `N`/`e` → `"Read-only mode: write actions are disabled"`; **every** later callback and worker re-checks `read_only` and returns silently (defense in depth, tests call internals directly); senders get `read_only=` too.

### B2. State (app.py:173-229)

`entity` (tickets|problems|changes, default tickets), `detail_tab_idx`, `_items`, `_total`, `_schemas{res: schema}`, `_selected`, `_detail_cache{"res:id": record}`, `_details{"res:id": DetailPage}`, `_sub_cache{"res:id:tab": TabData}`, `_g_pending` + `_g_pending_pane`, `_pending_detail_key`, `_pending_sub_key`, debounce timers (detail, entity), `_list_fr=11`/`_detail_fr=9`, `_page`, `_list_epoch`, `_write_failure`, `_per_page=100`, `_has_more`, `_loading_more`, `_active_filters` (= `config.get_default_where(entity)`, errors → []), `_filter_error`, suggestion state (`_suggestions`, `_suggestion_idx`, `_tab_lock`, `_tab_base`, completion timer + key), filter history (`_filter_history` ≤100, `_history_idx=-1`, `_history_saved`, `_history_nav_lock`), search state (`_search_mode`, `_search_term`, `_search_totals`, `_search_entity` (None=ALL), `_search_sort` relevance|created|modified, `_search_page`, `_search_has_more`, `_search_seq`, `_search_cache{entity: (items, totals, page, has_more)}`), site views (`_views{res: [SavedView]}`, `_view{res: SavedView}`, `_headers{tickets: …}`, `_me`, `_tz`, `_list_total`, `_list_sort="created_at"`, `_columns`). Lock-counters (`_tab_lock`, `_history_nav_lock`) exist only because programmatic `Input.value=` emits `Changed`; in Rust set the value without emitting and drop the locks.

### B3. Layout (compose 231 + app.tcss) — transparent backgrounds everywhere, `ansi_color=True` (Rich colors like `cyan`/`green`/`yellow` stay native ANSI; `$vars` are textual-dark RGB)

```
#header            1 row, padding 0 1                                     (dock top)
#main
  #list-pane       11fr (min 8), bottom border heavy $surface → $success when focus within
    #list          DataTable 1fr (min 4), row cursor, no zebra, 1-col scrollbars
    #filter-input  1 row, no border, padding 0 1 (hidden until "/")
    #search-input  1 row, no border, padding 0 1 (hidden until "f"), placeholder "search freshservice..."
    #suggestion-bar 1 row (hidden unless suggestions/hint)
    #filter-bar    1 row, muted
  #detail          9fr (min 8), top border heavy $surface → $success when focus within
    #detail-bar    1 row
    #state-flow-bar 1 row (hidden when empty)
    #reporter-line 1 row (hidden when empty)
    #detail-tabs   1 row
    #detail-body   Horizontal 1fr
      #detail-scroll  VerticalScroll 3fr, padding 0 1 → #detail-content
      #detail-side    VerticalScroll 2fr, padding 0 1, left border tall $surface (hidden unless details tab with a page) → #detail-props
#status-bar        1 row (dock bottom; covered by key-hints, see §0)
#key-hints         1 row, muted (dock bottom)
```
Filter input placeholder `"status=Open priority=High"`. `[`/`]` resize: `list_fr = clamp(list_fr + d, 3, 17)`, `detail_fr = clamp(detail_fr − d, 3, 17)`. Mount: focus the table, render header + key hints, detail `"[dim]loading rows...[/]"`, start list load.
Modal CSS: overlay `$background 60%`; `.dialog` w64 ≤90%, auto height ≤90%, padding 1 2, heavy `$accent` border, `$surface` bg, `$text`; `.dialog-title` margin-bottom 1; `.dialog-hint` margin-top 1 muted; buttons min-w14, `$panel` bg, tall `$primary` border, bold, focus → `$primary` bg / `$background` fg; warning buttons tall `$warning` border, focus warning bg. `#view-options`/`#status-options` auto height ≤18/≤16, no border, `$panel` bg; `#view-filter` margin-bottom 1; `#note-text` h8 tall `$primary` border; `#note-visibility`/`#note-preview` margin-top 1 ≤6 rows. HelpScreen dialog w76, ≤90% height, heavy `$accent`, padding 1 2.

### B4. Loading, workers, caches (no periodic refresh, no keepalive: data loads only on demand)

| Worker (group) | Trigger | Does (service functions) | Result guard / errors |
|---|---|---|---|
| `_load_list(append)` ("list", exclusive) 279 | mount, reload, entity switch, filter apply, view pick, load-more | `_fetch_list`: for each of tickets/changes/problems not yet in `_schemas`: `schema::load(res, c)` (**SessionError re-raised**; other errors → `{"fields": []}`); once: `views::current_user` → `_me`, `views::user_tz`; once per res: `views::load_views` → `_views`, `_view.setdefault(default_view)`; tickets once: `views::ticket_headers`. Filters → `query::build_query_hash_from_schema(c, res, schema, filters)` (ValueError → `_list_failed`). View used only without filters; `view.raw` → `views::view_settings` → order_by/type (default `created_at desc`). `service::list_page(res, client, page, per_page=100, query_hash, filter_name=view.id, order_by, order_type)`; `views::list_meta(data, items, page, per_page)` → (total, more). | `_accept_list`: drop unless `(epoch, entity)` current and not search mode; append requires `page == _page + 1`. Any exception → `_list_failed`: same guard, `loading_more=false`, toast (message, title `"List load failed; reload or retry"`, error). |
| `_load_detail(item, key)` ("detail", exclusive) 423 | row highlight, after 0.15 s debounce; `_after_write` | cache hit (both caches) → reuse; else `views::load_detail(c, res, id, schema)` → page; `page.record["_resource"]=res`; fill `_details` + `_detail_cache`. | Error and nothing cached → `"could not load: {clean_error}"` red in content; else stale cache. `_show_detail` only if `_pending_detail_key == key`. |
| `_load_sub_resource(tab, key)` ("sub", exclusive) 476 | non-details tab shown | cache `res:id:tab`; else `views::load_tab(c, res, page.record or item, tab)`; error → `TabData(tab, tab_label(tab), missing=Missing(clean_error))` **cached too** | render only if `_pending_sub_key == key` and still the selected item + tab. |
| `_load_search(append)` ("list", exclusive) 1177 | search apply / narrow / sort / more | load missing schemas (no try); `service::search_items(term, entity, sort, page, client)` → (items, totals) | `_populate_search` drops unless `seq == _search_seq`. Python: an exception here kills the app (worker error) → Rust: toast instead (divergence). |
| `_run_network_lookup` ("completion", exclusive) 927 | filter value typed (debounce 0.3 s) | `client.lookup_choices(link, prefix)` (lookup fields: rows sorted email/name prefix first, top 8, email if it starts with prefix else name) or `completion::_remote_user_values(kind, prefix)` top 8 | drop unless key current, not cycling, filter input visible+focused; errors ignored. |
| `_load_catalog` ("catalog") 1396 | N → Service Request | `catalog::list_items`, `catalog::list_categories` | error → FailureScreen `"Catalog unavailable: {exc}"`. |
| `_build_form` ("form") 1426 | N/e | `ticket_session(c)` / `sr_session(c, item_ref=display_id)` / `_form_session` (`change_session(c)` or `change_session(c, mode="edit", target=display_id|id, record=page.record copy without _resource or item)`) | error → FailureScreen `"Form unavailable: {exc}"`. |
| write workers ("write", not exclusive): `_send_form`, `_do_write`, `_send_ticket_note`, `_send_change_task`, `_execute_action`; readers ("task-create-review", "action-read", "task-read") | see B9 | | |

Row highlight (`on_data_table_row_highlighted`, 2135): find item by display id; `detail_tab_idx=0`; cached → show at once, else `_set_pending_detail(key)` (`"[dim]loading details...[/]"`); restart 0.15 s timer → `_load_detail`; cursor on last row and not loading → search mode with more → `_search_load_more`, else `_has_more` → `_load_more` (sets `_loading_more`, renders bars, `_load_list(append=True)`).
`_populate_list` (351): dedupe by `res:id` on append; on replace rebuild columns (`views::list_columns(res, schema, headers)`; search mode → none) with a leading `""` column; fallback columns `"", ID, STATUS, PRI, SUBJECT, REQUESTER` (`service::resolve_status`, `resolve_priority`, subject[:80], requester name). Cells from `views::list_row(item, res, columns, schema, me_id, tz)`: row number `"{n:03}"`; subject = `"#{did} "` dim cyan + `clip(val, 60)`; status green `clip(24)`; `assigned_to` ending `" (Me)"` and >40 → `clip(val[:-5], 35) + " (Me)"`; others `clip(val, 40 if assigned_to else 34)` or `"--"` dim. `clip(v, w)` = `v[:w-1] + "…"` when longer. Row key = display id. Not append: rows → `"[dim]loading details...[/]"` + cursor row 0 (fires highlight); none → `"[dim]no rows[/]"`. Then header, filter bar, status.
`_reload_list(msg="[dim]reloading...[/]")` (1272): epoch++, `_list_total=None`, clear items/page/has_more/loading/g_pending/filter_error, **clear detail + sub caches** (schemas, views, me stay), clear table, focus list, header, `_clear_detail(msg)`, load.
`_switch_entity(name)` (2173): no-op if same; set entity, tab 0, epoch++ (in-flight results dropped), reset has_more/loading, default filters, filter_error None, header; 0.2 s debounce → `_reload_list("[dim]loading rows...[/]")`.
`_after_write(res, item, msg)` (2622): toast msg; drop `_detail_cache[res:id]` and every `_sub_cache` key with that prefix; if it is the selected item → pending key + `_load_detail`.

### B5. Rendering (all user/server text escaped or styled as plain `Text`)

- Header (1292): normal `"[bold cyan]fsv[/] :: freshservice  "` + per tab `"[bold reverse] TICKETS [/] "` (active) / `"TICKETS  "` + `"  " + view_header` + `"  [dim]v views  ? help[/]"`. View header (1326): name = view name | `"Filtered"` (filters) | `"All {res}"`; range `"1-{rows}"` or `"0"`; of = `" of {list_total}"` when known else `" of {rows}{+ if has_more}"`; `"[bold]{name}[/][dim] · {range}{of} · Sort by: {views::sort_label(list_sort, schema)}[/]"`. Search: `"[bold cyan]fsv[/] :: [yellow bold]search[/] [yellow]{term}[/]  "`, ALL tab (reverse when entity None), per tab `LABEL(count)`, `"  [dim]{rows}/{total}|{rows} shown  sort:{relevance|date created|last modified}  Esc exit[/]"`.
- Filter bar (1473): part = search `"[yellow]search: {term}[/]  [dim]h/l narrow  s sort  Esc exit  f new search[/]"` | `"[red]filter error: {e}[/]"` | `"[yellow]filter: {filters joined}[/]"` | `"[dim]/ to filter[/]"`; then `"  [dim]focus={pane}  sel={did|-}  {rows/total|rows} rows[/]"` + `"  [yellow]loading more…[/]"`.
- Status bar (1604): `ENTITY` or `"SEARCH: {term}[ > ENTITY]"` + `" > {did} > {tab}"` + `"    "` + dim `"pane={pane} · {loading more…|N rows}"`.
- Key hints (1626), exact: `Tab pane  j/k move or scroll  J/K ×5  G/gg ↕  h/l entity/tab  gw/gt/gp/gc entity  1-9 detail tab  v views  [] resize  f search  / filter  o browser  ` + (write: `s status  n note  N/e create/edit  a actions  ! failure  `) + `r reload/reply  ` (read-only `r reload  `) + `y yank  q/Ctrl+C quit  ? help`, keys bold. (`gw` is advertised but unbound — keep.)
- Detail bar (1506) as plain `Text`: page → `"#{did} "` bold cyan, `subject[:90] + "  "` bold, `page.badge` bold green, `", due {page.due_text()}"` dim unless a ticket with resolved/closed stats. No page → non-change `"[{status}]"` bold green + 2 spaces, `"{did} {subject[:80]}  "`, priority dim.
- State flow (1492): `pages::lifecycle_markup(page.lifecycle, context=3)`; reporter `pages::reporter_markup(page)`; hide when empty.
- Tabs (1532): page → `pages::tab_bar(page, active, numbered=True)`; else fallback `DETAIL_TAB_LABELS` (app.py:47) `"[bold reverse] {n}:{tab} [/]"` / `"[dim]{n}:{tab}[/]"` joined by 2 spaces. Tab names = `page.tab_names()` (server tabs) else fallback list.
- Content (1551): details → page: `pages::main_lines + [""] + pages::notes_lines`, side `pages::side_lines`; no page: `_format_details` (side hidden). Other tab → side hidden, `"[dim]loading {tab}...[/]"`, worker; result `TabData` → `"\n".join([""] + pages::tab_lines(data, page.tz|_tz, page.schema) + [""])`; legacy list data → `_format_notes/_tasks/_approvals/_assets/_associations/_activity/_resolution` (1651-1882, port verbatim: section headers `[dim]── X ──[/]`, `[cyan]Label[/]` rows, icons ✓ ○ ✗ ⏳ 🔒, truncations 200/500/300 chars, `#TSK-{id}`, `L{level}`, empty → `"\n[dim]no {x}[/]\n"`).
- Help screen (90): exact text in app.py:104-154; any key closes.

### B6. Filter input (`/`)

Open (970): value = active filters joined by space, show + focus, cursor to end (after refresh); history idx 0 if `history[0] == value.strip()` else −1; saved = value; suggestions bar for value.
Keys while focused (on_key 1922): Esc cancel (hide, clear suggestion state + completion timer/key, history idx −1, focus list, filter bar); Tab → `_complete_suggestion`; ↑/ctrl+p → history older; ↓/ctrl+n → newer; Enter → apply. Other typing → `Changed` resets history idx, cycling, refreshes bar.
Apply (987): hide, reset suggestion/completion/history state, focus list; non-blank → `shlex.split` (ValueError → whitespace split) into `_active_filters`; history = `[entry] + rest without entry`, ≤100, saved to `filters_cache_path("tui_history")` as `{"history": [...]}` (`ensure_dirs`; errors ignored); blank → clear filters; `_reload_list("[dim]filtering...[/]")`.
History navigate (627): +1 older/−1 newer; no history and +1 → nothing; first move saves current text; bounds `[-1, len)`; −1 restores saved; cursor end; bar refresh.
Suggestions (`_get_suggestions`, 677) → (completions, hint): fields = `created_at, updated_at, due_by` + schema field names. Empty value → all fields. Inside an open quote (shlex fails) → none. Trailing space → all fields. Current token = after last space; no operator (`>= <= != ~= = > <`, first found in that order) → fields whose name starts with token or label starts with it (case-insensitive). With operator: date pseudo fields → hint `"YYYY-MM-DD  (operators: = != > >= < <=)"`; field match by exact then prefix name; no choices: requester → values from loaded items (`_extract_item_values`: dict name / string / custom_fields string, prefix match, ≤8) else hint `"name or email"`; agent/responder (`responder` falls back to `agent`) → `"agent name"`; group → items, then `completion::_cached_groups()` names → `"group name"`; department → `"department name"`; lookup → `"name"`; checkbox → `["true","false"]`; number → hint `"number"`; date → `"YYYY-MM-DD"`; else `"text"`. Choices → values/names starting with prefix.
Bar (780): none → dim hint or hidden; else first 8: first `[bold cyan]`, rest dim, `"  [dim](+N more)  tab→[/]"` or `"  [dim]tab→[/]"`. Always then maybe schedule a network lookup (874): requires `completion::_completion_network()`, not cycling, no trailing space, token with operator, value prefix ≥2 chars (else cancel timer), exact field name match, kind requesters / agents / lookup (needs `lookup_config.link`); dedupe by `"{kind}:{prefix}"`; 0.3 s timer → worker; results (non-empty) replace `_suggestions` and the bar (no window/overflow).
Tab completion (826): first Tab freezes `_tab_base` = value, idx 0; list = current suggestions or recomputed for base; none → reset. Field token: exactly one suggestion → `prefix + s + "="`, reset cycling, refresh bar; several → `prefix + s`, idx+1 (cycles). Value token → `prefix + field + op + (quoted s if it has a space)`, idx+1. Cursor end; bar shows a sliding 8-window centred on the active item with `…` ends, active bold cyan, `"  [dim]tab→[/]"`.

### B7. Search mode (`f`)

Open (1028): value = term if in search mode else empty; Enter → apply; Esc → hide + focus list. Apply (1034): blank → exit search mode (if in it) or just refocus; else seq++, clear search cache, enter mode with term, entity ALL, sort relevance, page 1, clear filters/filter error/items/caches/table, focus list, header, `"[dim]searching...[/]"`, `_load_search`. Populate (1193): totals; total = totals[entity] when narrowed (has_more = `len(items) == 30`) else sum (no paging for ALL); columns `"", ID, STATUS, SUBJECT, AGENT, GROUP` (raw `status`, `_agent`, `_group`); skip duplicate row keys; detail messages `"loading details..."` / `"[dim]no results[/]"`.
`h/l` (list pane) step ALL→tickets→problems→changes (wrap): save current `(items, totals, page, has_more)` for the old entity; cached target → restore (rebuild rows, cursor 0); ALL → reload; else narrow (`_search_narrow_entity`). `g`+`t|p|c` narrows directly. `s` cycles sort relevance→created→modified (clears cache, reloads). `r` re-applies the same term. `Esc` exits: seq++, clear, restore default filters, `_reload_list("[dim]loading rows...[/]")`. `/` is ignored in search mode; `v` too.

### B8. Key map (`on_key`, 1908) — ignored while any modal is on top

Order: modal on top → return. Search input focused: Esc only. Filter input focused: Esc/Tab/↑/ctrl+p/↓/ctrl+n. Then `g`-prefix: previous key `g` → `g` = top (detail: scroll home; list: row 0) / `t|p|c` = entity (or narrow in search) / anything else swallowed. Pane = detail when `#detail-scroll` has focus within.

| Key | Action |
|---|---|
| `g` | start prefix (remember pane) |
| `f` | open search input |
| `Esc` | exit search mode (when in it) |
| `s` (search mode) | cycle search sort |
| `/` | open filter input (not in search mode) |
| `Tab` | toggle pane focus (list table ↔ detail scroll) |
| `ctrl+j` / `ctrl+k` | focus detail / list |
| `?` | HelpScreen |
| `j`/`↓`, `k`/`↑` | list cursor ±1 / detail scroll ±1 line |
| `J`/`}`, `K`/`{` | ×5 |
| `G` | list last row / detail scroll end |
| `h`/`←`, `l`/`→` | detail: tab ∓1 (wrap); search: step search entity; else prev/next entity (wrap) |
| `o` | `webbrowser.open(service::item_url(res, id))`; false → toast `"Browser open failed"`; none selected → `"No row selected"` |
| `a` | action menu (B9) |
| `!` | reopen FailureScreen with `_write_failure` (if any) |
| `s`, `n`, `r` (detail pane + reply possible) | write key (B9) |
| `r` (otherwise) | search mode → re-run search; else `_reload_list()` |
| `y` | `pbcopy` the display id → `"Copied {did}"` / `"Copy failed: {exc}"` (warning) |
| `q` | quit |
| `[` / `]` | resize panes −1 / +1 |
| `v` | view picker (not in search; no views → `"No saved views on this account for {res}"` warning); pick → `_view[res]`, clear filters, `_reload_list("[dim]loading {name}...[/]")` |
| `N` | create chooser; `e` edit selected change |
| `1`–`9` | detail tab n (if a row is selected and n ≤ tab count) |
| table-native | PgUp/PgDn/Home/End/ctrl+Home/ctrl+End on the list; PgUp/PgDn/Home/End on the detail scroll; mouse click/scroll |

Hazard (unverified in Python): `FormScreen` is a non-modal `Screen`, so keys that bubble out of it (e.g. Tab from a Checkbox) reach `FsvApp.on_key`, which only guards `ModalScreen`. Rust: route keys only to the top screen.

### B9. Modals (`modals.py`) and write flows

| Modal | Content | Keys / result |
|---|---|---|
| `StatusScreen(choices, current)` | `"[bold]Change status[/]  [dim]current: {cur}[/]"`, OptionList (highlight current label else 0, focused), hint `"[dim]Enter select · Esc cancel[/]"` | Enter → `(id, label)`; Esc → None |
| `ConfirmScreen(title, lines)` | bold title; scrollable `#confirm-body` = lines joined `\n` as plain text; Buttons Confirm (warning) / Cancel (focused); hint `"[dim]y confirm · Esc cancel[/]"` | `y`/Confirm → true; Esc/Cancel → false. Tests read `_title`, `_lines`. |
| `NoteScreen(target, reply, can_toggle)` | `"[bold]Add note|reply[/]  [dim]{target}[/]"`, TextArea (focused), visibility line, preview (`"Preview: "` bold + body or dim `"(empty)"`), Continue (primary)/Cancel, hint `"[dim]Ctrl+S continue · Ctrl+T toggle · Esc cancel[/]"` | `public` starts = reply. Visibility: reply `"[b]Visibility: PUBLIC reply[/] [dim](sent to the requester)[/]"`; toggleable `"[b]Visibility: PUBLIC[/] [dim](Ctrl+T: private)[/]"` / `"[b]Visibility: Private[/] [dim](Ctrl+T: public)[/]"`; else `"… Private[/] [dim](use r for a public reply)[/]"`. ctrl+t toggle (not for reply); ctrl+s/Continue: blank → toast `"Note is empty"` warning, stay; else `(body.strip(), public)`; Esc → None. Printable keys go to the TextArea (q/o/y never reach the app). |
| `ViewPickerScreen(views, current, title, placeholder, show_view_kind)` | bold title, Input `#view-filter` (focused), OptionList `#view-options`, hint `"[dim]Enter open · ↑/↓ move · Esc cancel[/]"` | Filter = collapsed-whitespace casefold **substring** of name. Rows `"▸ "` for current else 2 spaces + name + `"  [dim](custom)[/]"` for non-system views when `show_view_kind`. Highlight current else 0. ↑/↓ move (intercepted at screen level), Enter/select → view; Esc → None. |
| `ChoiceScreen(choices, title)` | ViewPicker with `SimpleNamespace(value, name=label)`, placeholder `"type to filter choices"`, no kind marks | dismisses `value` (identity preserved) or None. |
| `ValueScreen(title, value="")` | plain title, Input (focused), hint `"Enter review · Esc cancel"` | Enter with non-blank → stripped text; blank Enter ignored; Esc → None. |
| `FailureScreen(message)` | title `"Write failed — inspect before retrying"`, scrollable plain message (never truncated), Close button | Esc/Close. Stored in `_write_failure`; `!` reopens. |
| `HelpScreen` | see B5 | any key closes |

Write flows (service functions named; every step re-checks read-only):
- **Status `s`** (2504): only tickets/changes (`actions.can_change_status`) else `"Status change is not available for {res}"`; choices `actions.status_choices(schema)` (schema status field choices `(id|value, value|name|str(id))`; none → `"No status choices in schema"`); StatusScreen → `actions.status_plan` (lines `"Status: {old}  ->  {new}"`, `safety::write_risks(res, {"status": id}, current, status_label)` or `"No side-effect warnings for this change."`, changes add `"Changing a change's status can send approval requests and fire workflows."`) → ConfirmScreen → worker `actions.apply_status` = `service::update_item(res, id, {"status": id})` → `_after_write("Status set to {label}")`.
- **Note `n`** (2530): `actions.can_note` (tickets, or changes/problems with `has_notes`) else `"Not available for this item type"`. Tickets → `ticket_notes::build_plan(display_id|id, body, public, notify_emails=[], body_format="plain")` (error → FailureScreen) → ConfirmScreen("Review exact ticket note", [plan.review() JSON indent 2, non-ASCII kept]) → public: second ConfirmScreen("Confirm PUBLIC visibility", ["This is a public note visible to the customer, not a reply.", "No explicit notification recipients. Tenant workflows may still notify people."]) → worker `ticket_notes::send(plan, client, reviewed_hash, public_confirmed, read_only)`; not verified → FailureScreen `"Note ID {id} submitted; readback unverified.\n{warning}\nDo not resend."`; verified → `_after_write(TICKETS, selected-if-same-ticket else {id, display_id: ticket_id}, "Note {id} verified")`. Changes/problems → `actions.note_plan(res, public, reply)` (title `Confirm private note|public note|reply`, `safety::note_risks(public)` / `reply_risks()` or `"Add a private note to this record."`) + `["", "> " + body]` → worker `service::add_note(res, id, body, public)` → `"Public note added"|"Private note added"`. (`apply_note` refuses tickets.)
- **Reply `r`** (detail pane, tickets): NoteScreen(reply) → note_plan(reply) → `service::add_reply` → `"Reply sent"`.
- **Actions `a`** (2214): ChoiceScreen("Selected record actions"): changes → `task-add "Add task"`, `task-update "Update selected task"`, `task-delete "Delete selected task"`, `link "Link initiating ticket"`, `unlink "Unlink initiating ticket"`, then for all → `download "Download selected attachment"`.
  - task-add: ValueScreen("New change task title — Open; reminder Never") → ChoiceScreen("Task owner", [none "No owner (default)", explicit "Explicit owner user ID or email"]) → explicit: ValueScreen("Explicit task owner user ID or exact email (no implicit self)") → options menu ChoiceScreen("Task optional fields — defaults: the form's first status / reminder; others omitted"): `review "Review exact task"`, per field `"{label}: {value | default / omitted}"` (group "Group ID or exact name", status "Status", note "Note (plain text)", notify_before "Notify Before", planned_start "Planned start (ISO with offset)", planned_end "Planned end (ISO with offset)", planned_effort "Planned effort (1h30m or 01:30)"), `clear "Clear optional fields"`; status → exclusive `task-create-choices` then ChoiceScreen(runtime status labels, "Task status"), notify_before → same worker then ChoiceScreen(runtime reminder labels, "Notify Before"), others → ValueScreen(label, current); after each value the menu re-opens. Review → worker: pop group, status (default "Open"); `change_tasks::status_label(status)`; dry `build_plan(cid, title, status_id=1, **values)`; `resolve_owner(owner)`; `build_plan(cid, title, owner_id, group_id=resolve_group(group), status, status_id=resolve_status(status), **values)` → ConfirmScreen("Review exact new change task", [plan.review() JSON]) → worker `change_tasks::send(plan, reviewed_hash=plan.hash, read_only)`; `!verified || unverified_fields` → FailureScreen `"Task ID {id} submitted; readback incomplete or unverified.\n{warning}\nDo not resend."`; else `_after_write(CHANGES, item, "Task {id} verified")`; exceptions → FailureScreen(`actions.failure_detail`).
  - link: ValueScreen("Search initiating tickets (ID or subject)") → targets `create::search_change_tickets(query)`. task-update/delete/unlink: `views::load_tab(client, CHANGES, item, "tasks"|"associations")` → `actions.target_inventory` (missing → error; associations need `parts["Tickets"]` else `"Initiating-ticket list unavailable; not an empty list"`; incomplete → warning `"Target inventory is partial or completeness is unknown"`). download: `actions.download_choices` (fresh `views::load_detail`, `cli_sr::_attachment_rows`, cloud files, change planning attachments via `int_get("changes/{id}/planning-fields")`, ticket conversations `views::load_conversations(all_pages=True)` + partial warnings). Errors → FailureScreen `"Could not load action targets: {exc}"`. No rows → FailureScreen `"No available targets.\n" + warnings`.
  - Target pick: ChoiceScreen(kind + `" — partial inventory; inspect warnings"` when warned; warnings toast 15 s), labels `"{human_display_id|display_id|id}: {title|subject|name|filename|attachment} {source}"`. task-update → worker `create::get_task_for_edit` + `task_status::fields_from` → ChoiceScreen("Update task {id} — choose one changed field", title/status/group_id "Group ID"/owner_id "Agent ID") → ValueScreen("Task {id}: {field}", current) → `actions.task_patch` (ValueError → FailureScreen; `{}` → toast `"No change; nothing sent"`) → confirm with `_before`. download → ValueScreen("Download destination directory", ".") → confirm `{destination}`. Others → confirm `{}`.
  - Confirm (2420): lines `"Record: {did} (ID {id})"`, `"Target: {hid|id} — {title|subject|name}"`, `"Action: {kind}"`, body JSON, safety line (tasks: `"Assignment/status changes may notify people. Deletion cannot be undone."`; link: `"Initiating-ticket relationship only."`; download: `"Source: {source}. Existing files are skipped; no overwrite."`), `"Before: {json}"` inserted at index 3, link/unlink append `change_associations::build_plan(...)`.review() JSON (ValueError → FailureScreen), inventory warnings → ConfirmScreen("Confirm {kind}").
  - Execute (2445): download → `actions.download_selected` (`cli_sr::_download`, exclusive create, skips existing, rejects unsafe names); status error/rejected → FailureScreen; else toast result JSON, title "Download result", 15 s. Others → `actions.apply_change_action` (`create::update_task`, `create::delete_task`, `change_associations::send(plan, client, reviewed_hash)` with the exact reviewed plan); link/unlink need `verified` else `"Association outcome unknown/unverified; do not resubmit"` → `"{kind} complete inventory verified"`. Tasks: readback `load_tab` + `target_inventory`; removal must be absent from a complete list, update must match body; failure → FailureScreen `"Write sent once; readback unverified: {exc}. Do not resend; inspect the record."`; ok → `"{kind} verified"`. **Never retry a send.**
- **Forms `N` / `e`** (1356): N → ChoiceScreen("Create", change/incident/sr). change/incident → `_launch_create` → formui missing → toast `"form UI not available"` (warning). sr → catalog worker → ChoiceScreen("Service catalog — category", [("all","All categories")] + categories) → `catalog::resolve_category` / `filter_items` → ChoiceScreen("Service catalog — search items", `"#{display_id} {name} · {category name}"`) → `sr_session(item_ref=display_id)`. `e`: must be a change (`"Select a change to edit"`); toast `"opening the form…"` 2 s. `_push_form` pushes `FormScreen(session)`; result save_draft → toast `"Draft saved: {path}"`; submit with plan → worker `session.send(plan)` → error FailureScreen(`failure_detail`) (partial outcomes in full: `created_id`, `steps`, `uploaded`, `error` + `"No automatic retry. Check the exact record and journal before another send; outcome may be unknown."`); ok → `actions.form_result(mode, out)` → edit `_after_write(item res, item, "Change updated")`; else toast `"{label} created {id}"` (`Change/CHN`, `Incident/INC`, `Service Request/SR`; human id, else `PREFIX-{display_id}`, else `ID {id}`, else `"(ID unavailable; inspect journal)"`) + `_reload_list()`.

### B10. Errors and session expiry

No re-login, no keepalive. `actions.clean_error` (103): SessionError → message or `"Session expired. Log in again."`; APIError → `"Server rejected the request (HTTP {status}): {description|message|errors|body, ≤200 chars}"`; else first 240 chars or type name. List errors → error toast; detail errors → red line in the pane; tab errors → cached `Missing` note rendered by `pages::tab_lines`; writes → persistent FailureScreen with `failure_detail` (test: SessionError during status write keeps the app running and stores the message).

### B11. Markup escaping and scroll reset

- Every server/user value inserted into markup goes through `escape` (Rich: backslash-escape `[` when followed by `[a-z#/@]…]`; trailing lone `\` doubled) — `_markup()` (app.py:86), header/view/filter strings, modal titles, option labels. Plain values are inserted as unstyled `Text` (detail bar, ConfirmScreen body, FailureScreen, ValueScreen title, review body). Rust: build `Span`s directly for user text; only constant templates and `fsv-views`/`pages` output go through the markup parser; the parser must treat `[` not followed by `[a-z#/@]` as literal (`[1]`, `[P4]` stay), and honour `\[` escapes.
- Scroll reset: `_clear_detail`, `_set_pending_detail`, details-tab render, other-tab placeholder and sub-content render all call `scroll_home(animate=False, immediate=True)` after refresh. Rust: set `detail_scroll = 0` whenever detail content is replaced.

### B12. Divergences / fixes to log in PARITY

Command palette dropped; search worker errors toast instead of crashing; keys from a pushed FormScreen do not reach app keys; DescriptionEditor preview keeps focus; status bar stays hidden behind key hints (parity) — or stack both rows (choose one, log it); clipboard copy in TextDialog via crossterm OSC 52; `y` keeps `pbcopy`.

---------------------------------------------------------------------------

## C. Test map → Rust (`TestBackend` + key scripts)

Fixtures to port first: `formui_tenants.py` (AcmeChanges, Globex over a mock transport: needs an injectable HTTP transport in `fsv_core::client` with request recording `gets(path)`, `writes()`, `paths()`), `acme_change_form`, `acme_catalog`, `acme_views` (World with `calls`), `formui_drive.py` → Rust `FormHarness` (`run_app(session, script, size=120×40)`, `settle()` = drain workers + fire due timers, `focus(key)`, `type_text`, `text_of(widget id)`, `fill_change(s)`).

| Python test | Pins | Rust test style |
|---|---|---|
| test_formui_units: fuzzy_* | rank order, accent/case fold | unit + golden |
| units: datetime_inputs, relative, date_errors | parse_when formats, preview text, error texts | unit + golden (fsv-forms) |
| units: check_files, path_completion | file rules/messages, `complete_path` | unit (tempdir) |
| units: answer_reveals, restricted_preselects, effects, problems, bad_typed_value | session notes / preselect / effects / problems / inline error | unit on FormSession |
| test_formui_qa #1 rejected_date | invalid text blocks review (plan not ok, hash cleared, submit disabled, error `Planned Start Date:`), text kept after back, fixing clears | harness |
| qa #2 enter_never_picks_previous | typing clears stale options; Enter with stale → no pick; late `_found` ignored | harness (call `found()` directly) |
| qa #3 calendar_keeps_instant; round-trip; form-zone text; DST | DatePicker `value()` preserves instant; `to_text` | unit + harness |
| qa #5 bad_draft_path, save_error | SavePath messages; dialog stays; then saves | harness |
| qa #6 escape_after_typing, untouched | dirty within debounce asks; first Esc closes combo; untouched cancels | harness with virtual clock |
| test_formui_change widgets (type_to_filter, multi, reveal, restricted, requester debounce=0.25 + one call, bootstrap once, typed date preview `Tue 26 Nov 2030 19:00 (+09:00 from your Freshservice profile tz)`, calendar right+enter+time, required block + back focus, review content/hash, narrow `<100`) | ComboBox, notes text, help panel, DateEditor, DatePicker, ReviewScreen | harness + mock tenant |
| change CLI tests (create sends once, cancel exit 1 "nothing was sent", dry-run json, copy-as-command round-trip, F6 dialog command text, update diff only `PUT {"priority": 3}` + `form_type=edit_form`, no-tty update error, `session.send`) | FormResult → caller exits, freshness re-fetch ≥2 | `fsv-cli` integration with `run_form` injected (trait/fn pointer) |
| test_formui_sr | requester first & never self; config default note; section reveal note; file type/count errors + list; required general attachment; SR banner + "Place request"; uploads only after submit (2 attachments then place-request); cancel "nothing was uploaded or sent"; draft round-trip + command | harness + CLI |
| test_formui_globex | no-rules notes "business rules could not be loaded … HTTP 404"; no `/ws/` paths; profile tz; choice descriptions; 403 dim notes on group/agent; requester `jörg`; typed-address fallback; SR without rules not ok, send raises | harness |
| test_formui_gallery | every screen renders (SVG >1000 B) incl. light theme, 80×30 | `insta` snapshots of `TestBackend` buffers (text) per screen + a styled-cell snapshot for themes |
| test_tui_actions | status cancel/confirm (one `update_item`, body contains `change status -> Review`), nonterminal no risk, private/public note confirm, reply only detail pane, empty note stays, read-only blocks keys, config read-only, SessionError → `_write_failure` & app alive, status_choices, typing in note doesn't trigger app keys | TUI harness with stubbed service trait |
| test_tui_change_actions | `apply_change_action` exact requests (`PUT changes/{id}/tasks/{tid}`, delete = `PUT …/tasks/update {"id": "…", "action": "delete"}`); per-kind review gate (no send before confirm, exactly one after `y`, association plan hash in lines); menu read-only; no retry on write/readback failure; `task_patch`; denied inventory; download skip/safe path; loader errors → FailureScreen; download inventory sources | unit + harness |
| test_tui_change_tasks | task-add chain, owner modes, review JSON contents, send kwargs `reviewed_hash`, `read_only=false`; failure no second send; read-only blocks everything (no client, no journal); all optional fields payload + hash; clear; no task-add for tickets | harness |
| test_tui_integration | N chooser placeholder, no "(custom)", dispatch change/incident/sr (category→item `item_ref=77`), cancel sends nothing; list loader has_more/page; read-only chooser; partial failure message untruncated + `!`; `form_result`; `_accept_list` epoch/entity/page guards, `of 1+` | harness + unit |
| test_tui_views | columns per tenant, header view text, detail bar/reporter/tabs/side, `g c` changes, server tabs order, tab `4` + `l`, view picker typing "team" → `filter=9001, order_by=updated_at, order_type=asc`, formui missing toast, edit hook pushes form with record, read-only N toast, Globex: no lifecycle, 403 tabs note, `v` no picker | harness + acme_views mock |
| test_tui_markup_escape | detail bar plain; `_format_*` escape `[...]` content | unit on formatters (assert plain text of produced `Text`) |
| test_tui_scroll_reset | scroll reset on clear/details/sub content | unit on state (`detail_scroll == 0`) |
| test_tui_ticket_notes | ticket-note review (`&lt;evidence&gt;`, `"notify_emails": []`), separate PUBLIC consent, send kwargs; unverified/unknown → FailureScreen, one send | harness |
| test_tui_read_only_config | config value table, flag wins | unit |

Harness rules: scripted keys use Textual names (`"ctrl+s"`, `"f2"`, `"escape"`, `"down"`, single chars) mapped to crossterm `KeyEvent`s; `click(id)` resolves a widget id to its last rendered `Rect`; workers run on an `InlineExecutor` (queued, drained by `settle()`); timers on a `ManualClock` (`advance(Duration)`); assertions read widget state (`app.screen::<FormScreen>().rows["subject"].error_text()`) or rendered buffer text.

---------------------------------------------------------------------------

## D. Ratatui architecture

### D1. Crates and modules

- `fsv-formui` (deps: fsv-core, fsv-forms, fsv-api, fsv-flows, ratatui, crossterm, ratatui-textarea, tui-markdown): `session.rs`, `lookups.rs`, `files.rs`; `kit/` (reusable runtime + widgets, `pub`, also used by fsv-tui): `runtime.rs` (event loop, `Timers`, `Executor`, `Clock`), `keys.rs`, `focus.rs`, `stack.rs` (screen/modal stack), `theme.rs`, `markup.rs` (Rich markup → `Line`), `widgets/{line_input, text_area, pick_list, combo, date_editor, calendar, toggle, button, markdown, scroll_view, virtual_table, footer, toast, dialog_frame}.rs`; `ui/{form_screen, review_screen, dialogs, date_picker}.rs`; `app.rs` (`run_form`, `form_available`). Split `kit` into its own crate later only if a third consumer appears.
- `fsv-tui` (deps: fsv-formui, fsv-views, fsv-api, fsv-flows): `app.rs` (model + update), `view.rs` (render), `filter.rs` (suggestions/history/tab completion, pure), `search.rs`, `loaders.rs` (worker jobs), `actions.rs` (port of actions.py, pure + service calls), `modals.rs`, `format.rs` (`_format_*`).
- Service seams: a `TuiServices` trait (list_page, search_items, load_detail, load_tab, update_item, add_note, add_reply, ticket_notes send, change_tasks send, apply_change_action, download…) with the real impl calling `fsv-api`/`fsv-views`; tests substitute recorders (Python tests monkeypatch these functions).

### D2. Model / update / view

```rust
enum Msg {
    Key(KeyEvent), Mouse(MouseEvent), Resize(u16, u16), Paste(String),
    Timer(TimerId),                       // debounce/close/notes-clear
    Worker { group: Group, gen: u64, out: WorkerOut },
    ScreenDone { tag: Cont, result: DialogResult },
}
enum Cmd { Spawn { group: Group, exclusive: bool, job: Job }, StartTimer(TimerId, Duration), CancelTimer(TimerId),
           Push(Box<dyn Screen>, Cont), Pop(DialogResult), Toast(Toast), Quit, Clipboard(String), OpenUrl(String) }
trait Screen { fn update(&mut self, msg: &Msg, cx: &mut Cx) -> Handled; fn view(&self, f: &mut Frame, area: Rect, cx: &ViewCx);
               fn on_result(&mut self, tag: Cont, r: DialogResult, cx: &mut Cx) {} fn is_modal(&self) -> bool; }
```
- `Cx` collects `Cmd`s; the runtime applies them after each `update`. Python callbacks passed to `push_screen(screen, cb)` become a `Cont` enum value stored with the pushed screen (`Cont::StatusPicked{item}`, `Cont::TaskOwner{item, title}`, `Cont::FormDone{session, mode, item}` …); on `Pop(result)` the runtime delivers `ScreenDone{tag, result}` to the screen below. `DialogResult` = `None | Bool | Text | Index | Value(Box<dyn Any>) | Form(FormResult)`.
- `FormScreen` holds `Arc<Mutex<FormSession>>` (workers call `suggestions` off-thread; the UI thread holds the lock only briefly). Alternative: workers get a cloned `SuggestCtx` (client, resolver, `AgentsGroups` Arc, item snapshot, pool snapshot) so the session stays `&mut` on the UI thread — preferred; choice pools are computed on the UI thread anyway.
- View is pure: `fn view(&self, f, area)`; every widget records its last `Rect` in a `HitMap` for mouse clicks and harness `click(id)`.

### D3. Event loop and key routing

```text
loop {
  draw if dirty  (run "after refresh" queue first)
  timeout = min(next timer deadline, 250ms)
  select: crossterm::event::poll(timeout) → read → Msg::Key/Mouse/Resize/Paste
          worker_rx.try_recv() (drain all)  → Msg::Worker   (drop if gen stale for exclusive groups)
          due timers                         → Msg::Timer
  dispatch(msg); apply Cmds
}
```
- Raw mode + alternate screen + mouse capture + bracketed paste on start, restored on exit/panic (panic hook). Keep `stdout` free of other writes while running.
- Workers: `std::thread` + `mpsc::Sender<(Group, u64, WorkerOut)>`; exclusive groups bump `gen[group]` on spawn; results with an old gen are dropped *in addition to* the Python guards (epoch/entity, search seq, pending detail/sub keys, combo `q == query()`, completion key). Workers catch panics (`catch_unwind`) and report as errors.
- Key dispatch (Textual order): (1) app priority (`ctrl+c` quit in TUI; `ctrl+q` quit / `ctrl+c` hint toast in FormApp) and screen priority (`ctrl+s` on FormScreen); (2) focused widget `handle_key` (inputs consume printable + editing keys, Enter → Submitted); (3) ancestors (ComboBox up/down/pgup/pgdn/esc when its input is focused; DateEditor F2; DescriptionEditor F3); (4) screen bindings (Tab/Shift+Tab focus, Esc, F1, F4, …); (5) base-screen app handler (`FsvApp.on_key` logic) only when no modal/form screen is on top.
- Key names: normalize crossterm keys to Textual names (`"slash"`, `"question_mark"`, `"left_square_bracket"`, `"right_curly_bracket"`, `"shift+n"`→`N`, `"exclamation_mark"`) so the B8 table and test scripts read the same.

### D4. Focus and layout

`FocusRing` of `WidgetId`s computed from the screen's visible+enabled focusables in compose order; Tab/Shift+Tab wrap. Focus-within styling (FieldRow boost, pane borders) via `focus.is_within(container)`. Scroll containers: `ScrollView { offset, content_height }` keeps the focused child visible (`scroll_visible` on ComboBox open and on focus change). Layout with `Layout::vertical/horizontal` + `Constraint::{Length, Fill(fr), Min, Max}`; `11fr/9fr` → `Fill(11)/Fill(9)` with `Min(8)`; narrow mode `width < 100`.

### D5. Widget set (kit)

| Widget | Implementation | Notes |
|---|---|---|
| `LineInput` | own (~400 lines) or `tui-input` 0.15.5 as the editing core | needs: Textual key bindings (§0), selection, horizontal scroll, placeholder, ghost suggestion + → accept, number restriction, disabled, tall border styles, cursor via `Frame::set_cursor_position`. `tui-input` (ratatui 0.30 / crossterm 0.29, updated 2026-09) covers value/cursor/ops; ghost + selection are ours. |
| `TextArea` | `ratatui-textarea` 0.9.4 (ratatui org fork of `tui-textarea`, ratatui-core 0.1, 2026-10) | `WrapMode::Word`, Tab → focus (intercept before the widget), map Textual bindings that differ (ctrl+z/y undo/redo match; f6/f7 select). `tui-textarea` 0.7 is stuck on ratatui 0.29 — do not use. |
| `PickList` | own on `ratatui::widgets::List` + `ListState` | highlight, page ±8, click, optional checkbox prefix, max height, hidden-when-empty. |
| `ComboBox` | own: `LineInput` + `PickList` popup (rendered inline under the input, like Textual, not floating) + chosen/note lines | state machine A5; `remote` searches via `Cmd::Spawn` + per-combo generation. |
| `DateEditor`, `CalendarGrid`, `DatePicker` | own | grid text built exactly as A5 (22×9). |
| `Toggle`, `Button` | own | Textual look: `▐X▌ Yes`; buttons 3 rows tall with variant colors, focused = reverse/bold. |
| `Markdown` preview | `tui-markdown` 0.3.13 (ratatui-core 0.1, 2026-11) | Only the F3 preview; no tests pin its output. |
| `VirtualTable` | own on `ratatui::widgets::Table` + `TableState`, rendering only the visible window | row keys, cursor, `RowHighlighted` emission on move/populate, 1-col scrollbar (`Scrollbar`), header bold, transparent bg. 100-row pages make full `Table` acceptable; window anyway for search/append growth. |
| `ScrollView` | own (`Paragraph::scroll` + `Scrollbar`) | `tui-scrollview` 0.6.8 is maintained but our content is `Text`; `Paragraph` with wrap + line count is enough. |
| `ModalStack` / `DialogFrame` | own: `Clear` + centered `Rect` (`Layout::centered`/`Flex::Center`), thick/heavy borders (`BorderType::Thick`), dim overlay by restyling the buffer beneath with `$background` 60% | `tui-popup` 0.7.7 exists but sizing rules (w64 ≤90%, auto height ≤90%) are simpler by hand. |
| `FooterHints`, `KeyHints` | own | from active bindings (`show=true`) / fixed TUI string. |
| `Toasts` | own overlay bottom-right, stacked, severity border (info primary, warning `$warning`, error `$error`), title bold, timeout timers | needed for test assertions (`app.notifications()`). |

### D6. Markup and styled output

`fsv_core::term` owns the Rich-markup parser (tags: colors, `bold/b`, `dim`, `italic`, `underline`, `reverse`, `not bold`, `[/]` closes last, `[/x]` closes x; escapes per B11). `kit::markup::to_text(&str, &Theme) -> ratatui::text::Text` maps named colors to ANSI `Color::{Cyan, Green, Yellow, Red, …}` (FsvApp `ansi_color=True`) and `$vars` to theme RGB. The review body: render `Review` through `fsv_core::term` into a styled-line buffer at a given width (preferred: a `StyledSink` impl of the term writer) and convert to `Text`; fallback `ansi-to-tui` 8.0.1 over ANSI output if `term` only emits ANSI strings.

### D7. Theme module

`Theme { primary, secondary, accent, warning, error, success, foreground, background, surface, panel, boost, text, text_muted, footer_key, footer_bg, border, border_blurred, input_selection, block_cursor }` with `textual_dark()`, `textual_light()`, `by_name()`; alpha colors pre-blended (`blend(fg, bg, a)`), `text`/`text_muted` = contrast color (white on dark, black on light) blended at 87%/60%. FsvApp uses `textual_dark()` with transparent backgrounds (`Color::Reset`) and ANSI named colors from markup; FormApp honors `forms.theme`.

### D8. Testing strategy

- Pure logic (FormSession, lookups, check_files, complete_path, fuzzy, when, filter suggestions/tab completion/history, actions.*, formatters, header/filter-bar strings) → unit tests + golden fixtures generated from Python (`port/golden/tui_*.py`).
- Screens → `Harness<App>` over `ratatui::backend::TestBackend` (sizes as in Python: 120×40, 80×30, 170×50), `InlineExecutor`, `ManualClock`, mock HTTP transport tenants. API: `press(&["ctrl+s"])`, `type_text("emer")`, `click("rv-submit")`, `settle()` (advance clock by ε, run due timers, drain workers until quiescent), `pause(Duration)`.
- Snapshots: `insta` 1.49 on `TestBackend` buffer text (and a `Buffer` style dump for the light/dark gallery); keep snapshots few — behavioral asserts carry the parity.
- Debounce tests use the virtual clock (requester search exactly one call after 0.25 s; Esc within 0.3 s debounce still asks).

### D9. Crate recommendations (checked on crates.io, Nov 2026)

| crate | version | status | use |
|---|---|---|---|
| ratatui | 0.30.2 (2026-06) | core | yes (workspace pin) |
| crossterm | 0.29.0 | ratatui 0.30 backend; features `bracketed-paste`, `events`, `osc52` | yes; enable `osc52` for clipboard |
| ratatui-textarea | 0.9.4 (2026-10-31) | ratatui-org fork of tui-textarea | yes (TextArea, note editor) |
| tui-input | 0.15.5 (2026-09) | ratatui 0.30 / crossterm 0.29 | optional core for `LineInput` |
| tui-markdown | 0.3.13 (2026-11) | ratatui-core 0.1 | yes (F3 preview) |
| ansi-to-tui | 8.0.1 (2026-01) | ratatui-core 0.1 | fallback for review rendering |
| insta | 1.49.0 | dev | snapshots |
| webbrowser | 1.2.7 | maintained | `o` open in browser (`open` 5.4.5 also fine) |
| unicode-width 0.2, unicode-normalization | — | — | cell widths, fold |
| tui-textarea 0.7 | 2024-10, ratatui 0.29 | stale | no |
| tui-popup 0.7.7, tui-scrollview 0.6.8, tui-widget-list 0.15.3 | maintained | not needed | skip (own simpler code) |
| ratatui-macros 0.7.2 | maintained | optional sugar | optional |
