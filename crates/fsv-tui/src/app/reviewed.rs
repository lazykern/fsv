//! Reviewed single-purpose writes: ticket/SR properties (`e` on a ticket),
//! the problem known-error mark and native planning text (`a` menu), and
//! ticket replies / notes with recipient or status options (`r`, Ctrl+O).
//! Each one builds a plan on a worker, shows its exact review, sends once
//! and shows the outcome; a verified outcome reloads the record.

use fsv_core::resource::{PROBLEMS, TICKETS};
use fsv_py::{Dumps, py_str, truthy};
use serde_json::{Value, json};

use super::{App, Cont, Msg, Ui};
use crate::actions;
use crate::backend::{BResult, ConversationRequest, Failure, Item, PropertyField, Res, ReviewPlan};
use crate::modals::{Modal, ModalResult, NoteModal};

pub const PROPERTY_PICKER: &str = "Edit ticket/SR property — one field per review";
pub const PROPERTY_VALUE_SUFFIX: &str =
    " — exact value/name/email; comma-separated for multiple values";
pub const KNOWN_ERROR_WARNING: &str = "This does not resolve/close the problem or publish a knowledge article. Reversal is not implemented.";
pub const PLANNING_PICKER: &str = "Native planning/analysis section";
pub const PUBLIC_CONVERSATION: &str =
    "Review every recipient. This public conversation may send email and trigger workflows.";

/// `json.dumps(v, indent=2)`.
fn dumps(v: &Value) -> String {
    Dumps::default().indent(2).dump(v)
}

/// `_ticket_conversation_options`' `options` dict.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConvOptions {
    pub pending: bool,
    pub cc: String,
    pub bcc: String,
    pub sender: String,
    pub notify: String,
}

impl ConvOptions {
    fn slot(&mut self, key: &str) -> Option<&mut String> {
        match key {
            "cc" => Some(&mut self.cc),
            "bcc" => Some(&mut self.bcc),
            "sender" => Some(&mut self.sender),
            "notify" => Some(&mut self.notify),
            _ => None,
        }
    }

    fn get(&self, key: &str) -> &str {
        match key {
            "cc" => &self.cc,
            "bcc" => &self.bcc,
            "sender" => &self.sender,
            "notify" => &self.notify,
            _ => "",
        }
    }

    /// `[v.strip() for v in options.get(key, "").split(",") if v.strip()]`.
    fn emails(&self, key: &str) -> Vec<String> {
        self.get(key)
            .split(',')
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
            .collect()
    }

    pub fn request(&self, public: bool, reply: bool) -> ConversationRequest {
        ConversationRequest {
            reply,
            public,
            cc: self.emails("cc"),
            notify: self.emails("notify"),
            bcc: self.emails("bcc"),
            sender: (!self.sender.is_empty()).then(|| self.sender.clone()),
            set_status: self.pending.then(|| "Pending".to_string()),
        }
    }
}

/// A modal's continuation inside these flows.
#[derive(Clone, Debug)]
pub enum Step {
    PropertyField {
        item: Item,
        fields: Vec<PropertyField>,
    },
    PropertyValue {
        item: Item,
        name: String,
    },
    PropertyReviewed {
        item: Item,
        plan: ReviewPlan,
    },
    KnownErrorReviewed {
        item: Item,
        plan: ReviewPlan,
    },
    PlanningSection {
        item: Item,
    },
    PlanningEdited {
        item: Item,
        section: String,
    },
    PlanningReviewed {
        item: Item,
        plan: ReviewPlan,
    },
    ConversationOption {
        item: Item,
        body: String,
        public: bool,
        reply: bool,
        options: ConvOptions,
    },
    ConversationValue {
        item: Item,
        body: String,
        public: bool,
        reply: bool,
        options: ConvOptions,
        key: String,
    },
    ConversationReviewed {
        item: Item,
        plan: ReviewPlan,
        public: bool,
    },
    ConversationPublic {
        item: Item,
        plan: ReviewPlan,
    },
}

fn push(modal: Modal, step: Step) -> Ui {
    Ui::Push(Box::new(modal), Cont::Reviewed(step))
}

fn failure(e: &Failure) -> Msg {
    Msg::Ui(vec![Ui::Failure(actions::failure_detail(e))])
}

/// `OutcomeScreen(title, json.dumps(result, indent=2))`, then `_after_write`
/// when the read-back verified the write.
fn sent(result: BResult<Value>, title: &str, res: Res, item: Item, verified: &str) -> Msg {
    match result {
        Err(e) => failure(&e),
        Ok(r) => {
            let mut ui = vec![Ui::Outcome {
                title: title.into(),
                message: dumps(&r),
            }];
            if r.get("verified").is_some_and(truthy) {
                ui.push(Ui::AfterWrite(res, item, verified.into()));
            }
            Msg::Ui(ui)
        }
    }
}

