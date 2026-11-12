# Reviewed operations and metadata freshness

## Local attachments

```sh
fsv tickets add-note INC-123 --body 'Acme evidence' --attach evidence.txt --attach trace.txt --dry-run --json
fsv tickets reply INC-123 'Acme update' --sender support@example.com --attach evidence.txt --dry-run --json
fsv tickets create --requester person@example.com --set 'Subject=Acme incident' --set 'Description=Acme evidence' --attach evidence.txt --dry-run --json
fsv problems note PRB-123 --body 'Acme evidence' --attach evidence.txt --dry-run --json
fsv attachments delete 701 --owner tickets/123 --child 901 --dry-run --json
```

Send with `--yes --expect-plan FULL_HASH --json`. Public ticket notes also need
`--confirm-public`. Local files are read and SHA-256 hashed for review; metadata,
owner, size limit and file bytes are rechecked before uploading. Each file uses
one JSON data-URI POST to `/api/_/attachments`; returned IDs are supplied in the
owning write's `attachments` array. Dry runs never upload. Independent exact-ID
parent/conversation/note reads verify normalized text, submitted fields, and file
IDs/names/sizes; upload echoes are not verification of the owning write.

**Size limit.** The limit comes from the tenant, never from a constant built
into fsv:

1. Explicitly byte-denominated `max_attachment_size_bytes` or
   `max_file_size_bytes` in the form metadata, if present.
2. Otherwise the agent app page (`GET /a/dashboard`), which sets
   `window.MAX_ATTACHMENT_LIMIT = <n>;`. The web UI divides file sizes by
   1048576 and checks the total of all files in one form against it, so fsv
   reads `<n>` as MiB per submit (40 gives 41,943,040 bytes) and checks the
   total of every `--attach` file.

The review's `attachment_size_limit` shows the bytes, the source, the tenant's
published value and the total of the files. Invalid, conflicting or ambiguous
values block, and so does an unreadable page. If the tenant publishes no limit,
the review shows `not published by the tenant`. Sending then needs
`--max-bytes N`, an explicit total limit in bytes. `--max-bytes` also replaces
a published limit. The limit is part of the plan hash, so a change between
review and send means reviewing again. Files over the limit are refused before
any upload. The server does not enforce the web's limit itself, so this check is
fsv's own guard.

Upload plus owning write is not atomic. A durable transaction guard precedes
the first upload, and known uploaded IDs are journaled. Any failed/ambiguous
step stops immediately, reports known IDs, and forbids replay; no automatic
cleanup, resend or duplicate override is provided. A missing upload ID blocks
the owning write. Attached replies require an explicitly configured sender;
combined attachment/status or reply-recipient variants remain refused.

Deletion requires an exact independently readable owner: `--owner tickets/ID`,
`problems/ID`, or `changes/ID`; add `--child CONVERSATION_OR_NOTE_ID` for ticket
conversations/problem notes or `--section NATIVE_KEY` for planning attachments.
The reviewed single DELETE uses the attachment ID. Readback requires its absence
and preservation of every other attachment ID on that exact owner; unavailable
inventories do not prove deletion. There is no attachment GET contract, and
unowned/orphan deletion is refused because independent verification is not proven.

## Problem status lifecycle

```sh
fsv problems status PRB-123 --to Resolve --dry-run --json
fsv problems status PRB-123 --to Closed --dry-run --json
fsv problems status PRB-123 --to Open --yes --expect-plan FULL_HASH --json
```

Status labels and IDs come from fresh problem form metadata, including tenant
custom statuses. The minimal status-only PUT is gated by fresh form rules and
an exact record. Closing additionally requires readable nonempty Root Cause,
Impact and Symptoms plus a linked solution/workaround, and required-for-closure
fields. Missing prerequisites are listed as `violated_closure_rules` before
sending; server closure rejections receive the same actionable explanation.
Review warns that the server may auto-fill planned dates; independent GET
readback reports those dates without inventing timestamps. Changed analysis,
solutions, metadata or record invalidates the review. Use a full hash and one
attempt; never replay an ambiguous or unverified transition. Offline tests do
not certify live tenant behavior.

## Ticket tasks

