# Freshservice wire contracts (agent web API)

**What this is.** A per-feature catalogue of the request shapes the
Freshservice agent web UI sends to its internal API, with method, path
template, payload keys, response shape, the readback that proves the effect,
and the pitfalls found. Every row says whether the contract was executed live
or only seen leaving the browser.

**Who it is for.** Developers adding a write or read to `fsv` (or another
client). Read [the overview](overview.md) first for the API families,
session and id model, and [forms and rules](forms-and-rules.md) before
writing anything that goes through a form.

**Verified on:** a Freshservice Enterprise trial tenant, 2026-11-11 to
2026-11-13, plus blocked (aborted, nothing delivered) captures and read-only
GETs on a production-like enterprise tenant. All ids, names and addresses
are invented; the path templates and key names are as observed.

**Confidence tags** (column *Proof*):

* **VERIFIED LIVE**: sent to a real tenant and independently read back.
* **CAPTURED**: request shape seen leaving the browser (usually with
  delivery aborted), not executed live in this exact form.
* **INFERRED**: reasoned from code, UI or neighbouring contracts.

**Conventions.** Paths are relative to `https://<tenant>.freshservice.com`.
`<id>` in a module path is the record's **display number** (`INC-42` → `42`)
unless stated. Writes to `/api/_/` carry `X-CSRF-Token` and
`Content-Type: application/json`, also when bodyless. Rich-text bodies are
HTML. Dates are ISO 8601 with the account's UTC offset unless stated.

A contract here is evidence about one tenant at one point in time, not a
published API. Re-verify on a new tenant (see
[live testing](../live-testing.md)).

## Contents

