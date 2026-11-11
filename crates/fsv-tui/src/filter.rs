//! The `/` filter input: field/value suggestions, Tab completion, history
//! (`↑`/`ctrl+p` older, `↓`/`ctrl+n` newer) and the suggestion bar.

use fsv_core::term::escape;
use serde_json::Value;

use crate::backend::Item;
use crate::input::LineInput;

const OPERATORS: &[&str] = &[">=", "<=", "!=", "~=", "=", ">", "<"];
const PSEUDO_DATE_FIELDS: &[&str] = &["created_at", "updated_at", "due_by"];
const WINDOW: usize = 8;
pub const HISTORY_MAX: usize = 100;

pub fn split_filter_token(token: &str) -> Option<(String, &'static str, String)> {
    for op in OPERATORS {
        if let Some((l, r)) = token.split_once(op) {
            return Some((l.trim().to_string(), op, r.to_string()));
        }
    }
    None
}

/// POSIX `shlex.split` (no comments). `None` on an unclosed quote or a
/// trailing escape.
pub fn shlex_split(s: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if in_word {
                    out.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            '\\' => {
                in_word = true;
                cur.push(chars.next()?);
            }
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        None => return None,
                        Some('\'') => break,
                        Some(ch) => cur.push(ch),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        None => return None,
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            None => return None,
                            Some(n @ ('\\' | '"' | '$' | '`' | '\n')) => cur.push(n),
                            Some(n) => {
                                cur.push('\\');
                                cur.push(n);
                            }
                        },
                        Some(ch) => cur.push(ch),
                    }
                }
            }
            c => {
                in_word = true;
                cur.push(c);
            }
        }
    }
    if in_word {
        out.push(cur);
    }
    Some(out)
}

fn lower(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase()
}

/// Names/values seen in the loaded rows (`_extract_item_values`).
pub fn extract_item_values(items: &[Item], fname: &str, prefix: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let want = prefix.to_lowercase();
    for item in items {
        let mut raw = item.get(fname).filter(|v| !v.is_null());
        if raw.is_none() && fname == "responder" {
            raw = item.get("agent").filter(|v| !v.is_null());
        }
        let mut val: Option<String> = match raw {
            Some(Value::Object(o)) => o.get("name").filter(|v| !v.is_null()).map(fsv_py::py_str),
            Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
            _ => None,
        };
        if val.is_none() {
            val = item
                .get("custom_fields")
                .and_then(|cf| cf.get(fname))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
        }
        if let Some(v) = val.filter(|v| !v.is_empty() && v.to_lowercase().starts_with(&want))
            && !out.contains(&v)
        {
            out.push(v);
            if out.len() >= 8 {
                break;
            }
        }
    }
    out
}

/// What completion reads from the app.
pub struct Source<'a> {
    pub fields: &'a [Value],
    pub items: &'a [Item],
    pub groups: &'a dyn Fn() -> Vec<String>,
}

fn all_fields(fields: &[Value]) -> Vec<String> {
    let mut out: Vec<String> = PSEUDO_DATE_FIELDS.iter().map(|s| s.to_string()).collect();
    for f in fields {
        let name = f.get("name").and_then(Value::as_str).unwrap_or("");
        if !name.is_empty() && !out.iter().any(|o| o == name) {
            out.push(name.to_string());
        }
    }
    out
}

fn match_field<'a>(fields: &'a [Value], field_lower: &str) -> Option<&'a Value> {
    fields
        .iter()
        .find(|f| lower(f, "name") == field_lower)
        .or_else(|| {
            fields
                .iter()
                .find(|f| lower(f, "name").starts_with(field_lower))
        })
}

