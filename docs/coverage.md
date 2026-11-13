# Command coverage

How much of fsv has been proven against a real Freshservice tenant, per
command. Rows were verified on a Freshservice Enterprise trial tenant,
2026-11; production-like behaviour was checked with read-only requests and
aborted UI captures only. Nothing here is a guarantee for another tenant:
plans, roles, forms and workflows differ (see the
[Freshservice knowledge base](freshservice/overview.md)).

## Summary

| Area | Rows | COVERED | UNSURE | GAP | N/A |
|---|---|---|---|---|---|
| Tickets | 79 | 50 | 26 | 2 | 1 |
| Changes | 53 | 36 | 12 | 4 | 1 |
| Problems and service requests | 62 | 42 | 19 | 1 | 0 |
| Cross-cutting | 73 | 35 | 36 | 2 | 0 |
| **All** | **267** | **163** | **93** | **9** | **2** |

N/A rows are local commands with no wire. Counts are per row: a few commands
appear both in their module table and in the cross-cutting table.

## Legend

* **COVERED** needs all three: a live send (or, for reads, a live read) on
  the trial tenant, an **independent readback** (a separate GET of the
  record or collection, never the write's own response echo), and offline
  tests that pin the wire and the refusals.
* **UNSURE**: implemented and tested offline, but the live proof is partial,
  read-only, stale after a later fix, or blocked by missing fixtures.
* **GAP**: not implemented, or no accepted wire exists for it.
* **Live-verified**: `yes` = live send; `yes (read)` = live read;
  `no (dry-run only)` = only the reviewed dry-run ran live; `stale (re-test)` =
  the live run predates a later fix; `partial` = some paths failed.
* **Readback independent**: whether the live proof re-read the result through
  a different request than the write (`n/a` for reads and local commands).
* **Production parity**: `read-verified` = the same read wires answered on a
  production-like tenant (read-only); `capture-only` = the write wire was
  captured from the web UI with the submit aborted; `none` = not checked.
* **Wire**: method and path template; `{id}` is a record id, `<tenant>` your
  Freshservice host. `/api/_/` is the agent web UI's internal API.
* **Offline tests**: files under `tests/` that exercise the row.

## Known gaps

Needs seeding on the test tenant before a live proof is possible:

* CMDB devices and services (asset add/remove/candidates on tickets,
  changes, problems and service requests; device on ticket create);
* ticket templates (template list/show and `create --template`);
* canned responses with placeholders (`tickets canned reply`);
* private-view metadata (private view create/rename/delete).

Blocked (no accepted wire or not reachable from a test tenant):

* project delete (it runs on a separate project-management host);
* legacy approvals respond/cancel (the legacy approval model is not available
  on the trial; only aborted captures exist);
* approval delegation;
* ticket split;
* the TUI (`fsv tui`) is not covered by this matrix.

## Tickets

| Command | Wire | Status | Live-verified | Readback independent | Production parity | Offline tests | Note |
|---|---|---|---|---|---|---|---|
| tickets ls (+--where/--view/--order-by/--all) | `GET /api/_/tickets?filter=... (list; filter wires in query.py)` | UNSURE | yes (read) | n/a | read-verified | test_filter_wires.py, test_query_matrix.py, test_query_edge_cases.py, test_query_hash_params.py, test_ticket_type_filter.py, test_views_cli.py, test_strict_filter_contract.py | Read verified live; row-by-row comparison with the web UI pending. |
| tickets get (--internal/--all-tabs) | `GET /api/v2/tickets/{id} + GET /api/_/tickets/{id}` | UNSURE | yes (read) | n/a | read-verified | test_ticket_details.py | Read verified live; row-by-row comparison with the web UI pending. |
| tickets activity | `GET internal ticket activity feed (start_token continuation; views.py)` | UNSURE | yes (read) | n/a | read-verified | test_ticket_lifecycle.py, test_views_cli.py | Read verified live; row-by-row comparison with the web UI pending. |
| tickets conversations | `GET /api/_/tickets/{id}/conversations` | COVERED | yes (read) | n/a | none | test_ticket_conversation_http.py, test_ticket_conversation.py | Live read verified. |
| tickets fields | `GET /api/_/ticket_form_fields (cached schema)` | UNSURE | yes (read) | n/a | read-verified | test_ticket_properties.py | Read verified live; row-by-row comparison with the web UI pending. |
| tickets lookup (sender/users/groups/choices) | `GET lookup endpoints via int_get` | UNSURE | yes (read) | n/a | none | test_templates_windows.py, test_ticket_conversation_http.py | Read verified live; row-by-row comparison with the web UI pending. |
| tickets search | `GET /api/_/search/... (global/keyword)` | UNSURE | yes (read) | n/a | none | test_list_filter_surface.py | Read verified live; row-by-row comparison with the web UI pending. |
| tickets url | local only (prints browser URL) | N/A | n/a (local) | n/a | none | — | Local only; no wire. |
| tickets create (core INC) | `POST /api/_/tickets` | COVERED | yes | yes | read-verified | test_ticket_create.py, test_ticket_create_cc.py | Live send verified by an independent readback. |
| tickets create --attach FILE | `POST /api/_/attachments then POST /api/_/tickets` | COVERED | yes | yes | none | test_attachments_reviewed.py, test_attachment_capture_hardening.py | Live send verified by an independent readback. |
| tickets create --device ID | `POST /api/_/tickets {ci_ids/device_ids}` | UNSURE | stale (re-test) | no | none | test_ticket_create.py | Needs seeded CMDB devices or services on the test tenant. |
| tickets create --cc | `POST /api/_/tickets {cc_emails}` | COVERED | yes | yes | none | test_ticket_create_cc.py | Live send verified by an independent readback. |
| tickets create --template NAME (prefill) | `GET /api/_/ticket-templates[/id] then POST /api/_/tickets` | UNSURE | no | no | read-verified | test_templates_windows.py | Needs a seeded ticket template. |
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
| tickets forward --conversation ID | `POST /api/_/tickets/{id}/forward {conversation_id}` | UNSURE | yes | yes | none | test_conversation_forward_source.py | Forward sent live; the source-conversation link is not exposed for readback. |
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
| tickets bulk ... --where --select-all (filtered) | `expands to <=50 explicit IDs then wires above` | UNSURE | stale (re-test) | no | none | test_ticket_bulk_filter_cli.py, test_ticket_bulk_set.py, test_filter_wires.py | Live result predates later fixes or was partial; re-test. |
| tickets bulk --json output | stdout JSON only | COVERED | n/a (local) | n/a | none | test_ticket_bulk_set.py | Local command; offline tests. |
| tickets bulk-priority | `PUT /api/_/bulk_ticket_actions/update_multiple {priority}` | COVERED | yes | yes | none | test_bulk_priority.py | Live send verified by an independent readback. |
| tickets tasks list | `GET /api/_/tickets/{id}/tasks` | COVERED | yes (read) | n/a | read-verified | test_ticket_tasks_reviewed.py, test_task_required_fields.py, test_task_status_portability.py | Live read verified. |
| tickets tasks add | `POST /api/_/tickets/{id}/tasks` | COVERED | yes | yes | read-verified | test_ticket_tasks_reviewed.py, test_task_required_fields.py, test_task_status_portability.py | Live send verified by an independent readback. |
| tickets tasks update | `PUT /api/_/tickets/{id}/tasks/{task_id}` | COVERED | yes | yes | read-verified | test_ticket_tasks_reviewed.py, test_task_required_fields.py, test_task_status_portability.py | Live send verified by an independent readback. |
| tickets tasks status | `PUT /api/_/tickets/{id}/tasks/{task_id} {status}` | COVERED | yes | yes | read-verified | test_ticket_tasks_reviewed.py, test_task_required_fields.py, test_task_status_portability.py | Live send verified by an independent readback. |
| tickets tasks delete | `PUT /api/_/tickets/{id}/tasks/update` | COVERED | yes | yes | read-verified | test_ticket_tasks_reviewed.py, test_task_required_fields.py, test_task_status_portability.py | Live send verified by an independent readback. |
| tickets time list | `GET /api/_/tickets/{id}/time_entries` | COVERED | yes (read) | n/a | read-verified | test_ticket_time.py, test_module_time_problem_tasks.py | Live read verified. |
| tickets time write create | `POST /api/_/tickets/{id}/time_entries` | COVERED | yes | yes | none | test_ticket_time.py, test_module_time_problem_tasks.py | Live send verified by an independent readback. |
| tickets time write update | `PUT /api/_/tickets/{id}/time_entries/{eid}` | COVERED | yes | yes | none | test_ticket_time.py, test_module_time_problem_tasks.py | Live send verified by an independent readback. |
| tickets time write delete | `DELETE /api/_/tickets/{id}/time_entries/{eid}` | COVERED | yes | yes | none | test_ticket_time.py, test_module_time_problem_tasks.py | Live send verified by an independent readback. |
| tickets watchers list | `GET /api/_/tickets/{id}/watchers` | COVERED | yes (read) | n/a | read-verified | test_watchers.py | Live read verified. |
| tickets watchers set --watch | `POST /api/_/tickets/{id}/watch {user_id}` | COVERED | yes | yes | none | test_watchers.py | Live send verified by an independent readback. |
| tickets watchers set --unwatch | `PUT /api/_/tickets/{id}/unwatch {user_id}` | COVERED | yes | yes | none | test_watchers.py | Live send verified by an independent readback. |
| attachments delete (--owner tickets/ID [--child CONV]) | `DELETE /api/_/attachments/{id}` | COVERED | yes | yes | none | test_attachments_reviewed.py, test_attachment_failure_matrix.py | Live send verified by an independent readback. |
| tickets download (--all) | `GET attachment URL (signed)` | COVERED | yes (read) | n/a | none | test_download_attachments.py, test_tui_download_collision.py | Live read verified. |
| templates list | `GET /api/_/ticket-templates` | UNSURE | yes (read) | n/a | read-verified | test_templates_windows.py | Needs a seeded ticket template. |
| templates show NAME_OR_ID | `GET /api/_/ticket-templates/{id}` | UNSURE | yes (read) | n/a | read-verified | test_templates_windows.py | Needs a seeded ticket template. |
| tickets canned folders | `GET /api/_/canned_response_folders` | UNSURE | yes (read) | n/a | read-verified | test_canned.py | Read verified live; row-by-row comparison with the web UI pending. |
| tickets canned list/show | `GET /api/_/canned_response_folders/{id}/canned_responses ; GET /api/_/canned_responses/{id}` | UNSURE | yes (read) | n/a | read-verified | test_canned.py | Read verified live; row-by-row comparison with the web UI pending. |
| tickets canned reply | `POST /api/_/tickets/{id}/reply (literal HTML from canned)` | UNSURE | no | no | none | test_canned.py | Needs a canned response with placeholders. |
| tickets scenarios list/show | `GET /api/_/scenario_automations` | UNSURE | yes (read) | n/a | read-verified | test_canned.py | Read verified live; row-by-row comparison with the web UI pending. |
| tickets scenarios apply | `PUT /api/_/tickets/execute_scenario` | UNSURE | yes | yes | none | test_canned.py | Partial live proof; a scenario may run workflows and send mail. |
| tickets views | `GET /api/_/ticket_filters` | UNSURE | yes (read) | n/a | read-verified | test_views_cli.py, test_tui_views.py | Read verified live; row-by-row comparison with the web UI pending. |
| tickets private-views show | `GET /api/_/ticket_filters/{id}` | UNSURE | yes (read) | n/a | read-verified | test_private_views.py | Read verified live; needs a UI-created private view to compare. |
| tickets private-views write create/edit/delete | `POST\|PUT\|DELETE /api/_/ticket_filters[/{id}]` | UNSURE | no | no | none | test_private_views.py | Needs private-view metadata (a UI-created private view). |
| tickets columns list | `GET /api/_/tickets/headers` | UNSURE | yes (read) | n/a | read-verified | test_list_preferences.py, test_views_cli.py | Read verified live; row-by-row comparison with the web UI pending. |
| tickets columns set | `POST /api/_/tickets/headers` | COVERED | yes | yes | none | test_list_preferences.py | Live send verified by an independent readback. |
| tickets export | `POST /api/_/tickets/export` | UNSURE | no (dry-run only) | no | none | test_list_preferences.py | Review-only by design: a send emails data to the signed-in agent and cannot be undone. |
| tickets associations (list) | `GET /api/_/tickets/{id}/associations ({id} for INC on prod)` | UNSURE | yes (read) | n/a | read-verified | test_ticket_details.py | Read verified live; row-by-row comparison with the web UI pending. |
| (missing) ticket-side associate: problem/change/parent/child | `PUT tickets/{id}/problems/{pid}; PUT tickets/{id}/changes/associate; POST tickets/{id}/associate-child-ticket…` | GAP | no | no | none | — | Not implemented. |
| tickets approvals (list) | `GET /api/_/tickets/{id}/approvals` | UNSURE | yes (read) | n/a | read-verified | test_ticket_details.py, test_approval_model.py, test_legacy_approvals.py | Read verified live; row-by-row comparison with the web UI pending. |
| tickets assets list/candidates | `GET associated-cis / associated_assets (driver probes endpoint)` | UNSURE | yes (read) | n/a | read-verified | test_asset_association_driver.py, test_legacy_asset_association.py | Needs seeded CMDB devices or services on the test tenant. |
| tickets assets add/remove | `PUT/POST associated-cis (driver)` | UNSURE | stale (re-test) | no | capture-only | test_asset_association_driver.py, test_legacy_asset_association.py | Needs seeded CMDB devices or services on the test tenant. |
| (missing) tickets restore / delete-forever / spam single / split / share / due-… | `(PUT tickets/{id}/restore, DELETE tickets/delete_forever ...)` | GAP | no | no | none | — | Not implemented. |

## Changes

| Command | Wire | Status | Live-verified | Readback independent | Production parity | Offline tests | Note |
|---|---|---|---|---|---|---|---|
| changes ls | `GET /api/_/changes?filter={view}&query_hash/where (service.py; default my_open)` | UNSURE | yes (read) | n/a | read-verified | test_views_cli.py, test_list_preferences.py, test_completion_gen.py | Read verified live; row-by-row comparison with the web UI pending. |
| changes search | `GET /api/_/changes/search?term= (service.py)` | UNSURE | yes (read) | n/a | none | — | Read verified live; row-by-row comparison with the web UI pending. |
| changes get (incl --tab/--internal) | `GET /api/_/changes/{id} + /tabs, /tasks, /notes, /approvals, /assets, /activities` | UNSURE | yes (read) | n/a | read-verified | test_change_workflow_helpers.py, test_read_completeness.py, test_change_nested_subcommands.py | Read verified live; row-by-row comparison with the web UI pending. |
| changes activity | `GET /api/_/changes/{id}/activities` | UNSURE | yes (read) | n/a | read-verified | test_activity_truncation.py | Read verified live; row-by-row comparison with the web UI pending. |
| changes tasks (list) | `GET /api/_/changes/{id}/tasks` | COVERED | yes (read) | n/a | read-verified | test_change_tasks.py, test_change_task_wire.py | Live read verified. |
| changes tasks-add | `POST /api/_/changes/{id}/tasks` | COVERED | yes | yes | read-verified | test_change_tasks.py, test_change_optional_fields.py, test_change_task_runtime_metadata.py, test_task_required_fields.py | Live send verified by an independent readback. |
| changes tasks-update | `PUT /api/_/changes/{id}/tasks/{task_id}` | COVERED | yes | yes | read-verified | test_change_task_update.py, test_change_form_update_reviewed.py | Live send verified by an independent readback. |
| changes tasks-delete | `PUT /api/_/changes/{id}/tasks/update (bodied delete)` | COVERED | yes | yes | read-verified | test_change_task_wire.py, test_change_form_update_reviewed.py, test_tui_change_actions.py | Live send verified by an independent readback. |
| changes approvals (list) | `GET /api/_/changes/{id}/approval-chains (parallel) ; GET /changes/{id}/approvals (legacy flat; create.py/stat…` | COVERED | yes (read) | n/a | read-verified | test_change_approvals.py, test_approval_model.py, test_legacy_approvals.py | Live read verified. |
| changes approval-request --cab | `POST /api/_/changes/{id}/approval-groups (chains); CAB members GET changes/{id}/cab-members` | COVERED | yes | yes | capture-only | test_change_approvals.py, test_approval_recipients.py, test_change_approval_groups.py | Live send verified by an independent readback. |
| changes approval-request --approver/--approver-group/agent | `POST /api/_/changes/{id}/approval-groups` | COVERED | yes | yes | none | test_change_approvals.py | Live send verified by an independent readback. |
| changes approval-request (legacy flat) | `POST /api/_/changes/{id}/approvals` | GAP | no | no | capture-only | test_legacy_approvals.py | Legacy approval model: not sendable on the trial tenant; offline and capture only. |
| changes approval-remind | `PUT /api/_/changes/{id}/approvals/{id}/remind? (legacy wire from ; chains model: no wire)` | GAP | no | no | capture-only | test_legacy_approvals.py | Legacy approval model: not sendable on the trial tenant; offline and capture only. |
| changes approval-respond | `PUT /api/_/changes/{id}/approvals/{approval_id}` | COVERED | yes | yes | none | test_change_approval_actions.py, test_change_approval_groups.py, test_tui_approvals.py | Live send verified by an independent readback. |
| changes approval-cancel | `PUT /api/_/changes/{id}/approval-groups/{gid}/cancel` | COVERED | yes | yes | none | test_change_approval_actions.py, test_change_approval_groups.py | Live send verified by an independent readback. |
| changes state | `GET /api/_/ws/{ws}/change_management/{flow_id}?include=state_info` | COVERED | yes (read) | n/a | read-verified | test_state_flow_render.py, test_change_lifecycle.py | Live read verified. |
| changes update --status (lifecycle transition) [= 'status'] | `PUT /api/_/changes/{id} (status) after lifecycle read` | COVERED | yes | yes | read-verified | test_change_lifecycle.py, test_change_form_update_reviewed.py | Live send verified by an independent readback. |
| changes update (fields/--set/--priority/--agent/--group/--due-by) | `PUT /api/_/changes/{id}` | COVERED | yes | yes | read-verified | test_change_form_update_reviewed.py, test_change_lifecycle.py, test_formui_change.py | Live send verified by an independent readback. |
| changes update --planning/--description/--file (planning via update) | `POST\|PUT /api/_/changes/{id}/planning-fields[/{id}] + upload` | COVERED | yes | yes | read-verified | test_planning_edit_clear.py, test_planning_readback.py, test_planning_state.py | Live send verified by an independent readback. |
| changes planning-text | `POST /api/_/changes/{id}/planning-fields?id={section}; PUT .../planning-fields/{id}; DELETE (clear)` | COVERED | yes | yes | read-verified | test_planning_text.py, test_planning_edit_clear.py | Live send verified by an independent readback. |
| changes create | `POST /api/_/changes then PUT /api/_/changes/{id} (+planning/attach)` | COVERED | yes | yes | read-verified | test_create_flags.py, test_formui_change.py, test_change_optional_fields.py | Live send verified by an independent readback. |
| changes create --attach (before/after-create) | `POST /api/_/attachments + POST /api/_/changes` | COVERED | yes | yes | none | test_attachments_reviewed.py, test_attachment_failure_matrix.py | Live send verified by an independent readback. |
| changes clone | `POST /api/_/changes + tasks + planning-fields + PUT assets/associate (create.py)` | COVERED | yes | yes | none | test_clone_failure_matrix.py, test_change_workflow_helpers.py, test_create_flags.py | Live send verified by an independent readback. |
| changes delete | `DELETE /api/_/changes/{id} (bodyless, {id})` | COVERED | yes | yes | none | test_change_delete.py | Live send verified by an independent readback. |
| changes notes (list) | `GET /api/_/changes/{id}/notes` | COVERED | yes (read) | n/a | read-verified | test_attachments_reviewed.py, test_change_workflow_helpers.py | Live read verified. |
| changes add-note | `POST /api/_/changes/{id}/notes (service.py generic)` | COVERED | yes | yes | read-verified | test_attachments_reviewed.py | Live send verified by an independent readback. |
| changes attachments (create/update --attach, download, delete) | `POST /api/_/attachments; GET download; DELETE /api/_/attachments/{id}` | COVERED | yes | yes | read-verified | test_attachments_reviewed.py, test_attachment_capture_hardening.py, test_attachment_failure_matrix.py | Live send verified by an independent readback. |
| changes download | `GET attachment URLs (service.py/attachments.py)` | COVERED | yes (read) | n/a | none | test_attachments_reviewed.py | Live read verified. |
| changes associations ls/search | `GET /api/_/changes/{id}/tickets ; GET /changes/tickets/search?module_type=change&q=` | COVERED | yes (read) | n/a | read-verified | test_change_associations.py | Live read verified. |
| changes associations add | `PUT /api/_/changes/{id}/tickets/associate` | COVERED | yes | yes | read-verified | test_change_associations.py | Live send verified by an independent readback. |
| changes associations remove | `DELETE /api/_/changes/{id}/tickets/{tid}?module_type=ticket_cause` | COVERED | yes | yes | none | test_change_associations.py | Live send verified by an independent readback. |
| changes associations pick | `interactive; same wire as add` | UNSURE | no | n/a | none | test_change_associations.py | Interactive picker; sends the same wire as add. |
| changes assets list | `GET /api/_/changes/{id}/assets (legacy) ; integrated inventory reads` | UNSURE | yes (read) | n/a | read-verified | test_change_asset_categories.py, test_asset_association_driver.py | Needs seeded CMDB devices or services on the test tenant. |
| changes assets add | `PUT /api/_/changes/{id}/assets/associate (legacy) / integrated PUT (driver)` | UNSURE | stale (re-test) | no | capture-only | test_asset_association_driver.py, test_change_asset_categories.py | Needs seeded CMDB devices or services on the test tenant. |
| changes assets remove | `PUT /api/_/changes/{id}/assets/detach` | UNSURE | stale (re-test) | no | none | test_asset_association_driver.py | Needs seeded CMDB devices or services on the test tenant. |
| changes assets search / candidates / asset-picker | `GET integrated inventory search; GET changes/{id}/assets-to-associate (legacy candidates)` | GAP | yes (read) | n/a | capture-only | test_asset_picker.py, test_asset_association_driver.py | Needs seeded CMDB devices or services on the test tenant. |
| changes relationships list | `GET /api/_/changes/{id}/{problems\|tickets\|releases\|projects}` | COVERED | yes (read) | n/a | read-verified | test_change_relationships.py | Live read verified. |
| changes relationships write problem add/remove | `PUT /api/_/changes/{id}/problems/associate ; DELETE /changes/{id}/problems/{pid}` | COVERED | yes | yes | read-verified | test_change_relationships.py | Live send verified by an independent readback. |
| changes relationships write caused-incident add/remove | `PUT /api/_/changes/{id}/tickets/associate (module_type=ticket_cause)` | COVERED | yes | yes | read-verified | test_change_relationships.py | Live send verified by an independent readback. |
| changes relationships write project add/remove (+release) | `POST /api/_/freshrelease/settings/{route} then PUT /changes/{id}/{key}/associate; release PUT changes/{id}/re…` | UNSURE | stale (re-test) | no | none | test_change_relationships.py | Add creates a new project (remove only unlinks); partial live proof. |
| changes time list | `GET /api/_/changes/{id}/time_entries` | COVERED | yes (read) | n/a | read-verified | test_module_time_problem_tasks.py | Live read verified. |
| changes time write create/update/delete | `POST /changes/{id}/time_entries; PUT .../{eid}; DELETE .../{eid}` | COVERED | yes | yes | read-verified | test_module_time_problem_tasks.py | Live send verified by an independent readback. |
| changes watchers list | `GET /api/_/changes/{id} (watcher_ids)` | COVERED | yes (read) | n/a | read-verified | test_watchers.py | Live read verified. |
| changes watchers set | `POST /api/_/changes/{id}/watchers {user_id} ; DELETE /changes/{id}/unwatch?user_id=` | COVERED | yes | yes | none | test_watchers.py | Live send verified by an independent readback. |
| changes windows list | `GET /api/_/maintenance_windows` | COVERED | yes (read) | n/a | none | test_templates_windows.py | Live read verified. |
| changes windows create | `POST /api/_/change_windows/overlapping_windows then POST /api/_/maintenance_windows` | COVERED | yes | yes | none | test_templates_windows.py | Live send verified by an independent readback. Overlap is checked against the listed windows (dry-run and send), times are converted to the tenant zone (`--timezone`), skipped daylight-saving times are refused, and `--allow-overlap` is bound into the plan hash. |
| changes windows delete | `DELETE /api/_/maintenance_windows/{id}` | COVERED | yes | yes | none | test_templates_windows.py | Live send verified by an independent readback. |
| changes views | `GET /api/_/change-filters` | UNSURE | yes (read) | n/a | read-verified | test_views_cli.py, test_tui_views.py | Read verified live; row-by-row comparison with the web UI pending. |
| changes columns list | `GET /api/_/changes/headers or list prefs` | UNSURE | yes (read) | n/a | read-verified | test_list_preferences.py | Read verified live; row-by-row comparison with the web UI pending. |
| changes columns set | `POST /api/_/changes/headers` | COVERED | yes | yes | none | test_list_preferences.py | Live send verified by an independent readback. |
| changes export | `POST /api/_/changes/export` | GAP | no (dry-run only) | no | none | test_list_preferences.py | Review-only by design: a send emails data to the signed-in agent and cannot be undone. |
| changes fields / lookup | `GET /api/_/change_form_fields ; lookup user/group/choices` | UNSURE | yes (read) | n/a | read-verified | test_completion_gen.py, test_change_optional_fields.py | Read verified live; row-by-row comparison with the web UI pending. |
| changes url | local (browser URL print) | N/A | n/a (local) | n/a | none | test_completion_gen.py | Local only; no wire. |

## Problems and service requests

| Command | Wire | Status | Live-verified | Readback independent | Production parity | Offline tests | Note |
|---|---|---|---|---|---|---|---|
| problems ls | `GET problems?filter/query_hash (problem-filters views; paged)` | UNSURE | yes (read) | n/a | read-verified | test_filter_wires.py, test_views_cli.py, test_list_filter_surface.py, test_read_completeness.py | Read verified live; row-by-row comparison with the web UI pending. |
| problems get | `GET problems/{id} + planning-fields + solutions + tasks + notes (readback of all writes)` | COVERED | yes (read) | n/a | read-verified | test_problem_details.py, test_problem_internal_api.py | Live read verified. |
| problems activity | `GET problems/{id}/activities (page, start_token continuation)` | UNSURE | yes (read) | n/a | read-verified | test_activity_truncation.py, test_problem_internal_api.py | Read verified live; row-by-row comparison with the web UI pending. |
| problems tasks | `GET problems/{id}/tasks` | COVERED | yes (read) | n/a | read-verified | test_module_time_problem_tasks.py | Live read verified. |
| problems notes | `GET problems/{id}/notes` | UNSURE | yes (read) | n/a | read-verified | test_problem_internal_api.py | Read verified live; row-by-row comparison with the web UI pending. |
| problems add-note | `POST problems/{id}/notes {body} (reviewed body path; same wire as note)` | COVERED | yes | yes | none | test_problem_internal_api.py, test_attachments_reviewed.py | Live send verified by an independent readback. |
| problems export | `POST problems/export-ish native CSV request, 30 days, emails data (job/download unknown; code: list_preferenc…` | GAP | no (dry-run only) | no | none | test_list_preferences.py | Review-only by design: a send emails data to the signed-in agent and cannot be undone. |
| problems fields | `GET problem_form_fields (cached schema)` | UNSURE | yes (read) | n/a | read-verified | test_schema_cache.py, test_completion_matrix.py | Read verified live; row-by-row comparison with the web UI pending. |
| problems lookup | `GET search/autocomplete requesters/agents; bootstrap groups; field choices` | UNSURE | yes (read) | n/a | none | test_agent_owner_lookup.py | Read verified live; row-by-row comparison with the web UI pending. |
| problems views | `GET problem-filters` | UNSURE | yes (read) | n/a | read-verified | test_views_cli.py, test_private_views.py | Read verified live; row-by-row comparison with the web UI pending. |
| problems search | `GET search/problems (text search)` | UNSURE | yes (read) | n/a | none | test_search_routing.py | Read verified live; row-by-row comparison with the web UI pending. |
| problems url | local: prints browser URL | UNSURE | n/a (local) | n/a | none | — | Local or engine behaviour; proven through the module rows and offline tests. |
| problems download | `GET attachment file by id (attachments.py)` | COVERED | yes (read) | n/a | none | test_download_attachments.py, test_completion_gen.py | Live read verified. |
| problems columns list | `GET problems/headers` | COVERED | yes (read) | n/a | read-verified | test_list_preferences.py | Live read verified. |
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
| problems assets list | `GET problems/{id}/associated-cis?page&association_type=cis\|services (integrated); legacy GET problems/{id}/a…` | UNSURE | yes (read) | n/a | read-verified | test_integrated_assets.py, test_asset_association_driver.py, test_legacy_asset_association.py, test_asset_picker.py, test_tui_inventory_writes.py | Needs seeded CMDB devices or services on the test tenant. |
| problems assets candidates | `GET itam/core/device\|asset\|businessservice/api/?q= (picker, read-only)` | UNSURE | stale (re-test) | no | read-verified | test_integrated_assets.py, test_asset_association_driver.py, test_legacy_asset_association.py, test_asset_picker.py, test_tui_inventory_writes.py | Needs seeded CMDB devices or services on the test tenant. |
| problems assets add | `PUT problems/{id}/associate-ci {device_ids\|service_ids ints}` | UNSURE | stale (re-test) | no | none | test_integrated_assets.py, test_asset_association_driver.py, test_legacy_asset_association.py, test_asset_picker.py, test_tui_inventory_writes.py | Needs seeded CMDB devices or services on the test tenant. |
| problems assets remove | `PUT problems/{id}/disassociate-ci {cmdb_request_id per-record}` | UNSURE | stale (re-test) | no | none | test_integrated_assets.py, test_asset_association_driver.py, test_legacy_asset_association.py, test_asset_picker.py, test_tui_inventory_writes.py | Needs seeded CMDB devices or services on the test tenant. |
| problems time list | `GET problems/{id}/time_entries` | UNSURE | yes (read) | n/a | read-verified | test_module_time_problem_tasks.py | Read ran live but returned no rows; needs a populated fixture. |
| problems time write create\|delete | `POST problems/{id}/time_entries; DELETE problems/{id}/time_entries/{eid} (update action rejected for problems…` | COVERED | yes | yes | none | test_module_time_problem_tasks.py | Live send verified by an independent readback. |
| solutions create | `POST solutions/articles` | COVERED | yes | yes | none | test_solutions.py, test_solution_unlink_http.py | Live send verified by an independent readback. |
| solutions delete | `DELETE solutions/articles/{id} (trash)` | COVERED | yes | yes | none | test_record_delete.py | Live send verified by an independent readback. |
| solutions delete-forever | `DELETE solutions/articles/{id}/delete_forever` | COVERED | yes | yes | none | test_record_delete.py | Live send verified by an independent readback. |
| sr catalog | `GET catalog/service-categories?category_hierarchy=true; GET catalog/service-items (filter=frequently_requeste…` | UNSURE | yes (read) | n/a | read-verified | test_sr_catalog.py, test_catalog_safety.py | Read verified live; row-by-row comparison with the web UI pending. |
| sr fields | `GET catalog/service-items/{display_id} (+ business_rules)` | UNSURE | yes (read) | n/a | read-verified | test_sr_catalog.py, test_catalog_safety.py, test_formui_sr.py | Read verified live; row-by-row comparison with the web UI pending. |
| sr create | `POST catalog/service-items/{display}/place-request` | COVERED | yes | yes | none | test_sr_create.py, test_formui_sr.py, test_sr_recipients.py, test_sr_draft_recipients.py, test_catalog_safety.py | Live send verified by an independent readback. |
| sr get | `GET tickets/{id}; GET tickets/{id}/requested_items (+attachments)` | COVERED | yes (read) | n/a | read-verified | test_sr_get.py | Live read verified. |
| sr items list | `GET tickets/{id}/requested_items; GET .../requested_items/{item}?view=more_info` | COVERED | yes (read) | n/a | read-verified | test_sr_requested_items.py | Live read verified. |
| sr items write edit | `PUT tickets/{id}/requested_items/{item} {custom_fields:{dirty}}` | COVERED | yes | yes | none | test_sr_requested_items.py | Live send verified by an independent readback. |
| sr items write stage | `PATCH tickets/{id}/requested_items/{item}/update_stage` | COVERED | yes | yes | none | test_sr_requested_items.py | Live send verified by an independent readback. |
| sr items write add | `POST tickets/{id}/requested_items` | COVERED | yes | yes | none | test_sr_requested_items.py | Live send verified by an independent readback. |
| sr items write remove | `DELETE tickets/{id}/requested_items/{item} (non-parent only)` | COVERED | yes | yes | none | test_sr_requested_items.py | Live send verified by an independent readback. |
| sr assets list | `GET tickets/{id}/associated-cis (integrated) or legacy GET tickets/{id}/associated_assets (probe,` | UNSURE | yes (read) | n/a | read-verified | test_asset_association_driver.py, test_integrated_assets.py, test_legacy_asset_association.py | Needs seeded CMDB devices or services on the test tenant. |
| sr assets candidates | `GET itam/core/device\|asset\|businessservice/api/?q= ; legacy GET assets-to-associate` | UNSURE | stale (re-test) | no | capture-only | test_asset_picker.py, test_legacy_asset_association.py | Needs seeded CMDB devices or services on the test tenant. |
| sr assets add | `PUT tickets/{id}/associate-ci {device_ids\|service_ids ints}; legacy PUT tickets/{id}/associate_assets {item_…` | UNSURE | stale (re-test) | no | capture-only | test_integrated_assets.py, test_legacy_asset_association.py, test_tui_inventory_writes.py | Needs seeded CMDB devices or services on the test tenant. |
| sr assets remove | `PUT tickets/{id}/disassociate-ci {cmdb_request_id}` | UNSURE | stale (re-test) | no | none | test_integrated_assets.py, test_legacy_asset_association.py | Needs seeded CMDB devices or services on the test tenant. |
| sr approvals list | `GET tickets/{id}/approval-groups + approval-chains (parallel model) or legacy flat GET tickets/{id}/approvals` | COVERED | yes (read) | n/a | read-verified | test_sr_approvals.py, test_approval_model.py, test_legacy_approvals.py | Live read verified. |
| sr approvals request | `POST tickets/{id}/approval-groups {approval_groups:[{name,approval_type,email_content,notified_to}]}; legacy…` | COVERED | yes | yes | capture-only | test_sr_approvals.py, test_approval_recipients.py, test_legacy_approvals.py, test_approval_model.py | Live send verified by an independent readback. |
| sr approvals remind | `PUT tickets/{id}/approvals/{aid}/resend (bodyless)` | COVERED | yes | yes | capture-only | test_sr_approvals.py, test_legacy_approvals.py | Live send verified by an independent readback. |
| sr approvals approve | `PUT tickets/{id}/approvals/{aid}` | COVERED | yes | yes | none | test_sr_approvals.py, test_legacy_approvals.py, test_approval_model.py | Live send verified by an independent readback. |
| sr approvals reject | `PUT tickets/{id}/approvals/{aid}` | COVERED | yes | yes | none | test_sr_approvals.py, test_approval_model.py | Live send verified by an independent readback. |
| sr approvals cancel | `PUT tickets/{id}/approval-groups/{gid}/cancel` | COVERED | yes | yes | none | test_sr_approvals.py, test_approval_model.py, test_legacy_approvals.py | Live send verified by an independent readback. |

## Cross-cutting (search, forms, filters, views, exports, auth, cache, reviewed-write engine)

| Command | Wire | Status | Live-verified | Readback independent | Production parity | Offline tests | Note |
|---|---|---|---|---|---|---|---|
| fsv search QUERY [--sort --all --npages --json] | `GET https://{tenant}/search/all?term= (client.search -> /search/{entity})` | UNSURE | yes (read) | n/a | none | test_search_routing.py | Read verified live; row-by-row comparison with the web UI pending. |
| tickets\|changes\|problems search QUERY | `GET /search/{tickets\|changes\|problems}` | UNSURE | yes (read) | n/a | none | test_search_routing.py | Read verified live; row-by-row comparison with the web UI pending. |
| assets search --kind devices | `GET https://{tenant}/itam/core/assets/api/ ?page` | UNSURE | yes (read) | n/a | none | test_integrated_assets.py, test_asset_picker.py | Needs seeded CMDB devices or services on the test tenant. |
| assets search --kind services | `GET https://{tenant}/itam/core/<services family>/api/` | UNSURE | yes (read) | n/a | none | test_integrated_assets.py | Needs seeded CMDB devices or services on the test tenant. |
| assets search --page N / pagination, total_rows contract | `as the row above` | UNSURE | yes (read) | n/a | none | test_integrated_assets.py | Needs more than one page of inventory to prove paging. |
| templates list | `GET /api/_/ticket-templates` | UNSURE | no | n/a | read-verified | test_templates_windows.py | Needs a seeded ticket template. |
| templates show NAME | `GET /api/_/ticket-templates/{id}` | UNSURE | no | n/a | none | test_templates_windows.py | Needs a seeded ticket template. |
| tickets create --template NAME (prefill) | `same GET then reviewed create POST tickets` | UNSURE | no | no | none | test_templates_windows.py, test_ticket_create.py | Needs a seeded ticket template. |
| attachments delete ID --owner tickets/N (root attachment) | `DELETE /api/_/attachments/{id}; owner readback GET tickets/N` | COVERED | yes | yes | none | test_attachments_reviewed.py, test_attachment_failure_matrix.py, test_attachment_capture_hardening.py | Live send verified by an independent readback. |
| attachments delete ID --owner tickets/N --child CONV_ID | `DELETE /api/_/attachments/{id}; readback child conversation/note` | COVERED | yes | yes | none | test_attachments_reviewed.py | Live send verified by an independent readback. |
| attachments delete ID --owner changes/N | `DELETE /api/_/attachments/{id}; readback changes/N` | UNSURE | stale (re-test) | no | none | test_attachments_reviewed.py | Live result predates later fixes or was partial; re-test. |
| attachments delete ID --owner problems/N | `as the row above` | COVERED | yes | yes | none | test_attachments_reviewed.py | Live send verified by an independent readback. |
| attachments delete ID --owner ... --section KEY (planning section) | `DELETE /api/_/attachments/{id}; readback planning section` | UNSURE | no | no | none | test_attachments_reviewed.py | Not yet exercised live (dry-run or offline only). |
| --attach on tickets reply / add-note | `POST /api/_/attachments (base64 JSON) then reply/note POST` | COVERED | yes | yes | none | test_attachments_reviewed.py, test_attachment_capture_hardening.py | Live send verified by an independent readback. |
| tickets create --attach / changes create --attach (before-create) | `POST /api/_/attachments then create` | UNSURE | stale (re-test) | no | none | test_attachments_reviewed.py | Live result predates later fixes or was partial; re-test. |
| --attach on forward / inline images in reply/note | `POST /api/_/solutions/inline_images / attachments` | UNSURE | no (dry-run only) | no | none | test_inline_images.py | Not yet exercised live (dry-run or offline only). |
| tickets\|changes\|problems download ID [--all] | `GET attachment URLs (signed) per record` | COVERED | yes (read) | n/a | none | test_download_attachments.py, test_tui_download_collision.py | Live read verified. |
| form fields MODULE [--choices F] | `GET {module} form fields (internal /api/_/{module}_form_fields) + ws/N/business_rules` | UNSURE | yes (read) | n/a | none | test_rules_workspace.py, test_content_fields.py | Read verified live; row-by-row comparison with the web UI pending. |
| form explain MODULE [--set --from --preset --id --all] | `same + rule evaluation (forms/rules.py)` | UNSURE | yes (read) | n/a | none | test_content_fields.py, test_rules_workspace.py | Read verified live; row-by-row comparison with the web UI pending. |
| form dump MODULE [--mode new\|edit] | `same reads; local snapshot` | UNSURE | yes (read) | n/a | none | test_changes_form_cli.py | Read verified live; row-by-row comparison with the web UI pending. |
| form diff SNAPSHOT [--against] | `snapshot vs current metadata` | UNSURE | yes (read) | n/a | none | test_changes_form_cli.py | Read verified live; row-by-row comparison with the web UI pending. |
| form * for problems / sr | `rules GET business_rules?module_type=problem` | GAP | yes (read) | n/a | read-verified | test_problem_workflows.py | Problem and SR form rules are not modelled. |
| schema loader: ticket/change/problem form fields + choices cache | `GET {module} form fields; business_rules; cached` | UNSURE | yes (read) | n/a | none | test_schema_cache.py, test_cache_config_hardening.py | Read verified live; row-by-row comparison with the web UI pending. |
| schema loader: SR catalog item form + place-request rules | `GET catalog/service-items/{id}; business_rules module_type=service_request` | COVERED | yes (read) | n/a | none | test_sr_catalog.py, test_catalog_safety.py | Live read verified. |
| tickets views | `GET /api/_/ticket_filters` | UNSURE | yes (read) | n/a | read-verified | test_views_cli.py, test_search_routing.py | Read verified live; row-by-row comparison with the web UI pending. |
| changes views | `GET /api/_/change-filters` | UNSURE | yes (read) | n/a | read-verified | test_views_cli.py | Read verified live; row-by-row comparison with the web UI pending. |
| problems views | `GET /api/_/problem-filters` | UNSURE | yes (read) | n/a | read-verified | test_views_cli.py | Read verified live; row-by-row comparison with the web UI pending. |
| tickets private-views show | `GET ticket_filters/{id}` | UNSURE | yes (read) | n/a | capture-only | test_private_views.py | Read verified live; needs a UI-created private view to compare. |
| tickets private-views write clone\|rename\|delete | `POST ticket_filters; PUT/DELETE ticket_filters/{id}` | UNSURE | no | no | none | test_private_views.py | Needs private-view metadata (a UI-created private view). |
| --view NAME / default view | `filters_path GET then list GET` | UNSURE | yes (read) | n/a | read-verified | test_views_cli.py | Read verified live; row-by-row comparison with the web UI pending. |
| tickets columns list | `GET /api/_/tickets/headers` | COVERED | yes (read) | n/a | capture-only | test_list_preferences.py | Live read verified. |
| tickets columns set | `POST /api/_/tickets/headers (ordered map + frozen_columns); shared account pref` | COVERED | yes | yes | capture-only | test_list_preferences.py | Live send verified by an independent readback. |
| tickets export | `POST /api/_/tickets/export (CSV / date_filter30)` | UNSURE | no (dry-run only) | no | capture-only | test_list_preferences.py | Review-only by design: a send emails data to the signed-in agent and cannot be undone. |
| changes columns list | `GET /api/_/changes/headers` | COVERED | yes (read) | n/a | capture-only | test_list_preferences.py | Live read verified. |
| changes columns set | `POST /api/_/changes/headers (ordered map + frozen_columns); shared account pref` | COVERED | yes | yes | capture-only | test_list_preferences.py | Live send verified by an independent readback. |
| changes export | `POST /api/_/changes/export (CSV / date_filter30)` | UNSURE | no (dry-run only) | no | capture-only | test_list_preferences.py | Review-only by design: a send emails data to the signed-in agent and cannot be undone. |
| problems columns list | `GET /api/_/problems/headers` | COVERED | yes (read) | n/a | capture-only | test_list_preferences.py | Live read verified. |
| problems columns set | `POST /api/_/problems/headers (ordered map + frozen_columns); shared account pref` | COVERED | yes | yes | capture-only | test_list_preferences.py | Live send verified by an independent readback. |
| problems export | `POST /api/_/problems/export (CSV / date_filter30)` | UNSURE | no (dry-run only) | no | capture-only | test_list_preferences.py | Review-only by design: a send emails data to the signed-in agent and cannot be undone. |
| tickets canned folders | `GET /api/_/canned_response_folders` | COVERED | yes (read) | n/a | read-verified | test_canned.py | Live read verified. |
| tickets canned list FOLDER | `GET canned_response_folders/{id}/canned_responses` | COVERED | yes (read) | n/a | none | test_canned.py | Live read verified. |
| tickets canned show ID | `GET canned_responses/{id}` | COVERED | yes (read) | n/a | none | test_canned.py | Live read verified. |
| tickets canned reply TICKET ID | `reviewed reply POST conversations with literal HTML` | UNSURE | no | no | capture-only | test_canned.py | Needs a canned response with placeholders. |
| tickets scenarios list | `GET /api/_/scenario_automations` | COVERED | yes (read) | n/a | read-verified | test_canned.py | Live read verified. |
| tickets scenarios show ID | `GET scenario_automations (filter by id)` | COVERED | yes (read) | n/a | read-verified | test_canned.py | Live read verified. |
| tickets scenarios apply TICKET ID | `PUT /api/_/tickets/execute_scenario` | UNSURE | no | no | capture-only | test_canned.py | Partial live proof; a scenario may run workflows and send mail. |
| auth status | `GET /api/v2/agents/me + session check` | COVERED | yes (read) | n/a | none | test_session_store.py, test_client_session.py | Live read verified. |
| auth login [--domain --header --browser --store --force] | session file/Keychain; browser cookie capture | UNSURE | n/a (local) | n/a | none | test_session_store.py, test_client_session.py | Interactive or machine-local; not run by automated live tests. |
| auth logout | wipes session file+Keychain | UNSURE | n/a (local) | n/a | none | test_session_store.py | Interactive or machine-local; not run by automated live tests. |
| cache status | local | COVERED | n/a (local) | n/a | none | test_cache_cli.py, test_cache_config_hardening.py | Local command; offline tests. |
| cache refresh | `GETs schema/views/groups` | COVERED | yes (read) | n/a | none | test_cache_cli.py, test_schema_cache.py | Live read verified. |
| cache clear [schema\|views\|groups\|all] | local delete | COVERED | n/a (local) | n/a | none | test_cache_cli.py, test_cache_config_hardening.py | Local command; offline tests. |
| completion show | local shell/cache | COVERED | n/a (local) | n/a | none | test_completion_gen.py, test_completion_matrix.py, test_completion_autocomplete.py, test_completion_coverage_subprocess.py, test_completion_shell_fallback.py | Local command; offline tests. |
| completion install | local shell/cache | UNSURE | n/a (local) | n/a | none | test_completion_gen.py, test_completion_matrix.py, test_completion_autocomplete.py, test_completion_coverage_subprocess.py, test_completion_shell_fallback.py | Interactive or machine-local; not run by automated live tests. |
| completion refresh | local shell/cache | COVERED | n/a (local) | n/a | none | test_completion_gen.py, test_completion_matrix.py, test_completion_autocomplete.py, test_completion_coverage_subprocess.py, test_completion_shell_fallback.py | Local command; offline tests. |
| completion doctor | local shell/cache | COVERED | n/a (local) | n/a | none | test_completion_gen.py, test_completion_matrix.py, test_completion_autocomplete.py, test_completion_coverage_subprocess.py, test_completion_shell_fallback.py | Local command; offline tests. |
| journal ls | local journal file (see `fsv journal path`) | COVERED | n/a (local) | n/a | none | test_journal_editor.py | Local command; offline tests. |
| journal tail | local journal file (see `fsv journal path`) | COVERED | n/a (local) | n/a | none | test_journal_editor.py | Local command; offline tests. |
| journal path | local journal file (see `fsv journal path`) | COVERED | n/a (local) | n/a | none | test_journal_editor.py | Local command; offline tests. |
| config defaults show\|set\|clear (default --where) | local config | COVERED | n/a (local) | n/a | none | test_universal_config.py, test_list_preferences.py | Local command; offline tests. |
| {module} ls --where / --or / --no-defaults / --debug | `GET {module} list with query_hash params (filter=...)` | UNSURE | yes (read) | n/a | read-verified | test_filter_wires.py, test_strict_filter_contract.py, test_query_matrix.py, test_query_edge_cases.py, test_query_hash_params.py, test_list_filter_surface.py, test_ticket_type_filter.py | Read verified live; row-by-row comparison with the web UI pending. |
| --where text filters (subject/description) fail closed | refused client-side | UNSURE | n/a (local) | n/a | none | test_strict_filter_contract.py | Refused client-side; nothing is sent. |
| --where type=NAME (name not id) | `query_hash value:['Service Request']` | UNSURE | yes (read) | n/a | read-verified | test_ticket_type_filter.py | Read verified live; row-by-row comparison with the web UI pending. |
| --where date ranges, relative dates, numbers, multi-select | `query_hash conditions` | COVERED | yes (read) | n/a | capture-only | test_filter_wires.py | Live read verified. |
| --where approval_status on tickets (refused), changes (allowed) | `query_hash` | COVERED | yes (read) | n/a | none | test_strict_filter_contract.py | Live read verified. |
| --query-hash RAW | `query_hash raw JSON` | UNSURE | yes (read) | n/a | none | test_query_hash_params.py | Live run found a bug, since fixed; re-test pending. |
| reviewed-write: --dry-run emits plan_hash | reviewed_write.execute / review.py | COVERED | n/a (local) | n/a | capture-only | test_write_safety.py, test_reviewed_cli_output.py, test_reviewed_snapshot.py | Local command; offline tests. |
| reviewed-write: --yes --expect-plan guard + stale hash refusal | `as the row above` | COVERED | yes | yes | none | test_write_safety.py | Live send verified by an independent readback. |
| reviewed-write: plan hash determinism | journal.snapshot_hash | COVERED | n/a (local) | n/a | none | test_reviewed_snapshot.py | Local command; offline tests. |
| reviewed-write: bulk plan hash reporting | ticket_bulk | COVERED | n/a (local) | n/a | none | test_ticket_bulk.py | Local command; offline tests. |
| reviewed-write: readback verification (verified true/false) | readback.py, per-command verify callbacks | UNSURE | n/a (local) | n/a | none | test_readback_normalization.py, test_readback_values.py, test_conversation_recipient_readback.py | Local or engine behaviour; proven through the module rows and offline tests. |
| reviewed-write: unknown-outcome handling + journal record | execute() journal pending/failed | COVERED | n/a (local) | n/a | none | test_journal_editor.py, test_solution_unlink_http.py | Local command; offline tests. |
| reviewed-write: legacy un-hashed writes (reply, problems update, tickets update… | body-only preview | GAP | n/a (local) | n/a | none | test_legacy_write_guards.py | Older writes preview the body without a plan hash. |

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
