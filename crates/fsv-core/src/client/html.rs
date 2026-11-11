//! CSRF token and login-page detection on Freshservice HTML pages.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

use crate::pystr;

static META: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<meta\b[^>]*>").expect("regex"));
static ATTR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"([^\s=/>"']+)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>]+))"#).expect("regex")
});
static FW_DOMAIN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"https://([A-Za-z0-9-]+\.myfreshworks\.com)").expect("regex"));

/// `<meta name="csrf-token" content="...">` regardless of attribute order,
/// quote style or case; entities in the content are unescaped.
pub fn csrf_from_html(html: &str) -> Option<String> {
    for tag in META.find_iter(html) {
        let attrs: HashMap<String, String> = ATTR
            .captures_iter(tag.as_str())
            .map(|c| {
                let value = (2..=4).find_map(|i| c.get(i)).map_or("", |m| m.as_str());
                (c[1].to_lowercase(), pystr::html_unescape(value).into_owned())
            })
            .collect();
        let is_csrf = attrs.get("name").is_some_and(|n| n.to_lowercase() == "csrf-token");
        if let Some(content) = attrs.get("content").filter(|c| is_csrf && !c.is_empty()) {
            return Some(content.clone());
        }
    }
    None
}

/// HTML with no CSRF meta that shows a password field or Freshid markers.
pub fn looks_like_login_html(html: &str) -> bool {
    if csrf_from_html(html).is_some() {
        return false;
    }
    let low = html.to_lowercase();
    low.contains("type=\"password\"") || low.contains("type='password'") || low.contains("freshid") || low.contains("/login")
}

/// Freshworks identity host linked from an app page (`acme.myfreshworks.com`).
pub fn fw_domain_from_html(html: &str) -> Option<String> {
    FW_DOMAIN.captures(html).map(|c| c[1].to_string())
}
