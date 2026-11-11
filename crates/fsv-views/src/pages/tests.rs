//! Expected markup comes from running `fsv.pages` on the same fixtures
//! (`views.relative` pinned to the `<REL>` placeholder, replaced here by the
//! real relative time since the Rust clock is not injectable).

use jiff::tz::{self, TimeZone};
use serde_json::{Map, Value, json};

use fsv_api::site::{self, Column as SiteColumn, Lifecycle, LifecycleState, ListPage, Missing, PlanningEntry, Prop, Tab};
use fsv_core::resource::{CHANGES, Resource, TICKETS};
use fsv_core::term::Overflow;

use super::*;
use crate::detail::{Detail, TabData};
use crate::render::blocks_text;

const WHEN: &str = "2026-01-02T03:04:00Z";

fn tz7() -> TimeZone {
    TimeZone::fixed(tz::offset(7))
}

fn rel(s: &str) -> String {
    s.replace("<REL>", &site::relative(&json!(WHEN), None))
}

fn rels(lines: &[&str]) -> Vec<String> {
    lines.iter().map(|l| rel(l)).collect()
}

fn tab(name: &str, label: &str, count: Option<i64>) -> Tab {
    Tab { name: name.into(), label: label.into(), count }
}

fn detail(res: &'static Resource, record: Value, schema: Value) -> Detail {
    Detail::new(res, record, schema, json!({"id": 1}), tz7(), vec![tab("details", "Details", None)])
}

fn no_fields() -> Value {
    json!({"fields": []})
}

fn td(name: &str) -> TabData {
    TabData::new(name)
}

fn td_data(name: &str, data: Value) -> TabData {
    TabData { data: Some(data), ..TabData::new(name) }
}

fn obj(v: Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap()
}

fn lifecycle() -> Lifecycle {
    let st = |id: i64, name: &str, mark: &str| LifecycleState { id: json!(id), name: name.into(), mark: mark.into() };
    Lifecycle {
        name: "Flow".into(),
        states: vec![st(1, "Open", "done"), st(2, "Plan", "done"), st(3, "Build", "current"), st(4, "Test", "todo"), st(5, "Done [x]", "todo")],
    }
}

fn prop(name: &str, label: &str, value: &str, required: bool) -> Prop {
    Prop { name: name.into(), label: label.into(), value: value.into(), required }
}

#[test]
fn e_escapes_and_blanks_none() {
    assert_eq!([e(&Value::Null), e(&json!(3)), e(&json!("[bold]x[/]"))], ["", "3", "\\[bold]x\\[/]"]);
}

#[test]
fn section_is_bold_escaped() {
    assert_eq!(section("A [b]"), "[bold]A \\[b][/]");
}

fn list_page() -> ListPage {
    let col = |k: &str, l: &str| SiteColumn { key: k.into(), label: l.into() };
    ListPage {
        resource_name: "tickets".into(),
        items: vec![json!({}), json!({})],
        columns: vec![col("subject", "Subject"), col("status", "Status"), col("assigned_to", "Assigned to"), col("priority", "Priority")],
        rows: vec![
            obj(json!({"id": "INC-1", "subject": "Disk [x]", "status": "Open", "assigned_to": "Ops / Ann (Me)", "priority": ""})),
            obj(json!({"id": "INC-2", "subject": "Net", "status": "", "assigned_to": "Ops / Bob", "priority": "Low"})),
        ],
        view_name: None,
        page: 2,
        per_page: 30,
        total: None,
        has_next: true,
        order_by: "created_at".into(),
        order_type: "desc".into(),
        sort: "Created Date".into(),
        notes: Vec::new(),
    }
}

