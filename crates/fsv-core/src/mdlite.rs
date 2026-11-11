//! A tiny, safe Markdown → HTML converter for descriptions (`mdlite.py`).
//!
//! Supports paragraphs, line breaks, `#` headings, `-`/`*`/`1.` lists (one
//! level), fenced code blocks, `inline code`, **bold**, *italic*, and
//! `[text](url)` links with http(s)/mailto URLs only. All text is
//! HTML-escaped first, so raw HTML in the Markdown is shown as text, never
//! injected. Raw HTML goes through `description_html` instead.

use std::sync::LazyLock;

use regex::{Captures, Regex};

use crate::pystr::{html_escape, html_unescape, strip};

fn re(p: &str) -> Regex {
    Regex::new(p).expect("regex")
}

static HEADING: LazyLock<Regex> = LazyLock::new(|| re(r"^(#{1,6})\s+(.*?)\s*#*\s*$"));
static UL: LazyLock<Regex> = LazyLock::new(|| re(r"^\s*[-*+]\s+(.*)$"));
static OL: LazyLock<Regex> = LazyLock::new(|| re(r"^\s*\d+[.)]\s+(.*)$"));
static FENCE: LazyLock<Regex> = LazyLock::new(|| re(r"^\s*(```|~~~)"));
static LINK: LazyLock<Regex> = LazyLock::new(|| re(r"\[([^\]]+)\]\(([^)\s]+)\)"));
static CODE: LazyLock<Regex> = LazyLock::new(|| re(r"`([^`]+)`"));
static BOLD: LazyLock<Regex> = LazyLock::new(|| re(r"\*\*(.+?)\*\*|__(.+?)__"));
static ITALIC: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
    fancy_regex::Regex::new(r"(?<![*\w])\*(?!\s)(.+?)(?<!\s)\*(?![*\w])|(?<![_\w])_(?!\s)(.+?)(?<!\s)_(?![_\w])")
        .expect("regex")
});
static SAFE_URL: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^(?:https?://|mailto:)"));
static STASHED: LazyLock<Regex> = LazyLock::new(|| re(r"\x00(\d+)\x00"));

fn group<'a>(c: &'a Captures<'_>, i: usize) -> &'a str {
    c.get(i).map_or("", |m| m.as_str())
}

/// Escape, then apply inline markup. Code spans are protected.
fn inline(text: &str) -> String {
    let mut codes: Vec<String> = Vec::new();
    let stashed = CODE.replace_all(text, |c: &Captures<'_>| {
        codes.push(format!("<code>{}</code>", html_escape(group(c, 1), false)));
        format!("\x00{}\x00", codes.len() - 1)
    });
    let out = html_escape(&stashed, false);
    let out = LINK.replace_all(&out, |c: &Captures<'_>| {
        let label = group(c, 1);
        let url = html_unescape(group(c, 2));
        if !SAFE_URL.is_match(&url) {
            return group(c, 0).to_string();
        }
        format!("<a href=\"{}\">{label}</a>", html_escape(&url, true))
    });
    let out = BOLD.replace_all(&out, |c: &Captures<'_>| {
        let inner = if c.get(1).is_some() { group(c, 1) } else { group(c, 2) };
        format!("<strong>{inner}</strong>")
    });
    let out = italic(&out);
    STASHED
        .replace_all(&out, |c: &Captures<'_>| {
            group(c, 1).parse::<usize>().ok().and_then(|i| codes.get(i).cloned()).unwrap_or_else(|| group(c, 0).into())
        })
        .into_owned()
}

fn italic(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for caps in ITALIC.captures_iter(text) {
        let Ok(caps) = caps else { return text.to_string() };
        let Some(whole) = caps.get(0) else { continue };
        out.push_str(&text[last..whole.start()]);
        let inner = caps.get(1).or_else(|| caps.get(2)).map_or("", |m| m.as_str());
        out.push_str(&format!("<em>{inner}</em>"));
        last = whole.end();
    }
    out.push_str(&text[last..]);
    out
}