```sh
fsv tickets tasks list INC-123 --json
fsv tickets tasks add INC-123 --title "Acme check" --set "Tier=Standard" --dry-run --json
fsv tickets tasks status INC-123 701 --to Completed --dry-run --json
fsv tickets tasks delete INC-123 701 --dry-run --json
```

Send each reviewed write with `--yes --expect-plan FULL_HASH --json`. Required
custom fields, status IDs (including custom statuses), and reminder IDs come from
fresh `ticket_task_fields`. Required means `required_for_create` (top level or in
`field_options`, boolean or `"true"`); `required_for_closure` is not checked on
create. `notify_before` (Never) is sent only when the metadata defines it.
`workspace_id` comes from the parent ticket and is not sent. Custom keys are sent
without the `cf_` prefix, like the web form. Every required field fsv cannot fill
is listed in one error. Unsupported custom types block rather than guess.
`add` and `update` accept `--owner USER_ID_OR_EXACT_EMAIL` and `--group ID_OR_EXACT_NAME`
only when fresh task metadata defines unique active `owner_id`/`group_id` fields.
Email uses the shared active-agent resolver; names and implicit self assignment
are not accepted for owners. Update submits only supplied title/assignment fields.
The resolved IDs and fresh form/inventory are hash-bound and checked before send.

Deletion uses the native PUT tasks/update action, not DELETE. Complete inventory
and metadata drift checks precede one journaled attempt; independent exact-ID
readback compares submitted values. Missing or partial readback is unverified
and must not be replayed. Tests are offline, not live tenant certification.

## Problem incident associations

```sh
fsv problems link-incident PRB-7 INC-123 --dry-run --json
fsv problems link-incident PRB-7 INC-123 --yes --expect-plan FULL_HASH --json
fsv problems unlink-incident PRB-7 INC-123 --dry-run --json
```

A fresh exact problem, incident type and complete association inventory are
required at review and checked again before sending. The unlink request uses a
JSON body even though its method is DELETE. A single journaled request is
followed by complete independent association readback; never resend an
ambiguous or unverified outcome. These offline tests do not establish live
acceptance for another tenant.

## List filter limits

Date `>=` and `<=` use inclusive ranges in the actual list `query_hash`.
Strict date `>` / `<`, date `!=`, numeric inequalities and contains (`~=`)
remain refused: the reviewed query builders have no proven exact wire for those
operations. They are never broadened to inclusive/equality filters or sent as
invented operator names. The captured relative-period token wire
(`Created=Last 6 months` → `is_greater_than: six_months`) is **not** evidence for
strict comparisons to a calendar date, arbitrary numeric inequalities, or
contains. Relative periods support `=` / `>=` only; inverse periods fail closed.

CLI and shared builders use the same strict expression grammar. Malformed
operators (`==`, `=>`, `<>`, `~~=`), missing fields/values, controls and unmatched
quotes are rejected, with no default-to-equality operator fallback. Quote literal
values beginning with an operator, e.g. `--where "Subject='=Acme'"`. Unsupported
filters stop before any list request; offline two-tenant HTTP tests assert the
exact supported range/period query hashes and absence of broadened requests.
Use an inclusive bound only when it matches the intended query.

## Ticket note edits and deletion

```sh
fsv tickets note-edit INC-123 99001 --body 'Updated note' --dry-run --json
fsv tickets note-edit INC-123 99001 --body 'Updated note' --yes --expect-plan FULL_HASH --json
fsv tickets note-delete INC-123 99001 --dry-run --json
```

Only an exact note in a complete ticket conversation history is eligible.
Review includes the prior note; the full hash binds its state and the submitted
body. Text is escaped as plain text. Deletion is irreversible from this CLI.
A single PUT or DELETE is journaled before sending; exact-ID independent
readback verifies the result. Partial histories and unknown outcomes are never
proof of deletion; inspect the ticket and journal and do not resend.

## Ticket assignment and pick-up

```sh
fsv tickets assign INC-123 --unassign --dry-run --json
fsv tickets assign INC-123 --agent agent@example.com --group 'Acme Desk' --dry-run
fsv tickets pick-up INC-123 INC-124 --dry-run --json
```