#[test]
fn list_table_columns_styles_and_cells() {
    let t = list_table(&list_page());
    let headers: Vec<&str> = t.columns.iter().map(|c| c.header.as_str()).collect();
    assert_eq!(headers, ["Subject", "Status", "Assigned to", "Priority"]);
    let styles: Vec<&str> = t.columns.iter().map(|c| c.style.as_str()).collect();
    assert_eq!(styles, ["", "green", "", ""]);
    let ratios: Vec<Option<usize>> = t.columns.iter().map(|c| c.ratio).collect();
    assert_eq!(ratios, [Some(3), None, None, None]);
    assert!(t.columns.iter().all(|c| c.overflow == Overflow::Fold));
    assert!(!t.pad_edge && !t.expand && !t.show_lines);
    assert_eq!(t.rows[0], ["Disk \\[x]\n[dim cyan]#INC-1[/]", "Open", "Ops / Ann [bold](Me)[/]", "[dim]--[/]"]);
    assert_eq!(t.rows[1], ["Net\n[dim cyan]#INC-2[/]", "[dim]--[/]", "Ops / Bob", "Low"]);
}

#[test]
fn list_header_title_and_range() {
    assert_eq!(list_header(&list_page()), "[bold]All tickets[/][dim] · 31-32 of 32+ · Sort by: Created Date[/]");
    let page = ListPage {
        resource_name: "changes".into(),
        items: Vec::new(),
        rows: Vec::new(),
        view_name: Some("Mine [x]".into()),
        page: 1,
        total: Some(0),
        has_next: false,
        sort: "Due".into(),
        ..list_page()
    };
    assert_eq!(list_header(&page), "[bold]Mine \\[x][/][dim] · 0 of 0 · Sort by: Due[/]");
}

#[test]
fn lifecycle_markup_full_window_and_empty() {
    let lc = lifecycle();
    assert_eq!(
        lifecycle_markup(Some(&lc), None),
        "[green]✓ Open[/] [dim]›[/] [green]✓ Plan[/] [dim]›[/] [bold black on green] Build [/] [dim]›[/] [dim]Test[/] [dim]›[/] [dim]Done \\[x][/]"
    );
    assert_eq!(
        lifecycle_markup(Some(&lc), Some(1)),
        "[dim]…[/] [dim]›[/] [green]✓ Plan[/] [dim]›[/] [bold black on green] Build [/] [dim]›[/] [dim]Test[/] [dim]›[/] [dim]…[/]"
    );
    assert_eq!(lifecycle_markup(None, None), "");
    assert_eq!(lifecycle_markup(Some(&Lifecycle { name: "x".into(), states: vec![] }), None), "");
    let todo = |id: i64, n: &str| LifecycleState { id: json!(id), name: n.into(), mark: "todo".into() };
    let no_current = Lifecycle { name: "x".into(), states: vec![todo(1, "A"), todo(2, "B")] };
    assert_eq!(lifecycle_markup(Some(&no_current), Some(0)), "[dim]A[/] [dim]›[/] [dim]…[/]");
}

fn d_ticket() -> Detail {
    let mut d = detail(
        &TICKETS,
        json!({"id": 12, "subject": "Disk [full]", "status": 2, "due_by": "2026-03-01T10:00:00Z", "source": 2,
               "requester": {"name": "Ann"}, "created_at": WHEN}),
        json!({"fields": [{"name": "status", "choices": [{"id": 2, "value": "Open"}]},
                          {"name": "source", "choices": [{"id": 2, "value": "Portal"}]}]}),
    );
    d.requested_for = Some("Self".into());
    d
}

fn d_change() -> Detail {
    let mut d = detail(&CHANGES, json!({"display_id": 7, "subject": "Patch", "due_by": "2026-03-01T10:00:00Z", "stats": {"closed_at": "x"}}), no_fields());
    d.lifecycle = Some(lifecycle());
    d
}

