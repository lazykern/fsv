# Form engine (`fsv.forms`)

`fsv.forms` rebuilds Freshservice's create/edit forms from the same metadata
the web UI loads, runs the same client-side business rules, validates
answers, and builds the create request bodies the web sends. It does no HTTP:
callers fetch the metadata and inject a lookup resolver.

```
form fields JSON ─┐
service item JSON ┼─> FormSpec ─┐
business rules ───────────────> RulesEngine ─> FormState ─> review rows
                                                        └─> build_*_payload ─> POST body
```

```python
from fsv.forms import FormSpec, RulesEngine, FormState, build_change_payload, plan_hash

spec = FormSpec.from_form_fields(form_fields_json, module="change")
rules = RulesEngine(spec, business_rules_json, module="change")
state = FormState(spec, rules, mode="new", resolver=resolver)

diff = state.set("Stage", "Staging")         # label or API name
print(diff.describe(spec))                    # ["added: Staging Start, ... (required: ...)"]
state.missing_required()                      # [Field, ...]
state.explain(["Staging Start"])              # why it is visible/required, by rule
state.to_review()                             # [("Change Type", "Normal"), ...]
body = build_change_payload(state, "jane@example.com", "Jane Doe")  # incl. br_validation_excludes
plan = plan_hash(body)                        # for --dry-run / --expect-plan
```

## Metadata sources

| Form | Fields | Rules |
|---|---|---|
| Change | `GET /api/_/change_form_fields` (or `/api/v2/change_form_fields`) | `business_rules_query("change", ...)` |
| Ticket | `GET /api/_/ticket_form_fields` (or `/api/v2/ticket_form_fields`) | `business_rules_query("ticket", ...)` |
| Service request | `GET /api/_/catalog/service-items/<id>` | item sections (below); `business_rules_query("service_request", item_id=...)` |

### Fetching the business rules (`fsv.forms.fetch`)

The server filters the rules for the form context, so fetch them the way the
web does when it opens the form. `business_rules_query()` builds that request
(no HTTP happens here):

```python
q = business_rules_query("ticket", workspace_id=2)            # new ticket, agent UI
q.url("https://acme.freshservice.com")
# https://acme.freshservice.com/api/_/ws/2/business_rules?ember_api=true&module_type=ticket
#   &item_id=null&requester=&requested_for=&include_workspace=false
#   &filter_by_default_form_action=true&form_type=new_form&user_type=null&module_id=
#   &cf_name_prefix=customFields.
business_rules_query("service_request", workspace_id=2, item_id=9000009001)   # catalog item
business_rules_query("change", workspace_id=2, form_type="edit", module_id=4242)  # edit form
```

`fsv.formmeta.fetch_rules(client, module, workspace_id, **query)` makes
that request portably. `workspace_id` may be one id or a list of
candidates; each `/api/_/ws/<id>/business_rules` is tried in turn, then
the workspace-less `/api/_/business_rules` (accounts without workspaces).
A workspace that answers with an empty rule set ends the search. When
every path answers 403/404 it returns an empty rule set marked
`unavailable` with a one-line reason (the form then runs on the metadata's
static required/visible flags; reviews warn, and `fsv form fields` /
`form explain` print the reason on stderr via `formmeta.rules_note()`).
A 5xx (e.g. a tenant whose problem rules always answer 500) stops the
search at once with the same `unavailable` shape and `unavailable_status`
>= 500; it is never cached. Problem create/status writes then proceed with
a `rules unavailable` warning in the review (and `--json`), and block only
when the write depends on the rules: an answer on a field the static form
hides, or a field not on the form (`formmeta.rules_gate()`). Ticket and
change writes keep blocking on unavailable rules.

**Which workspace.** `formmeta.load_rules()` (used by every form face)
picks the workspace like this:

1. `fsv config set forms.workspace_id N`: an override, used alone;
2. the record's own `workspace_id` (edit forms and detail pages);
3. the workspace the form's custom fields belong to (`workspace_id` in the
   form-fields metadata);