Send a reviewed plan only with `--yes --expect-plan FULL_HASH`. Group-only
assignment explicitly clears the previous agent. Unassignment uses a single
`PUT tickets/ID`, never bulk update: bulk null responders can silently do
nothing. Agent/group membership is checked from fresh bootstrap metadata on
both sides; missing or inconsistent membership blocks assignment. Pick-up uses
the current runtime agent and verifies every ticket independently. Rules and
fresh records are checked again before the single request; partial or unknown
outcomes must not be resent.

## Irreversible ticket merge

```sh
fsv tickets merge INC-123 INC-124 INC-125 --note-primary 'Combined records' --note-secondary 'Merged into primary' --dry-run --json
fsv tickets merge INC-123 INC-124 INC-125 --note-primary 'Combined records' --note-secondary 'Merged into primary' --yes --expect-plan FULL_HASH --json
```

The first ticket survives. Notes are private by default; `--public-primary` or
`--public-secondary` explicitly selects public visibility. Merge is irreversible.
Complete ticket and conversation reads and runtime Closed status metadata are
required before review and checked again before the single `complete_merge`
request. The precheck endpoint is never used as a merge. Independent readback
checks closure, moved conversation IDs, notes and the primary status; any
missing fields or partial history leave the result unverified. Never resend an
ambiguous merge; inspect the exact tickets and journal instead.

Change legacy flags use the same form engine, business rules, changed-field
payload, duplicate journal, and independent readback as `--set`:

```sh
fsv changes update CHN-123 --status Planning --dry-run --json
fsv changes update CHN-123 --status Planning --yes --expect-plan REVIEWED_HASH
```

Review the full JSON payload before sending. A mismatched hash refuses the
write. Dry runs do not send mutations. A same-status edit is a no-op.

Ticket and service-request property forms have an explicit reviewed mode:

```sh
fsv tickets update INC-123 --reviewed --set 'Subject=Acme maintenance' --priority High --dry-run
fsv tickets update INC-123 --reviewed --set 'Subject=Acme maintenance' --priority High --yes --expect-plan REVIEWED_HASH
```

It resolves choices and lookups from fresh metadata, evaluates edit rules, and
supports the captured subject/description/requester/assignment, category/type,
priority/urgency/impact/source/status and runtime custom-field update shape.
Tags and planned dates can be included in the same review. Uncaptured core
fields and combined Resolved/Closed workflows are refused. Readback reports
missing fields as unavailable, not verified. Legacy basic update flags remain
available; `--reviewed` selects this stricter path explicitly. A real runtime
`default_requested_for` field uses the separately captured SR
`requested_for_id` update key; this does not enable that field for change or
incident creation. Beneficiary edits discard the previous person's rule
attributes; missing exact beneficiary-dependent context blocks instead of
reusing actor/old-person metadata.

The main TUI's **e** key on a ticket/SR opens a metadata-backed property
picker, then a value/choice picker and exact before/after/hash review. Each
review changes one property. Choices come from current metadata; unsupported
and synthetic fields are not offered. Results remain in a scrollable outcome
screen. Combined multi-field required transitions should use the reviewed CLI
or the web form, not a sequence of single-field edits.

## BCC replies and whole-ticket forwards

```sh
fsv tickets reply INC-123 'Acme update' --bcc person@example.com --sender support@example.com --dry-run
fsv tickets forward INC-123 'Acme update' --to person@example.com --sender support@example.com --dry-run
```

Both require `--yes --expect-plan FULL_HASH` to send after review. Recipients
must resolve uniquely and are checked again before sending. `--sender` must
be the configured support mailbox: it is explicitly user-asserted, never
inferred or certified against an unproven mailbox endpoint. BCC and forward
payloads have blocked-wire evidence, not accepted-send proof. Whole-ticket
forward is not conversation forwarding; BCC+CC/status, alternate-To, and
reply-all remain refused or unsupported. Missing BCC/sender readback fields
remain unavailable, and a successful send is never replayed to obtain them.

The reviewed property and reply/note-Pending paths also have real
`Client`/`httpx.MockTransport` tests with distinct Acme/Globex schemas and
runtime status IDs. They check exact HTTP bytes/headers, readback identity,
missing/contradictory fields, stale metadata and 403/429/500/disconnect guards.
CC display/email resolution is rechecked before send without changing the
reviewed payload. These are offline transport proofs, not accepted live sends.