#[test]
fn header_lines_due_unless_closed_ticket() {
    assert_eq!(header_lines(&d_ticket()), ["[bold cyan]#INC-12[/]  [bold]Disk \\[full][/]", "[bold green]Open[/], due Sun, 1 Mar 2026 17:00"]);
    let closed = detail(&TICKETS, json!({"id": 13, "subject": "x", "due_by": "2026-03-01T10:00:00Z", "stats": {"closed_at": "2026-03-02T00:00:00Z"}}), no_fields());
    assert_eq!(header_lines(&closed), ["[bold cyan]#INC-13[/]  [bold]x[/]", "[bold green] (late)[/]"]);
    assert_eq!(header_lines(&d_change()), ["[bold cyan]#CHN-7[/]  [bold]Patch[/]", "[bold green][/], due Sun, 1 Mar 2026 17:00"]);
}

#[test]
fn status_line_is_lifecycle_or_empty() {
    assert_eq!(
        status_line(&d_change()),
        "[green]✓ Open[/] [dim]›[/] [green]✓ Plan[/] [dim]›[/] [bold black on green] Build [/] [dim]›[/] [dim]Test[/] [dim]›[/] [dim]Done \\[x][/]"
    );
    assert_eq!(status_line(&d_ticket()), "");
}

#[test]
fn reporter_markup_ticket_change_and_creator() {
    assert_eq!(reporter_markup(&d_ticket()), rel("[bold]Ann[/] reported <REL> [dim](Fri, 2 Jan 10:04)[/] via [bold]Portal[/]   [dim]Request for:[/] Self"));
    assert_eq!(reporter_markup(&d_change()), "");
    let d = detail(&CHANGES, json!({"created_by_name": "Cy", "created_at": WHEN}), no_fields());
    assert_eq!(reporter_markup(&d), rel("[bold]Cy[/] reported <REL> [dim](Fri, 2 Jan 10:04)[/]"));
}

#[test]
fn tab_bar_active_counts_and_numbers() {
    let mut d = detail(&CHANGES, json!({"id": 1}), no_fields());
    d.tabs = vec![tab("details", "Details", None), tab("notes", "Notes", Some(3)), tab("tasks", "Tasks [x]", Some(0))];
    assert_eq!(tab_bar(&d, "details", false), "[bold reverse] Details [/]  [dim]Notes (3)[/]  [dim]Tasks \\[x][/]");
    assert_eq!(tab_bar(&d, "notes", true), "[dim]1:Details[/]  [bold reverse] 2:Notes (3) [/]  [dim]3:Tasks \\[x][/]");
}

#[test]
fn indent_pads_escapes_and_keeps_blank_lines() {
    assert_eq!(indent("a\n\n[b]", "  "), ["  a", "", "  \\[b]"]);
    assert_eq!(indent("", "  "), ["  "]);
    assert_eq!(indent("x", "    "), ["    x"]);
}

#[test]
fn main_lines_full_details_tab() {
    let mut d = detail(
        &TICKETS,
        json!({"id": 12, "description": "<p>Line one</p><p>[two]</p>",
               "attachments": [{"name": "a.txt", "size": 2048}, {"name": "b.bin"}]}),
        no_fields(),
    );
    d.requested_items = Some(vec![
        json!({"item": {"name": "Laptop"}, "quantity": 2, "stage": {"name": "Fulfil"}, "custom_fields": {"cpu": "M4"}}),
        json!({"item": {}, "quantity": 1}),
    ]);
    d.planning = Some(vec![
        PlanningEntry { name: "p1".into(), label: "Reason".into(), help: "Why".into(), text: "Because\nreasons".into(), files: vec!["plan.pdf".into()] },
        PlanningEntry { name: "p2".into(), label: "Impact".into(), help: "Impact".into(), text: String::new(), files: vec![] },
    ]);
    d.conversations = Some(vec![
        json!({"id": 1, "body": "<p>Hello</p>", "user": {"name": "Ann"}, "incoming": true, "attachments": [{"name": "log.txt"}]}),
        json!({"id": 2, "private": true, "source": 2}),
    ]);
    d.conversations_meta = obj(json!({"count": 5}));
    d.conversations_missing = Some(Missing::new("conversations after page 1", Some(403)));
    assert_eq!(
        main_lines(&d),
        [
            "[bold]Requested items[/]",
            "  [bold]Laptop[/] × 2   [dim]Stage:[/] Fulfil",
            "    [cyan]cpu[/]  M4",
            "  [bold]Requested item[/]   [dim]Stage:[/] -",
            "",
            "[bold]Description[/]",
            "  Line one",
            "  \\[two]",
            "",
            "[bold]Attachments[/]",
            "  📎 a.txt [dim](2.0 KB)[/]",
            "  📎 b.bin",
            "",
            "[bold]Planning[/]",
            "  [cyan]Reason[/]  [dim]Why[/]",
            "    Because",
            "    reasons",
            "    📎 plan.pdf",
            "  [cyan]Impact[/]",
            "    [dim]empty[/]",
            "",
            "[bold]Conversations (5)[/]",
            "  [dim]… 3 earlier (use --all-conversations)[/]",
            "  [bold]Ann[/] replied · [green]public[/] ·  [dim]()[/]",
            "    Hello",
            "    📎 log.txt",
            "",
            "  [bold]Someone[/] private note · [yellow]🔒 private[/] ·  [dim]()[/]",
            "    [dim](empty)[/]",
            "",
            "  [yellow]incomplete: conversations after page 1: not permitted for your role (403)[/]",
        ]
    );
}