4. only when those are missing or refused: the account itself, the
   session's `*workspace*` cookie, `GET /api/_/bootstrap/me`
   (`current_workspace_id`, `primary_workspace_id`, `workspace_ids`, ...)
   and `GET /api/_/workspaces` (the `primary` one first), all optional;
5. the workspace-less path.

The workspace the rules came from is kept as `meta.workspace_id` (the
engine filters rules by it). Edit-form rules are cached per record
(`module_id`), since the server filters them by record.
Form-field and resource-schema caches are keyed by tenant AND workspace
(`<domain>--ws<id>--<name>.json`, `wsauto` without a `forms.workspace_id`
override), so switching the override never reuses another workspace's
fields. Files written before this keying (`<domain>--<name>.json`) are never
read again.

Parameters: `module_type`, `workspace_id` (path), `item_id` (catalog item
`id`; `null` otherwise), `form_type` (`new`/`edit`), `requester` and
`requested_for` (user ids, empty when unknown), `module_id` (the record's
display id on an edit form), `user_type` (`null`, as the agent UI sends),
`include_workspace`, `filter_by_default_form_action` (sent except for service
requests) and `cf_name_prefix` (`customField.` for service requests,
`customFields.` otherwise). `q.path`, `q.params` and `q.query_string()` give
the parts.

`parse_business_rules(data)` reads both response shapes and returns a
`RulesResponse(rules, user, shape, evaluable, raw)`:

* `{"business_rules": [...], "user": {...}}` (`shape="form"`): the
  `ember_api=true` response; pass `raw` (or the original data) to
  `RulesEngine` and `user` to the `RuleContext`.
* `{"business_rule": [...]}` (`shape="listing"`): the plain
  `GET .../business_rules` admin listing, every rule of the workspace with
  `position` but **no** conditions or actions. `evaluable` is False and
  `RulesEngine` raises `FormRuleError` for it.

## FormSpec

`FormSpec.from_form_fields(data, module)` accepts the response object
(`{"change_fields": [...]}`, `{"ticket_fields": [...]}`) or a bare list.
`FormSpec.from_service_item(data)` accepts `{"service_item": {...}}` or the
inner object.

Each `Field` has `api_name`, `label`, `kind`, `choices`, `required_base`,
`position`, `helptext`/`hint`, `placeholder`, `default`, `is_custom`, nested
level info, sections, and file limits.

### Kinds

| `field_type` / formserv type | kind |
|---|---|
| `custom_text`, `default_subject`, `default_planned_effort`, formserv `1001` | `text` |
| `custom_paragraph`, formserv `1008` | `textarea` |
| `default_description`, planning fields | `richtext` |
| `custom_number`, `custom_decimal` | `number` |
| `custom_checkbox` | `checkbox` |
| `custom_dropdown`, `default_change_type/impact/priority/risk/status/urgency/source/ticket_type`, formserv `1003` | `dropdown` |
| `custom_multi_select_dropdown` | `multi_select` |
| `custom_date`, `default_planned_*_date` | `datetime` (`date` when `date_only`) |
| formserv `1007` | `date` (`datetime` when `field_options.date_only` is `"false"`) |
| `default_requester`, `custom_lookup`/`custom_multi_lookup` with user data source | `lookup_user` |
| `custom_lookup`/`custom_multi_lookup` with another data source and metadata lookup link | `lookup` (each selected ID verified through that endpoint) |
| `default_group` / `default_agent` | `lookup_group` / `lookup_agent` |
| `default_change_window`, other lookup data sources | `lookup` |
| `default_department` | `department` |
| `default_category`, `nested_field` | `nested` (one `Field` per level) |
| formserv `1014` | `attachment` |
| `custom_content`, `custom_static_rich_text`, `nested_content`, any other `*content*`/`*static*` type, formserv `1024` | `notice` (HTML in `content`, not an input) |

Unknown types become `text` and add a line to `spec.warnings`.

