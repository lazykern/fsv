# Freshservice: product model, API families and the agent web UI

**What this is.** A platform-level description of how Freshservice is put
together as seen by a browser-based agent client: the modules, the internal
API families the agent web app calls, how a session authenticates, how
records are identified and how the web UI lays them out. It also explains
how to probe which plan features a tenant has.

**Who it is for.** Anyone (person or coding agent) extending `fsv` or writing
another client that drives Freshservice through the same internal API the
agent web UI uses, without access to any private capture.

**Verified on:** a Freshservice Enterprise trial tenant, 2026-11-11 to
2026-11-13, plus read-only (GET-only) observations of a production-like
enterprise tenant. All examples use synthetic values (Acme, `example.com`,
small invented ids).

**Confidence tags** used on every statement:

* **[VERIFIED LIVE]**: the request was sent and the result independently read
  back (on the trial unless marked *read-only*, which means a GET on a
  production-like tenant).
* **[CAPTURED]**: the request shape was seen leaving the browser (often with
  delivery aborted), but this exact request was not executed live.
* **[INFERRED]**: reasoned from metadata, UI text, code or several
  observations; not directly proven.

Related pages: [forms and rules](forms-and-rules.md),
[lifecycles and workflows](lifecycles-and-workflows.md),
[wire contracts](wire-contracts.md), [quirks and pitfalls](quirks-and-pitfalls.md),
[live-testing methodology](../live-testing.md).

## 1. Product model

### Workspaces

* A tenant has one or more **workspaces** (for example "IT" and "HR"). Most
  records carry `workspace_id`; form fields, business rules and many admin
  paths are scoped per workspace (`/api/_/ws/<workspace id>/...`).
  [VERIFIED LIVE]
* A tenant can be single-workspace in practice while still exposing a global
  workspace (often id 1, used for shared things such as default knowledge
  base folders) and the service-desk workspace (often id 2). Do not assume
  either number: read `GET /api/_/workspaces` or `GET /api/_/bootstrap/me`
  (`current_workspace_id`, `primary_workspace_id`, `workspace_ids`).
  [VERIFIED LIVE]