#[test]
fn main_lines_missing_sections_and_empty_lists() {
    let mut d = detail(&TICKETS, json!({"id": 1}), no_fields());
    d.requested_items_missing = Some(Missing::new("requested items", Some(403)));
    d.planning_missing = Some(Missing::new("planning", Some(404)));
    d.conversations_missing = Some(Missing::new("conversations", Some(500)));
    assert_eq!(
        main_lines(&d),
        [
            "[dim]requested items: not permitted for your role (403)[/]",
            "",
            "[bold]Description[/]",
            "  [dim]No description[/]",
            "",
            "[dim]planning: not available on this account (404)[/]",
            "",
            "[dim]conversations: unavailable (500)[/]",
        ]
    );
    let mut d = detail(&TICKETS, json!({"id": 1}), no_fields());
    d.requested_items = Some(vec![]);
    d.conversations = Some(vec![]);
    assert_eq!(
        main_lines(&d),
        [
            "[bold]Requested items[/]",
            "  [dim]none[/]",
            "",
            "[bold]Description[/]",
            "  [dim]No description[/]",
            "",
            "[bold]Conversations[/]",
            "  [dim]no conversations[/]",
        ]
    );
}

#[test]
fn conversation_lines_public_and_private() {
    let cv = json!({"id": 1, "body": "<p>Hi</p>\n<p>[there]</p>", "user": {"name": "Ann"}, "created_at": WHEN,
                    "incoming": true, "attachments": [{"name": "x.log"}]});
    assert_eq!(
        conversation_lines(&cv, Some(&tz7())),
        rels(&["  [bold]Ann[/] replied · [green]public[/] · <REL> [dim](Fri, 2 Jan 10:04)[/]", "    Hi", "", "    \\[there]", "    📎 x.log", ""])
    );
    assert_eq!(
        conversation_lines(&json!({"id": 2, "private": true, "source": 2}), Some(&tz7())),
        ["  [bold]Someone[/] private note · [yellow]🔒 private[/] ·  [dim]()[/]", "    [dim](empty)[/]", ""]
    );
}

#[test]
fn human_size_units() {
    let got: Vec<String> = [
        json!(0), json!(512), json!(1024), json!(1536), json!(5 * 1024 * 1024), json!(3_u64 * 1024 * 1024 * 1024),
        json!(5_u64 * 1024 * 1024 * 1024 * 1024), json!("2048"), json!("abc"), Value::Null, json!(true), json!(-5),
    ]
    .iter()
    .map(human_size)
    .collect();
    assert_eq!(got, ["0 B", "512 B", "1.0 KB", "1.5 KB", "5.0 MB", "3.0 GB", "5120.0 GB", "2.0 KB", "abc", "None", "1 B", "-5 B"]);
}