## Structured output

Watchers (`list`/`set`), time (`list`/`write`), `planning-text`,
`mark-known-error`, `bulk-priority`, and ticket `forward` accept `--json`
and `--output json` (also `-o json`). Their existing JSON default and
review/result schemas are unchanged. Other formats are rejected before a client
is opened; these commands do not flatten structured reviews into tables or CSV.
Output options never bypass `--dry-run` or the full-hash send gate.

## Canned responses and scenarios

```sh
fsv tickets canned folders --json
fsv tickets canned list --folder 702 --output json
fsv tickets canned show 701 --json
fsv tickets canned reply INC-123 --response 701 --content-pointer /canned_response/content --sender support@example.com --dry-run --json
fsv tickets scenarios list --json
fsv tickets scenarios show 703 --json
fsv tickets scenarios apply INC-123 --scenario 703 --dry-run --json
```

Inspect `show` before choosing the content JSON pointer; detail key names are
not assumed portable. Canned insertion sends only the selected literal HTML in
the ordinary reviewed reply body, with no canned-ID wire field. Placeholders
are refused, not expanded. Attachments/signatures are not inserted. Review the
entire body and source hash, then use `--yes --expect-plan FULL_HASH`. The source
is re-read before sending; metadata/content drift blocks the send. Legacy reply
hashes and durable send guards are preserved.

Scenario `show` selects an exact ID from the captured list route, never an
inferred detail endpoint. `apply` uses the captured single-ticket native execute
wire, **not** canned insertion: automation can send mail, mutate records or run
workflows. The definition and exact ticket are hash-bound and refreshed before
a single attempt. Exact ticket readback is attempted, but `verified` remains
false because complete scenario side effects cannot be established. Never
repeat an unverified execution. Both scenario and canned reads expose server
responses without claiming to exhaust any undocumented pagination.

All tests use synthetic sources and offline transports. Blocked browser wires
establish shape only, not accepted live sends or portable template schemas.

## Private ticket views

```sh
fsv tickets private-views show 701 --json
fsv tickets private-views write 701 create --name 'Acme personal queue' --dry-run --json
fsv tickets private-views write 702 edit --name 'Acme renamed queue' --dry-run --json
fsv tickets private-views write 702 delete --dry-run --json
```

`create` clones the source's ordinary query and workspace unchanged; `edit`
renames an exact view. Every action requires fresh self ownership, an explicitly
false default flag, and named private visibility metadata (`Me`, `Only me`, or
`Private`). The numeric visibility enum is taken from that runtime metadata,
never hardcoded. If an account exposes only a number, mutation is **blocked**;
this adapter does not infer privacy from ownership. Synthetic metadata tests do
not prove that every account exposes those labels. Shared/default views,
advanced queries, arbitrary filter edits and other sort modes remain unsupported.

Use `--yes --expect-plan FULL_HASH` after each review. Source/target drift blocks
sending. Creation requires the exact returned view ID and reads that object,
never a same-name or latest match. Deletion requires both exact-target 404 (also
HTML/empty responses) and absence from a complete inventory. Ambiguous write
outcomes keep the durable single-attempt guard. The captured private lifecycle
was accepted, but these adapters were exercised offline only.

## Problem workflows

```sh
fsv form dump problems
fsv form explain problems --set 'Status=Open'
fsv problems create --requester person@example.com --set 'Subject=Acme investigation' --set 'Description=Acme details' --set 'Status=Open' --set 'Priority=Low' --set 'Impact=Low' --dry-run --json
fsv problems write-child PRB-123 note '<div>Acme evidence</div>' --dry-run --json
fsv problems write-child PRB-123 task 'Acme follow-up' --dry-run --json
```

Creation uses the shared form/rule engine, fresh metadata, resolved requester,
required-field/choice validation and dependency gates. Unavailable rules are not
an empty ruleset; incomplete dependencies and rule messages block sending.
Enums come from runtime choices. Only the captured top-level create keys are
sent; there are no files or inferred validation-token keys. Unsupported required
fields must be handled in the web form. Each send requires the full reviewed
hash and uses a durable single-attempt journal plus exact returned-ID readback.
Missing readback fields remain unavailable, not matched.

