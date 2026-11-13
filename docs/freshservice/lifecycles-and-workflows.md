# Freshservice lifecycles, status models, workflows and SLA effects

**What this is.** How records move through statuses in Freshservice: the
ticket, service-request and problem status models, the change lifecycle
(state flows with transitions, entry criteria and per-status field locks),
and the server-side automation (workflow automators, SLA policies,
time-based rules) whose side effects change what a client reads back after a
write.

**Who it is for.** Anyone writing status changes or verifying writes against
Freshservice, and anyone replicating a production lifecycle on a test tenant.

**Verified on:** a Freshservice Enterprise trial tenant, 2026-11-11 to
2026-11-13 (with a lifecycle and automations configured to imitate a
production-like tenant), plus read-only reads of a production-like
enterprise tenant's metadata and activity feeds. Status names, ids and
workflow names below are invented examples.

**Confidence tags:** **[VERIFIED LIVE]** sent and read back;
**[CAPTURED]** request shape seen, not executed live; **[INFERRED]**
reasoned from metadata, code, activity text or UI. *Read-only* means a GET
on a production-like tenant.

## 1. Tickets (incidents)

* Platform default statuses: **Open 2, Pending 3, Resolved 4, Closed 5**.
  Tenants add custom statuses (for example "Waiting on vendor" with some
  higher id). Each status choice says whether the SLA timer runs in it.
  [VERIFIED LIVE]
* There is **no transition graph** for tickets: any status the form offers
  can be set, subject to rules and closure requirements. [VERIFIED LIVE]
