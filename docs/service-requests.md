# Service requests (`fsv sr`)

`fsv sr` browses the service catalog, shows an item's form the way the web
shows it, places service requests through the same form engine as
`fsv changes create` (see [forms.md](forms.md)), and reads requests back.

```
fsv sr catalog [--category X] [--search Q] [--popular] [--categories] [-o json]
fsv sr fields ITEM [-o json]
fsv sr create ITEM [--from draft.yaml | --set Label=Value ... | --edit] [--attach FILE ...]
               [--requester EMAIL] [--dry-run] [-o json] [--yes] [--expect-plan HASH]
               [--allow-duplicate] [--ack] [--no-input] [--preset NAME]
fsv sr get SR-ID [--items] [--attachments] [--download DIR] [--force] [-o json]
```

`ITEM` is the catalog item id (`77` or `#77`) or its name: an exact,
case-insensitive name, else a part of the name or short description that
matches one item only. Ambiguous names list the candidates and exit 2.

> **`fsv sr create` sends a real service request to the catalog item's
> fulfilment team.** Approvals route via the requester's manager and the
> fulfilment group is notified. Always start with `--dry-run`.

## Browsing

`fsv sr catalog` lists every item (all pages) with its id, name, category
and short description. `--category` takes a category name or id and
includes its sub-categories; `--search` keeps items whose name, short
description or category contain every word (matched locally);
`--popular` shows the frequently requested items; `--categories` lists the
categories only.

`fsv sr fields ITEM` shows the item form:

* labels in web order, required flags, placeholders/help, information
  blocks (notices);
* every choice, with its description when the metadata has one, and the
  fields it reveals (`• Lower cost → reveals: 1.2 Cost target`);
* fields revealed by a choice, indented, with `shown when <field> = <choice>`
  and `required (when shown)`;
* fields a business rule hides on load (`hidden at load by rule ...`);
* file fields: allowed types and the maximum number of files;
* the item's general-attachment rule: `required`, `optional` or
  `not accepted`.

## Creating a request

In a terminal, `fsv sr create 77` with no inputs opens the item's
**interactive form** (requester first, choices that reveal their section's
fields, file fields with type/count checks, review, then submit); see
[formui.md](formui.md). For files and scripts:

```sh
fsv sr create 77 --dry-run > sr.yaml          # annotated draft for item 77
$EDITOR sr.yaml                               # fill it in
fsv sr create 77 --from sr.yaml --dry-run     # review + payload + plan hash
fsv sr create 77 --from sr.yaml --yes --expect-plan 495643b6f6621131
```

### Draft file

Same format as change drafts, keyed by the item's field labels (or internal
names when a label is not unique):

```yaml
requester: jane.doe@example.com
answers:
  Title: Dark mode
  Details: Please add dark mode.
  Needed by: 2030-12-03
  1. Expected benefit: More revenue (Total USD)
  1.1 Revenue target: "+5%"      # revealed by the choice above
  Priority: Low                  # unique prefix of "Low - cosmetic, reports"
  Overview deck (.pptx): deck.pptx   # file field: path(s)
attachments: [brief.pdf]         # general request attachments
ack: []                          # acknowledge blocking rule messages
```

* Answers go through the form engine: section fields appear only after
  their choice is picked, business rules show/hide/mandate fields, and
  hidden answers are dropped (listed in the review).
* Choices match case-, whitespace- and Unicode-normalisation-insensitively,
  or by a unique prefix; the review warns which choice a prefix matched.
  The request sends the choice's exact value (double spaces included).
* File-field paths in a draft resolve against the draft's folder;
  `--set "Overview deck (.pptx)=deck.pptx"` paths against the current
  directory. `--attach FILE` adds a general attachment.
* `description_md`/`description_html` and `planning:` are refused: catalog
  items have no description or planning fields.
* `--edit` (advanced) opens the annotated draft in `$EDITOR` and reopens it, with the
  errors on top, until every required field is answered (new required
  fields appear as your answers reveal them).
* With no input at all, `--dry-run` prints the empty annotated draft.

### Requester

The requester decides approval routing (their manager), notifications and
who owns the request, so it is never defaulted silently:

1. `--requester EMAIL` (or an exact full name);
2. `requester:` in the draft;
3. the config key `defaults.sr_requester` (`config.json`:
   `{"defaults": {"sr_requester": "jane.doe@example.com"}}`);
4. in a terminal, fsv asks.

Without a terminal and without any of the above, `create` exits 2. The
lookup must match one person exactly (email or full name); in a terminal
several matches offer a numbered pick. fsv never uses the logged-in agent.
The review shows `Requester: Jane Doe (approvals route via Jane Doe’s
manager)`.

### Review, dry run and plan hash

