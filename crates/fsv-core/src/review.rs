//! The review screen every write ends in: what will be sent, in the web's
//! words, before anything is sent (`review.py`).

use serde_json::{Map, Value, json};

use crate::term::{Column, Console, Overflow, Table, escape};

/// Shown instead of a plan when validation failed.
pub const NO_PAYLOAD: &str = "No payload yet — fix the problems above";

/// One attachment queued for upload.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReviewAttachment {
    pub target: String,
    pub name: String,
    pub size: u64,
    pub sha256: String,
    pub path: String,
}

/// A rule message (`text`, `blocking`, `acknowledged`, `rule`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReviewMessage {
    pub text: String,
    pub blocking: bool,
    pub acknowledged: bool,
    pub rule: Value,
}

/// Everything the review screen shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Review {
    pub title: String,
    /// `(label, display value)` in web order.
    pub rows: Vec<(String, String)>,
    /// Updates: `(label, old, new)`.
    pub changes: Vec<(String, String, String)>,
    pub notify: Vec<String>,
    pub messages: Vec<ReviewMessage>,
    pub dropped: Vec<String>,
    /// Populated / disabled / replaced / conflicts.
    pub notes: Vec<String>,
    pub attachments: Vec<ReviewAttachment>,
    pub risks: Vec<String>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub metadata: Map<String, Value>,
    pub plan_hash: String,
}

fn group_thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn strs(v: &[String]) -> Value {
    Value::Array(v.iter().cloned().map(Value::String).collect())
}

impl Review {
    pub fn new(title: impl Into<String>) -> Self {
        Review { title: title.into(), ..Review::default() }
    }

    /// `dataclasses.asdict` shape with rows/changes as objects.
    pub fn to_dict(&self) -> Value {
        json!({
            "title": self.title,
            "rows": self.rows.iter().map(|(a, b)| json!({"label": a, "value": b})).collect::<Vec<_>>(),
            "changes": self.changes.iter().map(|(a, b, c)| json!({"label": a, "old": b, "new": c})).collect::<Vec<_>>(),
            "notify": strs(&self.notify),
            "messages": self.messages.iter().map(|m| json!({
                "text": m.text, "blocking": m.blocking, "acknowledged": m.acknowledged, "rule": m.rule,
            })).collect::<Vec<_>>(),
            "dropped": strs(&self.dropped),
            "notes": strs(&self.notes),
            "attachments": self.attachments.iter().map(|a| json!({
                "target": a.target, "name": a.name, "size": a.size, "sha256": a.sha256, "path": a.path,
            })).collect::<Vec<_>>(),
            "risks": strs(&self.risks),
            "errors": strs(&self.errors),
            "warnings": strs(&self.warnings),
            "metadata": self.metadata,
            "plan_hash": self.plan_hash,
        })
    }

    /// Texts of blocking messages not yet acknowledged.
    pub fn blocking_unacked(&self) -> Vec<String> {
        self.messages.iter().filter(|m| m.blocking && !m.acknowledged).map(|m| m.text.clone()).collect()
    }

    /// Print the review to `console`.
    pub fn render(&self, console: &Console) {
        let p = |s: &str| console.print(s);
        p(&format!("[bold]──────── Review: {} ────────[/bold]", escape(&self.title)));
        if !self.rows.is_empty() {
            let mut t = Table::new().show_header(false).no_box().pad_edge(false);
            t.add_column(Column::new("").style("cyan").no_wrap());
            t.add_column(Column::new("").overflow(Overflow::Fold));
            for (label, value) in &self.rows {
                t.add_row([escape(label), escape(value)]);
            }
            console.print_table(&t);
        }
        if !self.changes.is_empty() {
            let mut t = Table::new().show_header(true).no_box().pad_edge(false);
            t.add_column(Column::new("Field").style("cyan").no_wrap());
            t.add_column(Column::new("Old").overflow(Overflow::Fold));
            t.add_column(Column::new("").no_wrap());
            t.add_column(Column::new("New").overflow(Overflow::Fold));
            for (label, old, new) in &self.changes {
                let dash = |s: &str| if s.is_empty() { "—".to_string() } else { escape(s) };
                t.add_row([escape(label), dash(old), "→".to_string(), dash(new)]);
            }
            console.print_table(&t);
        }
        let section = |title: &str, items: &[String], style: &str| {
            if items.is_empty() {
                return;
            }
            p(&format!("[bold]{title}[/bold]"));
            for it in items {
                if style.is_empty() {
                    p(&format!("  {}", escape(it)));
                } else {
                    p(&format!("  [{style}]{}[/{style}]", escape(it)));
                }
            }
        };
        section("Will notify / create work for", &self.notify, "yellow");
        let msgs: Vec<String> = self
            .messages
            .iter()
            .map(|m| {
                let tag = if m.blocking && !m.acknowledged {
                    "BLOCKING (needs --ack or y)"
                } else if m.blocking {
                    "acknowledged"
                } else {
                    "message"
                };
                format!("[{tag}] {}", m.text)
            })
            .collect();
        section("Messages", &msgs, "magenta");
        section("Dropped (hidden by rules, not sent)", &self.dropped, "dim");
        section("Rule notes", &self.notes, "dim");
        if !self.attachments.is_empty() {
            let items: Vec<String> = self
                .attachments
                .iter()
                .map(|a| {
                    let short: String = a.sha256.chars().take(16).collect();
                    format!("{}: {}  {} B  sha256:{short}", a.target, a.name, group_thousands(a.size))
                })
                .collect();
            section("Attachments (uploaded after you confirm)", &items, "");
        }
        section("Requires confirmation", &self.risks, "yellow");
        section("Warnings", &self.warnings, "yellow");
        section("Problems (must be fixed before sending)", &self.errors, "red");
        if !self.metadata.is_empty() {
            let md = &self.metadata;
            let s = |k: &str| fsv_py::py_str(md.get(k).unwrap_or(&Value::Null));
            let asof = md.get("updated_at").filter(|v| crate::value::truthy(v)).map_or("unknown".to_string(), fsv_py::py_str);
            p(&format!(
                "[dim]form metadata as of {} (fetched {}, {}, fingerprint {})[/dim]",
                escape(&asof),
                escape(&s("fetched_at")),
                escape(&s("source")),
                escape(&s("fingerprint"))
            ));
        }
        if !self.plan_hash.is_empty() {
            p(&format!("[bold]plan[/bold] {}", self.plan_hash));
        }
    }
}
