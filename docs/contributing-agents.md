# Contributing: guidance for coding agents

**What this is.** Working rules for an agent (or person) changing this
repository: where things live, the reviewed-write pattern every mutation
follows, how offline tests are written, the hygiene rules for public
content, and where Freshservice knowledge is recorded.

**Who it is for.** Coding agents and contributors who clone the repository
without access to any private tenant, report or capture.

**Verified on:** the repository as of 2026-11-13; platform facts referenced
here were verified on a Freshservice Enterprise trial tenant, 2026-11-11 to
2026-11-13 (see the [knowledge base](freshservice/overview.md) for
confidence tags).

## 1. Layout

| Path | What lives there |
|---|---|
| `src/fsv/cli.py`, `cli_sr.py`, `*_cli.py` | Typer command surfaces. Keep them thin: parse arguments, call a module, render. |
| `src/fsv/client.py`, `session.py`, `config.py` | HTTP client (session cookies, CSRF, retries for idempotent reads only, empty-body handling), session storage, configuration. |
| `src/fsv/forms/` | Form engine: `spec.py` (field metadata), `rules.py` (business-rules engine), `state.py`, `payload.py` (payloads and `br_validation_excludes`), `plan.py` (plan hash, metadata fingerprint), `fetch.py`. |
| `src/fsv/formmeta.py`, `schema.py`, `cache.py` | Runtime metadata loading and caching (keyed by tenant and workspace). |
| `src/fsv/reviewed_*.py`, `review.py`, `journal.py`, `readback.py` | Reviewed-write machinery: plans, snapshots, single send, journal, independent readback. |
| `src/fsv/<feature>.py` | One module per feature (`ticket_merge.py`, `change_approvals.py`, `legacy_approvals.py`, `integrated_assets.py`, `problem_status.py`, `change_lifecycle.py`, ...). |
| `src/fsv/views.py`, `query.py`, `pages.py` | Lists, filters (`query_hash` / `advanced_query_hash`), detail pages, pagination. |
| `src/fsv/tui/`, `src/fsv/formui/` | Textual TUI and interactive form UI. |
| `tests/` | Offline tests. Synthetic tenants: `acme_*.py`, `tests/forms/acme_forms.py`, `formui_tenants.py`; shifted tenant: `test_shifted_tenant.py`; constants lint: `test_no_tenant_constants.py`. |
| `docs/` | User and developer documentation; `docs/freshservice/` is the platform knowledge base. |

## 2. The reviewed-write pattern

Every new mutation follows the same shape. Copy an existing reviewed
command (for example ticket note edit, ticket assignment, change task
update) rather than inventing a new one.

1. **Contract first.** Implement only a request shape that has evidence:
   VERIFIED LIVE or at least CAPTURED, recorded in
   [wire contracts](freshservice/wire-contracts.md). A successful response
   elsewhere, a UI option or a key in the web app's JavaScript does not
   establish a request body. Unproven variants fail closed with a clear
   message.
2. **Resolve everything from runtime metadata.** Statuses, task statuses,
   reminder seconds, approval recipients, choice ids, field names, workspace
   ids: read them from the tenant, by label. Never a numeric literal.
3. **Build a plan, not a request.** `--dry-run` prints the exact method,
   path, payload, target snapshot, recipients and warnings, and a
   `plan_hash` over the payload plus the reviewed state (stable snapshot
   with signed URLs stripped, metadata fingerprint, selected ids).
4. **Send once.** `--yes --expect-plan FULL_HASH` recomputes the plan from
   fresh reads, refuses on any difference, journals the intent, then sends
   **one** request. No automatic retry of writes; no endpoint fallback that
   could perform a second write.
5. **Read back independently.** Verify on the exact target with a separate
   GET (exact conversation, exact task, complete association list).
   Report `verified`, mismatched fields and *unavailable* fields separately;
   an unknown outcome is reported as unknown and never replayed.
6. **Destructive and bulk.** Require `--confirm-destructive`, print the id
   list separately, cap with `--max-records`, never silently chunk.
7. **Rules.** Writes that go through a form evaluate the business rules
   (`fsv.forms`); hidden fields are refused, not dropped; unevaluable
   rules that could affect the write block it.

