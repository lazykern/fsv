# Reviewed operations and metadata freshness

## Captured-write verification boundaries

New bulk/relationship/time/task/requested-item inventories validate raw pages before the display
paginator can discard malformed rows or deduplicate identities. Duplicate IDs,
conflicting totals, invalid continuation flags and incomplete histories block
both authorization and verification. Initial-owner readback checks the hash-bound resolved requester ID when the
server exposes requester_id rather than echoing email; conflicting exposed
requester representations fail verification. Submitted dates use semantic instant
comparison, and returned owner IDs are journaled before independent readback.
Explicit sender readback compares the
email identity (the server can add a display name), not configured-mailbox
ownership. Recipient aliases must agree when simultaneously exposed.
Initial change uploads retain prior create-hash ambiguity guards without an
expiry/bypass; uncertain older transactions cannot be retried by upgrading the
review format. `changes create --max-bytes` requires `--attach-flow before-create`
rather than silently ignoring a limit on the legacy after-create route.


## Remaining accepted-wire blockers

The supplied capability ledgers do not establish the following writes. These are
**not implemented**, not guessed from public APIs, metadata choices, or usage:

| Remaining scope | Evidence boundary / required capture |
| --- | --- |
| Ticket/SR closure with planned dates and conditional required answers | No accepted combined closure/date/branch submission plus matching rule metadata and exact-record readback. Standalone lifecycle/property writes do not establish this combination. |
| Alert-to-incident actions or alert association add/remove | Usage proves attached alerts, not a manual action contract. Need exact inventory, alert/incident identity joins, accepted mutation and independent readback. |
| Arbitrary custom person/lookup branches and unsupported field kinds | Need accepted lookup resolution and exact branch create/update payload/readback; no fallback opaque IDs. |
| Inline-image upload on replies/notes | Successful upload response and final HTML are recorded, but upload request body/media type were omitted. No bodyless or multipart upload is assumed. |
| Existing-project association | The accepted project add is create-and-associate only. Existing-project linking needs its own wire. Project-task add/remove and project unlink are separately implemented. |
| Native ticket/change templates | Empty native inventories; a local preset is not native application evidence. |
| Reply-all, editable reply To, alternate sender configuration | No accepted variant in the supplied fixtures. Explicit CLI sender remains user-asserted, not mailbox discovery. |
| Maintenance/window create or association | Matching response was empty; backend/window controls did not yield an accepted association. A matching POST is not a write contract. |
| Native clone count/options | No native clone control or accepted options/count payload. Existing local clone utilities are not certified by these captures. |
| Approval delegation/group editing and uncaptured recipient kinds | No accepted mutation and exact approval-chain readback. |
| Combined BCC+CC/status, forward files/CC/BCC/status, cloud inserts | Existing separate variants do not prove these combinations. |
| Problem time update, running timers/other owners/date edits | Only problem create/delete and change manual CRUD are established here. |
| Project deletion (`projects delete`) | The recorded `DELETE /projects/{id}` with an exact-name confirmation body omitted its host, authentication and media type, and fsv has no independent exact project readback for an unlinked project. Not implemented. `changes delete` is implemented from the accepted bodyless `DELETE /api/_/changes/{id}` with exact `deleted:true` readback. |
| General task custom updates, ticket-task due/reminder/effort variants | Problem optional-field evidence is not a ticket-task contract; generic custom fields are not enabled. |

New scope stays CLI-only; no main-TUI parity, notification suppression, physical
inline-storage deletion, native all-pages bulk flag, or live-tenant certification
is claimed. Tests use synthetic offline transports; no live writes are required.


## Change/problem time and problem-task writes

```sh
fsv changes time list CHN-123 --json
fsv changes time write CHN-123 create --spent 00:01 --executed-at 2026-01-01T09:00:00+00:00 --note 'Acme work' --billable --dry-run --json
fsv changes time write CHN-123 update --entry 901 --spent 00:02 --note Updated --non-billable --dry-run --json
fsv changes time write CHN-123 delete --entry 901 --dry-run --json
fsv problems time write PRB-123 create --spent 00:01 --executed-at 2026-01-01T09:00:00+00:00 --note 'Acme work' --billable --dry-run --json
fsv problems time write PRB-123 delete --entry 902 --dry-run --json
fsv problems task-write PRB-123 71 update --title 'Acme task' --note Updated --planned-effort 1h30m --due-date 2026-01-03T09:00:00+00:00 --dry-run --json
fsv problems task-write PRB-123 71 complete --dry-run --json
fsv problems task-write PRB-123 71 delete --dry-run --json
```