Notes use the captured **body-only** wire: privacy and notification effects are
not established, so this command does not promise a private note. Task creation
is the captured narrow self-owned one-minute subset, with Open/Never resolved
from **problem** task metadata, never change metadata or literal enum IDs. Extra
required task fields block it. Task edit/complete/delete and broader scheduling
are not inferred from create. No accepted live problem write is claimed; all
adapter tests run offline with two synthetic tenants.

## Native columns and export requests

```sh
fsv tickets columns list --json
fsv tickets columns set --column ticket_subject --column requester --column status --dry-run --json
fsv changes columns set --column requester --column ticket_subject --column status --freeze requester --dry-run --json
fsv problems export --view 701 --field subject --field status_name --field display_id --dry-run --json
```

All three modules expose these commands. Column names/labels come from fresh
runtime headers. Ordered columns keep the captured display-ID anchor alongside
the subject; only change-requester freezing beyond that anchor was captured.
The final POST is single-attempt and hash-bound to the prior metadata. Readback
requires selected/position/frozen evidence and checks deselected columns too;
missing preference fields are unverified, not inferred. Account preference scope
is not proven private: review before altering it.

Native `export` is separate from `ls --output csv` (local read-only rendering).
Only the captured CSV, 30-day, created-at dialog variant is supported. Field keys
are the captured wire keys, labels come from runtime headers, and the full
ordinary saved-view query is copied without inventing choice IDs. Advanced
queries/custom export fields/other format-date variants are blocked. Use the
full hash with `--yes --expect-plan FULL_HASH` for either write.

Export requests may email data or launch a job. The source view is read back,
but **job/delivery/download verification is unavailable**: `verified` remains
false, and no filename, job ID, polling endpoint or successful download is
fabricated. An unknown response never causes another export attempt. The
contracts were browser-aborted; public tests use synthetic HTTP only.

## Read-only CI/asset picker and task limits

```sh
fsv changes asset-picker CHN-123 --search Acme --json
fsv changes asset-picker CHN-123 --search Acme --page 2 --select 701 --output json
```

The CLI selects an exact **display ID**, never an internal asset ID, from one
explicit search page. In the TUI, select a change and press `i`; search, pick or
cancel. The selected candidate appears in a persistent outcome screen. This is
available in read-only mode and issues GETs only. Empty results and duplicate
identities are handled without manufacturing candidates. Both interfaces label
the result as a single page, not a complete CMDB inventory.

Read-only selection **does not associate** an asset/CI. In writable mode the
`i` menu separately offers reviewed integrated device/CI and business-service
add/remove on tickets, SRs, problems and changes (see Integrated inventory below).
Physical-asset mutations and legacy association helpers are not enabled by this
picker. Task contracts remain method-specific; no maintenance writer or guessed
optional payload keys are added. Offline Pilot tests are not accepted live sends.

## Legacy-write transport coverage and residual safety debt

`tests/test_legacy_write_guards.py` drives real `Client`/`MockTransport` through
legacy change note/status, asset add/remove, task update/delete and task-clone
callers. Each caller is exercised for 403, 429, server failure, disconnect, CSRF
preparation failure and success. Assertions cover exact current method/path/body,
one automatic attempt, token preparation and failure audit rows. Clone tests
preserve the old envelope as compatibility evidence, not accepted native parity.

These are **transport checks, not durable-plan certification**. Legacy helpers
still lack reviewed full-hash gates, pre-send durable started guards across
process restarts, and independent exact-ID/field readback. Their transport audit
is written after the attempt; it must not be presented as the reviewed sender's
pre-send ambiguity guard. Broader clone base/side-effect transaction and
attachment rollback/failure matrices also remain unfinished. Use the dedicated
reviewed workflows where available; do not retry a legacy ambiguous write based
on these tests or interpret an old helper's response as independent verification.

## Self watchers

```sh
fsv tickets watchers list INC-123
fsv tickets watchers set INC-123 --watch --dry-run
fsv changes watchers set CHN-123 --unwatch --dry-run
```