/// `match_planning_row` found a row whose text or attachments cannot be
/// carried over unchanged.
fn planning_row_unsafe(row: &Value) -> bool {
    let Some(o) = row.as_object().filter(|o| !o.is_empty()) else {
        return false;
    };
    !o.contains_key("description")
        || o.get("attachments").is_some_and(truthy)
        || !o.contains_key("attachments")
}

/// `(str(d.get("name") or d["id"]), str(d.get("label") or d.get("name")))`.
pub fn planning_choices(defs: &[Value]) -> Vec<(Value, String)> {
    let s = |v: Option<&Value>| v.map_or("None".to_string(), py_str);
    defs.iter()
        .map(|d| {
            let name = s(d.get("name").filter(|v| truthy(v)).or_else(|| d.get("id")));
            let label = s(d
                .get("label")
                .filter(|v| truthy(v))
                .or_else(|| d.get("name")));
            (json!(name), label)
        })
        .collect()
}

impl App {
    pub(super) fn resume_reviewed(&mut self, step: Step, result: ModalResult) {
        use ModalResult as R;
        match (step, result) {
            (Step::PropertyField { item, fields }, R::Choice(name)) => {
                let name = py_str(&name);
                let Some(field) = fields.into_iter().find(|f| f.name == name) else {
                    return;
                };
                let modal = match field.choices {
                    Some(choices) => Modal::choices(&field.label, choices),
                    None => Modal::value(&format!("{}{PROPERTY_VALUE_SUFFIX}", field.label), ""),
                };
                self.push(modal, Cont::Reviewed(Step::PropertyValue { item, name }));
            }
            (Step::PropertyValue { item, name }, R::Choice(value)) => {
                self.prepare_ticket_property(item, name, value)
            }
            (Step::PropertyValue { item, name }, R::Text(value)) => {
                self.prepare_ticket_property(item, name, Value::String(value))
            }
            (Step::PropertyReviewed { item, plan }, R::Bool(true)) => {
                self.send_ticket_property(item, plan)
            }
            (Step::KnownErrorReviewed { item, plan }, R::Bool(true)) => {
                self.send_known_error(item, plan)
            }
            (Step::PlanningSection { item }, R::Choice(section)) => {
                let section = py_str(&section);
                if !section.is_empty() {
                    self.load_planning_editor(item, section);
                }
            }
            (Step::PlanningEdited { item, section }, R::Note(text, _)) => {
                self.prepare_planning_text(item, section, text)
            }
            (Step::PlanningReviewed { item, plan }, R::Bool(true)) => {
                self.send_planning_text(item, plan)
            }
            (
                Step::ConversationOption {
                    item,
                    body,
                    public,
                    reply,
                    mut options,
                },
                R::Choice(key),
            ) => match py_str(&key).as_str() {
                "review" => self.prepare_ticket_conversation(item, body, public, reply, options),
                "pending" => {
                    options.pending = !options.pending;
                    self.ticket_conversation_options(item, body, public, reply, options);
                }
                key => {
                    let title = if key == "sender" {
                        "Configured support email"
                    } else {
                        "Comma-separated plain emails (empty clears)"
                    };
                    let current = options.get(key).to_string();
                    let key = key.to_string();
                    self.push(
                        Modal::value_with(title, &current, true),
                        Cont::Reviewed(Step::ConversationValue {
                            item,
                            body,
                            public,
                            reply,
                            options,
                            key,
                        }),
                    );
                }
            },
            (
                Step::ConversationValue {
                    item,
                    body,
                    public,
                    reply,
                    mut options,
                    key,
                },
                R::Text(value),
            ) => {
                if let Some(slot) = options.slot(&key) {
                    *slot = value;
                }
                self.ticket_conversation_options(item, body, public, reply, options);
            }
            (Step::ConversationReviewed { item, plan, public }, R::Bool(true)) => {
                if public {
                    self.push(
                        Modal::confirm(
                            "Confirm PUBLIC mail/visibility",
                            vec![PUBLIC_CONVERSATION.into()],
                        ),
                        Cont::Reviewed(Step::ConversationPublic { item, plan }),
                    );
                } else {
                    self.send_ticket_conversation(item, plan, false);
                }
            }
            (Step::ConversationPublic { item, plan }, R::Bool(true)) => {
                self.send_ticket_conversation(item, plan, true)
            }
            _ => {}
        }
    }

    // ── ticket / SR properties ──────────────────────────────