#[test]
fn side_lines_summary_and_properties() {
    let mut d = detail(
        &CHANGES,
        json!({"display_id": 3, "status": 2, "priority": 2, "planned_start_date": "2026-02-01T09:00:00Z"}),
        json!({"fields": [{"name": "status", "label": "Status", "choices": [{"id": 2, "value": "Open"}]},
                          {"name": "priority", "label": "Priority"}]}),
    );
    d.properties = vec![prop("a", "Category", "Hardware", true), prop("b", "Short", "", false), prop("c", "A very long property label that is cut", "v [x]", false)];
    assert_eq!(
        side_lines(&d),
        [
            "[cyan]Status[/]              Open",
            "[cyan]Priority[/]            Medium",
            "[cyan]Planned start date[/]  Sun, 1 Feb 2026 16:00",
            "",
            "[bold]Properties[/]",
            "  [cyan]Category[/][red]*[/]                    Hardware",
            "  [cyan]Short[/]                        [dim]--[/]",
            "  [cyan]A very long property label …[/]  v \\[x]",
        ]
    );
    let d = detail(&CHANGES, json!({"display_id": 3}), no_fields());
    assert_eq!(side_lines(&d), ["[cyan]Status[/]  [dim]--[/]", "", "[bold]Properties[/]", "  [dim]no fields[/]"]);
}

#[test]
fn notes_lines_dim_bullets() {
    let mut d = detail(&CHANGES, json!({"id": 1}), no_fields());
    d.tabs_missing = Some(Missing::new("tabs", Some(404)));
    d.properties_note = Some("no form fields".into());
    assert_eq!(notes_lines(&d), ["[dim]· tabs: not available on this account (404)[/]", "[dim]· no form fields[/]"]);
}

#[test]
fn who_names_mappings_only() {
    let got: Vec<String> = [Some(json!({"name": " Ann "})), Some(json!("x")), Some(json!({"email": "e@x"})), None]
        .iter()
        .map(|v| who(v.as_ref()))
        .collect();
    assert_eq!(got, ["Ann", "", "e@x", ""]);
}

#[test]
fn status_text_mapping_or_scalar() {
    let got: Vec<String> = [Some(json!({"name": "N"})), Some(json!({"value": "V"})), Some(json!({"id": 4})), Some(json!({})), None, Some(json!(3))]
        .iter()
        .map(|v| status_text(v.as_ref()))
        .collect();
    assert_eq!(got, ["N", "V", "4", "", "", "3"]);
}

#[test]
fn notes_tab_rows_and_empty() {
    let t = TabData {
        notes: vec!["partial read".into()],
        ..td_data("notes", json!([
            {"user": {"name": "Ann"}, "private": true, "created_at": WHEN, "body": "<p>Fixed [it]</p>"},
            {"user_id": 9, "body_text": "plain"}
        ]))
    };
    assert_eq!(
        notes_tab(&t, Some(&tz7()), None),
        rels(&[
            "[bold]Ann[/] added a note · [yellow]🔒 private[/] · <REL> [dim](Fri, 2 Jan 10:04)[/]",
            "  Fixed \\[it]",
            "",
            "[bold]9[/] added a note · [green]public[/] ·  [dim]()[/]",
            "  plain",
            "",
        ])
    );
    assert_eq!(notes_tab(&td_data("notes", json!([])), Some(&tz7()), None), ["[dim]no notes[/]"]);
}

#[test]
fn tasks_tab_rows_and_empty() {
    let t = td_data("tasks", json!([
        {"title": "Back up", "human_display_id": "TSK-1", "status_name": "Open", "group": {"name": "Ops"}, "due_date": "2026-03-01T10:00:00Z"},
        {"display_id": 2, "status_info": {"name": "Done"}, "owner_name": "Bob"},
        {"id": 3, "status": 99}
    ]));
    assert_eq!(
        tasks_tab(&t, Some(&tz7()), None),
        [
            "○ [bold]Back up[/] [dim]TSK-1[/]",
            "    [dim]Open · Ops / None · due Sun, 1 Mar 2026 17:00[/]",
            "○ [bold]-[/] [dim]#2[/]",
            "    [dim]Done · None / Bob[/]",
            "○ [bold]-[/] [dim]#3[/]",
            "    [dim]99 (unknown)[/]",
        ]
    );
    assert_eq!(tasks_tab(&td("tasks"), Some(&tz7()), None), ["[dim]no tasks[/]"]);
}

