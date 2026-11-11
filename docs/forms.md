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
from fsv.forms import FormSpec, RulesEngine, FormState, build_change_payload

spec = FormSpec.from_form_fields(form_fields_json, module="change")
rules = RulesEngine(spec, business_rules_json, module="change")
state = FormState(spec, rules, mode="new", resolver=resolver)

diff = state.set("Environment", "Staging")   # label or API name
print(diff.describe(spec))                    # ["added: Staging Start, ... (required: ...)"]
state.missing_required()                      # [Field, ...]
state.to_review()                             # [("Change Type", "Normal"), ...]
body = build_change_payload(state, "jane@example.com", "Jane Doe")
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

`RulesEngine(spec, rules, module=..., user_type="agent", item_id=None,
dedupe="exact", strict=False)`; `engine.evaluate(values, mode="new"|"edit")`
returns an `EvaluatedForm`: `visible` (ordered), `required` (visible and
required), `disabled`, `values` (effective), `populated` (field → rule id),
`messages`.

**Loading**

* Kept: `state == "active"`, `rule_type == "client_side"`, not deleted,
  matching `module_type` and `item_id`. Others are listed in
  `engine.skipped`. Server-side rules are not evaluated.
* Applied per call: `form_type` `both` or `new_form` (mode `new`) /
  `edit_form` (mode `edit`); `user_type` `both` or the engine's user type.
* Duplicate rule ids: the endpoint can return the same id more than once,
  sometimes with *different* content (e.g. one reversible entry and one
  non-reversible entry with a longer action list). The default policy
  `exact` drops only identical copies and evaluates the remaining entries in
  order. Checked against a live tenant, this reproduced the web form's
  visible/required set exactly, while keeping only the first or last entry
  did not. `first`, `last` and `none` are available.

**Conditions**: `{"all": [...]}` or `{"any": [...]}`, items are conditions or
nested groups; `{"all": null}` always matches.

| operator | matches when |
|---|---|
| `is` / `includes` | the value (any selected value for multi-selects) equals one of the rule values |
| `is_not` / `does_not_include` | the opposite (so an empty field matches) |
| `selected` / `not_selected` | checkbox is ticked / not ticked (other kinds: has a value / is empty) |

Rule values are compared with each choice's value, label, id and display id
(whitespace-collapsed, case-insensitive), so both `"Emergency"` and `"4"`
match change type Emergency. Unknown operators or action properties skip the
whole rule and are reported in `engine.unsupported` (`strict=True` raises
`UnsupportedRuleError`). Actions on unknown fields are ignored with a warning.

**Actions and evaluation**

Each evaluation starts from the base state (`visible`, `required` from the
metadata) and applies every rule **in order**; a later rule overrides an
earlier one for the same field and property. So the result is a pure
function of the values.

| property | when the rule matches | when it does not and `is_reversible` |
|---|---|---|
| `show` / `hide` | show / hide | hide / show |
| `mandate` / `unmandate` | required / not required | not required / required |
| `disable` | read-only | editable again |
| `populate` | fill the field (see below) | nothing |
| `message` | add message (blocking if the rule `is_validation`) | nothing |

`populate` values are choice display ids (or ids/values). A populated value
is applied only while its rule matches and only if the user has not set the
field; it is reported in `populated` with the rule id and never overwrites
user input. Populated values feed back into conditions; the engine repeats
until they settle.

## FormState

`FormState(spec, rules, mode, values=None, resolver=None, tz=None)`. On `new`
it applies the defaults. `set(field, raw)` accepts a label or API name,
validates, re-evaluates, and returns a `Diff` (`appeared`, `disappeared`,
`became_required`, `no_longer_required`, `messages_added/removed`,
`populated`, `cleared`); `diff.describe(spec)` gives label-based lines.

| kind | accepted input |
|---|---|
| dropdown / department / nested | choice value or label, case-insensitive exact; ids for default fields. Nested levels only accept children of the chosen upper level; changing an upper level clears lower levels that no longer fit |
| multi_select | list, or comma-separated string (a single label containing commas is matched first) |
| checkbox | `yes/no/true/false/y/n` or bool; anything else is an error |
| number | int or float (`1,200` accepted) |
| date | `YYYY-MM-DD` or `date` |
| datetime | ISO date + time; no offset → **Asia/Bangkok** (override with `tz=` or `FSV_TZ`); `Z`/`+HH:MM` kept; date alone is rejected |
| lookups | `LookupValue`, numeric id, or text resolved via the resolver: one result, or one exact display/email match, else `AmbiguousLookupError` |
| attachment | path(s) or ids; checks allowed extensions and the file limit |

Fields disabled by a rule raise `FieldDisabledError`. Values of hidden fields
are kept in the state (so switching back restores them) but are never in
`payload_values()` or the payload.

* `missing_required()`: visible required fields without a value; a required
  checkbox must be ticked.
* `errors()`: missing fields plus blocking rule messages.
* `to_review()`: `(label, display)` rows for visible filled fields in form
  order (populated values marked `(auto)`), then `("Message"|"Blocked", text)`.

## Payloads

Only visible fields with a value are sent. Status is never taken from the
form state; pass `status=` explicitly to send it. Builders raise
`PayloadError` when required fields are missing or a blocking rule message is
active, unless `allow_incomplete=True` (for drafts and dry runs).

| Builder | Endpoint | Body |
|---|---|---|
| `build_change_payload(state, email, name)` | `POST /api/_/changes` | default fields at top level (`department_id`, `group_id`, `agent_id`, `change_type`/`priority`/`impact`/`risk` ids, ISO dates with offset), `custom_fields` (choice values, value lists, booleans, user ids), `email: "Name <email>"` |
| `build_ticket_payload(state, email, name)` | `POST /api/_/tickets` | same pattern; `agent` → `responder_id`, `urgency`/`source` ids |
| `build_sr_payload(item, state, email, field_attachments=, attachments=)` | `POST /api/_/catalog/service-items/<display_id>/place-request` | `email`, `custom_fields` keyed by item field name (date fields `YYYY-MM-DD`, file fields → attachment id lists), `attachments` (general) |

Files are uploaded first (`POST /api/_/attachments`); builders take the
returned ids. The SR builder enforces the item's `attachment_mandatory` and
`allow_attachments`.

`description` text that does not start with `<` is escaped and wrapped in
`<div>` blocks.

### `br_validation_excludes`

The web adds a `br_validation_excludes` token to every create body. Its
meaning, and whether the server requires it, is unknown. The engine never
fabricates it: pass `br_validation_excludes=` to include a value you
obtained, otherwise the key is omitted (`fsv.forms.BR_VALIDATION_EXCLUDES`).

### Not yet verified against a live create

* ticket `responder_id`, `type` (ticket type sent as its label) and change
  `change_window_id` keys;
* "request for someone else" and quantity on service requests (not built);
* whether non-reversible `populate` should keep its value after the rule
  stops matching (the engine recomputes it each time).