Send with `--yes --expect-plan FULL_HASH`. Identity comes from a fresh
signed-in-agent read; arbitrary subscriber changes are not supported. Ticket
and change methods differ and are kept separate. Complete membership is
required before review, and readback compares the full expected membership
including all other subscribers. Already-satisfied requests send nothing.
These exact self-watch lifecycles have accepted-contract evidence; no live
calls are made by the offline tests.

## Manual ticket time

```sh
fsv tickets time list INC-123
fsv tickets time write INC-123 create --spent 00:01 --executed-at 2030-01-02T10:00:00Z --note Acme --dry-run
fsv tickets time write INC-123 update --entry 456 --note Globex --dry-run
fsv tickets time write INC-123 delete --entry 456 --dry-run
```

Use `--yes --expect-plan FULL_HASH` after reviewing each operation. Only
self-owned, nonbillable, stopped manual entries are supported. Durations use
HH:MM and start timestamps require an explicit offset. Updates preserve the
unmodified time/note and do not change start time. Timers, billing, task-linked
time, and change/problem time mutations are not implemented. Reads must be
complete; create verification needs the exact returned entry ID. An absent
response ID or failed readback never triggers a resend or latest-entry guess.
Readback compares HH:MM and HH:MM:SS by seconds and explicit-offset timestamps
by instant, so server normalization does not create false mismatches. Missing
fields remain unavailable; invalid or genuinely different values mismatch.
Note-only updates preserve minute-aligned server durations; an existing duration
with nonzero seconds requires an explicit `--spent` rather than silent rounding.
Task and ticket-property timestamps share the same strict instant comparator;
planning text is verbatim, watchers compare normalized numeric memberships, and
SR recipient fields remain exact (they are not durations or timestamps).

## Main-TUI conversation options

In a ticket/SR note (`n`) or detail-pane reply (`r`), compose the text then
press **Ctrl+O** for recipient/status options. Notes support Notify-to and
Pending; replies support CC and Pending, or the separately captured BCC plus
explicit sender variant. Leave status unchanged by default. Choices and
recipient lookups are evaluated before exact payload/hash review. Public
conversations require a second visibility/mail confirmation. Results stay in
a scrollable outcome screen, including unverified readback and returned IDs;
there is no retry button. **Ctrl+S** keeps the simple compose/review path.
Read-only mode blocks option preparation and sends.

## Explicit-ticket bulk priority

```sh
fsv tickets bulk-priority INC-123 SR-124 --priority High --dry-run
fsv tickets bulk-priority INC-123 SR-124 --priority High --yes --expect-plan REVIEWED_HASH
```

Only captured bulk priority is implemented, with 1–100 explicit unique ticket
IDs and no query/view/all-pages expansion. Fresh per-ticket edit metadata and
rules must agree on the chosen priority enum. Every record/metadata hash is
bound to review, and every target is read independently after the single bulk
request. Partial or missing per-ticket results remain unverified and return
nonzero from the CLI; neither the whole batch nor failed members are retried.
This has blocked-wire/offline transport evidence, not accepted live bulk proof.
Spam/delete/merge/pick-up and scenario execution are not included.

## Problem known-error mark

```sh
fsv problems mark-known-error PRB-123 --dry-run
fsv problems mark-known-error PRB-123 --yes --expect-plan REVIEWED_HASH
```

The main TUI's selected-problem action menu also offers this exact reviewed
mark. This implements only captured `PUT problems/ID {known_error:true}`:
no reversal, resolution/closure, or knowledge publishing is inferred. Exact
problem identity and the boolean field must be readable; an already-marked
problem is a no-op. The fresh record binds the review, independent readback
checks the flag, and ambiguous sends retain a journal guard. Evidence is a
blocked request plus offline tests, not an accepted live lifecycle.

## Native planning text and problem analysis

```sh
fsv changes planning-text CHN-123 --section 'Rollout Plan' --description '<div>Acme plan</div>' --dry-run
fsv problems planning-text PRB-123 --section 'Root Cause' --description '<div>Acme evidence</div>' --dry-run
```