**Content-only blocks** (`notice`: static notes and guides) are part of the
form's layout, not inputs: `Field.is_input` is False and `spec.inputs()`
is `spec.ordered()` without them. Rules may show or hide them, but they are
never required, cannot be answered (`set()` raises), and are never sent,
reviewed, or listed by `fsv form fields` / `form explain` (which print a
one-line count of the blocks instead). The interactive form shows them
read-only at their position, like the web. Their HTML comes from
`content`, else `field_options.content`, else the description.

### Request for

The web change form always renders a **Request for** user field (DOM name
`requestedForId`, distinct from the requester) that the change form-fields
metadata does not list. `FormSpec.from_form_fields(..., module="change")`
adds it as a known default field right after the requester
(`api_name="requested_for"`, `field_type="default_requested_for"`, kind
`lookup_user`, `Field.synthetic=True`) unless the metadata already has a
`default_requested_for` field, which is then used as is. It is answered like
the requester (name or email via the user lookup), shown in the field order,
drafts and the interactive form, and read from a record's
`requested_for_id`.

It is **display-only**: `Field.sent` is False, payload builders leave it
out, and reviews show `(not sent)`. TODO: capture a web create/update that
fills Request for and send it with the key the web uses (likely
`requested_for_id`, unverified); until then fsv never guesses it on the wire.

### Planning fields

Planning fields (`default_change_reason`, `default_change_impact`,
`default_change_plan`, `default_backout_plan` and custom `planning_field`s;
`fsv.forms.spec.PLANNING_TYPES`) are not on the create form: fsv writes them
after the change exists, through `/api/_/changes/<id>/planning-fields`.
Their internal names differ between accounts for the same label, so fsv
never relies on a fixed name:

* drafts and presets key them by **label** (`planning: {Rollout Plan: [...]}`);
* `create.resolve_planning_definition()` matches the label first, then this
  account's name or id, then a looser match (prefixes, export label, help
  text); `planning_server_key()` gives the key this account's endpoint takes;
* `create.match_planning_row()` pairs a definition with a
  `change_planning_fields` row by name, then id, then label, then position
  (detail pages, downloads, updates);
* the interactive form offers every planning type by label.

### Details

* **Order**: by `position`; section children and nested levels follow the
  field that controls them. Fields with `position: null` (planning fields)
  are not on the create form (`on_form=False`).
* **Defaults**: default dropdowns the web pre-selects (change type, impact,
  priority, risk, urgency, status, source) default to their first visible
  choice. Choices with `visible: false` are kept but cannot be chosen.
* **Choice descriptions**: when a field's help text lists
  `Label - meaning || Label - meaning`, each meaning is attached to the
  matching choice as `Choice.description`.
* **Nested fields**: `nested_fields` levels (`level` 2, 3) become fields whose
  choices come from the upper level's `nested_options`
  (`spec.choices_for(field, values)`).
* **Sections** (dynamic fields revealed by a dropdown choice):
  service items use `field_options.section_info`
  (`[{"sectionFields": [field ids], "sectionChoices": [values]}]`, falling back
  to `choices[].dependent_ids.field`); ticket fields use
  `sections: [{"choice_ids", "ticket_field_ids"}]`. A section child is visible
  only while its parent is visible and holds one of the section's choices;
  its own `required` flag then applies.
* **Name mapping** (`spec.resolve_name`): rule and web DOM names are mapped
  to API names: `customFields.x` / `customField.sr_x` → `x`, camelCase →
  snake_case (`plannedStartDate`), and foreign keys `department_id` /
  `departmentId` → `department`, `responderId` → `agent`, `groupId` → `group`.
* **Service item**: `spec.item` has `display_id`, `allow_attachments`,
  `attachment_mandatory` and the subject template
  (`item.render_subject(values)` fills `{{custom_field.cf_<name>}}`).

## RulesEngine

```python
RulesEngine(spec, rules, *, module=None, user_type="agent", item_id=None,
            workspace_id=None, user=None, context=None, dedupe="exact",
            strict=True, model="event", same_value_fires=False,
            event_reverse="always", populate="restrict", system=None,
            event_scope="all")
engine.evaluate(values, mode="new"|"edit", *, base=None, order=None, events=None)
```

