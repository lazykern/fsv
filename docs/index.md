# Documentation index

- [Command cheat sheet](commands.md) — common commands and safe help examples for the full command surface.
- [Change approvals](change-approvals.md) — review, request, respond to, and cancel change approvals.
- [Change associations](change-associations.md) — review and verify change–ticket links.
- [Change tasks](change-tasks.md) — create, update, and delete reviewed change tasks.
- [Forms](forms.md) — form metadata, business rules, drafts, and review.
- [Form UI](formui.md) — interactive form controls, keys, and implementation.
- [Incident CC](incident-cc.md) — incident creation and CC recipient handling.
- [Incidents](incidents.md) — incident creation and validation limits.
- [Reviewed operations](reviewed-parity.md) — reviewed coverage, readback, freshness, and legacy-write safety boundaries.
- [Service requests](service-requests.md) — catalog forms, drafts, creation, and reading requests.
- [Service-request recipients](sr-recipients.md) — service-request recipient handling and verification.
- [Ticket conversation options](ticket-conversation-options.md) — reply recipients and send-and-status behavior.
- [Ticket lifecycle](ticket-lifecycle.md) — reviewed ticket status and resolution flows.
- [Ticket notes](ticket-notes.md) — reviewed notes, visibility, and delivery.
- [Ticket properties](ticket-properties.md) — reviewed ticket tags and planned dates.
- [Views](views.md) — saved views, filters, lists, detail pages, and TUI behavior.
- [This index](index.md) — links to every page in this documentation directory.

## Freshservice knowledge base

Platform behaviour of Freshservice as seen by an agent web client, with every
statement tagged VERIFIED LIVE, CAPTURED or INFERRED:

- [Overview](freshservice/overview.md) — product model, modules, API families, session and CSRF, ids vs display ids, how records appear in the web UI, feature-flag probes.
- [Forms and rules](freshservice/forms-and-rules.md) — form metadata shapes, field types, nested fields, lookups, required flags, client-side business rules and why writes must evaluate them.
- [Lifecycles and workflows](freshservice/lifecycles-and-workflows.md) — ticket, service-request, problem and change status models, change state flows, workflow automator side effects, SLA and due dates.
- [Wire contracts](freshservice/wire-contracts.md) — request shapes per feature (method, path, payload keys, response, readback, pitfalls) and their proof level.
- [Quirks and pitfalls](freshservice/quirks-and-pitfalls.md) — platform gotchas: string vs integer ids, bodyless requests, signed URLs, fields never echoed, derived values, 405/206/500/429 behaviour, UI traps, tenant differences.

## Working on fsv

- [Live-testing methodology](live-testing.md) — reviewed writes, blocked-submit captures, trial replication, throwaway records and cleanup, reading evidence, probing a new tenant.
- [Contributing for agents](contributing-agents.md) — repository layout, the reviewed-write pattern, offline test conventions, public-content hygiene, where knowledge lives.

Return to the [project README](../README.md).