Review, then pass `--yes --expect-plan FULL_HASH`. Change section identities
come from fresh field metadata; problem native sections are Root Cause,
Impact and Symptoms, with the captured semantic section keys. This is not a
custom-field solution editor. A fresh complete planning inventory determines
whether to create or edit before review and again before sending. Known-empty
sections use the first-write POST with `?id=SECTION`; populated sections use
PUT to `planning-fields/SECTION`. Both send a description-only body. Use
`--clear` instead of `--description` to review a bodyless DELETE on that same
section path:

```sh
fsv changes planning-text CHN-123 --section 'Rollout Plan' --clear --dry-run
fsv problems planning-text PRB-123 --section 'Root Cause' --clear --dry-run
```

Clear refuses already-empty sections. Unknown text state, stored empty rows
whose create/edit state is unproven, existing attachments or unavailable
attachment inventories block text-only writes. Whitespace/HTML is not stripped
to infer emptiness; contradictory text/empty representations fail closed.
These operations delete section text, not the section definition or parent record.

The review shows `operation`, `before` and `after` text alongside the exact
wire and full hash. A changed inventory invalidates review; sends use one
lower-level request without automatic write replay and independent GET
readback. Unknown/mismatched readback is not success and must not be resent.

In the main TUI use **a → Edit native planning/analysis text**, choose the
section, edit the multiline text/HTML, then **Ctrl+S** to review the exact
hash-bound save. Cancel sends nothing; results stay in the outcome screen.
Workaround/permanent-solution/KB publishing and planning-file changes are
not covered by this text-only workflow. Accepted native UI edits and clears
were independently read back on a self-only test account; portability tests
use synthetic fixtures. Mixed text/file writes remain unproven. The TUI edits
text; clearing is an explicit CLI `--clear` operation, not a blank editor save.

Schema reads use a seven-day cache, including legacy cache envelopes. Stale
reads are identified internally with `stale` and `source`, warn on stderr,
and start a background refresh; JSON stdout remains parseable. Force a
synchronous fetch before inspecting current choices:

```sh
fsv changes fields --choices status --refresh --json
fsv tickets lookup priority High --refresh --json
```

Explicit refresh failures are errors, not silent fallbacks to old definitions.
Reviewed form sends separately re-fetch fields and business rules before
submitting, so the schema display cache is not a write authorization source.


## Integrated inventory

`fsv assets search Acme --kind devices --json` searches integrated ITAM inventory,
not legacy asset display IDs. `--kind services` searches business services;
`--kind assets` is read-only physical-asset search. Unproven type filters and
physical-asset association writes remain disabled.

`tickets|sr|problems|changes assets list|add|remove` uses `associated-cis`,
`associate-ci`, and `disassociate-ci`. Add accepts several inventory IDs in one
reviewed PUT; removal accepts exactly one inventory ID and resolves its exact
`itam_cmdb_request_id` from the target's complete inventory. Example:
`fsv changes assets add CHN-23 81 82 --dry-run --json`, then repeat with
`--yes --expect-plan FULL_HASH`. Use `--kind services` for business services.
`changes ci-list|ci-add|ci-remove` are aliases. The legacy nested search alias
and `changes asset-picker` now read integrated Devices, not `assets-to-associate`.
The TUI `i` menu keeps read-only browsing separate from explicit add/remove.
Choose devices/CI or business services; add searches one page, while remove
reads the exact parent's complete associated inventory. Selection prepares the
same CLI plan/hash, never sends. Inspect the exact PUT/payload/hash and confirm
with **y**. Cancel sends nothing. One journaled attempt is independently read
back; unknown/unverified outcomes remain visible with no retry action. Read-only
mode exposes browsing only and blocks preparation and sends. Legacy physical
assets remain unsupported. Pilot/MockTransport tests cover every parent/kind,
cancellation, single send, readback, and refused replay after disconnect.

Incident and change creation accept repeatable `--device INVENTORY_ID`. The
`device_ids` array is included in the exact reviewed create payload and hash;
independent association readback is reported as `device_verification`.


## Service-request approvals