returns an `EvaluatedForm`: `visible` (ordered), `required` (visible and
required), `disabled`, `values` (effective), `populated` (field → rule id),
`messages`, `blocking_messages`, `excludes`, `decisions`, `conflicts`,
`refused`, `overridden`, `attachments_required`, `restricted` /
`restricted_by` / `unavailable` (populate restrictions, below),
`choices(name)`, `restriction_for(name)` and `explain()`.

**Context and filters.** `RuleContext(module, form, user_type, workspace_id,
item_id, user, requester, requested_for, system)` (or the matching keyword
arguments) decides which rules apply:

* Loaded: `state == "active"`, `rule_type == "client_side"`, not deleted,
  matching `module_type`; a rule with an `item_id` only for that catalog
  item; a rule's `workspace_id` must be in the context's workspace(s) when
  those are given (one id or a collection). Others are listed in
  `engine.skipped`.
* Per evaluation: `form_type` `both` or `new_form` (mode `new`) /
  `edit_form` (mode `edit`); `user_type` `both` or the context's user type.
* `user` defaults to the `user` object of the business_rules response.
  Conditions with `evaluate_on: "current_user"` read it;
  `"requester"`/`"requested_for"` read the matching context records;
  `"system"` reads `RuleContext.system` (e.g. `{"portal": <portal id>}`,
  the portal the form is opened in). A missing record makes those
  conditions see empty values (with a warning).
* `evaluate_on: "current_entity"` (edit forms) reads the saved record as
  loaded, not the live answers: `RuleContext.entity` when given, else the
  evaluation's load values (`base`). A condition on a record attribute that
  is not a form field (e.g. `requested_catalog_items`) and is missing from
  the record is reported by `unavailable_dependencies()`.
* Writes use `RulesEngine.write_gate()` (`FormState.rule_gate()`), which
  models what runtime metadata knows and blocks only on real risk:
  - `FormMeta.engine()` sets `system={"portal": false}`: fsv fills the
    agent-side form. `current_user` reads the runtime profile (groups,
    departments); a missing key is still unknown.
  - On a new record, `tags`, `approval_status` and `approval_request`
    conditions see empty values.
  - A rule whose condition cannot be evaluated is *unknown*. It blocks
    when its actions could hide, disable or require a field the write sends
    (answers and defaults) or that is required, or could require an empty
    field; otherwise it is a warning naming the rule id.
  - Actions on UI widgets that are not form fields (`tags`,
    `approval_request`, `config_item_ids`, `add_child_ticket`,
    `add_service_item`, `edit_service_item`) warn, unless the rule could
    require one or hide one that the write sends (for example devices
    sent as `device_ids`).
  - An answer you passed that the rules hide is refused with the hiding
    rule's id; it is never dropped silently.
  Warnings appear in the review and in `--json` (`warnings`, or
  `rule_warnings` for ticket transitions).
* The `enable` action clears a rule-disabled field; a reversible `enable`
  that stops matching disables it again (`enable` <-> `disable`).
* Duplicate rule ids: the endpoint returns a reversible and a
  non-reversible entry for many ids, with different action lists. The
  default `dedupe="exact"` drops only identical copies; both entries are
  evaluated in order. `first`, `last` and `none` exist for experiments.

**Strict by default.** An operator, action or `evaluate_on` the engine does
not implement raises `UnsupportedRuleError` (a `FormRuleError`) naming the
rule: computing the wrong required set silently is worse than refusing.
`strict=False` is the lenient mode: such rules are skipped and listed in
`engine.unsupported`. Actions on fields that are not on the form are ignored
with a warning.

**Conditions**: `{"all": [...]}` or `{"any": [...]}`, items are conditions or
nested groups; `{"all": null}` / `{"any": null}` (or no condition data)
means unconditional.