#[test]
fn approvals_tab_levels_sorted_ints_before_strings() {
    let t = td_data("approvals", json!([
        {"level_id": 2, "status": {"name": "approved"}, "member": {"name": "Ann"}, "updated_at": WHEN,
         "remark": [{"data": "ok [fine]"}, {"data": ""}, "x"]},
        {"level": 1, "approval_status": "rejected", "approver": {"name": "Bob"}},
        {"status": "requested", "member_id": 5},
        {"level_id": "B", "status": {"value": "pending"}}
    ]));
    assert_eq!(
        approvals_tab(&t, Some(&tz7()), None),
        rels(&[
            "[bold]Level 1[/]",
            "  [red]✗[/] [bold]Bob[/]  Rejected  [dim][/]",
            "  [yellow]◷[/] [bold]5[/]  Requested  [dim][/]",
            "[bold]Level 2[/]",
            "  [green]✓[/] [bold]Ann[/]  Approved  [dim]<REL>[/]",
            "      [dim]“ok \\[fine]”[/]",
            "[bold]Level B[/]",
            "  [yellow]◷[/] [bold][/]  Pending  [dim][/]",
        ])
    );
    assert_eq!(approvals_tab(&td_data("approvals", json!([])), None, None), ["[dim]no approvals requested[/]"]);
}

#[test]
fn assets_tab_config_items_and_empty() {
    let t = td_data("assets", json!([
        {"config_item": {"name": "srv-1", "ci_type_name": "Server", "display_id": 11}},
        {"display_name": "Laptop", "ci_type": {"name": "Hardware"}, "id": 5},
        {"asset_type_name": "Phone"}
    ]));
    assert_eq!(
        assets_tab(&t, None, None),
        ["[bold]srv-1[/]  [dim]Server · #11[/]", "[bold]Laptop[/]  [dim]Hardware · #5[/]", "[bold]-[/]  [dim]Phone · #[/]"]
    );
    assert_eq!(assets_tab(&td("assets"), None, None), ["[dim]none associated[/]"]);
}

#[test]
fn activity_tab_groups_by_local_day() {
    let t = td_data("activity", json!([
        {"created_at": WHEN, "actor": {"name": "Ann"}, "content": "<b>set</b> status", "sub_contents": ["a <i>b</i>"]},
        {"created_at": "2026-01-02T20:00:00Z", "content": "next day local"},
        {"content": "undated"}
    ]));
    assert_eq!(
        activity_tab(&t, Some(&tz7()), None),
        [
            "[bold]Fri, 2 Jan 2026[/]",
            "  [dim]10:04[/]  [bold]Ann[/] set status",
            "         [dim]· a b[/]",
            "[bold]Sat, 3 Jan 2026[/]",
            "  [dim]03:00[/]  [bold]System[/] next day local",
            "[bold]?[/]",
            "  [bold]System[/] undated",
        ]
    );
    assert_eq!(
        activity_tab(&t, None, None),
        [
            "[bold]Fri, 2 Jan 2026[/]",
            "  [dim]03:04[/]  [bold]Ann[/] set status",
            "         [dim]· a b[/]",
            "  [dim]20:00[/]  [bold]System[/] next day local",
            "[bold]?[/]",
            "  [bold]System[/] undated",
        ]
    );
    assert_eq!(activity_tab(&td("activity"), Some(&tz7()), None), ["[dim]no activity[/]"]);
}