* [Create and update records](#create-and-update-records)
* [Resolve and close](#resolve-and-close)
* [Conversations: notes, replies, forwards](#conversations)
* [Attachments and inline images](#attachments)
* [Tickets: assign, bulk, merge, spam, delete](#ticket-actions)
* [Tasks](#tasks)
* [Time entries](#time-entries)
* [Watchers](#watchers)
* [Approvals](#approvals)
* [Associations: assets, CIs, services](#assets-and-cis)
* [Associations: records, releases, projects](#record-associations)
* [Problems: planning sections, solutions, closure](#problems)
* [Planning fields](#planning-fields)
* [Service requests](#service-requests)
* [Maintenance windows](#maintenance-windows)
* [Templates](#templates)
* [Filters](#filters)
* [Views, columns, export](#views-columns-export)
* [Deletes, trash, restore](#deletes)
* [Reads: activity, conversations, search](#reads)

## Create and update records

| Feature | Request | Payload keys | Response | Readback | Proof |
|---|---|---|---|---|---|
| Incident create | `POST /api/_/tickets` | `subject`, `description`, `status`, `priority`, `urgency`, `impact`, `source`, `custom_fields`, requester as `email: "Name <addr>"` or `requester_id`, optional `cc_emails` (plain addresses), `type`, `workspace_id`, `attachments` (ids), `device_ids` (ints), `br_validation_excludes` | 201 `{ticket: {...}}` | `GET /api/_/tickets/<id>` | VERIFIED LIVE |
| Ticket update (properties) | `PUT /api/_/tickets/<id>` | only the changed fields (`priority`, `urgency`, `impact`, `source`, `type`, `category` chain, `group_id`, `responder_id`, `tags`, `planned_start_date`, `planned_end_date`, `planned_effort`, `custom_fields`, ...) plus `br_validation_excludes` | 200 `{ticket, meta}` | exact GET | VERIFIED LIVE |
| Convert Incident ↔ Service Request | `PUT /api/_/tickets/<id>` | `{"type": "Service Request"}` / `{"type": "Incident"}` | 200 | `type` and `human_display_id` (prefix flips, number kept) | VERIFIED LIVE |
| SR beneficiary edit | `PUT /api/_/tickets/<id>` | `requester_id`, `requested_for_id` | not delivered | — | CAPTURED |
| Change create | `POST /api/_/changes` | `subject`, `description`, `status`, `priority`, `impact`, `risk`, `change_type`, `group_id`, `agent_id`, `requester_id` or `email`, `planned_start_date`, `planned_end_date`, `custom_fields`, `attachments`, `device_ids`, `br_validation_excludes` | 201 `{change: {...}}` (does not echo `device_ids`) | exact GET; CIs via `associated-cis` | VERIFIED LIVE |
| Change update | `PUT /api/_/changes/<id>` | changed fields only | 200 | exact GET | VERIFIED LIVE |
| Problem create | `POST /api/_/problems` | `subject`, `description`, `status`, `priority`, `impact`, `custom_fields`, `email`, `attachments` | 201 | exact GET (`requester_id`; email is not echoed) | VERIFIED LIVE |
| Problem update | `PUT /api/_/problems/<id>` | changed fields (`status`, `agent_id`, `known_error`, `priority`, due date, planned dates, `custom_fields`) | 200 | exact GET | VERIFIED LIVE |
| Release create | `POST /api/_/releases` | `subject`, `description`, `status`, `priority`, `release_type`, group, dates | 201 | exact GET | VERIFIED LIVE |

Pitfalls:

* Status writes in the web include the current `group_id` and
  `responder_id` and, on some forms, **every** custom field (the web sends
  the whole dirty `custom_fields` map, many as `null`). Sending explicit
  nulls is not permission to clear fields; send only what you mean.
  [CAPTURED / INFERRED]
* Cloning a change by posting its readback fails with 422 on read-only keys
  (for example a risk-survey status key). Strip read-only keys.
  [VERIFIED LIVE]
* `device_ids` must be JSON **integers**; string ids were rejected.
  [VERIFIED LIVE]
* A nested `device42_cmdb_requests_attributes` key found in the shipped web
  code was rejected (400 invalid field). Source code is not acceptance
  evidence. [VERIFIED LIVE]
* On incidents, `priority` alone may not stick (derived from urgency and
  impact). [VERIFIED LIVE]

## Resolve and close

| Step | Request | Payload | Response | Proof |
|---|---|---|---|---|
| Save resolution note | `PUT /api/_/tickets/<id>` | `{"resolution_notes": "<html>", "br_validation_excludes": "..."}` | 200 | VERIFIED LIVE |
| Transition | `PUT /api/_/tickets/<id>` | `status`, current `group_id`, `responder_id`, `br_validation_excludes`, closure-required `custom_fields` | 200 | VERIFIED LIVE |
| Reply or note and set status | `POST /api/_/tickets/<id>/reply` or `/notes` | normal body plus `helpdesk_ticket: {status, group_id, responder_id, br_validation_excludes}` | 201 `{conversation}` | VERIFIED LIVE |
| Reopen | `PUT /api/_/tickets/<id>` | `{"status": <open id>, ...}` | 200 | VERIFIED LIVE |

* The resolution save is its own request; the UI blocks Resolved/Closed
  without a persisted note on tenants that require one. A failed or aborted
  first stage means the second is never sent. [VERIFIED LIVE]
* Readback: internal `ticket.resolution_notes` is an object
  `{notes_html, created_at, updated_at, user}`; `/api/v2` returns plain
  text. [VERIFIED LIVE]

## Conversations

| Feature | Request | Payload keys | Response / readback | Proof |
|---|---|---|---|---|
| Private or public note (ticket) | `POST /api/_/tickets/<id>/notes` | `body`, `private` (true by default in the UI), `conversation_from_summary: false`, `notify_emails: ["Name <addr>"]`, `attachments` | 201 `{conversation}`; `GET /api/_/tickets/<id>/conversations` (`private`, `source` 2 for notes) | VERIFIED LIVE |
| Note edit | `PUT /api/_/conversations/<conversation id>` | `body`, `conversation_from_summary: false` | 200 `{conversation}`, same id | VERIFIED LIVE |
| Note delete | `DELETE /api/_/conversations/<conversation id>` | none | 204 | VERIFIED LIVE |
| Reply | `POST /api/_/tickets/<id>/reply` | `body`, `full_text` (same HTML), `from_email` (a configured support mailbox, `"Name" <addr>`), `freddy_message_id: null`, `conversation_from_summary: false`, `translate_reply: false`, `cc_emails: ["Name <addr>"]`, `bcc_emails`, `attachments` | 201 `{conversation}` with `to_emails`, `cc_emails`, `bcc_emails`, `from_email`, `attachments` (`source` 0) | VERIFIED LIVE (incl. BCC) |
| Forward whole ticket | `POST /api/_/tickets/<id>/forward` | `body`, `from_email`, `notify_emails: ["Name <addr>"]`, `private: true`, `conversation_from_summary: false` | 201; recipients read back as `to_emails` | VERIFIED LIVE |
| Forward one conversation | `POST /api/_/tickets/<id>/forward` | as above plus `conversation_id` | 201; the source link is not independently readable | VERIFIED LIVE |
| Problem / change note | `POST /api/_/problems/<id>/notes`, `POST /api/_/changes/<id>/notes` | `body`, `attachments` (the problem UI sends no `private` key; VERIFIED LIVE: a problem note carrying `private` is rejected HTTP 400 `private: Unexpected/invalid field`, so problem-note visibility is not controllable) | 201; `GET .../notes` | VERIFIED LIVE |
| Scenario (macro) | `PUT /api/_/tickets/execute_scenario` | `scenario_id`, `ids` | not delivered | CAPTURED |

Pitfalls:

* **No @mentions** in notes. "Notify to" is the `notify_emails` list,
  filled from the agent autocomplete. A note with a payload the UI never
  produces answered 500. [VERIFIED LIVE]
* CC formats differ: reply `cc_emails` uses `Name <addr>` entries, ticket
  create uses plain addresses. [CAPTURED / VERIFIED LIVE]
* The reply composer prepends a greeting and quoted thread
  (`freshdesk_quote` blocks); a client is free to send a plain body.
  [VERIFIED LIVE]
* `notify_emails` on a forward is not echoed; verify recipients through
  `to_emails`. [VERIFIED LIVE]
* Loading a ticket page autosaves `POST /api/_/tickets/<id>/reply_draft`.
  Block it during captures. [VERIFIED LIVE]

## Attachments

Two-step model: upload bytes, then reference the returned id in the owning
write.

| Step | Request | Payload | Response | Proof |
|---|---|---|---|---|
| Upload | `POST /api/_/attachments` | JSON, **not multipart**: `{"content": "data:<mime>;base64,<b64>", "content_file_name", "content_file_size", "content_content_type"}` (the UI adds `content_updated_at`) | 200 flat `{id, name, size, content_type, attachment_url, canonical_url, created_at, ...}` | VERIFIED LIVE |
| Attach on create | owner create body `attachments: [<id>, ...]` | — | owner's `attachments[]` | VERIFIED LIVE (incident, change, problem, notes, replies) |
| Attach to existing change | `PUT /api/_/changes/<id>` | `{"attachments": [<all ids>]}` (replaces the whole list) | 200 | VERIFIED LIVE |
| Planning-section file | `POST /api/_/changes/<id>/planning-fields?id=<section>` | `{description, attachments: [<id>]}` (description required) | 201 | VERIFIED LIVE |
| SR file field | place-request `custom_fields: {<file field>: [<id>]}` | — | the requested item's field value | VERIFIED LIVE |
| SR general attachments | place-request `attachments: [<id>]` | — | on the **requested item**, not the ticket | VERIFIED LIVE |
| Delete | `DELETE /api/_/attachments/<id>` | none | 204; then 404 | VERIFIED LIVE |
| Download | `GET <attachment_url>` (signed CDN URL, expires in about a day) | — | file | VERIFIED LIVE |

Pitfalls:

* An upload id can be used once; if the owning write fails the upload is an
  orphan (delete it explicitly). [VERIFIED LIVE]
* The 40 MB limit is enforced only by the UI. [VERIFIED LIVE]
* `GET /api/_/attachments/<id>` answers 405. Read attachments through the
  owner. [VERIFIED LIVE]
* Reply/note attachments live on `conversations[].attachments`, not on the
  ticket. SR general attachments live on the requested item
  (`GET /api/_/tickets/<id>/requested_items/<rid>?view=more_info`).
  [VERIFIED LIVE]
* Re-read the owner before downloading: `attachment_url` signatures expire,
  and they rotate on every read (see
  [quirks](quirks-and-pitfalls.md#signed-urls)). [VERIFIED LIVE]
* Change tasks have no attachment control. [VERIFIED LIVE]

### Inline images in rich text

| Step | Request | Payload | Response | Proof |
|---|---|---|---|---|
| Upload | `POST /api/_/solutions/inline_images` | `multipart/form-data`, one part `file` (jpeg, png, gif, webp) | 201 `{"link": "<signed inline URL with token>", "fileid": <id>}` | VERIFIED LIVE |
| Embed | the body HTML | `<img src="<link>" class="fr-fil fr-dib" fileid="<id>" data-id="<id>">` | the server drops `fileid`, keeps `data-id` | VERIFIED LIVE |

* Inline images do **not** go into `attachments`. [VERIFIED LIVE]
* There is no delete route for inline images (DELETE on the attachment route
  answers 500; removing the image in the editor sends nothing). Orphans
  remain. [VERIFIED LIVE]
* Sending before the upload finished embeds a local `blob:` URL; the server
  accepts the reply and drops the image. [VERIFIED LIVE]

## Ticket actions

| Feature | Request | Payload | Response | Proof |
|---|---|---|---|---|
| Pick up (single) | `PUT /api/_/tickets/<id>` | `{"responder_id": <self agent id>}` | 200 | VERIFIED LIVE |
| Pick up (bulk) | `PUT /api/_/tickets/pick_up` | `{"ids": [...]}` | 204 | VERIFIED LIVE |
| Problem pick up | `PUT /api/_/problems/pick-up` | `{"ids": [...]}` | not delivered | CAPTURED |
| Unassign agent | `PUT /api/_/tickets/<id>` | `{"responder_id": null}` | 200 | VERIFIED LIVE |
| Assign / clear group | `PUT /api/_/tickets/<id>` | `{"group_id": <id>}` / `{"group_id": null}` | 200 | VERIFIED LIVE |
| Bulk update | `PUT /api/_/bulk_ticket_actions/update_multiple` | `{status?, priority?, group_id?, responder_id?, ids: [...], reply: {}}` | 200 `{"success": true}` | VERIFIED LIVE |
| Merge precheck | `POST /api/_/merge_tickets/merge` | `{target: {ticket_id}, source: {ticket_ids}}` | 200, echo; **nothing merged** | VERIFIED LIVE |
| Merge | `POST /api/_/merge_tickets/complete_merge` | `{target: {ticket_id, note, is_private}, source: {ticket_ids, note, is_private}}` | 200 `{count, target, source}` | VERIFIED LIVE |
| Spam (single) | `PUT /api/_/tickets/<id>/spam` | `{}` | 204; `spam: true` | VERIFIED LIVE |
| Spam (bulk) | `POST /api/_/tickets/spam_multiple` | `{"ids": [...]}` | 200 `{meta, success: {ids}}` | VERIFIED LIVE |
| Not spam | `PUT /api/_/tickets/<id>/unspam` | none | 204 | VERIFIED LIVE |

Pitfalls:

* **Bulk `responder_id: null` is a silent no-op** (200 success, agent
  unchanged). Unassign one ticket at a time. [VERIFIED LIVE]
* Assigning an agent who is not in the group answers 400 ("Assigned agent
  isn't a member of the group"). Setting a group with `responder_id: null`
  clears the agent. [VERIFIED LIVE]
* Merge `note` and `is_private` are mandatory on both sides (400 without).
  Secondaries become Closed and their conversations move to the primary;
  the primary gets generated notes. There is no unmerge. Treat the precheck
  200 as nothing. [VERIFIED LIVE]
* Bulk results can lag: an immediate readback can show the old value and a
  later one the new value. Re-read before concluding. [VERIFIED LIVE]
* Ticket "split" is offered only on an incoming requester reply; an agent
  cannot reach it without email. No contract. [VERIFIED LIVE]

## Tasks

Same shape for tickets, problems and changes (`<module>` = `tickets`,
`problems`, `changes`).

| Feature | Request | Payload | Response | Proof |
|---|---|---|---|---|
| Create | `POST /api/_/<module>/<id>/tasks` | `title`, `description` (HTML), `status`, `notify_before` (seconds), `group_id`, `owner_id`, `planned_start_date`, `planned_end_date`, `planned_effort` (`"1h 30m"`), `due_date`, `custom_fields` | 201 `{task: {id, human_display_id: "TSK-n", ...}}` | VERIFIED LIVE |
| Update | `PUT /api/_/<module>/<id>/tasks/<task id>` | changed fields only (`owner_id: null` unassigns) | 200 `{task}` | VERIFIED LIVE |
| Delete | `PUT /api/_/<module>/<id>/tasks/update` | `{"id": "<task id as string>", "action": "delete"}` | 200 `{tasks: [...remaining]}` | VERIFIED LIVE |
| List / exact | `GET /api/_/<module>/<id>/tasks`, `GET /api/_/<module>/<id>/tasks/<task id>` | — | `{tasks: [...], meta: {has_next}}`, `{task}` | VERIFIED LIVE |

Pitfalls:

* Required fields come from task metadata (`field_options.required_for_create`),
  which can include tenant custom fields and planned dates. [VERIFIED LIVE]
* `notify_before` is in seconds (15 minutes = 900). The **collection read
  omits it**; only the exact task read returns it. [VERIFIED LIVE]
* The owner key is `owner_id` internally and `agent_id` in `/api/v2`.
  [VERIFIED LIVE]
* Effort is a duration string; compare semantically (`1h`, `1h 0m`, `60m`).
  [VERIFIED LIVE]
* Completing a task rewrites planned dates and sets `closed_at`.
  [VERIFIED LIVE]
* A browser GET of `/api/v2/.../tasks` right after an update can be served
  from cache; read with `cache: no-store`. [VERIFIED LIVE]

## Time entries

| Feature | Request | Payload | Response | Proof |
|---|---|---|---|---|
| Add (manual) | `POST /api/_/<module>/<id>/time_entries` | `executed_at`, `timer_running: false`, `billable`, `time_spent: "hh:mm"`, `note`, `agent_id` | 201 `{time_entry}` | VERIFIED LIVE (tickets, problems, changes) |
| Start timer | same path | same without `time_spent`, `timer_running: true` | not delivered | CAPTURED |
| Update | `PUT /api/_/<module>/<id>/time_entries/<entry id>` | changed fields | 200 | VERIFIED LIVE |
| Delete | `DELETE /api/_/<module>/<id>/time_entries/<entry id>` | none | 204 | VERIFIED LIVE |

`time_spent` comes back as `00:01:00` internally and `00:01` from
`/api/v2`; compare durations, not strings. [VERIFIED LIVE]

## Watchers

| Feature | Request | Payload | Proof |
|---|---|---|---|
| Ticket watch / unwatch | `POST /api/_/tickets/<id>/watch`, `PUT /api/_/tickets/<id>/unwatch` | `{"user_id": <id>}` | CAPTURED |
| Change watch / unwatch | `POST /api/_/changes/<id>/watchers` `{"user_id"}`; `DELETE /api/_/changes/<id>/unwatch?user_id=<id>` | as shown | VERIFIED LIVE |
| Problem watch | `POST /api/_/problems/<id>/watchers` | `{"user_id"}` | CAPTURED |
| Read | `GET /api/_/<module>/<id>/watchers` | → `{"watcher_ids": [...]}` | VERIFIED LIVE |

## Approvals

Two models exist; probe which one a tenant has (see
[overview](overview.md#6-plan-and-feature-flags-and-how-to-probe-them)).

### Parallel approvals (approval groups and chains)

| Feature | Request | Payload | Response | Proof |
|---|---|---|---|---|
| Request | `POST /api/_/<tickets\|changes>/<id>/approval-groups` | `{"approval_groups": [{"name", "approval_type", "email_content", "notified_to": [<user ids>]}]}` | 200 `{approval_chain: {id, approval_status, approval_groups: [{id, name, approvals: [...]}]}, meta: {triggerable_level}}` | VERIFIED LIVE |
| Read | `GET /api/_/<module>/<id>/approval-chains` | — | `{approval_chains: [...]}`; each approval has `id`, `member_id`, `user_id`, `status {id, name}`, `type`, `remark[]`, `last_reminded_at`, `delegatee` | VERIFIED LIVE |
| Approve / reject | `PUT /api/_/<module>/<id>/approvals/<approval id>` | `{"approval_status": 1, "remark": ""}` / `{"approval_status": 2, "remark": "<text>"}` | 200 `{approval_chain, meta}` | VERIFIED LIVE |
| Cancel a group | `PUT /api/_/<module>/<id>/approval-groups/<group id>/cancel` | none | 200 | VERIFIED LIVE |
| Remind | `PUT /api/_/<module>/<id>/approvals/<approval id>/resend` | none (JSON content type required; 415 without) | 200 `{approval}`; `last_reminded_at` set | VERIFIED LIVE (service requests) |

* `approval_type`: 1 Everyone, 2 Anyone, 3 Majority, 4 First responder.
  1 VERIFIED LIVE; 2-4 CAPTURED.
* Approval status ids: 0 requested, 1 approved, 2 rejected, 3 cancelled;
  a sibling under First responder shows `peer_responded`. Record-level
  aggregate `approval_status`: 0 requested, 1 approved, 3 cancelled,
  4 not requested (2 rejected INFERRED). [VERIFIED LIVE]
* **Recipient expansion is client-side.** Agent groups and CABs are only
  pickers: `GET /api/_/changes/<id>/groups/<group id>/members?page=1&per_page=30`
  returns `approval_users`; `GET /api/_/cabs` lists CABs and
  `GET /api/_/changes/<id>/cab-members?cab_id=<id>` returns
  `approval_agents`. The request carries only the flattened user ids, no
  group or CAB id; members start unchecked. [VERIFIED LIVE]
* The request response can contain older groups of the same chain; find the
  new group by exact name and recipients in the chain read.
  [VERIFIED LIVE]
* Approved or rejected approvals cannot be cancelled (400 "already approved
  and cannot be updated"); cancelling keeps history. Requesting and
  cancelling leaves the aggregate status at cancelled, not at "not
  requested". [VERIFIED LIVE]
* Reject needs a remark in the UI; approve accepts an empty one.
  [VERIFIED LIVE]
* `GET .../approval-groups` answers 405 (POST only). [VERIFIED LIVE]
* No delegate write was found. [VERIFIED LIVE: control absent]

### Legacy flat approvals

| Feature | Request | Payload | Proof |
|---|---|---|---|
| Change request (CAB) | `POST /api/_/changes/<id>/approvals` | `{"approval_type": 1, "notified_to": [<agent ids of CAB members>]}` | CAPTURED |
| Ticket / SR request | `POST /api/_/tickets/<id>/approvals` | `{"approval_type", "email_content": "<template HTML with {{placeholders}}>", "notified_to": [<user ids>]}` | CAPTURED |
| Template | `GET /api/_/approvals/approval_template` | — (a JSON string of HTML) | VERIFIED LIVE, read-only |
| Read | `GET /api/_/<module>/<id>/approvals` | → `{approvals: [{id, member_id, user_id, level_id, status, type, remark, last_reminded_at, member, user, ...}], meta: {triggerable_level, approval_type}}` | VERIFIED LIVE, read-only |
| Remind | `PUT /api/_/<module>/<id>/approvals/<approval id>/resend` | none | CAPTURED |

* No group name, message wrapper or CAB id; levels (`level_id`) are created
  by workflows. Match readbacks by `member_id`, level and status.
  [VERIFIED LIVE, read-only]
* No cancel control exists in the legacy UI; approve/reject were not
  captured. [VERIFIED LIVE, read-only: control absent]

## Assets and CIs

### Integrated ITAM inventory

| Feature | Request | Payload | Response | Proof |
|---|---|---|---|---|
| Candidate devices | `GET /itam/core/device/api/?order=&page=1&page_size=50&q=<text>` | — | `{page, error, total_rows, data: [{device_pk, name: {id, label, url}, ...}]}` | VERIFIED LIVE |
| Candidate services | `GET /itam/core/businessservice/api/?columns=<list>&page=1&page_size=50&q=<text>` | — | same envelope, `businessservice_pk` | VERIFIED LIVE |
| Add CIs | `PUT /api/_/<tickets\|problems\|changes>/<id>/associate-ci` | `{"device_ids": [<int>, ...]}` or `{"businessapp_ids": [<int>]}` | 200 `{}` | VERIFIED LIVE (bulk too) |
| Remove one | `PUT /api/_/<module>/<id>/disassociate-ci` | `{"itam_cmdb_request_id": "<link id from readback>"}` | 204 | VERIFIED LIVE |
| Read | `GET /api/_/<module>/<id>/associated-cis?page=1&association_type=cis\|services` | — | `{data: [{item_id, item_name, category, itam_cmdb_request_id, ...}], meta: {per_page, total_count, total_pages}}` | VERIFIED LIVE |
| At create | `device_ids` at the top level of the ticket/change create body | — | associations appear in the read | VERIFIED LIVE |
| Re-analyze impacted services | `GET /api/_/changes/<id>/refresh-impacted-services` | — | 202, empty | VERIFIED LIVE (a GET that **schedules work**) |
| Create inventory device | `POST /itam/core/device/api/add/?<r=...>` | `multipart/form-data` with a JSON `data` field | 200 `{data: {id, ...}, meta, ...}` | VERIFIED LIVE |

* Remove is one link per request, by the link id from the exact record's
  readback; never by item id. The UI "Remove" removes one row even when
  several are selected. [VERIFIED LIVE]
* Ids must be integers. A 200 on inventory add with an empty name and no id
  is not a creation. [VERIFIED LIVE]

### Legacy asset model

| Feature | Request | Payload | Proof |
|---|---|---|---|
| Candidates | `GET /api/_/assets-to-associate?entity=<change\|ticket>&per_page=30&page=1&user_id=<requester>&entity_id=<id>&include_meta=true` | → `{assets: [{id, display_id, name, ...}], meta}` | VERIFIED LIVE, read-only |
| Add to change | `PUT /api/_/changes/<id>/assets/associate` | `{"item_ids": [<asset display id>]}` | CAPTURED |
| Add to ticket | `PUT /api/_/tickets/<id>/associate_assets` | `{"item_ids": [<asset display id>]}` | CAPTURED |
| Read | `GET /api/_/changes/<id>/assets`, `GET /api/_/tickets/<id>/associated_assets` | → `{associated_assets: [{id, config_item: {...}}]}` | VERIFIED LIVE, read-only |
| Remove | not captured | — | — |

On a legacy tenant, `associated-cis` answers 404; on an integrated tenant,
`assets-to-associate` answers 403 `require_feature`. [VERIFIED LIVE]

## Record associations

| Feature | Request | Payload | Response / readback | Proof |
|---|---|---|---|---|
| Change ← caused incident | `PUT /api/_/changes/<id>/tickets/associate` | `{"item_ids": [<ticket>], "module_type": "ticket_cause"}` | 200; `GET /api/_/changes/<id>/tickets?filter=associated_tickets_caused` | VERIFIED LIVE |
| Unlink caused incident | `DELETE /api/_/changes/<id>/tickets/<ticket>?module_type=ticket_cause` | `{"type": "unlink"}` | 204 | VERIFIED LIVE |
| Change ↔ problem | `PUT /api/_/changes/<id>/problems/associate` | `{"item_ids": [<problem>], "module_type": "problem"}` | 200; `GET /api/_/changes/<id>/problems` | VERIFIED LIVE |
| Unlink problem | `DELETE /api/_/changes/<id>/problems/<problem>` | `{"type": "unlink"}` | 204 | VERIFIED LIVE |
| Ticket → initiating change | `PUT /api/_/tickets/<id>/changes/associate` | `{"item_ids": [<change>], "change_type": "change_cause", "type": "link"}` | 200; `GET /api/_/tickets/<id>/changes?change_type=change_cause` | VERIFIED LIVE |
| Unlink | `PUT /api/_/tickets/<id>/changes/<change>` | `{"type": "unlink", "change_type": "change_cause"}` | 200 | VERIFIED LIVE |
| Ticket ↔ problem | `PUT /api/_/tickets/<id>/problems/<problem>` | `{"type": "link"}` / `{"type": "unlink"}` | 200; `GET /api/_/tickets/<id>/problems` | VERIFIED LIVE |
| Problem ← incidents | `PUT /api/_/problems/<id>/tickets/associate` | `{"item_ids": [...]}` | not delivered | CAPTURED |
| Child tickets | `POST /api/_/tickets/<parent>/associate-child-tickets` | `{"ticket_ids": [...]}` | 200; `GET /api/_/tickets/<parent>/child-tickets?page=1&per_page=30&type=active` | VERIFIED LIVE |
| Parent ticket | `POST /api/_/tickets/<child>/associate-parent-ticket` | `{"ticket_id": <parent>}` | 200 | VERIFIED LIVE |
| Unlink child / parent | `DELETE /api/_/tickets/<parent>/remove_child?ticket_id=<child>`, `DELETE /api/_/tickets/<child>/remove_parent?ticket_id=<parent>` | none | 204 (no confirm) | VERIFIED LIVE |
| Change ↔ release | `PUT /api/_/changes/<id>/releases/<release>` | `{"type": "link"}` / `{"type": "unlink"}` | 200 / 204; `GET /api/_/changes/<id>/releases` | VERIFIED LIVE |
| Project create and link | `POST /api/_/freshrelease/settings/create_fr_project` | `{"project": {name, description, end_date, owner_email, project_category, public_workspace, ...}}` | outer `{status, body}`; `GET /api/_/freshrelease/settings/fetch_linked_fr_projects?placeholder[resourceId]=<id>&placeholder[resourceType]=Change` | VERIFIED LIVE |
| Link existing project task | `POST /api/_/freshrelease/settings/create_issue` | `{issue: {id: "<KEY-n>"}, placeholder: {projectId: "<KEY>"}, remote_resource_id, remote_resource_type, ...}` | `linked_issues` read | VERIFIED LIVE |
| Unlink task / project | `POST /api/_/freshrelease/settings/unlink_issue`, `/unlink_fr_project` | issue id or project key and id | 200 | VERIFIED LIVE |

* The candidate pickers (`filter=list&module_type=...`) are not
  association reads; do not verify with them. [VERIFIED LIVE]
* `child-tickets` answers **400 when there are none**; the changes read
  needs `change_type` (400 without). [VERIFIED LIVE]
* Freshrelease proxy answers wrap the real status in `{status, body}`;
  check the nested body and the linked inventory. [VERIFIED LIVE]

## Problems

| Feature | Request | Payload | Response | Proof |
|---|---|---|---|---|
| Article for a solution | `POST /api/_/solutions/articles` | `{title, description, status: 1 (draft), article_type: 1 permanent \| 2 workaround, folder_id: <int>, source: "problem"}` | 201 `{article}` | VERIFIED LIVE |
| Link / unlink solution | `PUT /api/_/problems/<id>/solutions/<article id>` | `{"type": "link"}` / `{"type": "unlink"}` | 200; `GET /api/_/problems/<id>/solutions` → `{articles: [...]}` | VERIFIED LIVE |
| Known error | `PUT /api/_/problems/<id>` | `{"known_error": true}` | 200 | VERIFIED LIVE (mark) |
| Close / reopen | `PUT /api/_/problems/<id>` | `{"status": <closed id>}` / `{"status": <open id>}` | 200, or 400 `violated_closure_rules` | VERIFIED LIVE |

* The UI creates the article first, then links it in a second request.
  Workaround vs permanent is the article's `article_type`; the link body is
  the same. [VERIFIED LIVE]
* Folders: `GET /api/_/solutions/categories?per_page=300`,
  `GET /api/_/solutions/folders?category_id=<id>&per_page=300`; the default
  Drafts folder may sit in another workspace than the problem, and the link
  still works. [VERIFIED LIVE]
* `folder_id` must be an integer (a string was rejected). [VERIFIED LIVE]

## Planning fields

For changes and problems (`<module>` = `changes`, `problems`); the section
key is the semantic name (problem sections `problem_cause`,
`problem_impact`, `problem_symptom`; change sections are default or `cfp_`
custom names), never a numeric row id.

| Feature | Request | Payload | Response | Proof |
|---|---|---|---|---|
| First write | `POST /api/_/<module>/<id>/planning-fields?id=<section>` | `{"description": "<html>", "attachments"?: [<id>]}` | 201 | VERIFIED LIVE |
| Edit existing | `PUT /api/_/<module>/<id>/planning-fields/<section>` | `{"description": "<html>"}` | 200 `{change_planning_field \| problem_planning_field: {id, name, description, description_text, attachments}}` | VERIFIED LIVE |
| Clear | `DELETE /api/_/<module>/<id>/planning-fields/<section>` | none | 204 | VERIFIED LIVE |
| Read | `GET /api/_/<module>/<id>/planning-fields` | — | `{change_planning_fields \| problem_planning_fields: [...]}` | VERIFIED LIVE |

* Use POST only for an empty section and PUT for a populated one; a second
  POST is not a replacement. A cleared section may remain as an empty
  placeholder row. [VERIFIED LIVE]
* Planning sections can be locked per change state. [VERIFIED LIVE]

## Service requests

| Feature | Request | Payload | Response | Proof |
|---|---|---|---|---|
| Catalog | `GET /api/_/catalog/service-categories?category_hierarchy=true`, `GET /api/_/catalog/service-items?page=N&per_page=30` | — | lists | VERIFIED LIVE |
| Item form | `GET /api/_/catalog/service-items/<display id>` | — | `{service_item}` | VERIFIED LIVE |
| Place request | `POST /api/_/catalog/service-items/<display id>/place-request` | `email`, `requested_for` (an **address**, not an id), `cc_emails`, `custom_fields` (item field names), `attachments`, `br_validation_excludes` | 200 | VERIFIED LIVE |
| Requested items | `GET /api/_/tickets/<id>/requested_items` | — | list | VERIFIED LIVE |
| Edit answers | `PUT /api/_/tickets/<id>/requested_items/<row id>` | `{"custom_fields": {<changed only>}}` | 200 `{requested_item}` | VERIFIED LIVE |
| Fulfilment stage | `PATCH /api/_/tickets/<id>/requested_items/<row id>/update_stage` | `{"stage": <n>}` | 200 `{requested_item: {stage: {id, name}}}` | VERIFIED LIVE |
| Add item | `POST /api/_/tickets/<id>/requested_items` | `{"item_id": "<catalog display id as string>", "custom_fields": {...}}` | 200 | VERIFIED LIVE |
| Remove item | `DELETE /api/_/tickets/<id>/requested_items/<row id>` | none | 204 (not the parent item) | VERIFIED LIVE |

* The SR detail payload names the kind `type` (not `ticket_type`).
  [VERIFIED LIVE]
* Catalog items with lookup field types were not submitted. [INFERRED]

## Maintenance windows

| Feature | Request | Payload | Response | Proof |
|---|---|---|---|---|
| Overlap check | `POST /api/_/change_windows/overlapping_windows` | `{type: "blackout_events", start_date: "DD-MM-YYYY", start_time: "h:mmam", end_date, end_time, criteria: [{name: "change_type", operator: "is", value: "1"}], workspace_id}` | 200 `{"match_found": false}` (no conflict; `true` = conflict) | VERIFIED LIVE |
| Create | `POST /api/_/maintenance_windows` | `{"change_window": {name, description, start_date, start_time, end_date, end_time, change_event: {additional_info: {}, criteria: [...]}, workspace_id, alert_suppression}}` | 201 `{maintenance_window: {id, ...}}` | VERIFIED LIVE |
| List | `GET /api/_/maintenance_windows` | — | `{maintenance_windows: [...]}` | VERIFIED LIVE |
| Delete | `DELETE /api/_/maintenance_windows/<id>` | none | 200 `{success: true}` | VERIFIED LIVE |
| Windows matching a change | `POST /api/_/change_windows/matching_windows` | `{itil_change: {id: <internal change id>}, type: "maintenance_events", source: "ember"}` | 200 | VERIFIED LIVE (read-like POST) |
| Associate a window to a change | not captured | — | — | — |

The UI's overlap check sends `type: "blackout_events"` even when the form
creates a maintenance window, and it answered `{"match_found": false}` while
the new maintenance window overlapped an existing maintenance window with the
same change-type condition (both were created). Treat it as a blackout-window
check only; detect maintenance-window overlaps from the window list's
`start_time`/`end_time` (a read-only GET, so fsv also evaluates it during a
plain dry-run). Whether a `maintenance_events` type is accepted here
was not captured. [VERIFIED LIVE / INFERRED for the blackout-only scope]

Time zone. The create and overlap bodies carry wall-clock dates and times
with no offset, and the tenant stores them in the signed-in agent's profile
zone: the list then answers `start_time`/`end_time` as ISO 8601 with that
zone's offset (e.g. `2030-11-14T03:00:00-05:00`). A wall clock meant for
another zone and sent unchanged lands hours away. fsv reads the tenant zone
from `GET /api/v2/agents/me` `time_zone` (Rails or IANA name), converts the
user's times (`--timezone`, else `FSV_TZ`, else the tenant zone) to it before
the overlap checks and the send, refuses when that zone is unreadable, and
refuses wall-clock times a daylight-saving change makes nonexistent or
repeated. [VERIFIED LIVE for the storing zone and the offset form in the list;
INFERRED that the agent profile zone, not a separate account zone, is the one
applied: both were equal on the tested tenant]

The window page takes several seconds to render its form, and the UI
refuses an empty criteria row; the API accepts `criteria: []`. Some
`change_windows` GET variants answer 500 even on a working tenant.
[VERIFIED LIVE]

## Templates

| Feature | Request | Payload | Response | Proof |
|---|---|---|---|---|
| List | `GET /api/_/ticket-templates`, `GET /api/_/change-templates` | — | `{ticket_templates: [{id, name, type, workspace_id}], meta: {max_template_count}}` | VERIFIED LIVE |
| Detail (apply in the new-ticket form) | `GET /api/_/ticket-templates/<id>` | — | `{ticket_template: {id, name, template_data: {subject, status, source, priority, urgency, impact, description, custom_fields, ...}}}` | VERIFIED LIVE |
| Create | `POST /ws/<ws>/admin/ticket_templates` | `multipart/form-data`: `itil_form_template[name]`, `itil_form_template[description]`, `itil_form_template[accessible_attributes][access_type]`, `template_data[subject]`, `template_data[priority\|urgency\|impact\|status\|source]` | 302 to the template list | VERIFIED LIVE |
| Delete | `DELETE /ws/<ws>/admin/ticket_templates/<id>` | none | **404 JSON, but the template is gone** | VERIFIED LIVE |

Applying a template is client-side prefill; nothing is written until the
ticket is submitted. [VERIFIED LIVE]

## Filters

Lists are GETs. Two encodings [VERIFIED LIVE]:

* **Basic panel** (tickets, problems, changes):
  `query_hash=<JSON array>` (and an empty `advanced_query_hash=` on tickets
  and changes).
* **Advanced builder** (tickets, changes; problems have none):
  `advanced_query_hash=<JSON {"all": [...], "any": [...]}>&query_hash=`.
  The UI adds a workspace condition to `all`: tickets
  `{"condition": "workspace_id", "operator": "is_in", "value": [{"id": <ws>}], "type": "default"}`,
  changes `{"condition": "workspace_id", "operator": "is", "value": <ws>, "type": "default"}`.
  Match-any puts user conditions in `any` and keeps the workspace in `all`.

Condition: `{"condition": <field>, "operator": <op>, "value": <v>, "type": "default" | "custom_field"}`.

| Field type | Operators (UI) | Wire value |
|---|---|---|
| Default dropdowns (status, priority, impact, urgency, source, group, agent, requester, department) | includes / excludes → `is_in` / `is_not_in` | arrays; ticket status ids as strings `["2"]`, priority/impact as ints on tickets but strings on problems; `"-1"` unassigned, `"0"` me / my groups |
| Ticket type | `is_in` | the type **name** (`["Service Request"]`); a numeric id is ignored and the list is unfiltered |
| Category (default) | `is` / `is_in` | label |
| `custom_dropdown` | `is_in` / `is_not_in` | labels; `"-1"` = empty |
| `custom_multi_select_dropdown` | `is_in` / `is_not_in` | **integer choice ids** |
| `nested_field` | `is` / `is_in` / `is_not_in` | labels |
| `custom_number` | `is`, `is_not`, `is_greater_than_or_equal`, `is_less_than_or_equal` | value as a **string** |
| Dates (created, due by (`due_by_date`), closed, planned, custom dates) | `on` + `today`/`yesterday`/`tomorrow`/`week`/`month`/`year`; `in_the_last` / `in_the_next` + `five_minutes` ... `one_week`, `one_month`, `two_months`, `six_months`, `one_year`; between → `is_in_the_range` with `{"from", "to"}` | see below |
| Change approval status | `is_in` | `["0"]` requested, `["1"]` approved, ... |
| Problem known error | `is` | `"1"` |
| Tags | includes / excludes | tag names |

Date semantics measured live [VERIFIED LIVE]:

* `is_in_the_range` (and the basic panel's `is_greater_than` with a
  `{from, to}` object) is a **whole-day, both-ends-inclusive** range. Plain
  `YYYY-MM-DD` and ISO with offset are accepted.
* **Only `from` or only `to` → HTTP 500.** For `>= D` send a far-future
  `to`; for `<= D` an early `from`.
* There is no strict `>` / `<`; shift by a day (`> D` = from `D+1`).
* `on` with a literal date is ignored (unfiltered rows); `is_less_than` with
  a value returns nothing; an unknown operator (for example `custom_date`)
  answers **206 with `meta.search_error: true` and unfiltered rows**
  (problems answered 200 with zero rows instead).
* The basic panel's "created within" preset sends `is_greater_than` with a
  minutes string (`"240"`) or a token (`"today"`).

Other filter facts:

* **Text fields cannot be filtered** (no subject, description, text or
  paragraph conditions in either builder). Use global search
  (`GET /search/all?term=...`) or filter client-side. [VERIFIED LIVE]
* Ticket `approval_status` is a column but not a filter (206
  `search_error`); changes accept it. [VERIFIED LIVE, read-only]
* No `updated_at` control was seen in the problem filter panel; `fsv`
  refuses it for problems. [INFERRED]
* A filtered list is not proof of the predicate: one subject condition
  returned unrelated records. Before any write based on a filtered list,
  check each returned row satisfies the predicate. [VERIFIED LIVE]

## Views, columns, export

| Feature | Request | Payload | Proof |
|---|---|---|---|
| View definitions | `GET /api/_/ticket_filters`, `GET /api/_/ticket_filters/<id>`, `GET /api/_/change-filters` | → `{ticket_filter: {query_hash, advanced, ...}}` | VERIFIED LIVE |
| Private view create | `POST /api/_/ticket_filters` → 201 | `{name, workspace_id, visibility, query_hash: [..., {condition: "workspace_id", operator: "is", value: [{id: N}], type: "default"}], advanced_query_hash: null, order_by: "created_at", order_type: "desc"}` | VERIFIED LIVE |
| Private view edit / delete | `PUT /api/_/ticket_filters/<id>`, `DELETE /api/_/ticket_filters/<id>` | same body / none | VERIFIED LIVE |
| Visibility codes | "Save as" dialog radios (tickets and changes) | Myself = 3, All agents = 1, Agents in groups = 2. Exact reads return only the number (no label); the list rows carry only `{id, name, user_id, default, advanced}` plus `meta.editable_filter_ids` | CAPTURED |
| Private view read | `GET /api/_/ticket_filters/<id>` | `{id, name, default: false, visibility: 3, user_id, group_id: null, group_ids: [], query_hash, advanced_query_hash, order_by, order_type, meta: {workspace_id: [{id, name}]}}`: **no top-level `workspace_id`**, and the workspace condition reads back as a string (`value: "N"`), not the write form | CAPTURED |
| Change private view create | `POST /api/_/change-filters` → 200 | `{name, workspace_id, visibility: 3, query_hash: null, advanced_query_hash: {all: [..., {condition: "workspace_id", operator: "is", value: N, type: "default"}]}, order_by, order_type}`; read `GET /api/_/change-filters/<id>` → `{change_filter: {..., visibility, user_id, group_ids: [], default: false}}`. Rename/delete wires for change views are not captured | CAPTURED |
| Column selection | `POST /api/_/<tickets\|changes\|problems>/headers` | `{"columns": {<key>: <position>}, "frozen_columns": [...]}` | CAPTURED |
| Export | `POST /api/_/<module>/export` | `{format: "csv", date_filter: 30, export_fields: {<key>: <label>}, ...}` | CAPTURED (job and delivery, possibly by email, unknown) |

## Deletes

| Record | Soft delete | Restore | Permanent | Proof |
|---|---|---|---|---|
| Ticket | `DELETE /api/_/tickets/<id>` (204, `deleted: true`, still readable); bulk `DELETE /api/_/tickets/delete_multiple?ids=1,2` | `PUT /api/_/tickets/<id>/restore` (204) | `DELETE /api/_/tickets/delete_forever?ids=<id>` (204, **no confirm**; then 404 on internal and v2) | VERIFIED LIVE |
| Problem | `DELETE /api/_/problems/<id>` (204; `deleted: true`; listed under `?filter=deleted`) | `PUT /api/_/problems/<id>/restore` | `DELETE /api/_/problems/delete-forever?ids=<id>` (only from trash) | VERIFIED LIVE |
| Change | `DELETE /api/_/changes/<id>` (204) | — | — | VERIFIED LIVE (readback was `deleted: true` once and 404 once) |
| Release | `DELETE /api/_/releases/<id>` (204) | — | — | VERIFIED LIVE |
| KB article | `DELETE /api/_/solutions/articles/<id>` (trash; GET then shows `deleted: true`) | — | `DELETE /api/_/solutions/articles/<id>/delete_forever` (then 404) | VERIFIED LIVE |
| Project (Freshrelease host) | `DELETE https://<tenant>.freshrelease.com/projects/<id>` with `{"delete_project_name": "<exact name>"}` | — | — | VERIFIED LIVE |

* Ids in bulk deletes go in the **query string**, not a JSON body.
  [VERIFIED LIVE]
* Updates to a trashed record answer **405**. [VERIFIED LIVE]
* Spam and Trash are system views (`/a/tickets/view/spam`,
  `/a/tickets/view/deleted`). [VERIFIED LIVE]

## Reads

| Read | Request | Notes | Proof |
|---|---|---|---|
| Record | `GET /api/_/<module>/<id>` (optional `include=requester,stats`) | `deleted`, `spam`, `type`, `human_display_id`, `state_flow_id`, `state_traversal`, `approval_status` | VERIFIED LIVE |
| Tabs | `GET /api/_/<module>/<id>/tabs` | tab names and counts | VERIFIED LIVE |
| Conversations | `GET /api/_/tickets/<id>/conversations?per_page=50` | notes `source` 2, replies `source` 0 | VERIFIED LIVE |
| Activity | `GET /api/_/<module>/<id>/activities` | continue with `?start_token=<token>` on the **same internal path**; the `next_page_url` it returns points at `/api/v2`, which answers 404 for a session | VERIFIED LIVE |
| Bootstrap | `GET /api/_/bootstrap/me`, `/bootstrap/account`, `/bootstrap/agents_groups` | identity, privileges, features, agents and groups with member ids | VERIFIED LIVE |
| Self | `GET /api/v2/agents/me` | agent id, email, groups, workspaces | VERIFIED LIVE |
| Global search | `GET /search/all?term=<text>`, `GET /search/<entity>?term=` | the only text search | VERIFIED LIVE |
| Agent autocomplete | `GET /api/_/search/autocomplete/agents?q=<text>&qf=name` | match on exact email | VERIFIED LIVE |
| Requester autocomplete | `GET /api/_/search/autocomplete/requesters?q=<text>&qf=details` | user ids | VERIFIED LIVE |