    /// `_load_ticket_property_fields`.
    pub(super) fn load_ticket_property_fields(&mut self, item: Item) {
        if self.read_only {
            return;
        }
        self.spawn(move |b| match b.ticket_property_fields(&item.ref_id()) {
            Err(e) => failure(&e),
            Ok(fields) => {
                let choices = fields
                    .iter()
                    .map(|f| (json!(f.name), f.label.clone()))
                    .collect();
                Msg::Ui(vec![push(
                    Modal::choices(PROPERTY_PICKER, choices),
                    Step::PropertyField { item, fields },
                )])
            }
        });
    }

    /// `_prepare_ticket_property`.
    pub(super) fn prepare_ticket_property(&mut self, item: Item, name: String, value: Value) {
        if self.read_only {
            return;
        }
        self.spawn(
            move |b| match b.ticket_property_plan(&item.ref_id(), &name, &value) {
                Err(e) => failure(&e),
                Ok(plan) => Msg::Ui(vec![push(
                    Modal::confirm("Review exact ticket property", vec![dumps(&plan.review)]),
                    Step::PropertyReviewed { item, plan },
                )]),
            },
        );
    }

    /// `_send_ticket_property`.
    fn send_ticket_property(&mut self, item: Item, plan: ReviewPlan) {
        if self.read_only {
            return;
        }
        self.spawn(move |b| {
            sent(
                b.send_ticket_property(&plan),
                "Property result — do not resend",
                &TICKETS,
                item,
                "Ticket property verified",
            )
        });
    }

    // ── problem known-error mark ────────────────────────────

    /// `_prepare_known_error`.
    pub(super) fn prepare_known_error(&mut self, item: Item) {
        if self.read_only {
            return;
        }
        self.spawn(move |b| match b.known_error_plan(&item.ref_id()) {
            Err(e) => failure(&e),
            Ok(plan) => Msg::Ui(vec![push(
                Modal::confirm(
                    "Review known-error mark",
                    vec![dumps(&plan.review), KNOWN_ERROR_WARNING.into()],
                ),
                Step::KnownErrorReviewed { item, plan },
            )]),
        });
    }

    /// `_send_known_error`.
    fn send_known_error(&mut self, item: Item, plan: ReviewPlan) {
        if self.read_only {
            return;
        }
        self.spawn(move |b| {
            sent(
                b.send_known_error(&plan),
                "Known-error result — do not resend",
                &PROBLEMS,
                item,
                "Known-error state verified",
            )
        });
    }

    // ── native planning text ────────────────────────────────

    /// `_load_planning_sections`.
    pub(super) fn load_planning_sections(&mut self, item: Item) {
        if self.read_only {
            return;
        }
        self.spawn(move |b| {
            let choices = match b.planning_definitions(item.res.name) {
                Err(e) => return failure(&e),
                Ok(defs) => planning_choices(&defs),
            };
            if choices.is_empty() {
                return failure(&Failure::value(
                    "No native planning sections in current metadata",
                ));
            }
            Msg::Ui(vec![push(
                Modal::choices(PLANNING_PICKER, choices),
                Step::PlanningSection { item },
            )])
        });
    }

    /// `_load_planning_editor`: refuses a section whose text or attachments
    /// would not survive the save.
    fn load_planning_editor(&mut self, item: Item, section: String) {
        if self.read_only {
            return;
        }
        self.spawn(move |b| {
            let row = match b.planning_row(item.res.name, &item.ref_id(), &section) {
                Err(e) => return failure(&e),
                Ok(row) => row,
            };
            if row.as_ref().is_some_and(planning_row_unsafe) {
                return failure(&Failure::value(
                    "Planning text/attachment preservation is unavailable",
                ));
            }
            let initial = match row.as_ref().and_then(|r| r.get("description")) {
                None | Some(Value::Null) => String::new(),
                Some(v) => py_str(v),
            };
            Msg::Ui(vec![push(
                Modal::Note(NoteModal::planning(&section, &initial)),
                Step::PlanningEdited { item, section },
            )])
        });
    }

    /// `_prepare_planning_text`.
    fn prepare_planning_text(&mut self, item: Item, section: String, description: String) {
        if self.read_only {
            return;
        }
        self.spawn(move |b| {
            match b.planning_text_plan(item.res.name, &item.ref_id(), &section, &description) {
                Err(e) => failure(&e),
                Ok(plan) => Msg::Ui(vec![push(
                    Modal::confirm("Review exact planning text", vec![dumps(&plan.review)]),
                    Step::PlanningReviewed { item, plan },
                )]),
            }
        });
    }