#[test]
fn item_line_id_subject_status() {
    let got: Vec<String> = [
        json!({"human_display_id": "CHN-1", "subject": "S [x]", "status_name": "Open"}),
        json!({"display_id": 2, "name": "N", "status": {"name": "Closed"}}),
        json!({"id": 3, "title": "T"}),
        json!({}),
    ]
    .iter()
    .map(item_line)
    .collect();
    assert_eq!(got, ["  [cyan]#CHN-1[/] S \\[x]  [dim]Open[/]", "  [cyan]#2[/] N  [dim]Closed[/]", "  [cyan]#3[/] T", "  [cyan]#[/] "]);
}

#[test]
fn parts_tab_skips_survey() {
    let t = TabData { parts: obj(json!({"Changes": [{"id": 1, "subject": "C"}], "Problems": [], "survey": {"a": 1}})), ..td("associations") };
    assert_eq!(parts_tab(&t, None, None), ["[bold]Changes (1)[/]", "  [cyan]#1[/] C", "[bold]Problems (0)[/]", "  [dim]none[/]"]);
    assert_eq!(parts_tab(&td("associations"), None, None), ["[dim]nothing associated[/]"]);
}

#[test]
fn risk_tab_scores_factors_survey() {
    let t = TabData {
        parts: obj(json!({"survey": {"q1": "yes", "q2": 2, "q3": [1], "q4": true}})),
        ..td_data("risk", json!({"overall_score": 0, "computed_risk": {"name": "High"},
                                  "basic_risk_awareness": [{"name": "impact_level", "operator": ">", "value": 3}, "x"]}))
    };
    assert_eq!(
        risk_tab(&t, None, None),
        [
            "[cyan]Overall score[/]  0",
            "[cyan]Computed risk[/]  High",
            "",
            "[bold]2 factors considered[/]",
            "  Impact level > [bold]3[/]",
            "",
            "[bold]Risk assessment[/]",
            "  [cyan]q1[/]  yes",
            "  [cyan]q2[/]  2",
            "  [cyan]q4[/]  True",
        ]
    );
    assert_eq!(
        risk_tab(&td_data("risk", json!({"computed_risk": ""})), None, None),
        ["[cyan]Overall score[/]  [dim]--[/]", "[cyan]Computed risk[/]  [dim]--[/]"]
    );
}

#[test]
fn resolution_tab_note_absent_empty_missing() {
    let t = td_data("resolution_notes", json!({"resolved_at": WHEN, "closed_at": "2026-01-03T03:04:00Z", "note": "Fixed\n[it]"}));
    assert_eq!(
        resolution_tab(&t, Some(&tz7()), None),
        ["[cyan]Resolved[/]  Fri, 2 Jan 2026 10:04", "[cyan]Closed[/]  Sat, 3 Jan 2026 10:04", "", "[bold]Resolution note[/]", "  Fixed", "  \\[it]"]
    );
    let mut absent = td_data("resolution_notes", json!({}));
    absent.meta.insert("absent".into(), json!(true));
    assert_eq!(resolution_tab(&absent, None, None), ["[dim]no resolution note (404)[/]"]);
    assert_eq!(resolution_tab(&td_data("resolution_notes", json!({})), None, None), ["[dim]no resolution yet[/]"]);
    let denied = TabData { missing: Some(Missing::new("Resolution", Some(403))), ..td_data("resolution_notes", json!({})) };
    assert!(resolution_tab(&denied, None, None).is_empty());
}

#[test]
fn generic_tab_rows_text_and_parts() {
    assert_eq!(generic_tab(&td_data("time_entries", json!([{"id": 1, "subject": "T"}, "x"])), None, None), ["  [cyan]#1[/] T"]);
    assert_eq!(generic_tab(&td_data("time_entries", json!([])), None, None), ["[dim]nothing here[/]"]);
    assert_eq!(generic_tab(&td_data("time_entries", json!("text [x]")), None, None), ["text \\[x]"]);
    let parts = TabData { parts: obj(json!({"A": []})), ..td("time_entries") };
    assert_eq!(generic_tab(&parts, None, None), ["[bold]A (0)[/]", "  [dim]none[/]"]);
}