Send with `--yes --expect-plan FULL_HASH`. Time is self-owned/manual only;
create defaults billable (as captured), and change update can set billable.
Durations/instants are normalized only for independent v2 readback. Problem time
update, timer actions, other-owner time and executed-date edit remain blocked.
Problem task update also accepts runtime status/owner/group/reminder and planned
start/end. No inferred task custom-field or null-clearing contract. Exact native
task GET verifies owner_id, reminder, dates, description and semantic effort.
Completion is status-only with fresh closure requirements; the server may rewrite
planned dates and stop OLA timers. Delete uses native PUT tasks/update with its
captured string task identity, then complete inventory readback (never guessed
HTTP DELETE). Unknown or unverified operations must not be replayed.


## SR requested-item writes

```sh
fsv sr items list SR-123 --json
fsv sr items write SR-123 edit --item 901 --answers-json '{"title":"Acme request","details":"Updated details"}' --dry-run --json
fsv sr items write SR-123 stage --item 901 --stage 2 --dry-run --json
fsv sr items write SR-123 add --catalog-item 71 --answers-json '{"title":"Acme request","details":"Additional item"}' --dry-run --json
fsv sr items write SR-123 remove --item 902 --dry-run --json
```

Send with `--yes --expect-plan FULL_HASH`. Answers are changed API-name plain
text/paragraph fields only, validated through fresh catalog metadata and rules.
Nonempty/unavailable exact-item editability restrictions fail closed.
Only stage 2 (observed Delivered in the accepted capture) is enabled; other stage
IDs/labels require evidence, not inference. Answer and stage writes are separate.
Removal requires an explicitly non-parent item; add preserves the captured
catalog display-ID string wire. Internal item IDs, catalog display IDs and
requested-item IDs are distinct. Readback uses complete exact-ticket inventory
and exact-item detail, checking every submitted answer and preserving other items.


## Change relationships

```sh
fsv changes relationships list CHN-123 caused-incident --json
fsv changes relationships write CHN-123 caused-incident add --item 71 --dry-run --json
fsv changes relationships write CHN-123 problem remove --item 81 --dry-run --json
fsv changes relationships write CHN-123 release add --item 91 --dry-run --json
fsv changes relationships write CHN-123 project-task add --item ACME-1 --project-key ACME --name 'Acme task' --dry-run --json
fsv changes relationships write CHN-123 project remove --item ACME --dry-run --json
```

Kinds: `caused-incident`, `problem`, `release`, `project`, `project-task`.
Use `write ... remove` for unlink and `list` for exact association inventories.
Send with `--yes --expect-plan FULL_HASH`. Project proxy writes validate nested
status/body and independent linked inventories; incomplete/multiple pages fail
closed. Task add takes an explicit existing task key and name, not a task create.
The project must already be linked. `project add --project-json` explicitly
**creates and associates** a new private project; JSON requires `name`,
`description`, `start_date`, `end_date`, `owner_email`, `project_category` text.
It is not an existing-project link: that wire is absent and remains blocked.
A project create is not rolled back if association/readback fails; never replay.
Caused incidents use their own associated-caused filter, never the candidate list.
Release unlink uses PUT, not DELETE. No maintenance-window contract is inferred.


## Ticket trash, restore and delete forever

```sh
fsv tickets trash list --json
fsv tickets restore INC-58 --dry-run --json
fsv tickets delete-forever INC-58 INC-59 --max-records 2 --dry-run --json
fsv tickets delete-forever INC-58 INC-59 --max-records 2 --yes --expect-plan FULL_HASH --confirm-destructive
```