`fsv sr approvals list SR-23 --json` reads complete nested chains. `request`
requires `--name`, `--message`, repeatable `--approver USER_ID_OR_EMAIL` and an
optional `--rule everyone|anyone|majority|first-responder`. Emails resolve through
requester autocomplete, not agent-profile IDs. `approve|reject|remind SR-23 ID`
use an approval-record ID; `cancel SR-23 ID` uses the approval-group ID. Reject
requires `--remark`. Every write requires dry-run review then
`--yes --expect-plan FULL_HASH`; review binds complete approval history.
Bodyless cancel/reminder requests retain JSON media type; reminder uses `resend`,
not an invented remind endpoint. Independent GET verifies exact new recipients,
decision remark history, cancellation, or a later reminder timestamp. Immutable
approved/rejected history is not cancellable. Notifications cannot be recalled.
Delegation remains unsupported because its write contract is unproven.


## Change task update

`changes tasks-update CHN-23 17 --title 'Acme review' --owner 81 --group 91
--status 'In Progress' --note 'Review evidence' --planned-start 2026-12-01T09:00:00Z
--planned-end 2026-12-01T10:00:00Z --planned-effort '1h 30m'
--notify-before 900 --dry-run --json` reviews the captured PUT. Repeat with
`--yes --expect-plan FULL_HASH` to send exactly once. Reminder values resolve
against runtime metadata in **seconds**. `--owner none` clears assignment;
`--agent` remains an owner alias. `--system`/`--environment` resolve custom
choices through runtime metadata. Only supplied fields are updated; unchanged
fields are not invented. Exact task GET verifies normalized dates, rich-text
note, owner alias, duration, and custom values. Unproven editor/due-date writes
are retired and fail closed. Task edits may notify assignees or schedule reminders.


## Knowledge solutions

`fsv solutions create --title 'Acme workaround' --body 'Synthetic guidance'
--type workaround --dry-run --json` creates a **draft only** after full-hash
confirmation. `--type permanent` selects the other accepted application enum.
Category/folder IDs and defaults come from complete live metadata; pass exact
`--category` and `--folder` names/IDs to override. Folder workspace is preserved,
not silently copied from the problem. Publication and attachments are unsupported.

Create and link are separate reviewed writes: `fsv problems link-solution PRB-23
ARTICLE_ID --dry-run --json`, then `--yes --expect-plan FULL_HASH`.
`unlink-solution` uses the same article ID and exact-target readback. Verification
reads `articles` from the linked-solutions inventory, not an invented `solutions`
envelope. `problems status PRB-23 --to Closed` accepts persisted, nondeleted
workaround/permanent links once analysis and runtime closure-required fields are
satisfied; incomplete inventories still block closure.


## Change group/CAB approvers

`fsv changes approval-request CHN-23 --name 'Acme review' --message 'Review'
--approver-group 'Acme reviewers' --cab 'Release review' --dry-run --json`
resolves exact runtime groups/CABs and reads eligible `approval_users` from that
change. These explicit flags select **all** eligible members; unlike opening the
native chooser, they are not merely browsing checkboxes. Review every expanded
recipient. Optional repeatable `--approver` adds individuals. Overlapping sources
are deduplicated and flattened to `notified_to` user IDs; no group/CAB identity
is sent in the create body. The full hash includes recipient-source membership,
and a pre-send read blocks changed membership. Empty, incomplete, duplicate, or
unavailable sources fail closed. Use `--yes --expect-plan FULL_HASH` only after
review; email side effects cannot be undone by cancellation.


## Required task fields

Change task creation now reads nested `field_options.required_for_create`
(including string `"true"`), validates required custom fields and planned dates,
and binds fresh metadata to its review hash. Parent-provided workspace is not
invented in the task payload. Supply repeatable `--set FIELD=VALUE`, or
`--system`/`--environment`, plus required `--planned-start`/`--planned-end` dates.
Custom choice IDs resolve from runtime field metadata; `cf_` names become native
`custom_fields` keys. The TUI create path also preflights required fields and
blocks unsupported custom inputs rather than submitting an incomplete task.

Read-only trial dry-runs reproduced the change bug: the previous CLI emitted a
valid-looking plan while omitting two required custom fields and both planned
dates. The fixed dry-run blocks those omissions; supplying metadata-resolved
values produces a plan. Ticket task creation already blocked its required custom
field; its existing `tickets tasks add --set FIELD=VALUE` succeeds in dry-run when
provided. Both checks exercised the real client with non-GET requests disabled;
no live task was created or updated.
