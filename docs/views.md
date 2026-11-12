# Lists and detail pages like the web

`fsv changes|tickets|problems ls` and `get`, and the TUI, lay records out the
way the Freshservice agent web does, so what you see in the terminal matches
what colleagues see in the browser. Everything comes from your account's own
metadata (saved views, form fields and their order, business rules, the
change lifecycle, planning fields, the tab list); nothing is hard-coded to one
account.

## Lists

```bash
fsv changes ls                       # the view the server marks default
fsv changes views                    # saved views by name; ▸ = the default
fsv changes ls --view "All Changes"  # by name (a unique prefix is enough)
fsv tickets ls --view "new & my"
```

* Columns: **Subject** (with `#CHN-…` under it) · Requester · Status ·
  Priority · **Assigned to** (`Group / Agent`, `(Me)` on you) · Department.
  Tickets show **State** (New, Response Due, Overdue, …) instead of
  Department, in the order of the web's ticket list headers.
* Header: `<view> · 1-30 of N · Sort by: Created Date`. A view's own sort is
  used unless you pass `--order-by`.
* Column labels come from your form (another language shows in that
  language).
* `-o json` prints the raw rows; `-o csv|tsv` the same columns as the table.

### Filters by label

`--where` takes the labels of the web's filter panel as well as internal
field names:

```bash
fsv changes ls --where Agents=Me --where "Created=Last 6 months"
fsv changes ls --where "Groups=Unassigned" --where "Environment=(empty)"
fsv changes ls --where "Planned Start Date>=2030-01-01"
fsv changes ls --where status=Open            # internal names still work
```

* `Me` (agents/requesters), `Unassigned` / `(empty)` (people, groups,
  departments, dropdowns without such a choice).
* Created / Updated / Due by take dates or a period: Today, Yesterday,
  This week, Last week, This month, Last month, Last 2 months,
  Last 6 months, Last 24 hours.
* `--debug` prints the conditions sent to the server.
* Filters replace the default view; combine them with `--view NAME` to
  narrow a view.
* `approval_status` is a change list filter only (`--where approval_status=0`,
  raw platform numbers). The ticket list ignores it (206 `search_error`,
  unfiltered rows), so fsv refuses it for tickets, in `--where` and in a raw
  `--query-hash`; read the value per record instead. Any filtered list answer
  marked `search_error` is refused rather than shown.

## Detail pages

```bash
fsv changes get CHN-1234                      # header, lifecycle, Details
fsv changes get CHN-1234 --tab approvals      # one tab
fsv changes get CHN-1234 --all-tabs           # every tab
fsv tickets get SR-5678 --all-conversations   # the whole thread
fsv changes get CHN-1234 -o json | jq .view   # the same sections as data
```

A change shows:

1. `#CHN-1234  Subject`, then `<Status>, due <date>`;
2. the **lifecycle bar** (`✓ Open › [Planning] › Awaiting Approval › …`) when
   the account has one;
3. `<Requester> reported 2 days ago (Mon, 9 Nov 14:07)`;
4. the **tab bar** with the tabs the server lists for that record, in its
   order (Details · Devops · Risk · Notes · Tasks · Assets · Impacted services
   · Associations · Approvals · Activity);
5. **Details**: Description, Attachments, **Planning** (each planning field
   with its help text and content/files, or *empty*) beside a summary card
   (Status, Priority, Resolution due by + time left, Approval, Planned
   Start/End) and **Properties**: the form's fields in form order, with
   required stars, display names instead of ids, and visibility decided by
   the edit-form business rules.

A ticket or service request shows the status badge (`Closed (late)` when it
was resolved after its due date), `… reported … via Phone`, `Request for:
Self`, **Requested items** (item, Stage, the answers by label), Description,
Attachments, **Conversations** (author, private/public, when; the latest few,
`--all-conversations` for all) and Properties. Tabs: Related tickets, Tasks,
Assets, Associations, Approvals, Activity, Resolution.

`--tab` accepts `notes tasks approvals assets impacted associations activity
risk devops` for changes and `related tasks assets associations approvals
activity resolution` for tickets; asking for a tab the record does not show
lists the ones it does. `-o json` keeps the record's own keys and adds the
page under `view` (`tabs`, `lifecycle`, `details`, `summary`,
`properties`, `tab_data`, `notes`).

Dates are shown in your Freshservice profile time zone (`FSV_TZ` overrides
it).

## Accounts without some features

Each optional part degrades on its own and says so in a dim note at the end
instead of failing the page:

| Missing on the account | What fsv does |
|---|---|
| workspaces (`/ws/<n>/` paths) | uses the plain paths |
| business rules (403/404/none) | Properties use the form's static visibility |
| change lifecycle | the header's status line only |
| planning fields | no Planning section |
| tabs list (403/404) | Details plus a basic set of tabs |
| saved views (403/404) | lists all records; `views` says none are listed |
| a tab's endpoint (403/404) | that tab shows "not permitted"/"not available" |
| agents/me (403) | no `(Me)` marker; the system time zone |

## TUI

* The list uses the same columns, opens the default view and shows the same
  `<view> · 1-N of M · Sort by: …` header; **`v`** opens a view picker
  (type to filter by name).
* The detail pane shows the header with status badge and due, the lifecycle
  bar, the reporter line and the server's tabs (`1`-`9`, `h/l`); Details has
  Description, Planning or Requested items and Conversations with
  Properties on the right.
* **`N`** opens the full-screen form for a new change and **`e`** edits the
  selected change (when `fsv.formui` is available; otherwise a toast says the
  form UI is not available). `s` status, `n` note, `r` reply, `o` open in the
  browser and `y` copy keep working.