The review lists the catalog item, requester, the subject the item's
template will produce, every value that will be sent (by label), rule notes,
dropped answers, messages, every file (target field, name, size, sha256),
warnings and problems, the metadata timestamp, and the safety line
*"This sends a real service request to the catalog item's fulfilment
team."*

`--dry-run` adds the exact request body. Files are not uploaded yet, so
their ids appear as placeholders that are replaced after upload:

```json
{
  "email": "jane.doe@example.com",
  "custom_fields": {
    "title_100": "Dark mode",
    "goal": "More revenue  (Total USD)",
    "goal_1_kpi": "+5%",
    "priority": "Low - cosmetic, reports",
    "overview_deck": ["<upload 1: deck.pptx>"]
  },
  "br_validation_excludes": "<engine-built token>",
  "attachments": ["<upload 2: brief.pdf>"]
}
```

* `email` is the bare address (the change and ticket bodies use
  `Name <email>`); `custom_fields` are keyed by the item's field names;
  dates are `YYYY-MM-DD`; file fields take attachment-id lists; general
  files go in `attachments`; `br_validation_excludes` is built from the
  engine state (never accepted from the caller), keyed by the item id.
* `plan_hash` covers the body, the target item and every file's name,
  size and sha256. `--yes --expect-plan HASH` (full hash or a 12+ character
  prefix) refuses to send anything else (exit 4).
* `-o json` prints the plan (payload, uploads with digests, review,
  warnings, errors, safety line).

### Sending

* In a terminal: the review, then `[y] place request  [e] edit  [s] save
  draft  [n] cancel`. Only `y` sends.
* Without a terminal: `--yes` is required (exit 2 otherwise); a plan with
  problems or unacknowledged blocking messages exits 2.
* Before sending, fsv checks the journal: the same plan sent (or started
  with an unknown outcome) in the last 24 hours exits 4 unless
  `--allow-duplicate`.
* Right before sending, the item and its rules are fetched again. If the
  plan changed, `--yes`/`--expect-plan`/no-terminal runs exit 4 (review
  again); in a terminal fsv shows what changed and asks.
* Files are uploaded only now (`POST /api/_/attachments`), file fields
  first, then general attachments; then the request is placed. If an upload
  or the request fails, nothing is placed and the ids of files already
  uploaded are printed (they are not attached to anything).
* Every attempt is journaled under the plan hash (`fsv journal tail`):
  `started`, then `done` with the created id, or `failed`.

### Local checks before anything is sent

* required fields (after rules and sections), valid choices;
* file fields: allowed types and the maximum number of files;
* general attachments: required when the item says so (unless a rule
  lifts it), refused when the item accepts none;
* every file exists.

## Reading a request

`fsv sr get SR-123` (or `123`) prints the requested item's values by
**label** (labels come from the item definitions embedded in the request,
else from the catalog item form) and the attachments with their source
(the request, or the file field they were uploaded to). `--items` or
`--attachments` limit the output; `-o json` for scripts.

`--download DIR` saves the attachments into `DIR`:

* a directory beginning with `-` must be written with `./`, such as
  `--download ./--name`; option-looking values are rejected before any request;
* server-supplied names containing `..`, `/` or `\`, absolute or drive
  paths, control characters, or that resolve outside `DIR` (symlinks
  included) are rejected and reported; the command then exits 2;
* existing files are kept unless `--force`.

## Exit codes

`0` ok · `1` cancelled / unknown outcome · `2` validation or usage (missing
requester, problems, no terminal without `--yes`) · `3` session expired ·
`4` plan changed or duplicate · `5` server rejected.

## Endpoints

| Step | Request |
|---|---|
| categories | `GET /api/_/catalog/service-categories?category_hierarchy=true` |
| items | `GET /api/_/catalog/service-items?aurora_search=true&page=N&per_page=30`, `?filter=frequently_requested` |
| item form | `GET /api/_/catalog/service-items/<display_id>` |
| rules | `business_rules_query("service_request", workspace_id=<item workspace>, item_id=<item id>)` |
| requester | `GET /search/autocomplete/requesters?q=...` |
| uploads | `POST /api/_/attachments` |
| place | `POST /api/_/catalog/service-items/<display_id>/place-request` |
| read | `GET /api/_/tickets/<id>`, `GET /api/_/tickets/<id>/requested_items[/<rid>?view=more_info]` |

## Not yet verified against a live request

* the place-request response shape (fsv reads `id`/`display_id` from the
  top level or one level down);
* "request for someone else" and quantity (not built);
* how service-request field names appear in `br_validation_excludes`
  lists (fsv uses `<name>_<account id>` like other modules);
* the shape of file-field values in requested items (ids or attachment
  objects; both are read).