The user-facing description is in [reviewed operations](reviewed-parity.md)
and the README's safety model.

## 3. Offline tests

* **Network is disabled** in tests (`tests/conftest.py` makes DNS fail and
  isolates config, cache and keychain). Subprocess tests must install the
  same guard.
* **Use real response shapes.** Build fixtures from native response
  envelopes (sanitised and synthetic), not from request echoes. Echo-only
  fixtures hide server-added defaults, rich-text normalisation, alternate
  keys (`type` vs `ticket_type`, `owner_id` vs `agent_id`), string ids in
  metadata, collections that omit fields, and empty JSON-labelled bodies.
* **Exercise the real HTTP path.** Prefer `fsv.client.Client` with an
  `httpx.MockTransport` over mocking the sender; assert the exact bytes
  sent, the number of writes, and that invalid input sends nothing.
* **Shifted tenant.** `tests/test_shifted_tenant.py` moves every
  tenant-configurable id away from the platform defaults (status ids,
  task statuses, reminder seconds, two workspaces with different fields).
  New features that read such values must pass on it.
* **No hardcoded tenant ids.** `tests/test_no_tenant_constants.py` is an AST
  lint that fails on numeric status/reminder constants in `src/fsv` outside
  a small allowlist of platform constants. Do not extend the allowlist for
  tenant data.
* **Signed URLs rotate.** Fixtures that model attachments or catalog icons
  should re-sign on every GET, so hash stability is actually tested.
* **Failure matrix.** For writes, test timeout, 403, 404, 409, 429, 5xx and
  disconnect, and assert no second request.
* Run the full suite with a timeout long enough for it (`uv run pytest -q`
  takes several minutes) before each commit.

## 4. Hygiene rules for public content

This repository is public. Everything you add (code, tests, docs, commit
messages) must be free of tenant data.

* **Synthetic values only**: companies Acme and Globex, people, addresses
  and hosts under `example.com` (or the reserved `.test` domain), invented
  ids (small numbers, or the clearly synthetic ranges the existing fixtures
  use).
* **Never** copy from a private capture or report: names of people,
  email addresses, tenant hostnames, record or user ids, ticket subjects,
  department, group, CAB, workflow, catalog-item or custom-field names,
  non-English field labels from a tenant, counts of an organisation's
  records, usage percentages, or who approves what. Paraphrase into
  platform-generic knowledge ("a typical enterprise tenant", "a tenant with
  workflow automators", "for example a field named Region").
* **No secrets**: no cookies, CSRF tokens, session files, signed URL
  queries or `br_validation_excludes` values from a real tenant.
* **Scan before every commit.** Run the repository's identifier denylist
  over the added lines (`git diff --cached`) and fix every hit; also look
  for anything that resembles an email address, a long numeric id, a
  hostname other than the synthetic ones, or an organisation-specific
  percentage. Stage files by explicit path, one logical change per commit.
* Commit locally; push only when asked.

## 5. Where knowledge lives

| Need | Read |
|---|---|
| How Freshservice is put together; API families; session; feature probes | [freshservice/overview.md](freshservice/overview.md) |
| Form metadata, required flags, business rules | [freshservice/forms-and-rules.md](freshservice/forms-and-rules.md), engine details in [forms.md](forms.md) |
| Statuses, change lifecycles, workflows, SLA | [freshservice/lifecycles-and-workflows.md](freshservice/lifecycles-and-workflows.md) |
| Exact request shapes and their proof level | [freshservice/wire-contracts.md](freshservice/wire-contracts.md) |
| Platform gotchas | [freshservice/quirks-and-pitfalls.md](freshservice/quirks-and-pitfalls.md) |
| How to test live, capture safely, replicate a tenant | [live-testing.md](live-testing.md) |
| What each `fsv` command guarantees | [reviewed-parity.md](reviewed-parity.md) and the feature pages in the [index](index.md) |

When a live test or capture teaches something new about the platform, add
it to the knowledge base page it belongs to, with a confidence tag and
synthetic values, in the same change as the code that relies on it.