/// `FsvApp._get_suggestions`: `(completions, hint)`.
pub fn suggestions(value: &str, src: &Source<'_>) -> (Vec<String>, Option<String>) {
    let fields = src.fields;
    if value.is_empty() {
        return (all_fields(fields), None);
    }
    if shlex_split(value).is_none() {
        return (Vec::new(), None);
    }
    if value.ends_with(' ') {
        return (all_fields(fields), None);
    }
    let current = value.rsplit(' ').next().unwrap_or("");
    if current.is_empty() {
        return (Vec::new(), None);
    }
    let cur_lower = current.to_lowercase();
    let Some((field_part, _op, val_prefix)) = split_filter_token(current) else {
        let mut matches: Vec<String> = Vec::new();
        for name in PSEUDO_DATE_FIELDS {
            if name.starts_with(&cur_lower) && !matches.iter().any(|m| m == name) {
                matches.push(name.to_string());
            }
        }
        for f in fields {
            let name = f.get("name").and_then(Value::as_str).unwrap_or("");
            if (name.to_lowercase().starts_with(&cur_lower)
                || lower(f, "label").starts_with(&cur_lower))
                && !matches.iter().any(|m| m == name)
            {
                matches.push(name.to_string());
            }
        }
        return (matches, None);
    };
    let field_lower = field_part.to_lowercase();
    let val_lower = val_prefix.to_lowercase();
    if PSEUDO_DATE_FIELDS.contains(&field_lower.as_str()) {
        return (
            Vec::new(),
            Some("YYYY-MM-DD  (operators: = != > >= < <=)".into()),
        );
    }
    let Some(matched) = match_field(fields, &field_lower) else {
        return (Vec::new(), None);
    };
    let ftype = lower(matched, "field_type");
    let fname = lower(matched, "name");
    let choices = matched
        .get("choices")
        .and_then(Value::as_array)
        .filter(|c| !c.is_empty());
    let Some(choices) = choices else {
        let from_items = |name: &str, hint: &str| {
            let vals = extract_item_values(src.items, name, &val_prefix);
            if vals.is_empty() {
                (vals, Some(hint.to_string()))
            } else {
                (vals, None)
            }
        };
        if fname == "requester" || ftype.contains("requester") {
            return from_items("requester", "name or email");
        }
        if fname == "agent" || fname == "responder" || ftype.contains("agent") {
            return from_items("responder", "agent name");
        }
        if fname == "group" || ftype.contains("group") {
            let mut vals = extract_item_values(src.items, "group", &val_prefix);
            if vals.is_empty() {
                vals = (src.groups)()
                    .into_iter()
                    .filter(|g| g.to_lowercase().starts_with(&val_lower))
                    .collect();
            }
            return if vals.is_empty() {
                (vals, Some("group name".into()))
            } else {
                (vals, None)
            };
        }
        if fname == "department" || ftype.contains("department") {
            return from_items("department", "department name");
        }
        if ftype.contains("lookup") {
            return from_items(&fname, "name");
        }
        if ftype.contains("checkbox") {
            return (vec!["true".into(), "false".into()], None);
        }
        if ftype.contains("number") {
            return (Vec::new(), Some("number".into()));
        }
        if ftype.contains("date") {
            return (Vec::new(), Some("YYYY-MM-DD".into()));
        }
        return (Vec::new(), Some("text".into()));
    };
    let mut out = Vec::new();
    for c in choices {
        let v = ["value", "name"]
            .iter()
            .find_map(|k| c.get(*k).filter(|v| fsv_py::truthy(v)))
            .map(fsv_py::py_str)
            .unwrap_or_default();
        if !v.is_empty() && v.to_lowercase().starts_with(&val_lower) {
            out.push(v);
        }
    }
    (out, None)
}

/// A network lookup the app should (re)schedule after 0.3 s.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LookupRequest {
    pub key: String,
    pub prefix: String,
    pub kind: String,
    pub link: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Lookup {
    Nothing,
    Cancel,
    Schedule(LookupRequest),
}

/// The part of `_maybe_schedule_network_lookup` after the network check.
pub fn network_lookup(value: &str, fields: &[Value], cycling: bool, current_key: &str) -> Lookup {
    if cycling || value.ends_with(' ') {
        return Lookup::Nothing;
    }
    let current = value.rsplit(' ').next().unwrap_or("");
    let Some((field_part, _, prefix)) = split_filter_token(current) else {
        return Lookup::Nothing;
    };
    if prefix.trim().chars().count() < 2 {
        return Lookup::Cancel;
    }
    let fl = field_part.to_lowercase();
    let Some(matched) = fields.iter().find(|f| lower(f, "name") == fl) else {
        return Lookup::Nothing;
    };
    let fname = lower(matched, "name");
    let ftype = lower(matched, "field_type");
    let (kind, link) = if fname == "requester" || ftype.contains("requester") {
        ("requesters", None)
    } else if fname == "agent" || fname == "responder" || ftype.contains("agent") {
        ("agents", None)
    } else if ftype.contains("lookup") {
        match matched
            .get("lookup_config")
            .and_then(|c| c.get("link"))
            .and_then(Value::as_str)
            .filter(|l| !l.is_empty())
        {
            Some(l) => ("lookup", Some(l.to_string())),
            None => return Lookup::Nothing,
        }
    } else {
        return Lookup::Nothing;
    };
    let key = format!("{kind}:{prefix}");
    if key == current_key {
        return Lookup::Nothing;
    }
    Lookup::Schedule(LookupRequest {
        key,
        prefix,
        kind: kind.into(),
        link,
    })
}