`trash list` reads one page of the Trash view (`GET tickets?filter=deleted`;
`filter=trash` is rejected). The trash holds every agent's deleted tickets, so
writes take explicit IDs only: no filters, no select-all, no empty-trash.
`restore` sends `PUT tickets/<id>/restore` (no body) for one ticket that a fresh
exact read shows as `deleted: true`, and verifies `deleted: false` on a
separate read. `delete-forever` requires `--max-records N` (at most 50) and
`--confirm-destructive`, prints the ID list and subjects on stderr, and only
accepts tickets already in the trash. Only the single-ID request
(`DELETE tickets/delete_forever?ids=<id>`) is captured, so each ticket is sent
as its own request, in order; each one is verified by a 404 on the exact
ticket, and the run stops at the first failed or unverified ticket with the
rest listed as not attempted. Permanent deletion cannot be undone; never replay
an uncertain run.

## Single-ticket spam

```sh
fsv tickets spam INC-58 --dry-run --json
fsv tickets spam INC-58 --yes --expect-plan FULL_HASH --confirm-destructive
fsv tickets unspam INC-58 --dry-run --json
```

`spam` sends `PUT tickets/<id>/spam` with `{}` and `unspam` sends `PUT
tickets/<id>/unspam` without a body, the ticket page's wires. A fresh exact
read must show the opposite `spam` flag (and the ticket not in the trash);
readback is the `spam` flag on a separate read. Spam hides the ticket from
every view, so it also needs `--confirm-destructive`. Several tickets: `tickets
bulk spam|unspam`.

## Frozen bulk ticket actions

```sh
fsv tickets bulk status INC-123 INC-124 --status Pending --dry-run --json
fsv tickets bulk assign INC-123 INC-124 --agent agent@example.com --group 'Acme Desk' --dry-run --json
fsv tickets bulk pickup INC-123 INC-124 --dry-run --json
fsv tickets bulk merge INC-123 INC-124 --primary INC-123 --note-primary Combined --note-secondary Merged --dry-run --json
fsv tickets bulk spam INC-123 INC-124 --dry-run --json
fsv tickets bulk unspam INC-123 INC-124 --dry-run --json
fsv tickets bulk delete --where 'Priority=Low' --select-all --dry-run --json
fsv tickets bulk scenario INC-123 INC-124 --scenario 'Assign to Acme Desk' --dry-run --json
```

Send with `--yes --expect-plan FULL_HASH`. Spam and delete additionally require
`--confirm-destructive`; every invocation prints the exact ID list on stderr.
Filtered selection uses the supported list-query builder, complete pagination,
and at most 100 unique IDs; it expands to explicit IDs, not an unproven native
all-pages mutation flag. Rebuild checks selection/metadata/records before send.
No chunking, ambiguous retry, null assignment or bulk closure. Group and agent
must both be supplied for the accepted assignment variant. Pickup and merge
reuse their existing reviewed planners, durable guards and independent readback.

`bulk scenario` sends `PUT tickets/execute_scenario {"scenario_id", "ids"}` for
explicit IDs only (no filters). `--scenario` must match exactly one scenario by
ID or exact name (case-insensitive only when no exact match exists); an ID that
is also another scenario's name, or several matches, is refused. Scenarios are
never chosen by position. The review and stderr print the resolved scenario ID
and name and warn that its actions (ticket type changes, assignments, emails,
notes) run asynchronously (HTTP 202) and cannot be proven: the result reads
each exact ticket back for identity but keeps `verified: false`. Check the
scenario name before sending; never replay.


## Exact-conversation forward

`fsv tickets forward INC-123 'Acme update' --conversation 901 --to user@example.com
--sender support@example.com --dry-run --json` reviews the captured ticket forward
wire with an integer `conversation_id`. The source must exist exactly once in a
complete history; source edits invalidate the review. Send using
`--yes --expect-plan FULL_HASH`. Source selection is bound to the review; returned
conversations do not expose source identity, so readback verifies delivery fields,
not an independently returned source link (`source_link_verified: false` is
reported separately). No attachment/CC/BCC/status combinations.


## Accepted initial attachment scope

