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
        if i > 0 && (s.len() - i).is_multiple_of(3) {
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
        Review {
            title: title.into(),
            ..Review::default()
        }
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
        self.messages
            .iter()
            .filter(|m| m.blocking && !m.acknowledged)
            .map(|m| m.text.clone())
            .collect()
    }

    /// Print the review to `console`.
    pub fn render(&self, console: &Console) {
        let p = |s: &str| console.print(s);
        p(&format!(
            "[bold]──────── Review: {} ────────[/bold]",
            escape(&self.title)
        ));
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
                let dash = |s: &str| {
                    if s.is_empty() {
                        "—".to_string()
                    } else {
                        escape(s)
                    }
                };
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
                    format!(
                        "{}: {}  {} B  sha256:{short}",
                        a.target,
                        a.name,
                        group_thousands(a.size)
                    )
                })
                .collect();
            section("Attachments (uploaded after you confirm)", &items, "");
        }
        section("Requires confirmation", &self.risks, "yellow");
        section("Warnings", &self.warnings, "yellow");
        section(
            "Problems (must be fixed before sending)",
            &self.errors,
            "red",
        );
        if !self.metadata.is_empty() {
            let md = &self.metadata;
            let s = |k: &str| fsv_py::py_str(md.get(k).unwrap_or(&Value::Null));
            let asof = md
                .get("updated_at")
                .filter(|v| crate::value::truthy(v))
                .map_or("unknown".to_string(), fsv_py::py_str);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Review {
        let msg = |text: &str, blocking, acknowledged, rule: Value| ReviewMessage {
            text: text.into(),
            blocking,
            acknowledged,
            rule,
        };
        Review {
            title: "Change CHN-1 [x]".into(),
            rows: vec![("Subject".into(), "Fix [bold]it[/bold]".into()), ("Priority".into(), "High".into())],
            changes: vec![
                ("Status".into(), "Open".into(), String::new()),
                ("Group".into(), String::new(), "Ops".into()),
            ],
            notify: vec!["Requester".into()],
            messages: vec![
                msg("Need CAB", true, false, Value::Null),
                msg("FYI", true, true, json!(1)),
                msg("hello", false, false, Value::Null),
            ],
            dropped: vec!["Hidden".into()],
            notes: vec!["populated X".into()],
            attachments: vec![ReviewAttachment {
                target: "change".into(),
                name: "a.txt".into(),
                size: 1234567,
                sha256: "0123456789abcdef0123".into(),
                path: "/p".into(),
            }],
            risks: vec!["closes".into()],
            errors: vec!["bad".into()],
            warnings: vec!["warn".into()],
            metadata: json!({"updated_at": "2030-01-01", "fetched_at": "now", "source": "cache", "fingerprint": "abc"})
                .as_object()
                .cloned()
                .unwrap(),
            plan_hash: "deadbeef".into(),
        }
    }

    #[test]
    fn to_dict_and_blocking_unacked_match_python() {
        let r = sample();
        assert_eq!(
            r.to_dict(),
            json!({"attachments": [{"name": "a.txt", "path": "/p", "sha256": "0123456789abcdef0123", "size": 1234567, "target": "change"}],
                   "changes": [{"label": "Status", "new": "", "old": "Open"}, {"label": "Group", "new": "Ops", "old": ""}],
                   "dropped": ["Hidden"], "errors": ["bad"],
                   "messages": [{"acknowledged": false, "blocking": true, "rule": null, "text": "Need CAB"},
                                {"acknowledged": true, "blocking": true, "rule": 1, "text": "FYI"},
                                {"acknowledged": false, "blocking": false, "rule": null, "text": "hello"}],
                   "metadata": {"fetched_at": "now", "fingerprint": "abc", "source": "cache", "updated_at": "2030-01-01"},
                   "notes": ["populated X"], "notify": ["Requester"], "plan_hash": "deadbeef", "risks": ["closes"],
                   "rows": [{"label": "Subject", "value": "Fix [bold]it[/bold]"}, {"label": "Priority", "value": "High"}],
                   "title": "Change CHN-1 [x]", "warnings": ["warn"]})
        );
        assert_eq!(r.blocking_unacked(), ["Need CAB"]);
        assert_eq!(
            Review::new("t").to_dict(),
            json!({"attachments": [], "changes": [], "dropped": [], "errors": [], "messages": [], "metadata": {},
                   "notes": [], "notify": [], "plan_hash": "", "risks": [], "rows": [], "title": "t", "warnings": []})
        );
    }

    #[test]
    fn render_matches_rich_at_width_80() {
        let _env = crate::testenv::isolated();
        crate::testenv::unset("FORCE_COLOR");
        let ((), text) = crate::term::capture(|| sample().render(crate::term::out()));
        assert_eq!(
            text,
            "──────── Review: Change CHN-1 [x] ────────\nSubject   Fix [bold]it[/bold]\nPriority  High               \n\
Field   Old      New\nStatus  Open  →  —  \nGroup   —     →  Ops\nWill notify / create work for\n  Requester\nMessages\n  \
[BLOCKING (needs --ack or y)] Need CAB\n  [acknowledged] FYI\n  [message] hello\nDropped (hidden by rules, not sent)\n  Hidden\n\
Rule notes\n  populated X\nAttachments (uploaded after you confirm)\n  change: a.txt  1,234,567 B  sha256:0123456789abcdef\n\
Requires confirmation\n  closes\nWarnings\n  warn\nProblems (must be fixed before sending)\n  bad\n\
form metadata as of 2030-01-01 (fetched now, cache, fingerprint abc)\nplan deadbeef\n"
        );
        let mut bare = Review::new("t");
        bare.metadata.insert("x".into(), json!(1));
        let ((), text) = crate::term::capture(|| bare.render(crate::term::out()));
        assert_eq!(
            text,
            "──────── Review: t ────────\nform metadata as of unknown (fetched None, None, fingerprint None)\n"
        );
    }
}