* Resolving or closing requires the fields flagged `required_for_closure`
  that remain visible after the edit-form rules run, and on many tenants a
  **resolution note**. The resolution note is saved by its own request
  before the status change (see [wire contracts](wire-contracts.md#resolve-and-close)).
  [VERIFIED LIVE]
* A business rule can make a field required for a non-closing status too
  (for example a "reason for pending" field when moving to Pending).
  [VERIFIED LIVE]
* "Send and set status" in the reply or note composer sends one request
  with the status nested in the conversation body. [VERIFIED LIVE]
* **Resolve/close auto-fills planned dates**: resolving or closing a ticket
  without planned dates gives it a short planned start/end window (about 15
  minutes). [VERIFIED LIVE]
* Reopen is an ordinary status write back to Open. [VERIFIED LIVE]
* Merge closes the secondary tickets (status 5) and moves their
  conversations to the primary; there is no unmerge. [VERIFIED LIVE]
* **Priority can be derived**: on an incident, a PUT of `priority` alone
  returned 200 but the stored priority stayed at the value implied by
  urgency and impact (a priority matrix). Change urgency/impact, or verify
  the field you sent. [VERIFIED LIVE, mechanism INFERRED]
* Time-based "supervisor" rules (for example "close resolved tickets after
  an hour") act later, without a user request, and appear in the activity
  feed. [VERIFIED LIVE, read-only]

## 2. Service requests

* Service requests share the ticket status model and add **requested items**
  with their own fulfilment **stage** (for example 1 Requested, 2 Delivered;
  read the stage names from the item) changed by a separate request.
  [VERIFIED LIVE]
* Catalog items carry little lifecycle configuration themselves (subject
  template, attachment-mandatory flag, document generation). On a typical
  enterprise tenant, approvals, status flips, group assignment, tasks and
  notifications for service requests are driven by **workflow automators**
  triggered on "service request is created" and "approval" events.
  [VERIFIED LIVE, read-only]
* Approved or rejected approvals are immutable history; only pending ones can
  be cancelled (parallel-approval model). [VERIFIED LIVE]

## 3. Problems

* Platform defaults: **Open 1, Change Requested 2, Closed 3**; tenants add
  custom statuses (for example "Resolve", "Monitoring", "For Deploy"). The
  same label can have **different ids on two tenants**; resolve by label.
  [VERIFIED LIVE]
* No transition graph: any status can be set. [VERIFIED LIVE]
* **Closure rules** (server-side, configurable, not readable as metadata):
  "analysis completed" (root cause, impact and symptoms sections filled) and
  "workaround or solution added" (a linked knowledge article). Violations
  answer HTTP 400 with `violated_closure_rules`. A tenant may enable one,
  both or neither; local pre-checks should warn rather than hard-refuse
  unless the rule was proven on that tenant. [VERIFIED LIVE / INFERRED]
* Closing a problem rewrites its planned start/end dates, even non-empty
  ones. [VERIFIED LIVE]
* Known error is a boolean on the record (`known_error`), set by a property
  update. [VERIFIED LIVE]
* A deleted (trashed) problem rejects updates with **HTTP 405**.
  [VERIFIED LIVE]

## 4. Changes: lifecycles (state flows)

### Change types and flows

* `change_type` choices are tenant-configurable; common ones are Normal 1,
  Standard 2, Emergency 4. Each type may be bound to a **state flow**
  (lifecycle); types without one have `state_flow: {}` and behave like the
  plain status list. [VERIFIED LIVE]
* The create form calls
  `GET /api/_/ws/<ws>/change-management/matching-lifecycle-info?change_type=<n>`
  to learn the flow for the chosen type. [VERIFIED LIVE]
* A change records its flow in `state_flow_id` and the states it passed
  through in `state_traversal`. [VERIFIED LIVE]
* The flow definition is
  `GET /api/_/ws/<ws>/change_management/<flow id>?include=state_info`
  (plain `/api/_/change_management/<flow id>` on tenants without
  workspaces). Agents without admin access may get 403 on the collection
  but can still read the flow of a change. [VERIFIED LIVE]
* A stock trial flow is linear (Open → Planning → Awaiting Approval →
  Pending Release → Pending Review → Closed). A production-like flow can
  have many more states, branches, a rejection loop back to Open and
  separate flows per change type. [VERIFIED LIVE]

### Flow structure

```json
{
  "state_flow": {
    "id": 77, "name": "Normal",
    "state_list": [{"id": 1, "value": "Open"}, {"id": 2, "value": "Planning"}, {"id": 7, "value": "Peer Review"}],
    "transitions": {
      "1": [{"to_state": 2, "entry_criteria": [{"name": "department_id", "operator": "is_not_empty"}]}],
      "2": [{"to_state": 7, "entry_criteria": [{"fields": {"501": {"name": "customFields.region", "operator": "is_not_empty"}}}]}]
    },
    "state_properties": {
      "field_lock_config": {"7": [{"501": "customFields.region"}, {"0": "status"}]}
    }
  }
}
```

(Invented values and operator names; key structure as observed.) [VERIFIED LIVE]

* `transitions` is keyed by the from-state; each edge lists `to_state` and
  `entry_criteria`. Criteria come in two shapes: flat `{name, operator,
  value}` or `{fields: {<field id>: {name, operator, value}}}`.
  [VERIFIED LIVE]
* Typical criteria: a field must be set (department, a custom field), the
  approval status must be approved, or all tasks must be completed.
  [VERIFIED LIVE, read-only]
* `field_lock_config` lists, per state, the fields locked while the change
  is in that state. A lock on `status` freezes the change in that state
  (only automation moves it). Locks can cover custom fields, default fields
  and planning sections; the detail page shows such inputs disabled with a
  "locked for current status" message and disables the matching "Add
  <planning section>" menu items. [VERIFIED LIVE]

### What the server does

* A status write that is not an edge, has unmet entry criteria, or touches a
  locked field answers **HTTP 400 without a useful reason**. A client should
  pre-check edges, criteria it can evaluate from the record, and locks
  before sending, and warn about criteria it cannot evaluate.
  [VERIFIED LIVE]
* Unlocked fields can still be updated in a state that locks others.
  [VERIFIED LIVE]
* Records stay bound to the flow (version) they were created with. When an
  admin archives or rebuilds a flow, older records can point at a flow
  whose read fails (HTTP 500 was seen on a trial with a damaged flow).
  Export a flow before editing it and note which records it binds.
  [VERIFIED LIVE / INFERRED]

## 5. Tasks

* Task statuses come from the task-field metadata (stock: Open, In
  Progress, Completed, Blocked; tenants add more, for example Canceled).
  Ids are tenant data. [VERIFIED LIVE]
* **Completing a task has side effects**: it sets `closed_at`, stops the
  task OLA timer (`ola_stopped_at`), and rewrites `planned_start_date` /
  `planned_end_date` to around the completion time. [VERIFIED LIVE]
* A lifecycle entry criterion can require every task to be completed before
  the change can move on. [VERIFIED LIVE, read-only]

## 6. Workflow automators

Workflow automators are server-side flows configured per module. On a
typical enterprise tenant they drive much of the lifecycle; on a fresh trial
there are none. [VERIFIED LIVE, read-only]

* **Triggers** seen: record created, status changed (from any to any, or
  between specific statuses), field updated, approval event (approved,
  rejected, cancelled). [VERIFIED LIVE, read-only]
* **Actions** seen: set status or other fields (department, custom fields),
  send emails to people or groups, request approvals (creating legacy
  approval levels), add tasks with group, due date and reminder, start a
  sub-flow, wait for a later event. [VERIFIED LIVE, read-only]
* They run **asynchronously** after the triggering write. An immediate
  readback can show the agent's value, and a later one the workflow's.
  [VERIFIED LIVE]

### How workflows change readbacks

| Write | What a workflow can do | Seen as |
|---|---|---|
| Set a status | rewrite it to another status (for example every problem status change ends in "Monitoring") | write X, read Y; `verified: false` unless the client knows |
| Create a change | stamp department or a custom field, add tasks | fields and tasks the client never sent |
| Approve | move the change to a later state and email a team | status changes after the approval call |
| Cancel an approval | set the change to Rejected | status change after the cancel |
| Save a note, then change status (two writes) | change routing in between, so the second write is no longer valid | the second write is never sent |

[VERIFIED LIVE on the replicated trial; patterns VERIFIED LIVE, read-only
on activity feeds]

### Verifying writes under workflows

* Read the **activity feed**. The agent's row says, for example, "set status
  as **Resolve**" in `content`; a workflow row's `content` says "executed
  <workflow> workflow from <event> event" and the field it set is in
  `sub_contents`. [VERIFIED LIVE]
* Treat "my write is recorded, and a newer workflow entry set the value" as
  verified-with-note. Without both entries, the outcome is unverified; do
  not resend. [VERIFIED LIVE]
* Report multi-write operations by phase (first write sent, first verified,
  second sent) so "the second write was never attempted" is not reported as
  an unknown outcome. [VERIFIED LIVE]

## 7. SLA and due dates

* SLA policies (per priority, group, category and so on, with business-hours
  calendars) set `due_by` and first-response due dates on tickets and
  problems. Changing priority recomputes them: on a trial, a priority change
  moved a problem's due date by a month. [VERIFIED LIVE]
* Status choices carry an SLA-timer flag: moving to a status whose timer is
  off (typically Pending) pauses the clock. [VERIFIED LIVE / INFERRED]
* Changes carry `due_by` and `manual_dueby`; tasks carry `due_date`,
  `task_due_status` text and an OLA timer. [VERIFIED LIVE]
* A production-like tenant can have several SLA policies plus workflow-set
  SLA fields; the activity feed names the policy that applied.
  [VERIFIED LIVE, read-only]

## 8. Consequences for a client

1. Resolve every status, task status and reminder by label from fresh
   metadata; never compare against numeric literals. [VERIFIED LIVE]
2. Pre-check change transitions, entry criteria and field locks from the
   state flow; the server's 400 does not say why. [VERIFIED LIVE]
3. Expect workflow side effects; verify through the activity feed instead
   of field equality alone, and never resend because a readback differs.
   [VERIFIED LIVE]
4. Expect closure requirements (fields, resolution note, problem closure
   rules) and server-written dates on close. [VERIFIED LIVE]
5. To test any of this, replicate the lifecycle, rules, workflows and SLA on
   a test tenant first; see [live testing](../live-testing.md#trial-replication).