/// Lookup rows → suggestion values (email first when it matches).
pub fn lookup_values(mut rows: Vec<Value>, prefix: &str) -> Vec<String> {
    let r = prefix.to_lowercase();
    let starts = |row: &Value, k: &str| {
        row.get(k)
            .filter(|v| fsv_py::truthy(v))
            .map(fsv_py::py_str)
            .unwrap_or_default()
            .to_lowercase()
            .starts_with(&r)
    };
    rows.sort_by_key(|row| {
        if starts(row, "email") || starts(row, "name") {
            0
        } else {
            1
        }
    });
    let mut out: Vec<String> = Vec::new();
    for row in rows.iter().take(8) {
        let name = row
            .get("name")
            .filter(|v| fsv_py::truthy(v))
            .map(fsv_py::py_str)
            .unwrap_or_default();
        let email = row
            .get("email")
            .filter(|v| fsv_py::truthy(v))
            .map(fsv_py::py_str)
            .unwrap_or_default();
        let v = if !email.is_empty() && email.to_lowercase().starts_with(&r) {
            email
        } else {
            name
        };
        if !v.is_empty() && !out.contains(&v) {
            out.push(v);
        }
    }
    out
}

/// Filter input state.
#[derive(Debug, Default)]
pub struct FilterState {
    pub visible: bool,
    pub input: LineInput,
    pub suggestions: Vec<String>,
    pub idx: usize,
    pub tab_base: Option<String>,
    /// Suggestion bar markup; `None` hides it.
    pub bar: Option<String>,
    pub history: Vec<String>,
    pub history_idx: isize,
    pub history_saved: String,
    pub lookup_key: String,
}

impl FilterState {
    pub fn new(history: Vec<String>) -> FilterState {
        let mut history = history;
        history.truncate(HISTORY_MAX);
        FilterState {
            input: LineInput::new("status=Open priority=High"),
            history,
            history_idx: -1,
            ..Default::default()
        }
    }

    pub fn open(&mut self, current: &str, src: &Source<'_>) {
        self.input.set_value(current);
        self.visible = true;
        self.history_idx = if self.history.first().is_some_and(|h| h == current.trim()) {
            0
        } else {
            -1
        };
        self.history_saved = current.to_string();
        self.idx = 0;
        self.tab_base = None;
        self.update_bar(src);
    }

    /// Hide and reset suggestion / history state (cancel and apply).
    pub fn close(&mut self) {
        self.visible = false;
        self.bar = None;
        self.suggestions.clear();
        self.idx = 0;
        self.tab_base = None;
        self.lookup_key.clear();
        self.history_idx = -1;
    }

    /// Record an applied filter at the top of the history.
    pub fn remember(&mut self, entry: &str) {
        self.history.retain(|h| h != entry);
        self.history.insert(0, entry.to_string());
        self.history.truncate(HISTORY_MAX);
    }

    /// Typing (not code) changed the value.
    pub fn typed(&mut self, src: &Source<'_>) {
        self.history_idx = -1;
        self.idx = 0;
        self.tab_base = None;
        self.update_bar(src);
    }

    /// `direction`: +1 older (up / ctrl+p), -1 newer (down / ctrl+n).
    pub fn history_navigate(&mut self, direction: isize, src: &Source<'_>) {
        if self.history.is_empty() && direction > 0 {
            return;
        }
        if self.history_idx == -1 {
            self.history_saved = self.input.value().to_string();
        }
        let new = self.history_idx + direction;
        if new >= self.history.len() as isize || new < -1 {
            return;
        }
        self.history_idx = new;
        self.idx = 0;
        self.tab_base = None;
        let v = if new == -1 {
            self.history_saved.clone()
        } else {
            self.history[new as usize].clone()
        };
        self.input.set_value(&v);
        self.update_bar(src);
    }

