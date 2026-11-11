# Reviewed change-task creation

`fsv changes tasks-add` creates an **Open** task on one change. The name follows
`tasks-update` and `tasks-delete`. Main TUI: select a change, press **a**, choose
**Add task**, enter the title, choose **No owner (default)** or an explicit user
ID/email, then review and confirm. Escape cancels. Read-only TUI mode blocks the
menu, review and sender.

## CLI

Review locally with no owner or use an explicit user ID:

```bash
fsv changes tasks-add CHN-1234 --title "Acme verification" --dry-run --json
fsv changes tasks-add CHN-1234 --title "Acme verification" --owner 9000002001 --dry-run --json
```

Default: **no owner**, never implicit self. To assign yourself, pass your actual
user ID or exact agent email using `--owner`. Email resolution is read-only,
requires exactly one active agent with that exact email and a `user_id`, and
never falls back to a fuzzy name, agent record ID or requester. A numeric user
ID avoids lookup. Email dry runs may read the tenant; numeric/no-owner dry runs
do not construct a client or write a journal.

After inspecting the review, repeat the exact arguments with the **full** hash
from `plan_hash`:

```bash
fsv changes tasks-add CHN-1234 --title "Acme verification" --owner 9000002001 \
  --yes --expect-plan "$PLAN_HASH" --no-input --json
```

`PLAN_HASH` must contain the full hash from your own dry run, not a prefix. It
binds the tenant, change display ID, title, owner and exact payload. Without
`--yes`, an interactive confirmation is required. `--no-input` never prompts;
`--yes` without the matching `--expect-plan` fails. `--read-only` refuses a send.

## Request contract and limits

A blocked UI request established this contract, illustrated with invented data:

```http
POST /api/_/changes/1234/tasks
Content-Type: application/json

{"title":"Acme verification","owner_id":9000002001,"status":1,"notify_before":0}
```

There is **no `change_task` wrapper**. Status stays Open (`1`); reminder stays
Never (`0`). Blank group, note, dates and planned effort are omitted. No-owner
mode omits `owner_id` rather than inventing a null/default owner; the captured
request had an explicit owner, so owner omission remains an unverified variant.
These optional fields, attachments and other statuses are not configurable in
this create flow. The accepted live response was **not observed**. No live
create is claimed or needed by the offline tests.

**Never is a reminder setting, not a guarantee of no notifications.** Creating
or assigning a task may notify people and trigger tenant workflows.

## Verification and unknown outcomes

Before the POST, the sender must read a complete paginated task inventory and
prepare the session. It persists a body-free `started` journal entry before
sending, and refuses if that entry cannot be read back. It sends the POST **at
most once**, including on HTTP 403, rate limits, timeouts and malformed responses.
It does not use the internal helper that retries writes on 403.

- A positive numeric ID in a `task` object or a bare response is retained and
  read back by that exact ID. Title and owner must match to report `verified`.
  Read-back failure returns the known ID with `verified: false` and a warning;
  it never substitutes a different title match or repeats the POST.
- Without a returned ID, the sender reads the complete task inventory again.
  Exactly one **new ID absent before the POST**, with the **exact title and
  owner**, identifies the task. An omitted owner field in a read-back row is not
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
`task_id`, `verified`, `readback` and `plan_hash`. Check `verified`, not just the
exit status. The main TUI refreshes the selected change only after verification;
unknown/unverified outcomes stay visible in its persistent failure screen.