| operator | single value | multi-select |
|---|---|---|
| `is` / `includes` | value is one of the rule values | any selected value is one of them |
| `is_not` / `does_not_include` | value is none of them (an empty field matches) | no selected value is one of them |
| `selected` / `not_selected` | checkbox ticked / not ticked (other kinds: has a value / is empty) | |

Rule values are compared with each choice's value, label, id and display id
(whitespace-collapsed, case-insensitive), so `"4"` and `"Emergency"` both
match change type Emergency.

**Actions**

| property | when the rule matches | reverse (reversible rule stops matching) |
|---|---|---|
| `show` / `hide` | show / hide | hide / show |
| `mandate` / `unmandate` | required / not required | not required / required |
| `disable` | read-only | editable again |
| `populate` | **restrict the dropdown's choices** to the listed ones (choice **display ids**); the value is not set | full choice list again |
| `message` | add message; blocking if the rule `is_validation` | message removed |

Empty `name` lists are no-ops. Action targets are mapped like conditions
(`description_html` is the description field).

### `populate`: restricted choices

On the web, `populate` does **not** fill a field: it limits the dropdown to
the listed choices plus the empty `--` choice, and the field keeps its value
(empty on a new form; a single remaining choice is not auto-selected).
`values` are choice `display_id`s (then id, value or label); for nested
fields they are a tree, `[{"display_id": 1, "nested_options":
[{"display_id": 2, "nested_options": null}]}]`, where `null` (or an empty
list) leaves every option below that choice on offer. A reversible rule that
stops matching restores the full list.

* `evaluated.restricted[field]` is a `ChoiceRestriction` (`offers(value)`,
  `below(value)`); `evaluated.restricted_by[field]` the populate (or restore)
  decision; `evaluated.choices(name)` / `state.choices(name)` the choices on
  offer now (nested levels included).
* `FormState.set` refuses a choice outside the restriction:
  `'Live' is not offered now: the choices are limited by rule 'X' (id N) to: Staging`
  (`ValueParseError.options` lists the allowed labels).
* A value given before a rule limited the choices is kept but reported:
  `evaluated.unavailable`, `state.unavailable_errors()` (part of
  `errors()`), a "Not offered" review row; payload builders refuse it.
* `Diff.restricted` / `Diff.unrestricted` and `describe()` show the change
  ("choices limited: Stage -> Staging").

`populate="set"` keeps the earlier behaviour (fill in the first value; in the
replay model it never overwrites a user value and feeds back until it
settles).

### Evaluation models

**`event` (default): what the web form does.**

1. On load, every rule is evaluated in array order against the load values
   (defaults on a new form, the record on an edit form). A reversible rule
   that does not match applies its reverse.
2. Each change of a field is a change event that runs **every** rule again,
   in array order, on the current form state (`event_scope="all"`). A
   matching rule applies its actions; a reversible rule that does not match
   applies its reverse every time (`event_reverse="always"`), so it can
   re-show a field an earlier rule in the array just hid. A non-reversible
   rule that stops matching leaves its effects in place.
3. The `br_validation_excludes` bookkeeping persists across events (the
   order is the first time any action touched a field).
4. `populate` changes no value, so it fires no change event.
5. Selecting the value a field already has is not a change event
   (`same_value_fires=True` changes that).

The earlier event semantics stay available: `event_scope="referenced"` runs
only the rules whose conditions read the changed field, and
`event_reverse="transition"` reverses a rule only when it goes from matching
to not matching.

Answers without an inherent order (YAML, `--set`) are replayed as change
events in web field order (`position`): a person filling the form top to
bottom. The result can differ from the web when a person answers out of
order; `explain()` and the conflict report show where order matters.

**`replay`: a pure function of the values.** Every evaluation starts from
the metadata and applies every rule in order; a later rule overrides an
earlier one. Its visible/required results match the web in most captures,
but the `br_validation_excludes` order does not (the bookkeeping restarts
with every evaluation).