    /// `FsvApp._update_suggestion_bar`: first 8 suggestions, else the hint.
    pub fn update_bar(&mut self, src: &Source<'_>) {
        let (sugg, hint) = suggestions(self.input.value(), src);
        self.suggestions = sugg;
        if self.suggestions.is_empty() {
            self.bar = hint.map(|h| format!("[dim]{}[/]", escape(&h)));
            return;
        }
        let visible = &self.suggestions[..self.suggestions.len().min(WINDOW)];
        let overflow = self.suggestions.len().saturating_sub(WINDOW);
        let mut parts = vec![format!("[bold cyan]{}[/]", escape(&visible[0]))];
        parts.extend(
            visible[1..]
                .iter()
                .map(|s| format!("[dim]{}[/]", escape(s))),
        );
        let suffix = if overflow > 0 {
            format!("  [dim](+{overflow} more)  tab→[/]")
        } else {
            "  [dim]tab→[/]".to_string()
        };
        self.bar = Some(parts.join("  ") + &suffix);
    }

    /// `FsvApp._show_suggestions_cycling`: an 8-wide window around `active`.
    fn show_cycling(&mut self, list: &[String], active: usize) {
        let total = list.len();
        let (lo, hi) = if total <= WINDOW {
            (0, total)
        } else {
            let lo = (active as isize - (WINDOW / 2) as isize).clamp(0, (total - WINDOW) as isize)
                as usize;
            (lo, lo + WINDOW)
        };
        let mut parts = Vec::new();
        if lo > 0 {
            parts.push("[dim]…[/]".to_string());
        }
        for (i, s) in list.iter().enumerate().take(hi).skip(lo) {
            parts.push(if i == active {
                format!("[bold cyan]{}[/]", escape(s))
            } else {
                format!("[dim]{}[/]", escape(s))
            });
        }
        if hi < total {
            parts.push("[dim]…[/]".to_string());
        }
        self.bar = Some(parts.join("  ") + "  [dim]tab→[/]");
    }

    /// `FsvApp._complete_suggestion`: Tab completes the current token,
    /// cycling through the suggestions.
    pub fn complete(&mut self, src: &Source<'_>) {
        let value = self.input.value().to_string();
        if self.tab_base.is_none() {
            self.tab_base = Some(value.clone());
            self.idx = 0;
        }
        let base = self.tab_base.clone().unwrap_or_default();
        let list = if self.suggestions.is_empty() {
            suggestions(&base, src).0
        } else {
            self.suggestions.clone()
        };
        if list.is_empty() {
            self.tab_base = None;
            return;
        }
        let (prefix, current) = match base.rfind(' ') {
            Some(i) => (base[..=i].to_string(), base[i + 1..].to_string()),
            None => (String::new(), base.clone()),
        };
        let idx = self.idx % list.len();
        let s = &list[idx];
        let new_value = match split_filter_token(&current) {
            None if list.len() == 1 => {
                let nv = format!("{prefix}{s}=");
                self.tab_base = None;
                self.idx = 0;
                if nv != value {
                    self.input.set_value(&nv);
                }
                self.update_bar(src);
                return;
            }
            None => {
                self.idx = (idx + 1) % list.len();
                format!("{prefix}{s}")
            }
            Some((field, op, _)) => {
                self.idx = (idx + 1) % list.len();
                let v = if s.contains(' ') {
                    format!("\"{s}\"")
                } else {
                    s.clone()
                };
                format!("{prefix}{field}{op}{v}")
            }
        };
        if new_value != value {
            self.input.set_value(&new_value);
        }
        self.show_cycling(&list, idx);
    }

