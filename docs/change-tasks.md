# Reviewed change-task creation

`fsv changes tasks-add` creates a task on one change, defaulting to **Open**.
The name follows `tasks-update` and `tasks-delete`. Main TUI: select a change,
press **a**, choose **Add task**, enter the title, choose **No owner (default)**
or an explicit user ID/email, then use the **Task optional fields** picker.
Choose fields to edit, **Clear optional fields** to reset, or **Review exact task**
to review and confirm. The same optional fields as the CLI are available using
existing value/choice dialogs (the TUI note input is single-line). Escape cancels.
Read-only TUI mode blocks the menu, composition, review and sender.

## CLI

Review locally with no owner or use an explicit user ID:

```bash
fsv changes tasks-add CHN-1234 --title "Acme verification" --dry-run --json
fsv changes tasks-add CHN-1234 --title "Acme verification" --owner 9000002001 --dry-run --json
```

Default: **no owner**, never implicit self. Pass a user ID, exact agent email,
or exact full name using `--owner`. Lookup is read-only and requires exactly one
active agent with a `user_id`. Email and full-name matches are case-insensitive;
there is no fuzzy name, agent record ID or requester fallback. A numeric user ID
avoids owner lookup. Group-name and metadata-backed status resolution may also
read the tenant.

After inspecting the review, repeat the exact arguments with the **full** hash
from `plan_hash`:

```bash
fsv changes tasks-add CHN-1234 --title "Acme verification" --owner 9000002001 \
  --yes --expect-plan "$PLAN_HASH" --no-input --json
```

`PLAN_HASH` must contain the full hash from your own dry run, not a prefix. It
binds the tenant, change display ID, title, owner, selected status label and exact
payload, including every optional field. Without
`--yes`, an interactive confirmation is required. `--no-input` never prompts;
`--yes` without the matching `--expect-plan` fails. `--read-only` refuses a send.

## Request contract and limits

A blocked UI request established this contract, illustrated with invented data:

```http
POST /api/_/changes/1234/tasks
Content-Type: application/json

{"title":"Acme verification","owner_id":9000002001,"status":1,"notify_before":0}
```

There is **no `change_task` wrapper**. Defaults remain Open (`1`) / Never (`0`).
Blank group, note, dates and effort are omitted. No-owner mode omits `owner_id`
rather than inventing a null/default owner; owner omission remains an unverified
variant. Optional-field blocked POST captures establish browser serialization,
not server acceptance, limits, notifications or normalization. The accepted live
create response was **not observed**. Attachments are not supported.

### Optional create fields

```bash
fsv changes tasks-add CHN-1234 --title "Acme verification" \
  --group 9000004001 --status "In Progress" --note "Check <Acme> & report" \
  --notify-before "30 Minutes" \
  --planned-start "2026-12-05T09:00:00-05:00" \
  --planned-end "2026-12-05T10:30:00-05:00" \
  --planned-effort "01:30" --dry-run --json
```

| Flag | Wire field / conversion |
|---|---|
| `--group` | `group_id`: positive numeric ID or one exact group name from runtime metadata; no fuzzy match |
| `--status` | `status`: Open (`1`), In Progress (`2`); Completed / Blocked require an exact runtime metadata ID |
| `--note` | `description`: plain text HTML-escaped inside `<div><p>…</p></div>`; newlines become `<br>` |
| `--notify-before` | `notify_before`: Never (`0`), 15 Minutes (`900`), 30 Minutes (`1800`), 45 Minutes (`2700`), 1 Hour (`3600`), 2 Hours (`7200`), in seconds |
| `--planned-start` / `--planned-end` | `planned_start_date` / `planned_end_date`: ISO date/time with explicit offset, serialized with seconds and that offset |
| `--planned-effort` | `planned_effort`: `1h30m`, `1h 30m` or `01:30` becomes `1h 30m`; hours-only / minutes-only supported locally |