**Choosing the model.** Every combination was scored against captures of a
live web form (kept outside this repository): 11 recorded click scenarios on
a change form (load; a yes/no question whose rules restrict another
dropdown, answered once and toggled yes/no/yes; picking a choice the
restricted dropdown does not offer; answering the restricted dropdown before
the question; switching change type, which hides and mandates fields; a
priority that triggers a message), 4 earlier form captures (initial change
form, a service item before and after a section choice), and the
`br_validation_excludes` token of a real change created in the web UI.
Each scenario checks every field's visibility and required flag, the
selection values after every step, and, for steps the web refused, the
options it offered.

| variant | score |
|---|---|
| event, populate=set, referenced, transition (previous default) | 8/16 |
| event, populate=restrict, referenced, transition | 11/16 |
| event, restrict, referenced, always reverse | 11/16 |
| event, restrict, all rules, transition | 11/16 |
| replay, populate=set | 8/16 |
| replay, populate=restrict | 15/16 (token order wrong) |
| event, populate=set, all rules, always reverse | 9/16 |
| **event, restrict, all rules, always reverse (default)** | **16/16** |

`populate="set"` fails every scenario where a rule restricts a dropdown (the
web leaves the value empty and refuses the other choices); running only the
rules that read the changed field misses the web re-showing fields through
non-matching reversible rules later in the array. The same default also
rebuilds a blocked ticket submit's token byte for byte from engine state
(below). Rerun the comparison whenever new scenario captures arrive.

### Explain and conflicts

* `evaluated.decisions[(field, "visible"|"required"|"disabled")]` is the
  `Decision` in effect: rule id, name, action, whether it was a reverse,
  and the field change that triggered it.
* `evaluated.explain(names=None)` returns an `Explanation` per field and
  property: the deciding rule, or `metadata`, `section` (its section's
  choice is not selected), `nested` (no options under the upper level),
  `parent` (the field it belongs to is hidden).
* `evaluated.conflicts`: matching rules that disagree on one field property
  (show vs hide, mandate vs unmandate, disable vs enable), with the winner.
  Show them in `--dry-run`.

## FormState

```python
FormState(spec, rules, mode="new"|"edit", values=None, *, resolver=None,
          tz=None, existing=None, event_order="position")
```

* `mode="new"` applies the defaults; `mode="edit"` takes the record in
  `existing=`. Both are the *load* values; `values=` and `set()` are the
  user's answers. `update --set` on an existing record must use
  `mode="edit"` so the edit-form rules run.
* `event_order`: `position` (answers replayed in field order, the
  YAML/`--set` contract), `input` (in the order first given), `history`
  (every `set()` call is a change event, repeats included, like clicks on
  the web). With `position`/`input` the state replays the *final* answers
  from scratch after every change, so editing an earlier answer is
  deterministic.
* `set(field, raw)` accepts a label or API name, validates, re-evaluates
  and returns a `Diff` (`appeared`, `disappeared`, `became_required`,
  `no_longer_required`, `messages_added/removed`, `populated`, `cleared`).
* Disabled fields refuse input (`FieldDisabledError` naming the rule) but
  keep populated values. An answer that a replay reaches after a rule
  disabled its field is refused too (the set is rolled back). An answer a
  later populate replaced is reported in `evaluated.overridden` and in the
  review ("Replaced").
* Nested fields: a lower level must be a child of the chosen upper level;
  changing an upper level clears every lower level (as the web does).
* Blocking messages (`is_validation` rules) must be acknowledged:
  `state.unacknowledged()`, `state.acknowledge(rule_id | text | message)`
  or `state.acknowledge()` for all. Until then they count in `errors()`
  and payload builders refuse.
* `dropped()`: fields with a value that will not be sent because they are
  hidden, with the hiding rule. `to_review()` lists them as "Dropped".
* `explain()` / `conflicts` delegate to the evaluated form; `explain()`
  adds a `choices restricted` line naming the populate rule.
* `choices(name)`: the choices the field offers now (nested levels and
  populate restrictions applied).