* The ticket and change list filters add a workspace condition automatically
  (see [wire contracts, filters](wire-contracts.md#filters)). [CAPTURED]

### Modules

| Module | Display prefix | What it holds | Notes |
|---|---|---|---|
| Tickets: incidents | `INC-` | break/fix tickets | Same table as service requests; `type` distinguishes them. [VERIFIED LIVE] |
| Tickets: service requests | `SR-` | catalog requests, each with one or more *requested items* | Converting a ticket between Incident and Service Request keeps the number and flips the prefix. [VERIFIED LIVE] |
| Problems | `PRB-` | root-cause records, known errors, solution links | Planning-style sections for root cause, impact, symptoms. [VERIFIED LIVE] |
| Changes | `CHN-` | change requests with a lifecycle (state flow), planning sections, approvals, tasks | The most rule- and workflow-heavy module. [VERIFIED LIVE] |
| Releases | `REL-` | release records linked to changes | Create and link proven; little else. [VERIFIED LIVE] |
| Tasks | `TSK-` | child tasks of tickets, problems, changes | Task fields are separate metadata per parent module. [VERIFIED LIVE] |
| Projects | (project key, e.g. `ACME-1`) | Freshrelease-backed projects and project tasks | Served by a separate host (below). [VERIFIED LIVE] |
| Assets / CMDB | asset display ids, inventory ids | devices, business services, physical assets | Two different API generations exist (legacy asset model vs integrated ITAM inventory). [VERIFIED LIVE] |
| Knowledge base | articles | categories, folders, articles (draft/published, workaround/permanent-solution types) | Problems link to articles as solutions. [VERIFIED LIVE] |
| Service catalog | catalog item display ids | item forms that create service requests | Each item has its own field set and its own rules. [VERIFIED LIVE] |

Display prefixes are configurable on newer tenants, so a client should read
`human_display_id` from the record rather than build it from a fixed prefix.
[INFERRED]

## 2. API families

The agent web app is an Ember single-page app. It talks to several path
families on the same tenant host, `https://<tenant>.freshservice.com`:

| Family | Example | Used for | Notes |
|---|---|---|---|
| Internal agent API `/api/_/` | `GET /api/_/tickets/42`, `PUT /api/_/changes/7` | nearly everything the agent UI does: records, conversations, tasks, approvals, filters, form metadata, business rules, bulk actions | Not a published, versioned API. Shapes are whatever the current web app needs. Writes need the CSRF header. [VERIFIED LIVE] |
| Workspace-scoped internal `/api/_/ws/<n>/` | `GET /api/_/ws/2/ticket_form_fields`, `GET /api/_/ws/2/business_rules?...` | form metadata, rules, lifecycle info, email templates | Some tenants answer only the scoped path, others only the plain one; try both. [VERIFIED LIVE] |
| Public REST `/api/v2/` | `GET /api/v2/agents/me`, `GET /api/v2/changes/7/tasks` | some reads; the session cookies are accepted for GETs | Returns UTC timestamps (`Z`) where the internal API returns the account's offset; some keys differ (`agent_id` in v2 vs `owner_id` internally on tasks). [VERIFIED LIVE] |
| Classic admin pages `/ws/<n>/admin/...` | `POST /ws/2/admin/ticket_templates` | a few admin features still served as Rails forms | `multipart/form-data` with a hidden CSRF field, answers with a redirect. [VERIFIED LIVE] |
| ITAM inventory `/itam/core/...` | `GET /itam/core/device/api/?page=1&page_size=50` | integrated asset inventory (devices, assets, business services) | Different envelope (`{page, error, total_rows, data}`) and identity model from `/api/_/`. [VERIFIED LIVE] |
| Search | `GET /search/all?term=...`, `GET /api/_/search/autocomplete/requesters?q=...` | global search box, people pickers | Global search is the only text search; list filters cannot filter on subject text. [VERIFIED LIVE] |
| Freshrelease (projects) | `https://<tenant>.freshrelease.com/<KEY>/issues` | the project module, embedded in Freshservice as an iframe | Freshservice proxies link/unlink through `/api/_/freshrelease/settings/...`. [VERIFIED LIVE] |
| Freshworks identity | `https://<org>.myfreshworks.com/api/v2/session` | login and session heartbeat | Separate org-level host. [VERIFIED LIVE] |
| Attachment CDN | `https://<tenant>.attachments.freshservice.com/...` (files), a shared inline-image host for rich-text images | downloads | Signed, expiring URLs. [VERIFIED LIVE] |

Rules of thumb:

* Follow the path family the web UI uses for a feature. The same record can
  be readable through `/api/v2` and `/api/_/`, but write shapes, accepted
  keys and side effects differ. [VERIFIED LIVE]
* The legacy Freshservice Projects API (`/api/_/projects`, `/api/v2/projects`)
  answers 403 `require_feature` on a tenant that uses Freshrelease projects;
  the project UI is `/a/project_management/projects`, an iframe to the
  Freshrelease host. [VERIFIED LIVE]

## 3. Authentication and session (browser client view)

* There are **no API keys** in this model. The client holds the browser
  session cookies of a signed-in agent (including the Freshworks
  `fw-session-id` cookie) and sends them to the tenant host. [VERIFIED LIVE]
* GETs need only the cookies. **Writes to `/api/_/` need an `X-CSRF-Token`
  header.** The token is in `<meta name="csrf-token" content="...">` on any
  agent app page (`/a/dashboard`, `/a/tickets`, `/a/changes`). [VERIFIED LIVE]
* JSON writes send `Content-Type: application/json` even when there is no
  body (bodyless PUT/DELETE); one endpoint answered **415** without it.
  [VERIFIED LIVE]
* An expired session shows up as a redirect (3xx) to the identity host, a 401,
  an HTML page where JSON was expected, or a 403 on every endpoint. A genuine
  permission 403 on one endpoint leaves the session usable; telling them
  apart needs a cheap probe of an app page. [VERIFIED LIVE]
* The session can be kept alive with a heartbeat `POST` to the identity
  host's `/api/v2/session`. [VERIFIED LIVE]
* After a 403 on a write, do not refresh the token and replay the write: the
  first request may have had a side effect. Resolve the token before sending.
  [INFERRED]
* Rate limiting: bursts of hundreds of GETs produce HTTP **429**. Clients
  should back off (honour `Retry-After` when present) and space sequential
  reads. [VERIFIED LIVE, read-only]

## 4. Ids: internal id vs display id

* Tickets, problems, changes and releases are addressed in `/api/_/` paths
  by their **display number** (`INC-42` → `/api/_/tickets/42`). The same
  record also has a large internal id visible in some payloads (for example
  a change's `change_id` in native metadata, or `itil_change.id` in the
  maintenance-window matching request). Use the one each endpoint expects.
  [VERIFIED LIVE]
* Child objects (conversations, tasks, time entries, approvals, attachments,
  requested items, association links) have their own ids, returned by the
  create call or the parent's list read. [VERIFIED LIVE]
* Catalog items have an internal `id` and a `display_id`; the place-request
  path uses the display id, business-rule tokens use the internal id.
  [VERIFIED LIVE]
* Legacy assets have a `display_id` (used in legacy association writes);
  integrated inventory items have an ITAM id, and each association has its
  own link id. They are not interchangeable. [VERIFIED LIVE / CAPTURED]
* Choice metadata often carries both `id` (the write enum) and `internal_id`
  or `display_id`. Status writes use the choice `id`, never `internal_id`.
  [VERIFIED LIVE]
* Ids may arrive as JSON strings in some metadata (task status choices are
  `"1"`, `"2"`, ...) and as numbers elsewhere; writes have been accepted as
  integers. See [quirks](quirks-and-pitfalls.md#ids-strings-and-numbers).
  [VERIFIED LIVE]

## 5. How records appear in the agent web UI

### List views

* Each module has system views (for example "My open", "Unassigned",
  "Closed", "Trash", "All", "I'm watching") plus per-user saved views. View
  definitions come from `GET /api/_/ticket_filters`, `GET /api/_/change-filters`
  and the problem equivalent. [VERIFIED LIVE]
* **System views are time-boxed**: they carry `created_at is_greater_than
  six_months` (or similar). A tenant with no recent records shows empty
  system views even though records exist. [VERIFIED LIVE, read-only]
* The list request is a GET with `filter=<view>`, `query_hash=` (basic panel)
  or `advanced_query_hash=` (advanced builder), `order_by`, `order_type`,
  `page`, `per_page` (max 100) and `include=` for embedded objects.
  [CAPTURED]
* Columns come from `GET /api/_/<module>/headers`; the user's column choice
  is saved with `POST /api/_/<module>/headers`. Some columns (for example a
  ticket's approval status) exist as columns but not as filters.
  [CAPTURED]

### Detail page

* Header actions: watch (star), Edit, Update/Associate menus, a "More"
  (kebab) menu with items such as Add time, Print, Merge, Delete, Mark as
  spam; which items appear depends on role and module. [VERIFIED LIVE]
* Tabs come from `GET /api/_/<module>/<id>/tabs` (with counts). Typical tabs:
  Details, Notes or Conversations, Tasks, Approvals, Associations,
  Assets/CMDB (named "inventory and services" on integrated-ITAM tenants),
  Activity, and module extras (Risk, DevOps, Resolution). [VERIFIED LIVE]
* **Properties panel** (right side): status, priority, group, agent and the
  custom fields, rendered from form metadata and business rules, with
  disabled inputs where a lifecycle field lock or a rule disables them.
  Edits send only the changed fields. [VERIFIED LIVE]
* **Conversations** (tickets): replies, forwards, private and public notes,
  each with its own id, recipients, attachments and `private` flag; read via
  `GET /api/_/tickets/<id>/conversations`. Problems and changes have
  **notes** instead (`/notes`). [VERIFIED LIVE]
* **Planning sections** (changes, problems): rich-text sections with optional
  files, keyed by a semantic name (for problems: root cause, impact,
  symptoms). [VERIFIED LIVE]
* **Tasks**: a list with status, owner, group, dates, reminder. [VERIFIED LIVE]
* **Approvals**: either approval chains/groups (parallel approvals) or a flat
  list of approvals with levels (legacy model). [VERIFIED LIVE]
* **Associations**: linked tickets, problems, changes, releases, projects,
  parent/child tickets, assets and services. [VERIFIED LIVE]
* **Activity**: an audit feed of field changes, notes, tasks, approvals and
  workflow executions (`GET /api/_/<module>/<id>/activities`). Workflow rows
  read "executed <workflow> workflow from <event> event" with the actions in
  `sub_contents`. [VERIFIED LIVE]

Opening a ticket page can itself write: the reply composer autosaves a draft
(`POST /api/_/tickets/<id>/reply_draft`) during load. [VERIFIED LIVE]

## 6. Plan and feature flags, and how to probe them

The same product behaves differently per tenant depending on plan and
account features. Probe; do not hardcode.

| Feature | Probe | What a missing feature looks like |
|---|---|---|
| Account feature list | `GET /api/_/bootstrap/account` → `subscription` (state, plan), `features` (a list of flag names), `temp_features`, `workspace_features` | absent flag name [VERIFIED LIVE] |
| Your privileges | `GET /api/_/bootstrap/me` → privileges per module (for example view-only on problems) | UI hides create/edit controls; writes answer 403 [VERIFIED LIVE, read-only] |
| Parallel approvals (approval groups and chains) | `GET /api/_/changes/<id>/approval-chains` | **HTTP 400** "Parallel approvals feature needs to be enabled for the requested action" → the tenant uses the legacy flat model [VERIFIED LIVE, read-only] |
| Legacy CMDB asset picker | `GET /api/_/assets-to-associate?entity=change&entity_id=<id>&include_meta=true` | **403** `require_feature` "Cmdb ... not supported in your plan" on an integrated-ITAM tenant [VERIFIED LIVE] |
| Integrated ITAM inventory | `GET /itam/core/device/api/meta/` | 404/403 when the tenant uses the legacy model [INFERRED] |
| Legacy projects | `GET /api/_/projects` | 403 `require_feature` when projects are Freshrelease-backed [VERIFIED LIVE] |
| Templates | `GET /api/_/ticket-templates`, `GET /api/_/change-templates` | an empty list; `meta.max_template_count` is the **current count**, not a plan cap [VERIFIED LIVE] |
| Problem/release templates | `GET /api/_/problem-templates` | 404: not a product resource [VERIFIED LIVE] |
| Maintenance windows | `GET /api/_/maintenance_windows` | 403 for roles without access; some `change_windows` reads answer 500 even when the feature works [VERIFIED LIVE] |
| Change lifecycle | `GET /api/_/ws/<n>/change-management/matching-lifecycle-info?change_type=<n>` | `state_flow: {}` for change types without a lifecycle [VERIFIED LIVE] |
| Business rules | `GET /api/_/ws/<n>/business_rules?...` | 403/404 (unavailable) or a persistent 500 for one module on some tenants [VERIFIED LIVE, read-only] |

Typical flag names seen in `features` include templates, request-for-change,
CAB meetings, Freshrelease projects and business hours. Treat the list as a
hint and confirm with an endpoint probe, because a flag does not show which
API generation the tenant uses. [VERIFIED LIVE / INFERRED]

## 7. Where to go next

* What a form allows and why writes must evaluate rules:
  [forms and rules](forms-and-rules.md).
* Status models, lifecycles, workflow side effects and SLA:
  [lifecycles and workflows](lifecycles-and-workflows.md).
* Exact request shapes per feature: [wire contracts](wire-contracts.md).
* Gotchas: [quirks and pitfalls](quirks-and-pitfalls.md).
* How these facts were established and how to probe a new tenant:
  [live-testing methodology](../live-testing.md).