#[test]
fn tab_lines_missing_notes_and_dispatch() {
    let denied = TabData { missing: Some(Missing::new("Notes", Some(403))), ..td("notes") };
    assert_eq!(tab_lines(&denied, None, None), ["[dim]Notes: not permitted for your role (403)[/]"]);
    let partial = TabData { missing: Some(Missing::new("Notes", Some(500))), notes: vec!["n1".into()], ..td_data("notes", json!([])) };
    assert_eq!(tab_lines(&partial, None, None), ["[dim]no notes[/]", "[dim]Notes: unavailable (500)[/]", "[dim]· n1[/]"]);
    assert_eq!(tab_lines(&td_data("bogus", json!([{"id": 1}])), None, None), ["  [cyan]#1[/] "]);
}

fn d_print() -> Detail {
    let mut d = detail(&CHANGES, json!({"display_id": 7, "subject": "Patch", "description": "Body"}), no_fields());
    d.tabs = vec![tab("details", "Details", None), tab("notes", "Notes", Some(1)), tab("tasks", "Tasks", None)];
    d.lifecycle = Some(lifecycle());
    d.planning_missing = Some(Missing::new("planning", Some(404)));
    d.tabs_missing = Some(Missing::new("tabs", Some(403)));
    d.properties = vec![prop("a", "Category", "HW", false)];
    d.set_tab(td_data("notes", json!([{"user_id": 9, "body_text": "hi"}])));
    d
}

const RULE80: &str = "────────────────────────────────────────────────────────────────────────────────";

#[test]
fn print_detail_details_tab_narrow() {
    let want = format!(
        "#CHN-7  Patch\n\n✓ Open › ✓ Plan ›  Build  › Test › Done [x]\n\n Details   Notes (1)  Tasks\n{RULE80}\nDescription\n  Body\n\nplanning: not available on this account (404)\n\nStatus  --\n\nProperties\n  Category   HW\n\n· tabs: not permitted for your role (403)\n"
    );
    assert_eq!(blocks_text(&detail_blocks(&d_print(), None, false), 80), want);
}

#[test]
fn print_detail_single_tab() {
    let want = format!(
        "#CHN-7  Patch\n\n✓ Open › ✓ Plan ›  Build  › Test › Done [x]\n\nDetails   Notes (1)   Tasks\n{RULE80}\n9 added a note · public ·  ()\n  hi\n\n\n· tabs: not permitted for your role (403)\n"
    );
    assert_eq!(blocks_text(&detail_blocks(&d_print(), Some("notes"), false), 80), want);
}

#[test]
fn print_detail_all_tabs() {
    let want = format!(
        "#CHN-7  Patch\n\n✓ Open › ✓ Plan ›  Build  › Test › Done [x]\n\n Details   Notes (1)  Tasks\n{RULE80}\nDescription\n  Body\n\nplanning: not available on this account (404)\n\nStatus  --\n\nProperties\n  Category   HW\n\nNotes ──────────────────────────────────────────────────────────────────────────\n9 added a note · public ·  ()\n  hi\n\n\n· tabs: not permitted for your role (403)\n"
    );
    assert_eq!(blocks_text(&detail_blocks(&d_print(), None, true), 80), want);
}

#[test]
fn print_detail_wide_grid() {
    let rule120 = "─".repeat(120);
    let want = format!(
        "#CHN-7  Patch\n\n✓ Open › ✓ Plan ›  Build  › Test › Done [x]\n\n Details   Notes (1)  Tasks\n{rule120}\nDescription                                                             Status  --                                      \n  Body                                                                                                                  \n                                                                        Properties                                      \nplanning: not available on this account (404)                             Category   HW                                 \n\n· tabs: not permitted for your role (403)\n"
    );
    assert_eq!(blocks_text(&detail_blocks(&d_print(), None, false), 120), want);
}