/// Convert Markdown to HTML (`<p></p>` for empty input).
pub fn to_html(md: &str) -> String {
    let normalized = md.replace("\r\n", "\n");
    let lines: Vec<&str> = normalized.split('\n').collect();
    let mut out: Vec<String> = Vec::new();
    let mut para: Vec<String> = Vec::new();
    let mut items: Vec<String> = Vec::new();
    let mut list_tag = "";

    fn flush_para(out: &mut Vec<String>, para: &mut Vec<String>) {
        if !para.is_empty() {
            let parts: Vec<String> = para.iter().map(|p| inline(p)).collect();
            out.push(format!("<p>{}</p>", parts.join("<br>")));
            para.clear();
        }
    }
    fn flush_list(out: &mut Vec<String>, items: &mut Vec<String>, list_tag: &mut &str) {
        if !items.is_empty() {
            let lis: String = items.iter().map(|x| format!("<li>{}</li>", inline(x))).collect();
            out.push(format!("<{0}>{lis}</{0}>", list_tag));
            items.clear();
        }
        *list_tag = "";
    }

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some(fence) = FENCE.captures(line) {
            flush_para(&mut out, &mut para);
            flush_list(&mut out, &mut items, &mut list_tag);
            let fence = group(&fence, 1).to_string();
            let mut code: Vec<&str> = Vec::new();
            i += 1;
            while i < lines.len() && !strip(lines[i]).starts_with(fence.as_str()) {
                code.push(lines[i]);
                i += 1;
            }
            out.push(format!("<pre><code>{}</code></pre>", html_escape(&code.join("\n"), false)));
            i += 1;
            continue;
        }
        if strip(line).is_empty() {
            flush_para(&mut out, &mut para);
            flush_list(&mut out, &mut items, &mut list_tag);
            i += 1;
            continue;
        }
        if let Some(m) = HEADING.captures(line) {
            flush_para(&mut out, &mut para);
            flush_list(&mut out, &mut items, &mut list_tag);
            let n = group(&m, 1).len();
            out.push(format!("<h{n}>{}</h{n}>", inline(group(&m, 2))));
            i += 1;
            continue;
        }
        let (mu, mo) = (UL.captures(line), OL.captures(line));
        if mu.is_some() || mo.is_some() {
            flush_para(&mut out, &mut para);
            let tag = if mu.is_some() { "ul" } else { "ol" };
            if !list_tag.is_empty() && list_tag != tag {
                flush_list(&mut out, &mut items, &mut list_tag);
            }
            list_tag = tag;
            let m = mu.or(mo);
            items.push(m.as_ref().map_or("", |c| group(c, 1)).to_string());
            i += 1;
            continue;
        }
        if !items.is_empty() && (line.starts_with(' ') || line.starts_with('\t')) {
            if let Some(last) = items.last_mut() {
                last.push(' ');
                last.push_str(strip(line));
            }
            i += 1;
            continue;
        }
        flush_list(&mut out, &mut items, &mut list_tag);
        para.push(strip(line).to_string());
        i += 1;
    }
    flush_para(&mut out, &mut para);
    flush_list(&mut out, &mut items, &mut list_tag);
    let html = out.concat();
    if html.is_empty() { "<p></p>".into() } else { html }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_is_converted_safely() {
        assert_eq!(
            to_html("# T\n\nHi **b** `x<y>` [l](https://example.com)"),
            "<h1>T</h1><p>Hi <strong>b</strong> <code>x&lt;y&gt;</code> <a href=\"https://example.com\">l</a></p>"
        );
        assert!(!to_html("<script>alert(1)</script>").contains("<script>"));
        assert!(!to_html("[x](javascript:alert(1))").contains("href"));
        assert_eq!(to_html("1. a\n2. b"), "<ol><li>a</li><li>b</li></ol>");
    }
}