    /// `FsvApp._apply_network_suggestions`: network results for `key`
    /// (dropped when stale, cycling or hidden).
    pub fn apply_network(&mut self, key: &str, values: Vec<String>) {
        if key != self.lookup_key || self.tab_base.is_some() || !self.visible || values.is_empty() {
            return;
        }
        let mut parts = vec![format!("[bold cyan]{}[/]", escape(&values[0]))];
        parts.extend(values[1..].iter().map(|s| format!("[dim]{}[/]", escape(s))));
        self.bar = Some(parts.join("  ") + "  [dim]tab→[/]");
        self.suggestions = values;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fields() -> Vec<Value> {
        vec![
            json!({"name": "status", "label": "Status", "field_type": "default_status", "choices": [{"id": 2, "value": "Open"}, {"id": 3, "value": "Pending"}]}),
            json!({"name": "priority", "label": "Priority", "choices": [{"id": 1, "value": "Low"}, {"id": 3, "value": "High"}]}),
            json!({"name": "group", "label": "Group", "field_type": "default_group"}),
            json!({"name": "summary", "label": "Summary", "field_type": "custom_text"}),
        ]
    }

    #[test]
    fn shlex_and_tokens() {
        assert_eq!(
            shlex_split(r#"a="b c" d"#).unwrap_or_default(),
            vec!["a=b c", "d"]
        );
        assert!(shlex_split("a=\"open").is_none());
        assert_eq!(
            split_filter_token("created_at>=2026-01-01"),
            Some(("created_at".into(), ">=", "2026-01-01".into()))
        );
    }

    #[test]
    fn suggestion_kinds() {
        let f = fields();
        let groups = || vec!["Network Ops".to_string(), "Desk".to_string()];
        let src = Source {
            fields: &f,
            items: &[],
            groups: &groups,
        };
        assert_eq!(suggestions("st", &src).0, vec!["status"]);
        assert_eq!(suggestions("status=o", &src).0, vec!["Open"]);
        assert_eq!(
            suggestions("created_at=", &src).1.as_deref(),
            Some("YYYY-MM-DD  (operators: = != > >= < <=)")
        );
        assert_eq!(suggestions("group=net", &src).0, vec!["Network Ops"]);
        assert_eq!(suggestions("summary=x", &src).1.as_deref(), Some("text"));
        assert!(suggestions("status=\"op", &src).0.is_empty());
        assert_eq!(suggestions("status=Open ", &src).0[0], "created_at");
    }

    #[test]
    fn tab_completes_and_cycles() {
        let f = fields();
        let groups = Vec::new;
        let src = Source {
            fields: &f,
            items: &[],
            groups: &groups,
        };
        let mut st = FilterState::new(Vec::new());
        st.open("", &src);
        st.input.set_value("stat");
        st.typed(&src);
        st.complete(&src);
        assert_eq!(st.input.value(), "status=");
        st.typed(&src);
        st.complete(&src);
        assert_eq!(st.input.value(), "status=Open");
        st.complete(&src);
        assert_eq!(st.input.value(), "status=Pending");
        st.complete(&src);
        assert_eq!(st.input.value(), "status=Open");
    }

    #[test]
    fn history_older_newer() {
        let groups = Vec::new;
        let src = Source {
            fields: &[],
            items: &[],
            groups: &groups,
        };
        let mut st = FilterState::new(vec!["a=1".into(), "b=2".into()]);
        st.open("typed", &src);
        st.history_navigate(1, &src);
        assert_eq!(st.input.value(), "a=1");
        st.history_navigate(1, &src);
        assert_eq!(st.input.value(), "b=2");
        st.history_navigate(1, &src);
        assert_eq!(st.input.value(), "b=2");
        st.history_navigate(-1, &src);
        st.history_navigate(-1, &src);
        assert_eq!(st.input.value(), "typed");
        st.remember("b=2");
        assert_eq!(st.history, vec!["b=2", "a=1"]);
    }

    // Expected bars: the Python methods run on the same input, with rich's
    // normalised closing tags (`[/dim]`, `[/bold cyan]`) written as `[/]`.

    fn bar_fields() -> Vec<Value> {
        let choices: Vec<Value> = (1..12)
            .map(|i| json!({"id": i, "value": format!("S{i}")}))
            .collect();
        vec![
            json!({"name": "status", "label": "Status", "choices": choices}),
            json!({"name": "requester", "label": "Requester", "field_type": "default_requester"}),
            json!({"name": "summary", "label": "Summary", "field_type": "custom_text"}),
        ]
    }

    fn bar_items() -> Vec<Item> {
        let t = &fsv_core::resource::TICKETS;
        [
            json!({"requester": {"name": "Alice"}}),
            json!({"requester": "alan@x.io"}),
            json!({"requester": null, "custom_fields": {"requester": "Al Custom"}}),
            json!({"responder": null, "agent": {"name": "Agent A"}}),
            json!({"requester": {"name": "Alice"}}),
            json!({"requester": {"name": "Bob"}}),
        ]
        .into_iter()
        .map(|d| Item::new(t, d))
        .collect()
    }

    #[test]
    fn extract_item_values_names_strings_custom_and_agent() {
        let items = bar_items();
        assert_eq!(
            extract_item_values(&items, "requester", "al"),
            vec!["Alice", "alan@x.io", "Al Custom"]
        );
        assert_eq!(
            extract_item_values(&items, "responder", ""),
            vec!["Agent A"]
        );
    }

    #[test]
    fn update_bar_overflow_hint_and_item_values() {
        let f = bar_fields();
        let items = bar_items();
        let groups = Vec::new;
        let src = Source {
            fields: &f,
            items: &items,
            groups: &groups,
        };
        let mut st = FilterState::new(Vec::new());
        st.input.set_value("status=");
        st.update_bar(&src);
        assert_eq!(
            st.bar.as_deref(),
            Some(
                "[bold cyan]S1[/]  [dim]S2[/]  [dim]S3[/]  [dim]S4[/]  [dim]S5[/]  [dim]S6[/]  [dim]S7[/]  [dim]S8[/]  [dim](+3 more)  tab→[/]"
            )
        );
        assert_eq!(st.suggestions.len(), 11);
        st.input.set_value("summary=x");
        st.update_bar(&src);
        assert_eq!(st.bar.as_deref(), Some("[dim]text[/]"));
        st.input.set_value("requester=al");
        st.update_bar(&src);
        assert_eq!(
            st.bar.as_deref(),
            Some("[bold cyan]Alice[/]  [dim]alan@x.io[/]  [dim]Al Custom[/]  [dim]tab→[/]")
        );
        assert_eq!(
            suggestions("requester=al", &src),
            (
                vec!["Alice".to_string(), "alan@x.io".into(), "Al Custom".into()],
                None
            )
        );
    }

    #[test]
    fn show_cycling_windows_around_the_active_suggestion() {
        let list: Vec<String> = (1..12).map(|i| format!("S{i}")).collect();
        let mut st = FilterState::new(Vec::new());
        st.show_cycling(&list, 0);
        assert_eq!(
            st.bar.as_deref(),
            Some(
                "[bold cyan]S1[/]  [dim]S2[/]  [dim]S3[/]  [dim]S4[/]  [dim]S5[/]  [dim]S6[/]  [dim]S7[/]  [dim]S8[/]  [dim]…[/]  [dim]tab→[/]"
            )
        );
        st.show_cycling(&list, 6);
        assert_eq!(
            st.bar.as_deref(),
            Some(
                "[dim]…[/]  [dim]S3[/]  [dim]S4[/]  [dim]S5[/]  [dim]S6[/]  [bold cyan]S7[/]  [dim]S8[/]  [dim]S9[/]  [dim]S10[/]  [dim]…[/]  [dim]tab→[/]"
            )
        );
        st.show_cycling(&list, 10);
        assert_eq!(
            st.bar.as_deref(),
            Some(
                "[dim]…[/]  [dim]S4[/]  [dim]S5[/]  [dim]S6[/]  [dim]S7[/]  [dim]S8[/]  [dim]S9[/]  [dim]S10[/]  [bold cyan]S11[/]  [dim]tab→[/]"
            )
        );
        st.show_cycling(&["a".into(), "b".into()], 1);
        assert_eq!(
            st.bar.as_deref(),
            Some("[dim]a[/]  [bold cyan]b[/]  [dim]tab→[/]")
        );
    }

    #[test]
    fn apply_network_shows_fresh_results_and_drops_stale() {
        let mut st = FilterState::new(Vec::new());
        st.visible = true;
        st.lookup_key = "requesters:al".into();
        st.apply_network("requesters:al", vec!["al@x.io".into(), "Alan [x]".into()]);
        assert_eq!(
            st.bar.as_deref(),
            Some("[bold cyan]al@x.io[/]  [dim]Alan \\[x][/]  [dim]tab→[/]")
        );
        assert_eq!(st.suggestions, vec!["al@x.io", "Alan [x]"]);
        st.bar = None;
        st.apply_network("requesters:zz", vec!["q".into()]);
        assert_eq!(st.bar, None);
        st.tab_base = Some("requester=al".into());
        st.apply_network("requesters:al", vec!["q".into()]);
        assert_eq!(st.bar, None);
    }
}