`tickets reply`, `tickets add-note` and `tickets create --attach FILE --max-bytes N`
use hash-bound two-step uploads and exact owning-record readback.
`changes create --attach FILE --attach-flow before-create --max-bytes N` uses
the same durable transaction. Review with `--dry-run --json`; send with
`--yes --expect-plan FULL_HASH`. No replay after an uncertain upload or send.
Initial change attachments cannot combine with devices/planning in this path.
Inline image uploads remain blocked: the accepted ledger records the response
and final image HTML but omits the upload request body/media type. A null capture
is not proof of a bodyless upload; no multipart contract is invented.


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
Impact and Symptoms and required-for-closure fields. A missing linked
solution/workaround is a review warning (the rule is tenant configuration:
observed on one tenant, only 2 of 62 closed problems had one); it blocks only
after this tenant has proven the rule by rejecting a closure with
`check_workaround_or_solution_added` (remembered per tenant in the cache).
Missing prerequisites are listed as `violated_closure_rules` before
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

The filter wires follow what the web's filter panel and advanced builder send:

| `--where` form | Wire | Modules |
|---|---|---|
| `created_at=2030-01-05` | `is_in_the_range {from: D, to: D}` | tickets, changes, problems |
| `created_at>=A` + `created_at<=B` | `is_in_the_range {from: A, to: B}` (whole days, both ends inclusive) | tickets, changes, problems (problems: `created_at` only) |
| `created_at>D` / `created_at<D` | shifted day boundary: `from D+1` / `to D-1` (no strict operator exists; `--debug` and a stderr note say so) | as above |
| `created_at=on:yesterday` (`today yesterday tomorrow week month year`) | advanced `on` | tickets, changes |
| `created_at=last:one_week` / `next:one_week` (`five_minutes one_week one_month two_months six_months one_year`) | advanced `in_the_last` / `in_the_next` | tickets, changes |
| `Created=Last 6 months` | basic `is_greater_than six_months` | tickets, changes; problems only `Created=today` |
| number field `= != >= <=` | advanced `is` / `is_not` / `is_greater_than_or_equal` / `is_less_than_or_equal`, string value | tickets, changes |
| multi-select `=`/`!=` label | advanced `is_in` / `is_not_in` with the integer choice id from runtime metadata | tickets, changes |

A date range needs both ends: the server answers a one-sided `{from}` or `{to}`
with HTTP 500, so `created_at>=D` alone is refused before any request (no
invented epoch/far-future bound). Dates are whole days (`YYYY-MM-DD`); time of
day is refused because the server ignores it. The old `custom_date` operator is
gone: it is not a server operator (206 `search_error` on tickets/changes, a
silently empty list on problems). Advanced conditions go in
`advanced_query_hash={"all":[...,workspace],"any":[]}` with an empty
`query_hash`; `--or` puts the conditions in `any` and the workspace item in
`all` (tickets `workspace_id is_in [{id}]`, changes `workspace_id is <id>`; the
workspace comes from `forms.workspace_id` or the form metadata, and is omitted
when neither names one). Problems have no advanced builder, so `--or`, relative
`on`/`last`/`next`, number and multi-select conditions are refused there, and so
are date fields other than `created_at`.

Text fields (subject, description, custom text/paragraph) have **no** list
filter in any list UI; `--where Subject=...` fails closed with "no list filter
exists ... nothing was sent" and points to `fsv <module> search`. Strict number
`>`/`<`, date `!=` and contains (`~=`) remain refused. A 206 `search_error`
response is refused ("nothing shown") instead of showing unfiltered rows, and
returned rows are re-checked client-side for `is_in`/`is_not_in` and number
predicates (a warning lists contradicting IDs). Filtered bulk writes accept only
basic `query_hash` filters.

CLI and shared builders use the same strict expression grammar. Malformed
operators (`==`, `=>`, `<>`, `~~=`), missing fields/values, controls and unmatched
quotes are rejected, with no default-to-equality operator fallback. Quote literal
values beginning with an operator. Unsupported filters stop before any list
request; offline two-tenant HTTP tests assert the exact query hashes and the
absence of broadened requests.

## Inline images in replies and notes

