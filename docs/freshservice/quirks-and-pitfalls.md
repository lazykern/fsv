# Freshservice quirks and pitfalls

**What this is.** The platform gotchas that broke or nearly broke a client
of the Freshservice agent web API, each with the symptom, the cause as far
as it is known, and what to do about it.

**Who it is for.** Anyone writing or reviewing code that reads from or
writes to Freshservice through the internal API, and anyone running live
tests against a tenant.

**Verified on:** a Freshservice Enterprise trial tenant, 2026-11-11 to
2026-11-13, plus read-only GETs and blocked (aborted) captures on a
production-like enterprise tenant. Examples use invented values.

**Confidence tags:** **[VERIFIED LIVE]** seen on a real tenant and read
back; **[CAPTURED]** request shape seen, not executed live;
**[INFERRED]** reasoned from code, UI or several observations.

See also: [wire contracts](wire-contracts.md) for the request shapes and
[live testing](../live-testing.md) for how to probe safely.

## Ids, strings and numbers

* The same id can be a string in one place and a number in another. Task
  metadata serialises choice ids as strings (`"1"`, `"900"`); approval
  responses quote chain, group and approval ids; most record reads use
  numbers. Parse ids as int **or** ASCII-digit string, reject booleans,
  signs and decimals. [VERIFIED LIVE]
* Writes are stricter than reads. `device_ids`, `businessapp_ids` and a
  knowledge-base `folder_id` must be JSON **integers**; strings were
  rejected even though reads tolerate strings. [VERIFIED LIVE]
* Some writes require strings: the task "delete" action sends
  `{"id": "<task id>", "action": "delete"}`, and adding a requested item
  sends the catalog item display id as a string. Copy each contract's
  types exactly. [VERIFIED LIVE]
* `true` is not `1`: keep booleans and numeric ids distinct when comparing
  custom-field readbacks. [VERIFIED LIVE]