    /// `_send_planning_text`.
    fn send_planning_text(&mut self, item: Item, plan: ReviewPlan) {
        if self.read_only {
            return;
        }
        self.spawn(move |b| {
            let res = item.res;
            sent(
                b.send_planning_text(&plan),
                "Planning result — do not resend",
                res,
                item,
                "Planning text verified",
            )
        });
    }

    // ── ticket replies and notes with options ───────────────

    /// `_ticket_conversation_options`: review, the Pending toggle, and the
    /// recipient fields for a reply (CC / BCC / sender) or a note (notify).
    pub(super) fn ticket_conversation_options(
        &mut self,
        item: Item,
        body: String,
        public: bool,
        reply: bool,
        options: ConvOptions,
    ) {
        if self.read_only {
            return;
        }
        let review = if reply {
            "Review exact reply"
        } else {
            "Review exact note"
        };
        let status = if options.pending {
            "Pending"
        } else {
            "unchanged"
        };
        let mut choices = vec![
            (json!("review"), review.to_string()),
            (
                json!("pending"),
                format!("Status: {status} (toggle Pending)"),
            ),
        ];
        if reply {
            choices.push((json!("cc"), "CC emails".into()));
            choices.push((json!("bcc"), "BCC emails (no CC/status combination)".into()));
            choices.push((
                json!("sender"),
                "Configured sender (required for BCC)".into(),
            ));
        } else {
            choices.push((json!("notify"), "Notify-to emails".into()));
        }
        let title = if reply {
            "Reply recipient/status options"
        } else {
            "Note notification/status options"
        };
        self.push(
            Modal::choices(title, choices),
            Cont::Reviewed(Step::ConversationOption {
                item,
                body,
                public,
                reply,
                options,
            }),
        );
    }

    /// `_prepare_ticket_conversation`, then `_review_ticket_conversation`.
    pub(super) fn prepare_ticket_conversation(
        &mut self,
        item: Item,
        body: String,
        public: bool,
        reply: bool,
        options: ConvOptions,
    ) {
        if self.read_only {
            return;
        }
        let req = options.request(public, reply);
        self.spawn(
            move |b| match b.ticket_conversation_plan(&item.ref_id(), &body, &req) {
                Err(e) => failure(&e),
                Ok(plan) => {
                    let public = reply || plan.raw.get("private") == Some(&Value::Bool(false));
                    Msg::Ui(vec![push(
                        Modal::confirm(
                            "Review exact ticket conversation",
                            vec![dumps(&plan.review)],
                        ),
                        Step::ConversationReviewed { item, plan, public },
                    )])
                }
            },
        );
    }

    /// `_send_ticket_conversation`.
    fn send_ticket_conversation(&mut self, item: Item, plan: ReviewPlan, public: bool) {
        if self.read_only {
            return;
        }
        self.spawn(move |b| {
            sent(
                b.send_ticket_conversation(&plan, public),
                "Conversation result — do not resend",
                &TICKETS,
                item,
                "Exact conversation verified",
            )
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_options_split_emails_and_map_pending() {
        let o = ConvOptions {
            pending: true,
            cc: " a@x.test, ,b@x.test ".into(),
            sender: String::new(),
            ..Default::default()
        };
        let r = o.request(true, true);
        assert_eq!(r.cc, ["a@x.test", "b@x.test"]);
        assert!(r.notify.is_empty() && r.bcc.is_empty());
        assert_eq!((r.sender, r.set_status), (None, Some("Pending".into())));
        let r = ConvOptions {
            sender: "help@x.test".into(),
            ..Default::default()
        }
        .request(false, false);
        assert_eq!(
            (r.sender.as_deref(), r.set_status),
            (Some("help@x.test"), None)
        );
        assert!(!r.reply && !r.public);
    }

    #[test]
    fn planning_choices_fall_back_like_python() {
        let defs = vec![
            json!({"name": "acme_plan", "label": "Acme plan", "id": 1}),
            json!({"name": "", "id": 2, "label": ""}),
            json!({"id": 3}),
        ];
        assert_eq!(
            planning_choices(&defs),
            vec![
                (json!("acme_plan"), "Acme plan".to_string()),
                (json!("2"), String::new()),
                (json!("3"), "None".to_string()),
            ]
        );
    }

    #[test]
    fn planning_row_unsafe_needs_description_and_empty_attachments() {
        assert!(!planning_row_unsafe(&json!({})));
        assert!(!planning_row_unsafe(
            &json!({"description": "x", "attachments": []})
        ));
        assert!(planning_row_unsafe(&json!({"attachments": []})));
        assert!(planning_row_unsafe(&json!({"description": "x"})));
        assert!(planning_row_unsafe(
            &json!({"description": "x", "attachments": [{"id": 1}]})
        ));
    }
}
