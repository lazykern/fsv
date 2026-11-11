# Interactive forms (`fsv.formui`)

`fsv changes create`, `fsv changes update ID` and `fsv sr create ITEM` open a
full-screen form in the terminal when they are run without inputs. It is
laid out like the Freshservice web form and runs the same form engine
([forms.md](forms.md)) as the YAML and `--set` faces, so the review, the
plan hash, the journal and the send path are the same ones.

```
┌ New Change ───────────────────────────────────────────────────────────────┐
│ + Staging Start (required), Staging End (required) appeared because       │
│   Cloud Only = No                                                         │
│ Requester *                                  │ Cloud Only * required      │
│ [ jane▌                                    ] │ Only the cloud team ...    │
│ ┌ Jane Doe <jane.doe@example.com>        ┐   │ Choices                    │
│ │ Jane Doerr <jane.doerr@example.com>    │   │  • Yes  • No               │
│ └────────────────────────────────────────┘   │ Rule effects               │
│ Cloud Only *                                 │  Picking Yes will hide     │
│ [ No                                       ] │  Staging Start, ...        │
│ 7 required field(s) missing: ...  ·  Ctrl+S review · F4 save draft · Esc  │
└───────────────────────────────────────────────────────────────────────────┘
```

## When it opens

| command | opens the form when |
|---|---|
| `fsv changes create` | stdin and stdout are terminals and there is no `--from`, `--set`, `--preset`, `--attach` or field flag (`--requester` only preselects) |
| `fsv changes update ID` | no flag at all (pre-filled from the change, the **edit** form's rules, only changed fields sent) |
| `fsv sr create ITEM` | no `--from`, `--set`, `--preset`, `--attach` |

* `--dry-run` with the form: the review is shown in the form, **Finish**
  prints the plan (review, payload, plan hash) to the terminal, nothing is
  sent.
* `--edit` (YAML in `$VISUAL`/`$EDITOR`) is still there as an advanced option;
  so are `--from`, `--set`, `--preset`, `--yes` and `--expect-plan` for scripts
  and agents.
* Without a terminal and without inputs the command exits 2 and suggests
  `--from` / `--set`. `FSV_NO_FORM=1` (or `TERM=dumb`) turns the form off;
  `create --dry-run > draft.yaml` (stdout not a terminal) still prints the
  annotated draft.
* `fsv config set forms.theme textual-light` picks a Textual theme;
  `forms.max_file_mb N` rejects larger files before upload.

## Using it

Keyboard first, mouse works everywhere.

| key | |
|---|---|
| Tab / Shift-Tab | next / previous field |
| type | filter a dropdown or checklist (case- and accent-insensitive, label and choice description, `stg` finds Staging); search people |
| ↑ ↓ PgUp PgDn | move in the open list |
| Enter | pick (dropdown, person), toggle (checklist), commit (text, date, files) |
| Esc | close the open list; on the form: cancel (asks first when something was entered, including text typed a moment ago that is not committed yet) |
| F2 | calendar for a date / date-time field |
| F3 | Markdown preview of the description |
| F1 | hide / show the help panel |
| Ctrl+S | review |
| F4 | save the draft (YAML) and leave |

* **Fields** are in web order with the label, a red `*` when required, the
  help text below; the help panel shows the focused field's help, its
  choices with their meanings, and what each choice does under the business
  rules ("Picking No will show Staging Start (required); limit Stage to
  Staging").
* **Live rules.** Every committed value updates the form: fields appear,
  disappear and become required at once, with a note naming the cause
  (`+ UAT Start (required) appeared because Infra Only = No`). A dropdown a
  rule restricts offers only the allowed choices; when a rule leaves exactly
  one, it is picked for you and the note says which rule did it. Blocking
  rule messages are shown and acknowledged on the review.
* **People** (requester and person lookups) are searched as you type, 250 ms
  after the last key, with the web's own call
  (`GET /search/autocomplete/requesters?q=…&qf=details`), shown as
  `Name <email>`. The requester is never filled with the logged-in user;
  `defaults.requester` (changes) / `defaults.sr_requester` (service
  requests) preselect one when set. **Groups and agents** are filtered locally
  from `GET /api/_/bootstrap/agents_groups`, fetched once. While a new
  search is pending the old results are cleared, and Enter or a click never
  picks a result of an earlier search.
* **Dates** are typed (`2026-11-26 19:00`, `26 Nov 2026 7pm`, `tomorrow 9am`,
  `+2d 10:00`, `fri 22:00`, an explicit `+07:00`) with a live preview:
  `Thu 26 Nov 2026 19:00 (+07:00 from your Freshservice profile tz)`. F2
  opens a month calendar with time suggestions (09:00 … 22:00). The time
  zone is the one the engine uses (`FSV_TZ`, the Freshservice profile from the
  rules response or `GET /api/v2/agents/me`, else the computer's). A local
  time that a daylight-saving change skips is refused, and one that happens
  twice asks for the offset (`2031-10-26 02:30 +01:00`). A value with another
  offset keeps its instant: the calendar shows it in the form zone, the text
  box keeps the offset (and seconds). Text that cannot be read stays in the
  box with its error and blocks the review until fixed or cleared; the
  earlier value is never sent silently.
* **Description**: Markdown in a text box, converted to HTML when the plan is
  built; F3 previews it. On an edit form the box starts with the current
  text; leaving it unchanged sends nothing.
* **Files** (attachments, planning files, service-item file fields): paths
  separated by `;`, `→` completes the path. Existence, allowed types, the
  file count and (optionally) the size are checked inline; files upload only
  after you submit the review.
* **Validation** is inline per field; *Review & submit* stays disabled until
  every visible required field is answered and no input is invalid. Ctrl+S
  always opens the review (it lists what is missing).

## Review

The review screen shows the same review as the CLI (`fsv.review.Review`):
values in web order (old → new on an edit), who is notified, fields dropped
because a rule hid them, rule notes, attachments with size and sha256, the
form metadata's age and the **plan hash**. F7 shows the exact request body.

| button | |
|---|---|
| **Create / Update / Place request** | leave the form with the plan; the command sends it (journal duplicate check, metadata freshness re-check, the write, uploads after the write) |
| **Back to edit** (Esc) | return to the form, focus on the first missing field |
| **Save draft** (F4) | write the YAML draft and leave; continue with `--from FILE` (a path that cannot be written shows the error in the dialog; try another) |
| **Copy as command** (F6) | save the draft and show `fsv changes create --from FILE --yes --expect-plan HASH` (or the `update` / `sr create` equivalent), with a Copy button |
| **Cancel** | leave; nothing is sent |

If the form metadata changed between the review and the send, the command
shows what changed and asks again (as with `--from`).

## Embedding: `FormScreen` in any Textual app

```python
from fsv.formui import FormScreen, FormResult, change_session, sr_session

session = change_session(client)                                   # new change
session = change_session(client, mode="edit", target=4242, record=change)  # edit
session = sr_session(client, 77)                                   # catalog item

def done(result: FormResult) -> None:
    if result.action == "submit":
        out = session.send(result.plan)   # duplicate check, freshness, writes
    elif result.action == "save_draft":
        print(result.saved_path)

app.push_screen(FormScreen(session), done)
```

* `FormScreen(session)` is a `Screen[FormResult]`; it never sends anything.
  It pushes its own review and dialogs and dismisses with
  `FormResult(action="submit" | "save_draft" | "cancel", state, plan,
  saved_path, session)`.
* `FormSession` (no Textual dependency) wraps the live `FormState`, the form
  metadata, the lookups, the mode (`new` | `edit`) and the record being
  edited. Its API, used by the screen and usable on its own:
  `items`, `view(key)`, `set(key, value) -> Outcome(notes, error)`,
  `suggestions(key, query)`, `help_for(key)`, `effects(key)`, `problems()`,
  `build_plan(ack=None, meta=None)`, `save_draft(path)`,
  `command_for(path, plan)`, `send(plan)`.
* `send()` is the non-interactive send path: it refuses (`FlowExit` 4) when
  the metadata changed since the review instead of asking. With
  `dry_run=True` it refuses to send.
* `change_session(client, mode="edit", target=ID)` fetches the record; a
  `record` without `custom_fields` (a list row) is re-fetched.
* `run_form(kind_or_session, **ctx) -> FormResult` runs the screen as a
  standalone app (the CLI face); `form_available()` tells whether a terminal
  is there.

Keys: spec fields use their API name; form-level inputs are
`__requester__` (service requests), `__attachments__` and
`planning:<label>`.

## Portability

The form works on any account:

* **No workspaces**: the business rules are fetched from
  `/api/_/business_rules` when the form metadata names no workspace (and as
  a fallback after a 404/403 on `/api/_/ws/<id>/business_rules`).
* **No business rules** (404/403/empty): the form runs on the metadata's own
  required/visible flags; the form and the review say so.
* **No catalog sections / dependent fields**: nothing to reveal; fields are
  static.
* **Restricted roles**: a 403 on the agents/groups list, the agent search or
  the requester search shows a dim note on the field instead of failing;
  without requester search an exact typed email can still be used.
* **Labels in any language**: filtering folds case and accents; dates accept
  ISO input everywhere and the preview never depends on the machine locale.
* **Time zone**: the Freshservice profile's (rules response, else the agent
  profile), else the computer's — always named in the preview.

The tests run every screen against two invented tenants: Acme (workspaces,
rules, sections) and Globex (none of them, German labels, 403s).

## Not yet verified against the live web

* Agent ids from the `agents_groups` bootstrap list (`user_id`, else `id`)
  as the `agent_id` the web sends.
* The web's per-file size limit (fsv only checks `forms.max_file_mb` when
  set).
* Choice and requester notes use the engine's position-order replay, like
  `--set`; a person answering out of order on the web may see a different
  result for order-sensitive rule sets (`fsv form explain` shows where).