* Display number vs internal id: paths take the display number; a few
  bodies need the internal id (maintenance-window matching uses the
  change's internal id). [VERIFIED LIVE]
* Integrated-inventory item ids, association link ids and legacy asset
  display ids are three different namespaces. [VERIFIED LIVE]

## Bodyless requests and empty responses

* Many writes have no body: cancel an approval group, remind an approver,
  unspam, restore, delete. They still need `Content-Type: application/json`
  (a remind without it answered **415**) and the CSRF header.
  [VERIFIED LIVE]
* Spam sends `{}` rather than nothing. [VERIFIED LIVE]
* Successful unlink and update calls can answer 200 or 204 with an empty
  body labelled `application/json`. Decoding that as JSON throws, and a
  client that treats the exception as failure reports an unknown outcome
  for a write that worked. Check for an empty body before decoding, then
  verify with an independent read. [VERIFIED LIVE]
* A GET for a deleted target can answer an **HTML 404**. Check the status
  before parsing JSON during cleanup. [VERIFIED LIVE]

## Signed URLs

* `attachment_url` values are signed CDN URLs (`Expires=...&Signature=...`)
  valid for about a day; inline-image links carry a token. Re-read the
  owner before downloading. [VERIFIED LIVE]
* Signatures **rotate on every GET** (attachments, avatars, catalog item
  icons, images embedded in rich text). Hashing a raw readback therefore
  never repeats, so a "has anything changed since review" check always
  fires. Strip only the signing parameters (or project attachments onto
  id/name/size) before hashing; keep everything else. [VERIFIED LIVE]
* The canonical `/helpdesk/attachments/<id>` URL needs the session and
  redirects; a 404 there does not prove the stored file is gone (inline
  images in particular). [VERIFIED LIVE]

## Readbacks that never echo what you sent

| Sent | Not returned by | Where to look instead |
|---|---|---|
| task `notify_before` (reminder) | the task **collection** read | the exact task read (`.../tasks/<id>`) [VERIFIED LIVE] |
| `email` on problem create | the problem record | `requester_id` [VERIFIED LIVE] |
| `device_ids` on change create | the create response | `associated-cis` read [VERIFIED LIVE] |
| `notify_emails` on a forward | the conversation | `to_emails` [VERIFIED LIVE] |
| `conversation_id` (source) of a per-conversation forward | anything readable | report as unverifiable [VERIFIED LIVE] |
| inline image `fileid` attribute | the stored body (keeps `data-id`) | `data-id` [VERIFIED LIVE] |
| merge notes per secondary | the merge response | conversations of each ticket [VERIFIED LIVE] |

Report such fields as *unavailable*, separately from *mismatched*; never
resend a write to obtain a missing field. [VERIFIED LIVE]

## Values the server derives or rewrites

* **Priority from urgency and impact** on incidents: a PUT of `priority`
  alone returned 200 and changed nothing. [VERIFIED LIVE, mechanism
  INFERRED]
* **Due dates from SLA policies**: priority changes move `due_by`.
  [VERIFIED LIVE]
* **Planned dates on resolve/close** (tickets get a short planned window;
  closing a problem rewrites its planned dates) and on **task completion**
  (planned dates move to the completion time; `closed_at` set).
  [VERIFIED LIVE]
* **Workflow automators** rewrite status and other fields after your write,
  asynchronously; see [lifecycles and workflows](lifecycles-and-workflows.md#6-workflow-automators).
  [VERIFIED LIVE]
* **Rich text is normalised**: `<p>` may become `<div>`, links gain
  `rel="noreferrer"`, line breaks are inserted, the editor wraps bodies in
  styled `<div>`s. Compare rich text after normalisation, but keep the
  submitted bytes for hashing. [VERIFIED LIVE]
* **Durations and times**: `time_spent` `00:01` vs `00:01:00`; effort
  `1h` vs `1h 0m`; timestamps in the **account's** time zone offset on the
  internal API and UTC on `/api/v2`. Compare instants and durations, not
  strings. [VERIFIED LIVE]
* The account time zone, not the client's, decides offsets: a tenant
  configured for a US zone returns `-05:00` even when the agent is
  elsewhere. [VERIFIED LIVE]

## Deleted, trashed and immutable records

* Soft-deleted tickets and problems stay readable with `deleted: true`;
  updates to them answer **HTTP 405**. [VERIFIED LIVE]
* `GET /api/_/attachments/<id>` answers 405 (the route has no GET).
  [VERIFIED LIVE]
* A ticket-template delete answers 404 although it deleted the template;
  verify by absence from the list. [VERIFIED LIVE]
* Irreversible from the UI or API: ticket merge, ticket delete-forever (no
  confirmation dialog), problem delete-forever, article delete-forever,
  sent emails and notifications. [VERIFIED LIVE]
* **Approval history is immutable.** Approved or rejected approvals cannot
  be cancelled (400); cancelling a pending group keeps it as history and
  leaves the record's aggregate approval status at "cancelled" rather than
  restoring "not requested". Plan test approvals as permanent marks.
  [VERIFIED LIVE]

## Lists, filters and paging

* An invalid filter answers **HTTP 206 with `meta.search_error: true` and
  unfiltered rows**. Treat any 206 or `search_error` as a failed filter,
  never as a result. [VERIFIED LIVE]
* A date range with only `from` or only `to` answers **HTTP 500**.
  [VERIFIED LIVE]
* Some invalid filters are silently ignored instead (a numeric ticket-type
  id, `on` with a literal date): the list looks normal but is unfiltered.
  Check returned rows against the predicate before using a filtered list
  to select records for a write. [VERIFIED LIVE]
* A Due By filter never returns Pending tickets: on this tenant type the
  Due By field is hidden and empty while a ticket is Pending, so a "due
  before/after" range silently skips them even though
  they are open work. List them separately with a status filter;
  `fsv tickets ls --where 'Due By...'` without a status filter prints a
  one-line stderr note saying so. [VERIFIED LIVE]
* System views include "created in the last six months"; on a quiet tenant
  they are empty while records exist. [VERIFIED LIVE, read-only]
* Activity feeds page with `start_token` on the **internal** path. The
  `next_page_url` in the response points at `/api/v2`, which answers 404
  for a browser session. [VERIFIED LIVE]
* Empty collections are not always empty arrays: `child-tickets` answers
  **400** when there are none; the ticket-to-change read answers 400
  without `change_type`. [VERIFIED LIVE]
* `per_page` is capped (100 for lists); always follow the paging metadata
  (`has_next`, totals, `next_page_url`, `total_pages`) and report
  incomplete inventories as incomplete. [VERIFIED LIVE / INFERRED]
* List and association rows use different keys from detail records:
  association rows carry `ticket_type` while the ticket detail has `type`,
  and ticket list rows return `approval_status` as null. Read the native
  key of the payload you have. [VERIFIED LIVE]

## Rate limits and retries

* Bursts of several hundred GETs triggered **HTTP 429**. Space sequential
  reads, honour `Retry-After` when present (cap the wait), and stop a loop
  on 429 rather than hammering. [VERIFIED LIVE, read-only]
* Retry only idempotent reads. A write that timed out, returned 5xx or 403
  after sending has an **unknown outcome**: read the exact target before
  doing anything else, and never resend blindly. [VERIFIED LIVE]
* Responses may carry `x-ratelimit-remaining` / `x-ratelimit-total`
  headers; log them when present. [INFERRED]

## UI traps (browser captures and manual tests)

* **Hidden dialogs in the DOM.** Agent pages keep hidden vendor dialogs
  (send feedback, request a demo, chat with us) with their own
  Submit/Cancel buttons. Clicking "the first button whose text is Submit"
  can hit the vendor feedback form, which files a ticket on the vendor's
  support desk; that cannot be rolled back from the tenant. Scope every
  click to the visible dialog or form you mean, and check the network log
  for the expected path. [VERIFIED LIVE: dialogs present; side effect
  INFERRED]
* **Page loads write.** Opening a ticket autosaves a reply draft
  (`POST /api/_/tickets/<id>/reply_draft`, 204). The UI also posts
  telemetry (`metrics`, `languages/detect`); exclude it when counting
  writes. [VERIFIED LIVE]
* **Some GETs do work**: "Re-analyze impacted services" is a GET that
  schedules a job (202). [VERIFIED LIVE]
* **Rich-text editor fill appends or prepends.** Filling a pre-populated
  editor (approval email template, existing note, reply with quote) adds
  to the existing text. Select all and insert, then check the exact text
  before saving. [VERIFIED LIVE]
* **Optimistic UI.** After an aborted request the UI may roll back, so a
  second click can send the same forward action again instead of the
  inverse. Inspect the actual bodies. [VERIFIED LIVE]
* **Slow forms.** Some admin forms (maintenance windows) take several
  seconds to render; a page showing only help text is not proof the
  feature is missing. [VERIFIED LIVE]
* **Selection by multi-select remove**: the CI "Remove" button removes one
  row even when several are checked. [VERIFIED LIVE]

## Caches and timing

* Saves are asynchronous: a GET right after a successful write can still
  show the old value, and bulk actions can lag. Re-read before reporting a
  mismatch; never resend because of one. [VERIFIED LIVE]
* A browser GET of `/api/v2` data right after an update can come from the
  HTTP cache; use `cache: "no-store"`. [VERIFIED LIVE]
* A client's own metadata cache goes stale when admins change fields,
  rules or flows. Fingerprint metadata into reviews and refetch before
  sending. [VERIFIED LIVE]

## The default config dir picks the tenant

fsv's default config dir holds the domain that `fsv auth login` configured
last, so a command run without the intended `FSV_CONFIG_DIR` silently talks
to the other tenant; a live check once hit the wrong tenant this way. fsv
now prints `fsv: tenant <tenant> (config <dir>)` on stderr, binds plan
hashes to the tenant, and refuses `--yes` writes until the tenant is
confirmed (`--tenant`, `FSV_ALLOW_TENANT` or `tenant.confirmed`). Keep one
config dir per tenant and set it explicitly. [INFERRED]

## Tenant differences (production-like vs trial)

| Area | A typical production-like tenant | A fresh trial tenant |
|---|---|---|
| Approvals | legacy flat approvals with levels; chain reads answer 400 | parallel approvals (groups and chains) |
| Assets | legacy asset model (`assets-to-associate`, `associated_assets`) | integrated ITAM inventory (`associate-ci`); legacy picker 403 |
| Business rules | many, including `current_entity` and `enable` actions; one module's rule endpoint may answer 500 | few or none |
| Change lifecycles | many states, branches, entry criteria, field locks | linear stock flow, no locks |
| Workflow automators | drive status, routing, tasks, approvals, notifications | none |
| SLA | several policies and calendars | default policy |
| Status ids | custom statuses with tenant-specific ids | different ids for the same labels |
| Role | many agents lack admin, delete and some read privileges (403) | the tester is account admin |
| Records | old data; time-boxed views can look empty | few records |

[VERIFIED LIVE, read-only on the production-like tenant; VERIFIED LIVE on
the trial]

Consequences: a write that passes on a trial proves the request shape, not
behaviour under rules, locks and workflows. Replicate those first (see
[live testing](../live-testing.md#trial-replication)), detect the
approval and asset models at runtime, and never hardcode status, reminder
or approval ids. [INFERRED]

Some "plan limits" seen on a trial were misdiagnoses: an empty template
list with `max_template_count: 0` (a count, not a cap), and a maintenance
window form that had not finished rendering. Check the right admin page
and wait before concluding a feature is missing. [VERIFIED LIVE]
