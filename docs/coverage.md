# Command coverage

How much of fsv has been proven against a real Freshservice tenant, per
command. Rows were verified on a Freshservice Enterprise trial tenant,
2026-11; production-like behaviour was checked with read-only requests and
aborted UI captures only. Nothing here is a guarantee for another tenant:
plans, roles, forms and workflows differ (see the
[Freshservice knowledge base](freshservice/overview.md)).

## Summary

| Area | Rows | COVERED | COVERED (read) | UNSURE | GAP | N/A | Overall covered |
|---|---|---|---|---|---|---|---|
| Tickets | 93 | 77 | 11 | 2 | 2 | 1 | 88 |
| Changes | 56 | 41 | 7 | 3 | 4 | 1 | 48 |
| Problems and service requests | 63 | 51 | 11 | 1 | 0 | 0 | 62 |
| Cross-cutting | 80 | 55 | 11 | 12 | 2 | 0 | 66 |
| **All** | **292** | **224** | **40** | **18** | **8** | **2** | **264** |

**Overall covered** is COVERED plus COVERED (read). N/A rows are local
commands with no wire. Counts are per row: a few commands
appear both in their module table and in the cross-cutting table.

## Legend

* **COVERED** needs all three: a live send (or, for reads, a live read) on
  the trial tenant, an **independent readback** (a separate GET of the
  record or collection, never the write's own response echo), and offline
  tests that pin the wire and the refusals.
* **COVERED (read)**: a read-only command (nothing to send) whose output was
  verified live against real data on the trial tenant, with offline tests;
  there is no write, so no readback applies.
* **UNSURE**: implemented and tested offline, but the live proof is partial,
  read-only, stale after a later fix, or blocked by missing fixtures.
* **GAP**: not implemented, or no accepted wire exists for it.
* **Live-verified**: `yes` = live send; `yes (read)` = live read;
  `no (dry-run only)` = only the reviewed dry-run ran live; `stale (re-test)` =
  the live run predates a later fix; `partial` = some paths failed.
* **Readback independent**: whether the live proof re-read the result through
  a different request than the write (`n/a (read)` for reads, `n/a` for local
  commands).
* **Production parity**: `read-verified` = the same read wires answered on a
  production-like tenant (read-only); `capture-only` = the write wire was
  captured from the web UI with the submit aborted; `none` = not checked.
* **Wire**: method and path template; `{id}` is a record id, `<tenant>` your
  Freshservice host. `/api/_/` is the agent web UI's internal API.
* **Offline tests**: files under `tests/` that exercise the row.

## Known gaps

Still UNSURE, and why:

* exports (tickets, changes, problems): review-only; delivery not verified
  (the server emails the file to the requesting agent, which fsv cannot read);
* change associations pick: interactive picker, not run by automated tests
  (it sends the same wire as `associations add`);
* change project relationships: adding a project creates a permanent project,
  which cannot be deleted from a test tenant;
* auth login and logout: not exercised live (they would replace or destroy
  the shared test session);
* schema freshness check: proving it needs a form-field edit in the web UI;
* planning-section attachment delete: needs an attachment uploaded through
  the web UI;
* asset search paging: the test tenant has only one page of devices;
* filtered bulk select-all, text-filter refusals, list type filters by name
  and raw query hashes: fixed or guarded after the last live run; re-test
  pending;
* readback verification as a whole: several verify callbacks were fixed after
  the last live run; re-test pending.

Needs seeding on the test tenant before a live proof is possible:

* attachment delete on a change and on a planning section (needs an
  attachment uploaded through a complete change create or the web UI);
* asset search paging (needs more than one page of inventory).

Blocked (no accepted wire or not reachable from a test tenant):

* project delete (it runs on a separate project-management host);
* legacy approvals respond/cancel (the legacy approval model is not available
  on the trial; only aborted captures exist);
* approval delegation (no delegate control in the captured approval UI);
* ticket split;
* share a ticket with another agent (share add-user);
* bulk close: a successful close needs a saved resolution note first;
* change private-view rename and delete (wires not captured; fsv refuses);
* the TUI (`fsv tui`) is not covered by this matrix.

## Tickets

| Command | Wire | Status | Live-verified | Readback independent | Production parity | Offline tests | Note |
|---|---|---|---|---|---|---|---|
| tickets ls (+--where/--view/--order-by/--all) | `GET /api/_/tickets?filter=... (list; filter wires in query.py)` | COVERED (read) | yes (read) | n/a (read) | read-verified | test_filter_wires.py, test_query_matrix.py, test_query_edge_cases.py, test_query_hash_params.py, test_ticket_type_filter.py, test_views_cli.py, test_strict_filter_contract.py | Read-only: verified live against real data; nothing to send. |
| tickets get (--internal/--all-tabs) | `GET /api/v2/tickets/{id} + GET /api/_/tickets/{id}` | COVERED (read) | yes (read) | n/a (read) | read-verified | test_ticket_details.py | Read-only: verified live against real data; nothing to send. |
| tickets activity | `GET internal ticket activity feed (start_token continuation; views.py)` | COVERED (read) | yes (read) | n/a (read) | read-verified | test_ticket_lifecycle.py, test_views_cli.py | Read-only: verified live against real data; nothing to send. |
| tickets conversations | `GET /api/_/tickets/{id}/conversations` | COVERED | yes (read) | n/a (read) | none | test_ticket_conversation_http.py, test_ticket_conversation.py | Live read verified. |
| tickets fields | `GET /api/_/ticket_form_fields (cached schema)` | COVERED (read) | yes (read) | n/a (read) | read-verified | test_ticket_properties.py | Read-only: verified live against real data; nothing to send. |
| tickets lookup (sender/users/groups/choices) | `GET lookup endpoints via int_get` | COVERED (read) | yes (read) | n/a (read) | none | test_templates_windows.py, test_ticket_conversation_http.py | Read-only: verified live against real data; nothing to send. |
| tickets search | `GET /api/_/search/... (global/keyword)` | COVERED (read) | yes (read) | n/a (read) | none | test_list_filter_surface.py | Read-only: verified live against real data; nothing to send. |
| tickets url | local only (prints browser URL) | N/A | n/a (local) | n/a | none | — | Local only; no wire. |
| tickets create (core INC) | `POST /api/_/tickets` | COVERED | yes | yes | read-verified | test_ticket_create.py, test_ticket_create_cc.py | Live send verified by an independent readback. |
| tickets create --attach FILE | `POST /api/_/attachments then POST /api/_/tickets` | COVERED | yes | yes | none | test_attachments_reviewed.py, test_attachment_capture_hardening.py | Live send verified by an independent readback. |
| tickets create --device ID | `POST /api/_/tickets {ci_ids/device_ids}` | COVERED | yes | yes | none | test_ticket_create.py | Live send verified by an independent readback. |
| tickets create --cc | `POST /api/_/tickets {cc_emails}` | COVERED | yes | yes | none | test_ticket_create_cc.py | Live send verified by an independent readback. |
| tickets create --template NAME (prefill) | `GET /api/_/ticket-templates[/{id}] then POST /api/_/tickets` | COVERED | yes | yes | read-verified | test_templates_windows.py | Live send verified by an independent readback. |
| tickets update (--reviewed --set/--priority/--tag/--planned-*) | `PUT /api/_/tickets/{id}` | COVERED | yes | yes | none | test_ticket_properties.py, test_ticket_properties_http.py, test_ticket_update_custom.py, test_ticket_update_identity.py | Live send verified by an independent readback. |
| tickets resolve (INC) | `PUT /api/_/tickets/{id} + PUT resolution_notes` | COVERED | yes | yes | read-verified | test_ticket_lifecycle.py, test_ticket_closure_conversation.py | Live send verified by an independent readback. |
| tickets resolve (SR with approval chain) | `PUT /api/_/tickets/{id}` | COVERED | yes | yes | none | test_ticket_lifecycle.py | Live send verified by an independent readback. |
| tickets close | `PUT /api/_/tickets/{id} (status 5)` | COVERED | yes | yes | none | test_ticket_lifecycle.py, test_ticket_closure_conversation.py | Live send verified by an independent readback. |
| tickets reopen | `PUT /api/_/tickets/{id} (status Open)` | COVERED | yes | yes | none | test_ticket_lifecycle.py | Live send verified by an independent readback. |
| tickets status ID STATUS (--set/--ack) | `PUT /api/_/tickets/{id}` | COVERED | yes | yes | none | test_ticket_lifecycle.py | Live send verified by an independent readback. |
| tickets reply (plain) | `POST /api/_/tickets/{id}/reply` | COVERED | yes | yes | none | test_ticket_conversation.py, test_ticket_conversation_http.py, test_conversation_recipient_readback.py | Live send verified by an independent readback. |
| tickets reply --bcc/--cc/--sender | `POST /api/_/tickets/{id}/reply` | COVERED | yes | yes | none | test_ticket_conversation.py, test_conversation_recipient_readback.py | Live send verified by an independent readback. |
| tickets reply --set-status Resolved\|Closed | `POST /api/_/tickets/{id}/reply + status PUT` | COVERED | yes | yes | none | test_ticket_closure_conversation.py | Live send verified by an independent readback. |
| tickets reply --attach | `POST /api/_/attachments + POST .../reply` | COVERED | yes | yes | none | test_attachments_reviewed.py | Live send verified by an independent readback. |
| tickets reply --inline-image | `POST /api/_/solutions/inline_images + POST .../reply` | COVERED | yes | yes | none | test_inline_images.py | Live send verified by an independent readback. |
| tickets add-note (private) | `POST /api/_/tickets/{id}/notes` | COVERED | yes | yes | none | test_ticket_notes.py, test_tui_ticket_notes.py | Live send verified by an independent readback. |
| tickets add-note --public --confirm-public | `POST /api/_/tickets/{id}/notes` | COVERED | yes | yes | none | test_ticket_notes.py | Live send verified by an independent readback. |
| tickets add-note --set-status / --notify-email / --notify-agent | `POST /api/_/tickets/{id}/notes` | COVERED | yes | yes | none | test_ticket_notes.py | Live send verified by an independent readback. |
| tickets add-note --inline-image | `POST /api/_/solutions/inline_images + POST .../notes` | COVERED | yes | yes | none | test_inline_images.py | Live send verified by an independent readback. |
| tickets note-edit | `PUT /api/_/conversations/{note_id}` | COVERED | yes | yes | none | test_ticket_note_mutation.py | Live send verified by an independent readback. |
| tickets note-delete | `DELETE /api/_/conversations/{note_id}` | COVERED | yes | yes | none | test_ticket_note_mutation.py | Live send verified by an independent readback. |
| tickets forward (whole ticket) | `POST /api/_/tickets/{id}/forward` | COVERED | yes | yes | none | test_ticket_conversation.py, test_conversation_forward_source.py | Live send verified by an independent readback. |
| tickets forward --conversation ID | `POST /api/_/tickets/{id}/forward {conversation_id}` | COVERED | yes | yes | none | test_conversation_forward_source.py | Live send verified by an independent readback. |
| tickets assign --agent/--group (single) | `PUT /api/_/tickets/{id}` | COVERED | yes | yes | none | test_ticket_assignment.py | Live send verified by an independent readback. |
| tickets assign --unassign | `PUT /api/_/tickets/{id}` | COVERED | yes | yes | none | test_ticket_assignment.py | Live send verified by an independent readback. |
| tickets pick-up | `PUT /api/_/tickets/pick_up {ids}` | COVERED | yes | yes | none | test_ticket_assignment.py, test_ticket_bulk.py | Live send verified by an independent readback. |
| tickets merge | `POST /api/_/merge_tickets/complete_merge` | COVERED | yes | yes | none | test_ticket_merge.py | Live send verified by an independent readback. |
| tickets bulk status (plain) | `PUT /api/_/bulk_ticket_actions/update_multiple` | COVERED | yes | yes | none | test_ticket_bulk.py, test_ticket_bulk_set.py | Live send verified by an independent readback. |
| tickets bulk status --set (rule-required answers) | `PUT /api/_/tickets/{id} per ticket` | COVERED | yes | yes | none | test_ticket_bulk_set.py | Live send verified by an independent readback. |
| tickets bulk assign | `PUT /api/_/bulk_ticket_actions/update_multiple` | COVERED | yes | yes | none | test_ticket_bulk.py | Live send verified by an independent readback. |
| tickets bulk pickup | `PUT /api/_/tickets/pick_up` | COVERED | yes | yes | none | test_ticket_bulk.py | Live send verified by an independent readback. |
| tickets bulk merge | `POST /api/_/merge_tickets/complete_merge` | COVERED | yes | yes | none | test_ticket_bulk.py, test_ticket_merge.py | Live send verified by an independent readback. |
| tickets bulk spam / unspam | `POST /api/_/tickets/spam_multiple, POST /api/_/tickets/unspam_multiple` | COVERED | yes | yes | none | test_ticket_bulk.py | Live send verified by an independent readback. |
| tickets bulk delete (soft) | `DELETE /api/_/tickets/delete_multiple?ids=` | COVERED | yes | yes | none | test_ticket_bulk.py | Live send verified by an independent readback. |
| tickets bulk ... --where --select-all (filtered) | `expands to <=50 explicit IDs then wires above` | UNSURE | stale (re-test) | no | none | test_ticket_bulk_filter_cli.py, test_ticket_bulk_set.py, test_filter_wires.py | Filter guard reworked after the last live run; re-test pending. |
| tickets bulk --json output | stdout JSON only | COVERED | n/a (local) | n/a | none | test_ticket_bulk_set.py | Local command; offline tests. |
| tickets bulk-priority | `PUT /api/_/bulk_ticket_actions/update_multiple {priority}` | COVERED | yes | yes | none | test_bulk_priority.py | Live send verified by an independent readback. |
| tickets tasks list | `GET /api/_/tickets/{id}/tasks` | COVERED | yes (read) | n/a (read) | read-verified | test_ticket_tasks_reviewed.py, test_task_required_fields.py, test_task_status_portability.py | Live read verified. |
| tickets tasks add | `POST /api/_/tickets/{id}/tasks` | COVERED | yes | yes | read-verified | test_ticket_tasks_reviewed.py, test_task_required_fields.py, test_task_status_portability.py | Live send verified by an independent readback. |
| tickets tasks update | `PUT /api/_/tickets/{id}/tasks/{task_id}` | COVERED | yes | yes | read-verified | test_ticket_tasks_reviewed.py, test_task_required_fields.py, test_task_status_portability.py | Live send verified by an independent readback. |
| tickets tasks status | `PUT /api/_/tickets/{id}/tasks/{task_id} {status}` | COVERED | yes | yes | read-verified | test_ticket_tasks_reviewed.py, test_task_required_fields.py, test_task_status_portability.py | Live send verified by an independent readback. |
| tickets tasks delete | `PUT /api/_/tickets/{id}/tasks/update` | COVERED | yes | yes | read-verified | test_ticket_tasks_reviewed.py, test_task_required_fields.py, test_task_status_portability.py | Live send verified by an independent readback. |
| tickets time list | `GET /api/_/tickets/{id}/time_entries` | COVERED | yes (read) | n/a (read) | read-verified | test_ticket_time.py, test_module_time_problem_tasks.py | Live read verified. |
| tickets time write create | `POST /api/_/tickets/{id}/time_entries` | COVERED | yes | yes | none | test_ticket_time.py, test_module_time_problem_tasks.py | Live send verified by an independent readback. |
| tickets time write update | `PUT /api/_/tickets/{id}/time_entries/{eid}` | COVERED | yes | yes | none | test_ticket_time.py, test_module_time_problem_tasks.py | Live send verified by an independent readback. |
| tickets time write delete | `DELETE /api/_/tickets/{id}/time_entries/{eid}` | COVERED | yes | yes | none | test_ticket_time.py, test_module_time_problem_tasks.py | Live send verified by an independent readback. |
| tickets watchers list | `GET /api/_/tickets/{id}/watchers` | COVERED | yes (read) | n/a (read) | read-verified | test_watchers.py | Live read verified. |
| tickets watchers set --watch | `POST /api/_/tickets/{id}/watch {user_id}` | COVERED | yes | yes | none | test_watchers.py | Live send verified by an independent readback. |
| tickets watchers set --unwatch | `PUT /api/_/tickets/{id}/unwatch {user_id}` | COVERED | yes | yes | none | test_watchers.py | Live send verified by an independent readback. |
| attachments delete (--owner tickets/ID [--child CONV]) | `DELETE /api/_/attachments/{id}` | COVERED | yes | yes | none | test_attachments_reviewed.py, test_attachment_failure_matrix.py | Live send verified by an independent readback. |
| tickets download (--all) | `GET attachment URL (signed)` | COVERED | yes (read) | n/a (read) | none | test_download_attachments.py, test_tui_download_collision.py | Live read verified. |
| templates list | `GET /api/_/ticket-templates` | COVERED | yes (read) | n/a (read) | read-verified | test_templates_windows.py | Live read verified. |
| templates show NAME_OR_ID | `GET /api/_/ticket-templates/{id}` | COVERED | yes (read) | n/a (read) | read-verified | test_templates_windows.py | Live read verified. |
| tickets canned folders | `GET /api/_/canned_response_folders` | COVERED | yes (read) | n/a (read) | read-verified | test_canned.py | Live read verified. |
| tickets canned list/show | `GET /api/_/canned_response_folders/{id}/canned_responses ; GET /api/_/canned_responses/{id}` | COVERED | yes (read) | n/a (read) | read-verified | test_canned.py | Live read verified. |
| tickets canned reply | `POST /api/_/tickets/{id}/reply (literal HTML from canned)` | COVERED | yes | yes | none | test_canned.py | Live send verified by an independent readback. |
| tickets scenarios list/show | `GET /api/_/scenario_automations` | COVERED | yes (read) | n/a (read) | read-verified | test_canned.py | Live read verified. |
| tickets scenarios apply | `PUT /api/_/tickets/execute_scenario` | COVERED | yes | yes | none | test_canned.py | Live send verified by an independent readback. |
| tickets views | `GET /api/_/ticket_filters` | COVERED (read) | yes (read) | n/a (read) | read-verified | test_views_cli.py, test_tui_views.py | Read-only: verified live against real data; nothing to send. |
| tickets private-views show | `GET /api/_/ticket_filters/{id}` | COVERED | yes (read) | n/a (read) | read-verified | test_private_views.py | Live read verified. |
| tickets private-views write create/edit/delete | `POST\|PUT\|DELETE /api/_/ticket_filters[/{id}]` | COVERED | yes | yes | none | test_private_views.py | Live send verified by an independent readback. |
| tickets columns list | `GET /api/_/tickets/headers` | COVERED (read) | yes (read) | n/a (read) | read-verified | test_list_preferences.py, test_views_cli.py | Read-only: verified live against real data; nothing to send. |
| tickets columns set | `POST /api/_/tickets/headers` | COVERED | yes | yes | none | test_list_preferences.py | Live send verified by an independent readback. |
| tickets export | `POST /api/_/tickets/export` | UNSURE | no (dry-run only) | no | none | test_list_preferences.py | Review-only; delivery not verified. |
| tickets associations (list) | `GET /api/_/tickets/{id}/associations ({id} for INC on prod)` | COVERED (read) | yes (read) | n/a (read) | read-verified | test_ticket_details.py | Read-only: verified live against real data; nothing to send. |
| (missing) ticket-side associate: problem/change/parent/child | `PUT tickets/{id}/problems/{pid}; PUT tickets/{id}/changes/associate; POST tickets/{id}/associate-child-ticket…` | GAP | no | no | none | — | Matrix placeholder; ticket-side problem, change and child links are proven in their own rows. |
| tickets approvals (list) | `GET /api/_/tickets/{id}/approvals` | COVERED (read) | yes (read) | n/a (read) | read-verified | test_ticket_details.py, test_approval_model.py, test_legacy_approvals.py | Read-only: verified live against real data; nothing to send. |
| tickets assets list/candidates | `GET associated-cis / associated_assets (driver probes endpoint)` | COVERED (read) | yes (read) | n/a (read) | read-verified | test_asset_association_driver.py, test_legacy_asset_association.py | List verified live; candidates are proven in the cross-cutting candidates row. |
| tickets assets add/remove | `PUT/POST associated-cis (driver)` | COVERED | yes | yes | capture-only | test_asset_association_driver.py, test_legacy_asset_association.py | Live send verified by an independent readback. |
| (missing) tickets restore / delete-forever / spam single / split / share / due-… | `(PUT tickets/{id}/restore, DELETE tickets/delete_forever ...)` | GAP | no | no | none | — | Remaining gaps: ticket split and sharing with another agent; trash, restore, delete-forever and spam have their own rows. |
| tickets private-views list | `GET /api/_/ticket_filters` | COVERED | yes (read) | n/a (read) | none | test_private_views.py | Live read verified. |
| tickets assets candidates | `GET https://<tenant>/itam/core/device\|asset\|businessservice/api/?q= (integrated inventory)` | COVERED | yes (read) | n/a (read) | none | test_asset_candidates.py | Live read verified. |
| tickets problems list\|candidates | `GET /api/_/tickets/{id}/problems` | COVERED | yes (read) | n/a (read) | none | test_ticket_associations.py | Live read verified. |
| tickets problems link\|unlink | `PUT /api/_/tickets/{id}/problems/{pid} {type}` | COVERED | yes | yes | none | test_ticket_associations.py | Live send verified by an independent readback. |
| tickets changes list\|candidates --kind | `GET /api/_/tickets/{id}/changes?type=...` | COVERED | yes (read) | n/a (read) | none | test_ticket_associations.py | Live read verified. |
| tickets changes link\|unlink --kind initiating\|initiated-by | `PUT /api/_/tickets/{id}/changes/associate; PUT /api/_/tickets/{id}/changes/{cid}` | COVERED | yes | yes | none | test_ticket_associations.py | Live send verified by an independent readback. |
| tickets children list\|candidates | `GET /api/_/tickets/{id}/child-tickets` | COVERED | yes (read) | n/a (read) | none | test_ticket_associations.py | Live read verified. |
| tickets children link\|unlink | `POST /api/_/tickets/{id}/associate-child-tickets; DELETE /api/_/tickets/{id}/remove_child?ticket_id={id}` | COVERED | yes | yes | none | test_ticket_associations.py | Live send verified by an independent readback. |
| tickets trash list | `GET /api/_/tickets?filter=deleted` | COVERED | yes (read) | n/a (read) | none | test_ticket_trash.py | Live read verified. |
| tickets restore | `PUT /api/_/tickets/{id}/restore` | COVERED | yes | yes | none | test_ticket_trash.py | Live send verified by an independent readback. |
| tickets delete-forever | `DELETE /api/_/tickets/delete_forever?ids=` | COVERED | yes | yes | none | test_ticket_trash.py | Live send verified by an independent readback. |
| tickets spam | `PUT /api/_/tickets/{id}/spam` | COVERED | yes | yes | none | test_ticket_spam.py | Live send verified by an independent readback. |
| tickets unspam | `PUT /api/_/tickets/{id}/unspam` | COVERED | yes | yes | none | test_ticket_spam.py | Live send verified by an independent readback. |
| tickets bulk scenario | `PUT /api/_/tickets/execute_scenario {scenario_id, ids}` | COVERED | yes | yes | none | test_ticket_bulk_scenario.py | Async: the send reports verified false by design; a separate read confirmed the effect. |

## Changes

| Command | Wire | Status | Live-verified | Readback independent | Production parity | Offline tests | Note |
|---|---|---|---|---|---|---|---|
| changes ls | `GET /api/_/changes?filter={view}&query_hash/where (service.py; default my_open)` | COVERED (read) | yes (read) | n/a (read) | read-verified | test_views_cli.py, test_list_preferences.py, test_completion_gen.py | Read-only: verified live against real data; nothing to send. |
| changes search | `GET /api/_/changes/search?term= (service.py)` | COVERED (read) | yes (read) | n/a (read) | none | — | Read-only: verified live against real data; nothing to send. |
| changes get (incl --tab/--internal) | `GET /api/_/changes/{id} + /tabs, /tasks, /notes, /approvals, /assets, /activities` | COVERED (read) | yes (read) | n/a (read) | read-verified | test_change_workflow_helpers.py, test_read_completeness.py, test_change_nested_subcommands.py | Read-only: verified live against real data; nothing to send. |
| changes activity | `GET /api/_/changes/{id}/activities` | COVERED (read) | yes (read) | n/a (read) | read-verified | test_activity_truncation.py | Read-only: verified live against real data; nothing to send. |
| changes tasks (list) | `GET /api/_/changes/{id}/tasks` | COVERED | yes (read) | n/a (read) | read-verified | test_change_tasks.py, test_change_task_wire.py | Live read verified. |
| changes tasks-add | `POST /api/_/changes/{id}/tasks` | COVERED | yes | yes | read-verified | test_change_tasks.py, test_change_optional_fields.py, test_change_task_runtime_metadata.py, test_task_required_fields.py | Live send verified by an independent readback. |
| changes tasks-update | `PUT /api/_/changes/{id}/tasks/{task_id}` | COVERED | yes | yes | read-verified | test_change_task_update.py, test_change_form_update_reviewed.py | Live send verified by an independent readback. |
| changes tasks-delete | `PUT /api/_/changes/{id}/tasks/update (bodied delete)` | COVERED | yes | yes | read-verified | test_change_task_wire.py, test_change_form_update_reviewed.py, test_tui_change_actions.py | Live send verified by an independent readback. |
| changes approvals (list) | `GET /api/_/changes/{id}/approval-chains (parallel) ; GET /changes/{id}/approvals (legacy flat; create.py/stat…` | COVERED | yes (read) | n/a (read) | read-verified | test_change_approvals.py, test_approval_model.py, test_legacy_approvals.py | Live read verified. |
| changes approval-request --cab | `POST /api/_/changes/{id}/approval-groups (chains); CAB members GET changes/{id}/cab-members` | COVERED | yes | yes | capture-only | test_change_approvals.py, test_approval_recipients.py, test_change_approval_groups.py | Live send verified by an independent readback. |
| changes approval-request --approver/--approver-group/agent | `POST /api/_/changes/{id}/approval-groups` | COVERED | yes | yes | none | test_change_approvals.py | Live send verified by an independent readback. |
| changes approval-request (legacy flat) | `POST /api/_/changes/{id}/approvals` | GAP | no | no | capture-only | test_legacy_approvals.py | Legacy approval model: not sendable on the trial tenant; offline and capture only. |
| changes approval-remind | `PUT /api/_/changes/{id}/approvals/{id}/remind (legacy model; the chains model has no reminder wire)` | GAP | no | no | capture-only | test_legacy_approvals.py | Legacy approval model: not sendable on the trial tenant; offline and capture only. |
| changes approval-respond | `PUT /api/_/changes/{id}/approvals/{approval_id}` | COVERED | yes | yes | none | test_change_approval_actions.py, test_change_approval_groups.py, test_tui_approvals.py | Live send verified by an independent readback. |
| changes approval-cancel | `PUT /api/_/changes/{id}/approval-groups/{gid}/cancel` | COVERED | yes | yes | none | test_change_approval_actions.py, test_change_approval_groups.py | Live send verified by an independent readback. |
| changes state | `GET /api/_/ws/{ws}/change_management/{flow_id}?include=state_info` | COVERED | yes (read) | n/a (read) | read-verified | test_state_flow_render.py, test_change_lifecycle.py | Live read verified. |
| changes update --status (lifecycle transition) [= 'status'] | `PUT /api/_/changes/{id} (status) after lifecycle read` | COVERED | yes | yes | read-verified | test_change_lifecycle.py, test_change_form_update_reviewed.py | Live send verified by an independent readback. |
| changes update (fields/--set/--priority/--agent/--group/--due-by) | `PUT /api/_/changes/{id}` | COVERED | yes | yes | read-verified | test_change_form_update_reviewed.py, test_change_lifecycle.py, test_formui_change.py | Live send verified by an independent readback. |
| changes update --planning/--description/--file (planning via update) | `POST\|PUT /api/_/changes/{id}/planning-fields[/{id}] + upload` | COVERED | yes | yes | read-verified | test_planning_edit_clear.py, test_planning_readback.py, test_planning_state.py | Live send verified by an independent readback. |
| changes planning-text | `POST /api/_/changes/{id}/planning-fields?id={section}; PUT .../planning-fields/{id}; DELETE (clear)` | COVERED | yes | yes | read-verified | test_planning_text.py, test_planning_edit_clear.py | Live send verified by an independent readback. |
| changes create | `POST /api/_/changes then PUT /api/_/changes/{id} (+planning/attach)` | COVERED | yes | yes | read-verified | test_create_flags.py, test_formui_change.py, test_change_optional_fields.py | Live send verified by an independent readback. |
| changes create --attach (before/after-create) | `POST /api/_/attachments + POST /api/_/changes` | COVERED | yes | yes | none | test_attachments_reviewed.py, test_attachment_failure_matrix.py | Live send verified by an independent readback. |
| changes clone | `POST /api/_/changes + tasks + planning-fields + PUT assets/associate (create.py)` | COVERED | yes | yes | none | test_clone_failure_matrix.py, test_change_workflow_helpers.py, test_create_flags.py | Live send verified by an independent readback. |
| changes delete | `DELETE /api/_/changes/{id} (bodyless, {id})` | COVERED | yes | yes | none | test_change_delete.py | Live send verified by an independent readback. |
| changes notes (list) | `GET /api/_/changes/{id}/notes` | COVERED | yes (read) | n/a (read) | read-verified | test_attachments_reviewed.py, test_change_workflow_helpers.py | Live read verified. |
| changes add-note | `POST /api/_/changes/{id}/notes (service.py generic)` | COVERED | yes | yes | read-verified | test_attachments_reviewed.py | Live send verified by an independent readback. |
| changes attachments (create/update --attach, download, delete) | `POST /api/_/attachments; GET download; DELETE /api/_/attachments/{id}` | COVERED | yes | yes | read-verified | test_attachments_reviewed.py, test_attachment_capture_hardening.py, test_attachment_failure_matrix.py | Live send verified by an independent readback. |
| changes download | `GET attachment URLs (service.py/attachments.py)` | COVERED | yes (read) | n/a (read) | none | test_attachments_reviewed.py | Live read verified. |
| changes associations ls/search | `GET /api/_/changes/{id}/tickets ; GET /changes/tickets/search?module_type=change&q=` | COVERED | yes (read) | n/a (read) | read-verified | test_change_associations.py | Live read verified. |
| changes associations add | `PUT /api/_/changes/{id}/tickets/associate` | COVERED | yes | yes | read-verified | test_change_associations.py | Live send verified by an independent readback. |
| changes associations remove | `DELETE /api/_/changes/{id}/tickets/{tid}?module_type=ticket_cause` | COVERED | yes | yes | none | test_change_associations.py | Live send verified by an independent readback. |
| changes associations pick | `interactive; same wire as add` | UNSURE | no | no | none | test_change_associations.py | Interactive picker; sends the same wire as add. |
| changes assets list | `GET /api/_/changes/{id}/assets (legacy) ; integrated inventory reads` | COVERED | yes (read) | n/a (read) | read-verified | test_change_asset_categories.py, test_asset_association_driver.py | Live read verified. |
| changes assets add | `PUT /api/_/changes/{id}/assets/associate (legacy) / integrated PUT (driver)` | COVERED | yes | yes | capture-only | test_asset_association_driver.py, test_change_asset_categories.py | Live send verified by an independent readback. |
| changes assets remove | `PUT /api/_/changes/{id}/assets/detach` | COVERED | yes | yes | none | test_asset_association_driver.py | Live send verified by an independent readback. |
| changes assets search / candidates / asset-picker | `GET integrated inventory search; GET changes/{id}/assets-to-associate (legacy candidates)` | GAP | yes (read) | n/a (read) | capture-only | test_asset_picker.py, test_asset_association_driver.py | Candidates read verified live; the change asset search and picker paths stay a gap. |
| changes relationships list | `GET /api/_/changes/{id}/{problems\|tickets\|releases\|projects}` | COVERED | yes (read) | n/a (read) | read-verified | test_change_relationships.py | Live read verified. |
| changes relationships write problem add/remove | `PUT /api/_/changes/{id}/problems/associate ; DELETE /changes/{id}/problems/{pid}` | COVERED | yes | yes | read-verified | test_change_relationships.py | Live send verified by an independent readback. |
| changes relationships write caused-incident add/remove | `PUT /api/_/changes/{id}/tickets/associate (module_type=ticket_cause)` | COVERED | yes | yes | read-verified | test_change_relationships.py | Live send verified by an independent readback. |
| changes relationships write project add/remove (+release) | `POST /api/_/freshrelease/settings/{route} then PUT /changes/{id}/{key}/associate; release PUT changes/{id}/re…` | UNSURE | stale (re-test) | no | none | test_change_relationships.py | Add creates a permanent project that cannot be deleted from a test tenant (remove only unlinks); partial live proof. |
| changes time list | `GET /api/_/changes/{id}/time_entries` | COVERED | yes (read) | n/a (read) | read-verified | test_module_time_problem_tasks.py | Live read verified. |
| changes time write create/update/delete | `POST /changes/{id}/time_entries; PUT .../{eid}; DELETE .../{eid}` | COVERED | yes | yes | read-verified | test_module_time_problem_tasks.py | Live send verified by an independent readback. |
| changes watchers list | `GET /api/_/changes/{id} (watcher_ids)` | COVERED | yes (read) | n/a (read) | read-verified | test_watchers.py | Live read verified. |
| changes watchers set | `POST /api/_/changes/{id}/watchers {user_id} ; DELETE /changes/{id}/unwatch?user_id=` | COVERED | yes | yes | none | test_watchers.py | Live send verified by an independent readback. |
| changes windows list | `GET /api/_/maintenance_windows` | COVERED | yes (read) | n/a (read) | none | test_templates_windows.py | Live read verified. |
| changes windows create | `POST /api/_/change_windows/overlapping_windows then POST /api/_/maintenance_windows` | COVERED | yes | yes | none | test_templates_windows.py | Live send verified by an independent readback. |
| changes windows delete | `DELETE /api/_/maintenance_windows/{id}` | COVERED | yes | yes | none | test_templates_windows.py | Live send verified by an independent readback. |
| changes views | `GET /api/_/change-filters` | COVERED (read) | yes (read) | n/a (read) | read-verified | test_views_cli.py, test_tui_views.py | Read-only: verified live against real data; nothing to send. |
| changes columns list | `GET /api/_/changes/headers or list prefs` | COVERED (read) | yes (read) | n/a (read) | read-verified | test_list_preferences.py | Read-only: verified live against real data; nothing to send. |
| changes columns set | `POST /api/_/changes/headers` | COVERED | yes | yes | none | test_list_preferences.py | Live send verified by an independent readback. |
| changes export | `POST /api/_/changes/export` | UNSURE | no (dry-run only) | no | none | test_list_preferences.py | Review-only; delivery not verified. |
| changes fields / lookup | `GET /api/_/change_form_fields ; lookup user/group/choices` | COVERED (read) | yes (read) | n/a (read) | read-verified | test_completion_gen.py, test_change_optional_fields.py | Read-only: verified live against real data; nothing to send. |
| changes url | local (browser URL print) | N/A | n/a (local) | n/a | none | test_completion_gen.py | Local only; no wire. |
| changes private-views list\|show | `GET /api/_/change-filters` | COVERED | yes (read) | n/a (read) | none | test_private_views_native.py | Live read verified. |
| changes private-views write create | `POST /api/_/change-filters` | COVERED | yes | yes | none | test_private_views_native.py | Live send verified by an independent readback. |
| changes private-views write rename\|delete | `PUT/DELETE /api/_/change-filters/{id}` | GAP | no | n/a | none | test_private_views_native.py | Refused by fsv: the rename and delete wires are not captured; use the web UI. |

## Problems and service requests

| Command | Wire | Status | Live-verified | Readback independent | Production parity | Offline tests | Note |
|---|---|---|---|---|---|---|---|
| problems ls | `GET problems?filter/query_hash (problem-filters views; paged)` | COVERED (read) | yes (read) | n/a (read) | read-verified | test_filter_wires.py, test_views_cli.py, test_list_filter_surface.py, test_read_completeness.py | Read-only: verified live against real data; nothing to send. |
| problems get | `GET problems/{id} + planning-fields + solutions + tasks + notes (readback of all writes)` | COVERED | yes (read) | n/a (read) | read-verified | test_problem_details.py, test_problem_internal_api.py | Live read verified. |
| problems activity | `GET problems/{id}/activities (page, start_token continuation)` | COVERED (read) | yes (read) | n/a (read) | read-verified | test_activity_truncation.py, test_problem_internal_api.py | Read-only: verified live against real data; nothing to send. |
| problems tasks | `GET problems/{id}/tasks` | COVERED | yes (read) | n/a (read) | read-verified | test_module_time_problem_tasks.py | Live read verified. |
| problems notes | `GET problems/{id}/notes` | COVERED (read) | yes (read) | n/a (read) | read-verified | test_problem_internal_api.py | Read-only: verified live against real data; nothing to send. |
| problems add-note | `POST problems/{id}/notes {body} (reviewed body path; same wire as note)` | COVERED | yes | yes | none | test_problem_internal_api.py, test_attachments_reviewed.py | Live send verified by an independent readback. |
| problems export | `POST /api/_/problems/export` | UNSURE | no (dry-run only) | no | none | test_list_preferences.py | Review-only; delivery not verified. |
| problems fields | `GET problem_form_fields (cached schema)` | COVERED (read) | yes (read) | n/a (read) | read-verified | test_schema_cache.py, test_completion_matrix.py | Read-only: verified live against real data; nothing to send. |
| problems lookup | `GET search/autocomplete requesters/agents; bootstrap groups; field choices` | COVERED (read) | yes (read) | n/a (read) | none | test_agent_owner_lookup.py | Read-only: verified live against real data; nothing to send. |
| problems views | `GET problem-filters` | COVERED (read) | yes (read) | n/a (read) | read-verified | test_views_cli.py, test_private_views.py | Read-only: verified live against real data; nothing to send. |
| problems search | `GET search/problems (text search)` | COVERED (read) | yes (read) | n/a (read) | none | test_search_routing.py | Read-only: verified live against real data; nothing to send. |
| problems url | local: prints browser URL | COVERED (read) | n/a (local) | n/a | none | — | Local URL builder; no request is sent. |
| problems download | `GET attachment file by id (attachments.py)` | COVERED | yes (read) | n/a (read) | none | test_download_attachments.py, test_completion_gen.py | Live read verified. |
| problems columns list | `GET problems/headers` | COVERED | yes (read) | n/a (read) | read-verified | test_list_preferences.py | Live read verified. |
| problems columns set | `PUT problems/headers selection (user preference; reversible)` | COVERED | yes | yes | none | test_list_preferences.py | Live send verified by an independent readback. |
| problems create | `POST problems` | COVERED | yes | yes | none | test_problem_workflows.py, test_problem_write_fixes.py | Live send verified by an independent readback. |
| problems update (--status/--priority/--agent/--group/--set) | `PUT problems/{id} (dirty keys; internal api)` | COVERED | yes | yes | none | test_problem_internal_api.py, test_problem_properties.py, test_problem_write_fixes.py | Live send verified by an independent readback. |
| problems update (planned start/end/effort) | `PUT problems/{id}` | COVERED | yes | yes | none | test_problem_properties.py, test_problem_write_fixes.py | Live send verified by an independent readback. |
| problems status --to In Progress (non-rewritten target) | `PUT problems/{id} after closure/rules preflight` | COVERED | yes | yes | none | test_problem_status.py, test_problem_write_fixes.py, test_shifted_tenant.py | Live send verified by an independent readback. |
| problems status --to Resolve / Closed / Open (workflow rewrite to same prior st… | `PUT problems/{id} {status}` | COVERED | yes | yes | none | test_problem_status.py, test_problem_write_fixes.py | Live send verified by an independent readback. |
| problems planning-text (set) | `POST problems/{id}/planning-fields?id=problem_cause\|problem_impact\|problem_symptom {description}; PUT/DELET…` | COVERED | yes | yes | read-verified | test_planning_text.py, test_planning_edit_clear.py, test_planning_error_ux.py, test_planning_state.py, test_planning_readback.py | Live send verified by an independent readback. |
| problems planning-text --clear | `DELETE problems/{id}/planning-fields/{key}` | COVERED | yes | yes | none | test_planning_edit_clear.py | Live send verified by an independent readback. |
| problems write-child note | `POST problems/{id}/notes (body html)` | COVERED | yes | yes | none | test_problem_workflows.py | Live send verified by an independent readback. |
| problems write-child task | `POST problems/{id}/tasks` | COVERED | yes | yes | none | test_problem_workflows.py, test_module_time_problem_tasks.py | Live send verified by an independent readback. |
| problems task-write update\|complete\|delete | `PUT problems/{id}/tasks/{task} (update/complete); PUT problems/{id}/tasks/update` | COVERED | yes | yes | none | test_module_time_problem_tasks.py | Live send verified by an independent readback. |
| problems note (`--attach`) | `POST problems/{id}/notes (+POST attachments)` | COVERED | yes | yes | none | test_attachments_reviewed.py, test_problem_internal_api.py | Live send verified by an independent readback. |
| problems mark-known-error | `PUT problems/{id}` | COVERED | yes | yes | none | test_problem_known_error.py, test_reviewed_cli_output.py | Live send verified by an independent readback. |
| problems link-incident | `PUT problems/{id}/tickets/associate` | COVERED | yes | yes | read-verified | test_problem_incidents.py | Live send verified by an independent readback. |
| problems unlink-incident | `DELETE problems/{id}/tickets/{display} body` | COVERED | yes | yes | none | test_problem_incidents.py | Live send verified by an independent readback. |
| problems link-solution | `PUT problems/{id}/solutions/{article}` | COVERED | yes | yes | read-verified | test_solutions.py, test_solution_unlink_http.py, test_problem_status.py | Live send verified by an independent readback. |
| problems unlink-solution | `PUT problems/{id}/solutions/{article} (bodyless {id})` | COVERED | yes | yes | none | test_solution_unlink_http.py, test_solutions.py | Live send verified by an independent readback. |
| problems delete | `DELETE problems/{id} (soft trash)` | COVERED | yes | yes | none | test_record_delete.py | Live send verified by an independent readback. |
| problems restore | `PUT problems/{id}/restore` | COVERED | yes | yes | none | test_record_delete.py | Live send verified by an independent readback. |
| problems delete-forever | `DELETE problems/delete-forever?ids={id} ({id} async; trashed only)` | COVERED | yes | yes | none | test_record_delete.py | Live send verified by an independent readback. |
| problems assets list | `GET problems/{id}/associated-cis?page&association_type=cis\|services (integrated); legacy GET problems/{id}/a…` | COVERED | yes (read) | n/a (read) | read-verified | test_integrated_assets.py, test_asset_association_driver.py, test_legacy_asset_association.py, test_asset_picker.py, test_tui_inventory_writes.py | Live read verified. |
| problems assets candidates | `GET itam/core/device\|asset\|businessservice/api/?q= (picker, read-only)` | COVERED | yes (read) | n/a (read) | read-verified | test_integrated_assets.py, test_asset_association_driver.py, test_legacy_asset_association.py, test_asset_picker.py, test_tui_inventory_writes.py | Live read verified. |
| problems assets add | `PUT problems/{id}/associate-ci {device_ids\|service_ids ints}` | COVERED | yes | yes | none | test_integrated_assets.py, test_asset_association_driver.py, test_legacy_asset_association.py, test_asset_picker.py, test_tui_inventory_writes.py | Live send verified by an independent readback. |
| problems assets remove | `PUT problems/{id}/disassociate-ci {cmdb_request_id per-record}` | COVERED | yes | yes | none | test_integrated_assets.py, test_asset_association_driver.py, test_legacy_asset_association.py, test_asset_picker.py, test_tui_inventory_writes.py | Live send verified by an independent readback. |
| problems time list | `GET problems/{id}/time_entries` | COVERED (read) | yes (read) | n/a (read) | read-verified | test_module_time_problem_tasks.py | Read-only: verified live against real data; nothing to send. |
| problems time write create\|delete | `POST problems/{id}/time_entries; DELETE problems/{id}/time_entries/{eid} (update action rejected for problems…` | COVERED | yes | yes | none | test_module_time_problem_tasks.py | Live send verified by an independent readback. |
| solutions create | `POST solutions/articles` | COVERED | yes | yes | none | test_solutions.py, test_solution_unlink_http.py | Live send verified by an independent readback. |
| solutions delete | `DELETE solutions/articles/{id} (trash)` | COVERED | yes | yes | none | test_record_delete.py | Live send verified by an independent readback. |
| solutions delete-forever | `DELETE solutions/articles/{id}/delete_forever` | COVERED | yes | yes | none | test_record_delete.py | Live send verified by an independent readback. |
| sr catalog | `GET catalog/service-categories?category_hierarchy=true; GET catalog/service-items (filter=frequently_requeste…` | COVERED (read) | yes (read) | n/a (read) | read-verified | test_sr_catalog.py, test_catalog_safety.py | Read-only: verified live against real data; nothing to send. |
| sr fields | `GET catalog/service-items/{display_id} (+ business_rules)` | COVERED (read) | yes (read) | n/a (read) | read-verified | test_sr_catalog.py, test_catalog_safety.py, test_formui_sr.py | Read-only: verified live against real data; nothing to send. |
| sr create | `POST catalog/service-items/{display}/place-request` | COVERED | yes | yes | none | test_sr_create.py, test_formui_sr.py, test_sr_recipients.py, test_sr_draft_recipients.py, test_catalog_safety.py | Live send verified by an independent readback. |
| sr get | `GET tickets/{id}; GET tickets/{id}/requested_items (+attachments)` | COVERED | yes (read) | n/a (read) | read-verified | test_sr_get.py | Live read verified. |
| sr items list | `GET tickets/{id}/requested_items; GET .../requested_items/{item}?view=more_info` | COVERED | yes (read) | n/a (read) | read-verified | test_sr_requested_items.py | Live read verified. |
| sr items write edit | `PUT tickets/{id}/requested_items/{item} {custom_fields:{dirty}}` | COVERED | yes | yes | none | test_sr_requested_items.py | Live send verified by an independent readback. |
| sr items write stage | `PATCH tickets/{id}/requested_items/{item}/update_stage` | COVERED | yes | yes | none | test_sr_requested_items.py | Live send verified by an independent readback. |
| sr items write add | `POST tickets/{id}/requested_items` | COVERED | yes | yes | none | test_sr_requested_items.py | Live send verified by an independent readback. |
| sr items write remove | `DELETE tickets/{id}/requested_items/{item} (non-parent only)` | COVERED | yes | yes | none | test_sr_requested_items.py | Live send verified by an independent readback. |
| sr assets list | `GET tickets/{id}/associated-cis (integrated) or legacy GET tickets/{id}/associated_assets (probe,` | COVERED | yes (read) | n/a (read) | read-verified | test_asset_association_driver.py, test_integrated_assets.py, test_legacy_asset_association.py | Live read verified. |
| sr assets candidates | `GET itam/core/device\|asset\|businessservice/api/?q= ; legacy GET assets-to-associate` | COVERED | yes (read) | n/a (read) | capture-only | test_asset_picker.py, test_legacy_asset_association.py | Live read verified. |
| sr assets add | `PUT tickets/{id}/associate-ci {device_ids\|service_ids ints}; legacy PUT tickets/{id}/associate_assets {item_…` | COVERED | yes | yes | capture-only | test_integrated_assets.py, test_legacy_asset_association.py, test_tui_inventory_writes.py | Live send verified by an independent readback. |
| sr assets remove | `PUT tickets/{id}/disassociate-ci {cmdb_request_id}` | COVERED | yes | yes | none | test_integrated_assets.py, test_legacy_asset_association.py | Live send verified by an independent readback. |
| sr approvals list | `GET tickets/{id}/approval-groups + approval-chains (parallel model) or legacy flat GET tickets/{id}/approvals` | COVERED | yes (read) | n/a (read) | read-verified | test_sr_approvals.py, test_approval_model.py, test_legacy_approvals.py | Live read verified. |
| sr approvals request | `POST tickets/{id}/approval-groups {approval_groups:[{name,approval_type,email_content,notified_to}]}; legacy…` | COVERED | yes | yes | capture-only | test_sr_approvals.py, test_approval_recipients.py, test_legacy_approvals.py, test_approval_model.py | Live send verified by an independent readback. |
| sr approvals remind | `PUT tickets/{id}/approvals/{aid}/resend (bodyless)` | COVERED | yes | yes | capture-only | test_sr_approvals.py, test_legacy_approvals.py | Live send verified by an independent readback. |
| sr approvals approve | `PUT tickets/{id}/approvals/{aid}` | COVERED | yes | yes | none | test_sr_approvals.py, test_legacy_approvals.py, test_approval_model.py | Live send verified by an independent readback. |
| sr approvals reject | `PUT tickets/{id}/approvals/{aid}` | COVERED | yes | yes | none | test_sr_approvals.py, test_approval_model.py | Live send verified by an independent readback. |
| sr approvals cancel | `PUT tickets/{id}/approval-groups/{gid}/cancel` | COVERED | yes | yes | none | test_sr_approvals.py, test_approval_model.py, test_legacy_approvals.py | Live send verified by an independent readback. |
| problems time update | `PUT /api/_/problems/{id}/time_entries/{eid}` | COVERED | yes | yes | none | test_module_time_problem_tasks.py | Live send verified by an independent readback. |

## Cross-cutting (search, forms, filters, views, exports, auth, cache, reviewed-write engine)

| Command | Wire | Status | Live-verified | Readback independent | Production parity | Offline tests | Note |
|---|---|---|---|---|---|---|---|
| fsv search QUERY [--sort --all --npages --json] | `GET https://<tenant>/search/all?term= (client.search -> /search/{entity})` | COVERED (read) | yes (read) | n/a (read) | none | test_search_routing.py | Read-only: verified live against real data; nothing to send. |
| tickets\|changes\|problems search QUERY | `GET /search/{tickets\|changes\|problems}` | COVERED (read) | yes (read) | n/a (read) | none | test_search_routing.py | Read-only: verified live against real data; nothing to send. |
| assets search --kind devices | `GET https://<tenant>/itam/core/assets/api/ ?page` | COVERED | yes (read) | n/a (read) | none | test_integrated_assets.py, test_asset_picker.py | Live read verified. |
| assets search --kind services | `GET https://<tenant>/itam/core/{services family}/api/` | COVERED | yes (read) | n/a (read) | none | test_integrated_assets.py | Live read verified. |
| assets search --page N / pagination, total_rows contract | `as the row above` | UNSURE | no | n/a (read) | none | test_integrated_assets.py | Only one page of devices on the test tenant, so paging is unproven. |
| templates list | `GET /api/_/ticket-templates` | COVERED | yes (read) | n/a (read) | read-verified | test_templates_windows.py | Live read verified. |
| templates show NAME | `GET /api/_/ticket-templates/{id}` | COVERED | yes (read) | n/a (read) | none | test_templates_windows.py | Live read verified. |
| tickets create --template NAME (prefill) | `GET /api/_/ticket-templates[/{id}] then POST /api/_/tickets` | COVERED | yes | yes | none | test_templates_windows.py, test_ticket_create.py | Live send verified by an independent readback. |
| attachments delete ID --owner tickets/N (root attachment) | `DELETE /api/_/attachments/{id}; owner readback GET tickets/N` | COVERED | yes | yes | none | test_attachments_reviewed.py, test_attachment_failure_matrix.py, test_attachment_capture_hardening.py | Live send verified by an independent readback. |
| attachments delete ID --owner tickets/N --child CONV_ID | `DELETE /api/_/attachments/{id}; readback child conversation/note` | COVERED | yes | yes | none | test_attachments_reviewed.py | Live send verified by an independent readback. |
| attachments delete ID --owner changes/N | `DELETE /api/_/attachments/{id}; readback changes/N` | COVERED | yes | yes | none | test_attachments_reviewed.py | Needs a change attachment uploaded through a complete change create; re-test pending. |
| attachments delete ID --owner problems/N | `as the row above` | COVERED | yes | yes | none | test_attachments_reviewed.py | Live send verified by an independent readback. |
| attachments delete ID --owner ... --section KEY (planning section) | `DELETE /api/_/attachments/{id}; readback planning section` | UNSURE | no | no | none | test_attachments_reviewed.py | Needs a planning-section attachment uploaded in the web UI. |
| --attach on tickets reply / add-note | `POST /api/_/attachments (base64 JSON) then reply/note POST` | COVERED | yes | yes | none | test_attachments_reviewed.py, test_attachment_capture_hardening.py | Live send verified by an independent readback. |
| tickets create --attach / changes create --attach (before-create) | `POST /api/_/attachments then create` | COVERED | yes | yes | none | test_attachments_reviewed.py | Live send verified by an independent readback. |
| --attach on forward / inline images in reply/note | `POST /api/_/solutions/inline_images / attachments` | COVERED | yes | yes | none | test_inline_images.py | Live send verified by an independent readback. |
| tickets\|changes\|problems download ID [--all] | `GET attachment URLs (signed) per record` | COVERED | yes (read) | n/a (read) | none | test_download_attachments.py, test_tui_download_collision.py | Live read verified. |
| form fields MODULE [--choices F] | `GET {module} form fields (internal /api/_/{module}_form_fields) + ws/N/business_rules` | COVERED (read) | yes (read) | n/a (read) | none | test_rules_workspace.py, test_content_fields.py | Read-only: verified live against real data; nothing to send. |
| form explain MODULE [--set --from --preset --id --all] | `same + rule evaluation (forms/rules.py)` | COVERED (read) | yes (read) | n/a (read) | none | test_content_fields.py, test_rules_workspace.py | Read-only: verified live against real data; nothing to send. |
| form dump MODULE [--mode new\|edit] | `same reads; local snapshot` | COVERED (read) | yes (read) | n/a (read) | none | test_changes_form_cli.py | Read-only: verified live against real data; nothing to send. |
| form diff SNAPSHOT [--against] | `snapshot vs current metadata` | COVERED (read) | yes (read) | n/a (read) | none | test_changes_form_cli.py | Read-only: verified live against real data; nothing to send. |
| form * for problems / sr | `GET /api/_/ws/{ws}/business_rules?module_type=problem` | GAP | yes (read) | n/a (read) | read-verified | test_problem_workflows.py | Problem and SR form rules are not modelled. |
| schema loader: ticket/change/problem form fields + choices cache | `GET {module} form fields; business_rules; cached` | UNSURE | yes (read) | n/a (read) | none | test_schema_cache.py, test_cache_config_hardening.py | Reads verified live; proving the freshness check needs a form-field edit in the web UI. |
| schema loader: SR catalog item form + place-request rules | `GET catalog/service-items/{id}; business_rules module_type=service_request` | COVERED | yes (read) | n/a (read) | none | test_sr_catalog.py, test_catalog_safety.py | Reads verified live; proving the freshness check needs a form-field edit in the web UI. |
| tickets views | `GET /api/_/ticket_filters` | COVERED (read) | yes (read) | n/a (read) | read-verified | test_views_cli.py, test_search_routing.py | Read-only: verified live against real data; nothing to send. |
| changes views | `GET /api/_/change-filters` | COVERED (read) | yes (read) | n/a (read) | read-verified | test_views_cli.py | Read-only: verified live against real data; nothing to send. |
| problems views | `GET /api/_/problem-filters` | COVERED (read) | yes (read) | n/a (read) | read-verified | test_views_cli.py | Read-only: verified live against real data; nothing to send. |
| tickets private-views show | `GET ticket_filters/{id}` | COVERED | yes (read) | n/a (read) | capture-only | test_private_views.py | Live read verified. |
| tickets private-views write clone\|rename\|delete | `POST ticket_filters; PUT/DELETE ticket_filters/{id}` | COVERED | yes | yes | none | test_private_views.py | Live send verified by an independent readback. |
| --view NAME / default view | `filters_path GET then list GET` | COVERED (read) | yes (read) | n/a (read) | read-verified | test_views_cli.py | Read-only: verified live against real data; nothing to send. |
| tickets columns list | `GET /api/_/tickets/headers` | COVERED | yes (read) | n/a (read) | capture-only | test_list_preferences.py | Live read verified. |
| tickets columns set | `POST /api/_/tickets/headers (ordered map + frozen_columns); shared account pref` | COVERED | yes | yes | capture-only | test_list_preferences.py | Live send verified by an independent readback. |
| tickets export | `POST /api/_/tickets/export (CSV / date_filter30)` | UNSURE | no (dry-run only) | no | capture-only | test_list_preferences.py | Review-only; delivery not verified. |
| changes columns list | `GET /api/_/changes/headers` | COVERED | yes (read) | n/a (read) | capture-only | test_list_preferences.py | Live read verified. |
| changes columns set | `POST /api/_/changes/headers (ordered map + frozen_columns); shared account pref` | COVERED | yes | yes | capture-only | test_list_preferences.py | Live send verified by an independent readback. |
| changes export | `POST /api/_/changes/export (CSV / date_filter30)` | UNSURE | no (dry-run only) | no | capture-only | test_list_preferences.py | Review-only; delivery not verified. |
| problems columns list | `GET /api/_/problems/headers` | COVERED | yes (read) | n/a (read) | capture-only | test_list_preferences.py | Live read verified. |
| problems columns set | `POST /api/_/problems/headers (ordered map + frozen_columns); shared account pref` | COVERED | yes | yes | capture-only | test_list_preferences.py | Live send verified by an independent readback. |
| problems export | `POST /api/_/problems/export` | UNSURE | no (dry-run only) | no | capture-only | test_list_preferences.py | Review-only; delivery not verified. |
| tickets canned folders | `GET /api/_/canned_response_folders` | COVERED | yes (read) | n/a (read) | read-verified | test_canned.py | Live read verified. |
| tickets canned list FOLDER | `GET canned_response_folders/{id}/canned_responses` | COVERED | yes (read) | n/a (read) | none | test_canned.py | Live read verified. |
| tickets canned show ID | `GET canned_responses/{id}` | COVERED | yes (read) | n/a (read) | none | test_canned.py | Live read verified. |
| tickets canned reply TICKET ID | `reviewed reply POST conversations with literal HTML` | COVERED | yes | yes | capture-only | test_canned.py | Live send verified by an independent readback. |
| tickets scenarios list | `GET /api/_/scenario_automations` | COVERED | yes (read) | n/a (read) | read-verified | test_canned.py | Live read verified. |
| tickets scenarios show ID | `GET scenario_automations (filter by id)` | COVERED | yes (read) | n/a (read) | read-verified | test_canned.py | Live read verified. |
| tickets scenarios apply TICKET ID | `PUT /api/_/tickets/execute_scenario` | COVERED | yes | yes | capture-only | test_canned.py | Live send verified by an independent readback. |
| auth status | `GET /api/v2/agents/me + session check` | COVERED | yes (read) | n/a (read) | none | test_session_store.py, test_client_session.py | Live read verified. |
| auth login [--domain --header --browser --store --force] | session file/Keychain; browser cookie capture | UNSURE | n/a (local) | n/a | none | test_session_store.py, test_client_session.py | Interactive login; not exercised live (it would replace the shared test session). |
| auth logout | wipes session file+Keychain | UNSURE | n/a (local) | n/a | none | test_session_store.py | Not exercised live (it would destroy the shared test session). |
| cache status | local | COVERED | n/a (local) | n/a | none | test_cache_cli.py, test_cache_config_hardening.py | Local command; offline tests. |
| cache refresh | `GETs schema/views/groups` | COVERED | yes (read) | n/a (read) | none | test_cache_cli.py, test_schema_cache.py | Live read verified. |
| cache clear [schema\|views\|groups\|all] | local delete | COVERED | n/a (local) | n/a | none | test_cache_cli.py, test_cache_config_hardening.py | Local command; offline tests. |
| completion show | local shell/cache | COVERED | n/a (local) | n/a | none | test_completion_gen.py, test_completion_matrix.py, test_completion_autocomplete.py, test_completion_coverage_subprocess.py, test_completion_shell_fallback.py | Local command; offline tests. |
| completion install | local shell/cache | COVERED | n/a (local) | n/a | none | test_completion_gen.py, test_completion_matrix.py, test_completion_autocomplete.py, test_completion_coverage_subprocess.py, test_completion_shell_fallback.py | Local command; offline tests. |
| completion refresh | local shell/cache | COVERED | n/a (local) | n/a | none | test_completion_gen.py, test_completion_matrix.py, test_completion_autocomplete.py, test_completion_coverage_subprocess.py, test_completion_shell_fallback.py | Local command; offline tests. |
| completion doctor | local shell/cache | COVERED | n/a (local) | n/a | none | test_completion_gen.py, test_completion_matrix.py, test_completion_autocomplete.py, test_completion_coverage_subprocess.py, test_completion_shell_fallback.py | Local command; offline tests. |
| journal ls | local journal file (see `fsv journal path`) | COVERED | n/a (local) | n/a | none | test_journal_editor.py | Local command; offline tests. |
| journal tail | local journal file (see `fsv journal path`) | COVERED | n/a (local) | n/a | none | test_journal_editor.py | Local command; offline tests. |
| journal path | local journal file (see `fsv journal path`) | COVERED | n/a (local) | n/a | none | test_journal_editor.py | Local command; offline tests. |
| config defaults show\|set\|clear (default --where) | local config | COVERED | n/a (local) | n/a | none | test_universal_config.py, test_list_preferences.py | Local command; offline tests. |
| {module} ls --where / --or / --no-defaults / --debug | `GET {module} list with query_hash params (filter=...)` | COVERED (read) | yes (read) | n/a (read) | read-verified | test_filter_wires.py, test_strict_filter_contract.py, test_query_matrix.py, test_query_edge_cases.py, test_query_hash_params.py, test_list_filter_surface.py, test_ticket_type_filter.py | Read-only: verified live against real data; nothing to send. |
| --where text filters (subject/description) fail closed | refused client-side | UNSURE | n/a (local) | n/a | none | test_strict_filter_contract.py | Refused client-side; nothing is sent. |
| --where type=NAME (name not id) | `query_hash value:['Service Request']` | UNSURE | yes (read) | n/a (read) | read-verified | test_ticket_type_filter.py | Read verified live; row-by-row comparison with the web UI pending. |
| --where date ranges, relative dates, numbers, multi-select | `query_hash conditions` | COVERED | yes (read) | n/a (read) | capture-only | test_filter_wires.py | Live read verified. |
| --where approval_status on tickets (refused), changes (allowed) | `query_hash` | COVERED | yes (read) | n/a (read) | none | test_strict_filter_contract.py | Live read verified. |
| --query-hash RAW | `query_hash raw JSON` | UNSURE | no | n/a (read) | none | test_query_hash_params.py | Live run found a bug, since fixed; re-test pending. |
| reviewed-write: --dry-run emits plan_hash | reviewed_write.execute / review.py | COVERED | n/a (local) | n/a | capture-only | test_write_safety.py, test_reviewed_cli_output.py, test_reviewed_snapshot.py | Local command; offline tests. |
| reviewed-write: --yes --expect-plan guard + stale hash refusal | `as the row above` | COVERED | yes | yes | none | test_write_safety.py | Live send verified by an independent readback. |
| reviewed-write: plan hash determinism | journal.snapshot_hash | COVERED | n/a (local) | n/a | none | test_reviewed_snapshot.py | Local command; offline tests. |
| reviewed-write: bulk plan hash reporting | ticket_bulk | COVERED | n/a (local) | n/a | none | test_ticket_bulk.py | Local command; offline tests. |
| reviewed-write: readback verification (verified true/false) | readback.py, per-command verify callbacks | UNSURE | n/a (local) | n/a | none | test_readback_normalization.py, test_readback_values.py, test_conversation_recipient_readback.py | Several verify callbacks were fixed after the last live run; re-test pending. |
| reviewed-write: unknown-outcome handling + journal record | execute() journal pending/failed | COVERED | n/a (local) | n/a | none | test_journal_editor.py, test_solution_unlink_http.py | Local command; offline tests. |
| reviewed-write: legacy un-hashed writes (reply, problems update, tickets update… | body-only preview | GAP | n/a (local) | n/a | none | test_legacy_write_guards.py | Older writes preview the body without a plan hash. |
| tickets private-views list | `GET /api/_/ticket_filters` | COVERED | yes (read) | n/a (read) | none | test_private_views.py | Live read verified. |
| changes private-views list\|show\|write create | `GET/POST /api/_/change-filters` | COVERED | yes | yes | none | test_private_views_native.py | Live send verified by an independent readback. |
| tickets/changes/problems/sr assets candidates | `GET https://<tenant>/itam/core/device\|asset\|businessservice/api/?q= (integrated inventory)` | COVERED | yes (read) | n/a (read) | none | test_asset_candidates.py | Live read verified. |
| tickets problems\|changes\|children (associations) | `PUT\|POST\|DELETE /api/_/tickets/{id}/{problems,changes,associate-child-tickets,remove_child}` | COVERED | yes | yes | none | test_ticket_associations.py | Live send verified by an independent readback. |
| tickets trash\|restore\|delete-forever\|spam\|unspam | `GET /api/_/tickets?filter=deleted; PUT /api/_/tickets/{id}/{restore,spam,unspam}; DELETE /api/_/tickets/delete_forever?ids={id}` | COVERED | yes | yes | none | test_ticket_trash.py, test_ticket_spam.py | Live send verified by an independent readback. |
| tickets bulk scenario | `PUT /api/_/tickets/execute_scenario {scenario_id, ids}` | COVERED | yes | yes | none | test_ticket_bulk_scenario.py | Async: the send reports verified false by design; a separate read confirmed the effect. |
| problems time update | `PUT /api/_/problems/{id}/time_entries/{eid}` | COVERED | yes | yes | none | test_module_time_problem_tasks.py | Live send verified by an independent readback. |

## Re-running a row

Writes follow the reviewed-write pattern; nothing is sent without a reviewed
hash:

```sh
# 1. review: prints the exact payload and a full plan_hash, sends nothing
fsv tickets update INC-1 --status Pending --dry-run
# 2. send once, bound to that review (refused if anything changed since)
fsv tickets update INC-1 --status Pending --yes --expect-plan <plan_hash>
# 3. the result reports "verified": true only after an independent readback;
#    re-read it yourself as well
fsv tickets get INC-1 --json
```

Use throwaway records on a test tenant, keep notifications to yourself, and
delete what you created (see [live-testing methodology](live-testing.md)).
An unknown outcome is never replayed: inspect the record and `fsv journal
tail` instead. Reads only need the command with `--json`, compared against
what the web UI shows for the same record or view.

See also [reviewed operations](reviewed-parity.md) for the per-command
safety boundaries.