| kind | accepted input |
|---|---|
| dropdown / department / nested | choice value or label, case-insensitive exact; ids for default fields. Dropdowns whose labels look like dates (meeting dates, "N/A") are matched as text, never parsed as dates |
| multi_select | list, or comma-separated string (a single label containing commas is matched first) |
| checkbox | `yes/no/true/false/y/n` or bool; anything else is an error |
| number | int or float (`1,200` accepted) |
| date | `YYYY-MM-DD` or `date` |
| datetime | ISO date + time; `Z`/`+HH:MM` kept; without an offset the time zone is resolved as below; date alone is rejected |
| lookups | `LookupValue`, numeric id, or text resolved via the resolver: one result, or one exact display/email match, else `AmbiguousLookupError` |
| attachment | path(s) or ids; checks allowed extensions and the file limit |

**Time zone** (`state.tz`, `state.tz_source`): `tz=` argument, then
`FSV_TZ`, then the Freshservice user's `time_zone` from the business_rules
`user` object (Rails names such as `"Tokyo"` are mapped to IANA names), then
the system time zone. Nothing is hardcoded.

## Payloads

Only visible fields with a value are sent. Status is never taken from the
form state; pass `status=` explicitly to send it. Builders raise
`PayloadError` when required fields are missing or a blocking message is
not acknowledged, unless `allow_incomplete=True` (drafts and dry runs).

| Builder | Endpoint | Body |
|---|---|---|
| `build_change_payload(state, email, name, *, status=None, account_id=None)` | `POST /api/_/changes` | default fields at top level (`department_id`, `group_id`, `agent_id`, `change_type`/`priority`/`impact`/`risk` ids, ISO dates with offset), `custom_fields` (choice values, value lists, booleans, user ids), `email: "Name <email>"`, `br_validation_excludes` |
| `build_ticket_payload(state, email, name, ...)` | `POST /api/_/tickets` | same pattern; `agent` → `responder_id`, `urgency`/`source` ids |
| `build_sr_payload(item, state, email, field_attachments=, attachments=, account_id=None)` | `POST /api/_/catalog/service-items/<display_id>/place-request` | `email`, `custom_fields` keyed by item field name (date fields `YYYY-MM-DD`, file fields → attachment id lists), `br_validation_excludes`, `attachments` (general) |

Files are uploaded first (`POST /api/_/attachments`); builders take the
returned ids. The SR builder enforces the item's `attachment_mandatory`
(unless a rule unmandated `attachment_ids`) and `allow_attachments`.

`description` text that does not start with `<` is escaped and wrapped in
`<div>` blocks.

### `br_validation_excludes`

The web sends this with every create. It tells the server which static
checks to skip, so fsv always computes it from the engine state and **never
accepts a token from the caller** (that would be a way around required
fields).

Format: an unsigned JWT-shaped token, two segments:
`base64('{"alg":"HS256","typ":"JWT"}') + "." + base64(payload)`, standard
base64 with padding, no signature. Payload (compact JSON):