`fsv tickets reply ID BODY --inline-image shot.png --sender support@example.com`
and `fsv tickets add-note ID --body TEXT --inline-image shot.png` (repeatable)
follow the editor's captured wire: each image is uploaded once with
`POST /api/_/solutions/inline_images` (multipart, single field `file`) which
answers 201 `{link, fileid}`; the returned link is embedded after the body as
`<div><img src="LINK" style="width: auto;" class="fr-fil fr-dib" fileid="ID" data-id="ID"></div>`
in `body` (and the reply's `full_text`). No attachment-id array is sent. The
review lists every image with its size and the attachment size limit (tenant
form metadata, the agent page `MAX_ATTACHMENT_LIMIT`, or `--max-bytes`); only
JPEG, PNG, GIF and WebP are accepted (the editor's file filter). The send is
single-attempt and journaled, every returned image id is journaled before the
reply/note POST, and readback requires the exact conversation's body to keep
each link and `data-id` (the server drops `fileid`). Combining with `--attach`,
`--cc`, `--bcc` or `--set-status` is refused as uncaptured.

There is **no delete path** for inline images: removing an image in the editor
sends no request and the probed delete routes answer 404/500. A failed or
unknown transaction leaves orphan images; the error lists their ids.

## Problem and article deletion

| Command | Wire | Readback |
|---|---|---|
| `fsv problems delete ID` | `DELETE /api/_/problems/<n>` (trash) | exact GET `deleted: true` |
| `fsv problems restore ID` | `PUT /api/_/problems/<n>/restore` | exact GET `deleted: false` |
| `fsv problems delete-forever ID` | `DELETE /api/_/problems/delete-forever?ids=<n>` (trashed problems only) | exact GET 404 |
| `fsv solutions delete ARTICLE_ID` | `DELETE /api/_/solutions/articles/<id>` (trash) | `deleted: true` or 404 |
| `fsv solutions delete-forever ARTICLE_ID` | `DELETE /api/_/solutions/articles/<id>/delete_forever` | exact GET 404 |

All are bodyless JSON requests, reviewed with `--dry-run` (title, deleted flag,
effect), hash-bound to the exact record snapshot, single-attempt and journaled
(no replay; an identical re-review of the same record keeps its hash and is
refused). `delete-forever` and article trash also need
`--confirm-destructive`. There is no captured article restore.

`projects delete` is **blocked**: the project UI is an iframe to a separate
product host with its own session; the request
(`DELETE /projects/<id>` with `{"delete_project_name": "<exact name>"}`) cannot
be authenticated by fsv's agent session and the host is not published in the
tenant metadata fsv reads, so nothing is implemented rather than hardcoding a
host.

## Ticket templates, maintenance windows and notify entries

`fsv templates list` / `fsv templates show NAME_OR_ID` read
`GET /api/_/ticket-templates` and `GET /api/_/ticket-templates/<id>` (the new
ticket form's template picker; `meta.max_template_count` is the current count,
not a cap). `fsv tickets create --template NAME` only prefills, as the picker
does: subject, status, source, priority, urgency, impact, category fields,
scalar custom fields and the description become draft answers, `--from`,
`--preset` and `--set` win key by key, and everything still passes the form's
fields, business rules and the normal review/hash gate. Keys that are not
prefilled (tags, nested custom values, ...) are named on stderr. Template
create/delete are classic admin form posts and are not implemented.

`fsv changes windows list|create|delete` follow the captured window page:
`create` needs at least one `--criteria change_type=<label or id>` (the UI's
mandatory "Applicable to Changes" row; only this condition is captured), takes
`--start/--end 'YYYY-MM-DD HH:MM'` (an offset may be added) and sends
`DD-MM-YYYY` / `h:mmam` values. The wire has no offset and the tenant reads it
in the agent profile zone (`/api/v2/agents/me` `time_zone`), so fsv reads the
entered times in `--timezone ZONE` (else `FSV_TZ`, else that tenant zone),
converts them to the tenant zone before every overlap check and the send, and
the review shows both renderings (`start`/`end` `input` and `tenant`). An
unreadable tenant zone, an unknown `--timezone`, or a time that a
daylight-saving change skips or repeats refuses. The zones and instants are
bound into the plan hash; journals of the earlier hash format still guard.
The plain `--dry-run` already evaluates the window-list part of the guard
(the list is a read-only GET): the review and `--json` show
`overlapping_window_ids` (ids, `[]`, or `unknown` for unreadable times) and an
`overlap_warning` line when any overlap exists; the blackout overlap POST is
not sent while reviewing. After `--yes --expect-plan` it first re-reads the maintenance window list and
refuses when any listed window's time range intersects the new one (criteria
are not compared; unreadable window times also refuse), then posts the UI's
overlap check (`POST /api/_/change_windows/overlapping_windows`, read-only in
effect; the UI sends `type: "blackout_events"`, so it does not see other
maintenance windows) and refuses when it reports an overlap or its answer
cannot be read as 'no overlap'. `--allow-overlap` skips both refusals and the
result then says which windows overlapped (or `unknown`). The flag is part of
the reviewed plan (review key `allow_overlap`, bound into the plan hash), so it
must be given at both the dry-run and the `--yes` run, and its review says
`overlapping windows exist: <ids>; you chose to create it anyway
(--allow-overlap)` when the list shows overlaps; then one
`POST /api/_/maintenance_windows` with the `change_window` body, verified in
the window list (name, and start/end by instant when the list shows offsets). `delete` is `DELETE /api/_/maintenance_windows/<id>`, verified
by absence. Matching windows to a change is not implemented (no accepted
contract in this round).

Notes have no inline @mention; the editor's "Notify to" picker sends
`notify_emails: ["Name <email>"]`. `fsv tickets add-note --notify-agent NAME`
resolves one active agent by exact name or email through the agent autocomplete
and adds that entry (repeatable, combinable with `--notify-email`).

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

## Ticket-side associations

```sh
fsv tickets problems candidates INC-41 --search outage --json
fsv tickets problems link INC-41 PRB-7 --dry-run --json
fsv tickets problems unlink INC-41 PRB-7 --dry-run --json
fsv tickets changes candidates INC-41 --kind initiating --json
fsv tickets changes link INC-41 CHN-9 --kind initiating --dry-run --json
fsv tickets changes link INC-41 CHN-9 --kind initiated-by --dry-run --json
fsv tickets changes unlink INC-41 CHN-9 --kind initiated-by --dry-run --json
fsv tickets children candidates INC-41 --search printer --json
fsv tickets children link INC-41 INC-42 --dry-run --json
fsv tickets children unlink INC-41 INC-42 --dry-run --json
fsv tickets children list INC-41 --json
```

These are the wires the ticket page sends. `problems link|unlink` sends `PUT
tickets/<id>/problems/<problem>` with `{"type": "link"|"unlink"}` and is
limited to verified incidents. `changes link` sends `PUT
tickets/<id>/changes/associate` with `{"item_ids": [change], "change_type",
"type": "link"}`; `--kind initiating` is `change_cause` (the change caused
this ticket) and `--kind initiated-by` is `change` (this ticket initiated the
change); `unlink` sends `PUT tickets/<id>/changes/<change>` with `{"type":
"unlink", "change_type"}`. `children link` sends `POST
tickets/<parent>/associate-child-tickets {"ticket_ids": [child]}` (one child
per plan) and `children unlink` sends `DELETE
tickets/<parent>/remove_child?ticket_id=<child>`.

Each plan reads the exact ticket and the exact problem, change or child (404 is
a one-line refusal), and the complete associated list, and refuses when the
requested state already holds. Children also read the `related_tickets` tab
`hierarchy` of both tickets: a child cannot take children and a ticket that
already has a parent cannot be linked again. Readback uses the associated
lists (`GET tickets/<id>/problems`, `GET tickets/<id>/changes?change_type=`,
`GET tickets/<parent>/child-tickets?type=active` plus the child's
`hierarchy`), never the picker lists. `candidates` prints one picker page or
search result (read-only; search is token based and new records can take a
short time to be indexed; the child suggestion list is filtered locally).
Single attempt, full `--expect-plan` hash, journal and no resend as everywhere.

## Private ticket and change views

```sh
fsv tickets private-views list --json
fsv tickets private-views show 701 --json
fsv tickets private-views write 701 create --name 'Acme personal queue' --dry-run --json
fsv tickets private-views write 702 rename --name 'Acme renamed queue' --dry-run --json
fsv tickets private-views write 702 delete --dry-run --json
fsv changes private-views list --json
fsv changes private-views write 711 create --name 'Globex my changes' --dry-run --json
```

`create` clones the source view's conditions; `rename` (alias `edit`) renames
an exact ticket view. A view is treated as private only when a fresh exact read
shows it owned by the signed-in agent, an explicitly false default flag, no
system flag, an empty `group_ids` list, and visibility code 3. The codes come
from the web UI's "Save as" radios, identical for tickets and changes:
Myself = 3, All agents = 1, Agents in groups = 2. Exact reads carry only the
number; a label, when an account exposes one, must name a private visibility.
`show` prints the read plus whether fsv treats the view as private and why not.
`list` filters the list to your own non-default views (the list has no
visibility).

The workspace comes from the read: the exact view read has no top-level
`workspace_id`, so fsv takes the one workspace that the read's
`meta.workspace_id` and its workspace condition agree on, and refuses when
they disagree or are missing. The workspace condition is rewritten into the
form the UI sends (tickets: `value: [{"id": N}]`; changes: `value: N`), while
the read returns a string. Other conditions are cloned unchanged. Change views
use `query_hash: null` with `advanced_query_hash: {"all": [...]}`. Only
created-at descending views are supported.

Change views: only `create` (`POST /api/_/change-filters`) has a captured
wire; `rename` and `delete` are refused with "not captured" and nothing is
sent.

Use `--yes --expect-plan FULL_HASH` after each review. Source/target drift blocks
sending. Creation requires the exact returned view ID and reads that object,
never a same-name or latest match; the readback compares name, visibility,
workspace, order and the conditions (workspace condition compared by id).
Deletion requires both exact-target 404 (also HTML/empty responses) and absence
from a complete inventory. Ambiguous write outcomes keep the durable
single-attempt guard. Tests use synthetic native read shapes offline.

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
`tickets|sr|problems|changes assets candidates ID [--search TEXT] [--kind
devices|services] [--page N]` probes the record's association driver first.
Only a legacy-driver record reads the legacy `assets-to-associate` list; when
that list answers 403 (plan without the CMDB feature) or 404, and on every
integrated or problem record, candidates come from the integrated inventory
search (the web picker's read, as `assets search`). Rows carry inventory IDs
for `assets add` and an `associated` flag from the exact record's complete
association list (`null` when the record has no association list). A search
page is not a complete inventory; `complete` says so.
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


## Main-TUI send-path audit

Enabled reply, ticket-note, approval, task, planning-text and property sends use
shared CLI plans and durable single-attempt senders, not legacy service helpers.
`tests/test_tui_send_transport.py` is a headless Textual Pilot harness over real
`Client`/`httpx.MockTransport`: fifteen action paths, exact reviewed HTTP payloads,
no mutation requests on cancel (including the second public consent), read-only changes
after review, independent GETs, 403/429/500/disconnect, unavailable readback and
refused replay. It also exercises the actual full change property form. These
are synthetic offline transport proofs, not live mutation certification.

Audit fixes:

- Ticket/SR status uses the reviewed property builder. Change status uses the
  full form/rule builder and exact payload/hash review, not a legacy status PUT.
- Change property updates now bind tenant, full fresh record and metadata to the
  review. CLI and TUI use one journaled PUT plus exact-target field readback.
  Unresolved prior payload-only update hashes remain blocked. Unknown outcomes
  cannot be replayed; JSON exposes verification, and unavailable/mismatched
  readback is nonzero from the CLI. Combined property/files/planning form sends
  are refused; use separate reviewed operations instead of a legacy transaction.
- Change task update/delete use shared reviewed builders. `tasks-delete` now
  needs `--yes --expect-plan FULL_HASH` after dry-run; deletion verifies the
  complete remaining ID set, not just absence of the removed task. The native
  `PUT tasks/update` body is unchanged.
- Problem notes use the CLI's body-only reviewed note plan. The composer and
  review explicitly say visibility/notification semantics are not proven; they
  never promise private notes or invent a `private` field.
- Change-note sending is disabled in the main TUI: no shared reviewed CLI
  contract establishes its visibility semantics. Use the web form. Legacy
  generic TUI reply/note/status/task entrypoints fail closed, even if called
  directly. Other legacy CLI helpers remain compatibility paths, not certified
  reviewed operations.
- Verified task/note/form outcomes remain visible, rather than disappearing
  into a success toast. Ambiguous/unverified results have no retry action.