Dates require an explicit timezone: date-only and naive local times are refused,
not interpreted in the host or tenant zone. `Z` becomes `+00:00`. Fractional
seconds are refused rather than silently discarded. End before start is refused.
Effort normalization is a CLI extension: the UI rejected a literal `01:30` and
sent `1h 30m`. Rich-text editor styling is not reproduced; the escaped HTML note
wrapper is a local plain-text adaptation, not byte-for-byte editor markup.

Only In Progress's new numeric status value was captured. Completed and Blocked
are observed labels, **not inferred IDs**. They require the read-only
`change_task_fields` status choices to supply one exact numeric match; missing,
ambiguous or conflicting metadata fails closed before any POST. No sequential
status numbering is assumed. This metadata-resolution path is tested offline;
its availability on a particular tenant is not claimed. Group names and those
two statuses may perform GETs even in a dry run; numeric groups with Open /
In Progress and numeric/no owner remain local-only.

### Reviewed update and deletion

Method-specific update evidence is now covered by the shared reviewed
`tasks-update` builder (see `docs/reviewed-parity.md`, Change task update).
The main TUI uses that builder for its one-field task editor, not the legacy
service helper.

`tasks-delete CHN-1234 TASK_ID --dry-run --json` reviews the native
`PUT tasks/update` action. Send only with `--yes --expect-plan FULL_HASH`.
The TUI delete action shows the same plan and requires explicit confirmation.
Fresh complete inventory must match review; independent readback requires the
removed ID to be absent and every other task ID preserved. Ambiguous sends
are journal-guarded and never replayed.

**Never is a reminder setting, not a guarantee of no notifications.** Creating
or assigning a task may notify people and trigger tenant workflows.

## Verification and unknown outcomes

Before the POST, the sender must read a complete paginated task inventory and
prepare the session. It persists a body-free `started` journal entry before
sending, and refuses if that entry cannot be read back. It sends the POST **at
most once**, including on HTTP 403, rate limits, timeouts and malformed responses.
It does not use the internal helper that retries writes on 403.

- A positive numeric ID in a `task` object or a bare response is retained and
  read back by that exact ID. Title/owner must match and every exposed payload
  field must agree to report `verified`. Numeric owner/group IDs and equivalent
  offset dates are compared semantically; other values (including HTML and effort)
  must match exactly. Read-back failure returns the known ID with
  `verified: false` and a warning;
  it never substitutes a different title match or repeats the POST.
- Without a returned ID, the sender reads the complete task inventory again.
  Exactly one **new ID absent before the POST**, with the **exact title and
  owner**, with no contradictory exposed payload fields, identifies the task.
  An omitted owner field in a read-back row is not
  proof of an unassigned task; no-owner verification requires `owner_id: null`.
- Zero/multiple matches, incomplete/unreadable read-back or transport errors
  leave the outcome **unknown**. No latest-task guess, automatic retry or
  resubmit is performed. The `started` journal entry remains unresolved.
- Definite HTTP client rejections are recorded as `failed`; timeouts, rate
  limits, server failures and session uncertainty remain `started`.
- A known created ID is journaled as `done`. Identical `started`/`done` plans
  remain blocked with no expiry or duplicate override in this task flow.

Inspect the exact change and local journal before considering any manual
recovery. Do not change a title or remove journal entries merely to bypass the
unknown-outcome guard. CLI errors exit with code 2; JSON success includes
`task_id`, `verified`, `identity_verified`, `fields_verified`, `verified_fields`,
`unverified_fields`, `mismatched_fields`, `readback` and `plan_hash`. A field absent
from GET is **unverified**, never silently treated as verified. `verified` retains
the identity/no-observed-contradiction meaning; `fields_verified` additionally
requires every submitted field to be returned and matched. Missing fields produce
an explicit warning, even if task identity is verified. No accepted optional-field
GET response was captured, so no fixed field-availability claim is made.
Check these fields, not just exit status. The main TUI refreshes only after full
verification; unknown, incomplete or unverified outcomes stay visible in its
persistent failure screen. No response normalization is guessed and no resend
is offered.