* change / ticket: `{"optional": L, "disabled": L, "hidden": L}`
* service request: `{"<item id>": {"optional": L, "disabled": L, "hidden": L}}`
  (the item's `id`, not its display id)

Each `L` is a JSON list *encoded as a string*, with `", "` separators:
`"[\"a_123\", \"priority\"]"`, or `"[]"`.

* Names: default fields bare (`priority`, `category`); custom fields
  `<name>_<account id>`. The account id comes from the metadata
  (`spec.account_id`: lookup links `field_name=<name>_<id>`, formserv
  `account_id`), else the builders' `account_id=` argument.
* `hidden`: fields a rule hid (or a reverse hid) and that are hidden now.
* `optional`: statically required fields that the rules leave not required.
* `disabled`: fields a rule disabled and that are disabled now.
* Order: the first time a rule action (or its reverse) touched that
  property of the field, in evaluation order. Fields not on the form are
  never listed.
* A custom nested field a rule hid takes its lower levels along: they are
  appended after the rule-driven entries to `hidden` and, when not
  required, to `optional` (default `category` levels are not; observed once
  each).

`build_br_validation_excludes(evaluated, kind, account_id=None,
item_id=None)` builds it; `excludes_lists()` returns the three lists;
`decode_br_validation_excludes()` is for inspection only.

This reproduces captured tokens byte for byte, end to end from metadata and
engine state: a change created in the web UI, a blocked service-request
submit, and a blocked new-ticket submit (rules fetched with
`business_rules_query`, a `system` portal condition, a nested-field
restriction and the nested-level cascade above).

## Plan hash and metadata freshness (`fsv.forms.plan`)

* `plan_hash(payload, files=None)`: sha256 over the canonical JSON (sorted
  keys, compact, UTF-8) of the request body plus, per file, its field, name,
  size and sha256 (`files`: `{field: path | [paths]}` or a list of general
  attachments). `br_validation_excludes` is **included**: it is
  deterministic (no time stamp or signature) and changes exactly when the
  rules' effect on the form changes, which an approver should re-review.
  `plan_matches(expected, actual)` accepts the full hash or a prefix of at
  least 12 hex characters. Use it for `--dry-run` / `--yes --expect-plan`.
* `metadata_fingerprint(form_fields, rules, item)`: sha256 of the canonical
  metadata (the rules response's `user` object included, since rules can
  read it) plus the newest `updated_at` ("form metadata as of ..."). Fetch
  again before submit, compare with `.changed()`, and re-run the engine if
  anything moved.

## API changes

All additive; existing calls keep working, but three defaults changed
behaviour (pass the old values to keep it):

* `RulesEngine(populate=...)`: new, default `"restrict"` (was effectively
  `"set"`). With `"restrict"`, `evaluated.populated` stays empty and
  `overridden` never fills; read `restricted` / `choices()` instead.
* `RulesEngine(event_scope=...)`: new, default `"all"` (was effectively
  `"referenced"`).
* `RulesEngine(event_reverse=...)`: default `"always"` (was
  `"transition"`). `event_reverse` and `event_scope` accept `None` for the
  default.
* `RulesEngine(system=...)`, `RuleContext.system`, `evaluate_on: "system"`.
* `EvaluatedForm.restricted`, `.restricted_by`, `.unavailable`,
  `.populate_mode`, `.choices()`, `.restriction_for()`; `ChoiceRestriction`;
  `offered_choices()`.
* `FormState.choices()`, `.unavailable_errors()`; `errors()` and the payload
  builders include unavailable values; review rows "Not offered".
* `Diff.restricted`, `Diff.unrestricted`.
* `business_rules_query()`, `BusinessRulesQuery`, `parse_business_rules()`,
  `RulesResponse`; `RulesEngine` accepts `{"business_rule": [...]}` only to
  refuse it with `FormRuleError` (the listing has no conditions).
* Ticket metadata: a field with `belongs_to_section: true` that no section
  lists starts hidden (with a warning). `description_html` resolves to
  `description`.
* Constants: `DEFAULT_POPULATE`, `POPULATE_MODES`, `DEFAULT_EVENT_SCOPE`,
  `EVENT_SCOPES`, `DEFAULT_EVENT_REVERSE`.

## Not yet verified against a live create

* ticket `responder_id`, `type` (ticket type sent as its label) and change
  `change_window_id` keys;
* the request key for the change form's "Request for" field (fsv shows it
  but does not send it; see above);
* "request for someone else" and quantity on service requests (not built);
* what the web does with a value that a later populate no longer offers
  (fsv keeps it and reports it as an error until changed);
* whether an empty `nested_options` list in a populate tree means "no
  options below" (fsv: no limit, like `null`);
* whether the web hides the ticket `Type` field when the form metadata's
  `meta.visible_type_field` is null (fsv shows it);
* whether the token's base64 is standard or URL-safe (captured tokens
  contain neither `+`/`/` nor `-`/`_`; fsv uses standard, like `btoa`), and
  how service-request field names appear in it (the captured lists were
  empty);
* how messages, the `is` operator on multi-selects, and `requester`
  conditions behave on the web (no capture yet).
