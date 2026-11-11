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

diff = state.set("Environment", "Staging")   # label or API name
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
| Change | `GET /api/_/change_form_fields` (or `/api/v2/change_form_fields`) | `GET /api/_/ws/2/business_rules` (module `change`) |
| Ticket | `GET /api/_/ticket_form_fields` (or `/api/v2/ticket_form_fields`) | business rules, module `ticket` |
| Service request | `GET /api/_/catalog/service-items/<id>` | item sections (below); business rules if the tenant has any for the item |

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
| `default_group` / `default_agent` | `lookup_group` / `lookup_agent` |
| `default_change_window`, other lookup data sources | `lookup` |
| `default_department` | `department` |
| `default_category`, `nested_field` | `nested` (one `Field` per level) |
| formserv `1014` | `attachment` |
| `custom_content`, formserv `1024` | `notice` (HTML in `content`, not an input) |

Unknown types become `text` and add a line to `spec.warnings`.

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
            event_reverse="transition")
engine.evaluate(values, mode="new"|"edit", *, base=None, order=None, events=None)
```

returns an `EvaluatedForm`: `visible` (ordered), `required` (visible and
required), `disabled`, `values` (effective), `populated` (field → rule id),
`messages`, `blocking_messages`, `excludes`, `decisions`, `conflicts`,
`refused`, `overridden`, `attachments_required`, and `explain()`.

**Context and filters.** `RuleContext(module, form, user_type, workspace_id,
item_id, user, requester, requested_for)` (or the matching keyword
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
  `"requester"`/`"requested_for"` read the matching context records. A
  missing record makes those conditions see empty values (with a warning).
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
| `populate` | set the value (choice **display id**) | nothing |
| `message` | add message; blocking if the rule `is_validation` | message removed |

Empty `name` lists are no-ops. `populate` values are choice `display_id`s
(looked up through `Field.choice_by_display_id`, then id/value).

### Evaluation models

**`event` (default): what the web form does.**

1. On load, every rule is evaluated in array order against the load values
   (defaults on a new form, the record on an edit form). A reversible rule
   that does not match applies its reverse.
2. Each change of field F is a change event: only rules whose conditions
   reference F run, in array order. A matching rule applies its actions; a
   reversible rule is reversed only when it goes from matching to not
   matching (`event_reverse="always"` reverses on every non-match).
3. `populate` does not cascade: filling a field fires no change event, so
   rules on that field do not run.
4. Selecting the value a field already has is not a change event
   (`same_value_fires=True` changes that).

Answers without an inherent order (YAML, `--set`) are replayed as change
events in web field order (`position`): a person filling the form top to
bottom. The result can differ from the web when a person answers out of
order; `explain()` and the conflict report show where order matters.

**`replay`: a pure function of the values.** Every evaluation starts from
the metadata and applies every rule in order; a later rule overrides an
earlier one. `populate` never overwrites a user value and feeds back until
it settles.

**Choosing the model.** Both models were scored against captured web forms
of a live tenant (the initial change form, the form after answering a
populate-driving question, a service item before/after a section choice)
and against the `br_validation_excludes` token of a real change created in
the web UI. All agree on the form captures; only the event model, with
transition-only reversal and no same-value events, reproduces the real
token byte for byte (the replay model gets the right fields in the wrong
order; reversing on every non-match re-shows fields the web keeps hidden).
`DEFAULT_MODEL` is therefore `event`. Rerun the comparison whenever new
scenario captures arrive.

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
* `explain()` / `conflicts` delegate to the evaluated form.

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

`build_br_validation_excludes(evaluated, kind, account_id=None,
item_id=None)` builds it; `excludes_lists()` returns the three lists;
`decode_br_validation_excludes()` is for inspection only.

This reproduces captured tokens byte for byte (a change created in the web
UI and a blocked service-request submit, end to end from metadata; a
ticket token given its lists, since that tenant's ticket rules were not
captured).

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

## Not yet verified against a live create

* ticket `responder_id`, `type` (ticket type sent as its label) and change
  `change_window_id` keys;
* "request for someone else" and quantity on service requests (not built);
* whether `populate` keeps its value after its rule stops matching (the
  engine keeps it in the event model);
* whether the token's base64 is standard or URL-safe (captured tokens
  contain neither `+`/`/` nor `-`/`_`; fsv uses standard, like `btoa`), and
  how service-request field names appear in it (the captured lists were
  empty);
* how messages, the `is` operator on multi-selects, and `requester`
  conditions behave on the web (no capture yet).
