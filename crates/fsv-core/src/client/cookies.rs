//! Cookie jar for the client. Cookies from the tenant host are kept
//! host-only on the tenant (any `Domain` attribute is ignored), so session
//! cookies are never sent to another host.

use url::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    /// Lowercase host or domain, no leading dot.
    pub domain: String,
    pub host_only: bool,
    pub path: String,
    pub secure: bool,
}

impl Cookie {
    fn domain_matches(&self, host: &str) -> bool {
        host == self.domain || (!self.host_only && host.ends_with(&format!(".{}", self.domain)))
    }

    fn path_matches(&self, path: &str) -> bool {
        path == self.path
            || (path.starts_with(&self.path)
                && (self.path.ends_with('/') || path[self.path.len()..].starts_with('/')))
    }
}

#[derive(Clone, Debug, Default)]
pub struct CookieJar {
    tenant: String,
    cookies: Vec<Cookie>,
}

impl CookieJar {
    pub fn new(tenant: &str) -> Self {
        CookieJar {
            tenant: tenant.to_ascii_lowercase(),
            cookies: Vec::new(),
        }
    }

    pub fn cookies(&self) -> &[Cookie] {
        &self.cookies
    }

    /// `_make_cookie`: host-scoped, Secure session cookie on the tenant.
    pub fn set_session(&mut self, name: &str, value: &str) {
        let tenant = self.tenant.clone();
        self.store(Cookie {
            name: name.into(),
            value: value.into(),
            domain: tenant,
            host_only: true,
            path: "/".into(),
            secure: true,
        });
    }

    fn store(&mut self, cookie: Cookie) {
        self.cookies.retain(|c| {
            !(c.name == cookie.name && c.domain == cookie.domain && c.path == cookie.path)
        });
        self.cookies.push(cookie);
    }

    /// Value of `name`, preferring the tenant-domain cookie when several exist (`_fw_cookie`).
    pub fn get(&self, name: &str) -> Option<&str> {
        let found: Vec<&Cookie> = self.cookies.iter().filter(|c| c.name == name).collect();
        let exact: Vec<&Cookie> = found
            .iter()
            .copied()
            .filter(|c| c.domain == self.tenant)
            .collect();
        let pick = if exact.is_empty() { found } else { exact };
        pick.last().map(|c| c.value.as_str())
    }

    /// `Cookie` header for a request to `url`, longest path first.
    pub fn header_for(&self, url: &Url) -> Option<String> {
        let host = url.host_str()?.to_ascii_lowercase();
        let path = if url.path().is_empty() {
            "/"
        } else {
            url.path()
        };
        let https = url.scheme() == "https";
        let mut matching: Vec<&Cookie> = self
            .cookies
            .iter()
            .filter(|c| c.domain_matches(&host) && c.path_matches(path) && (https || !c.secure))
            .collect();
        if matching.is_empty() {
            return None;
        }
        matching.sort_by_key(|c| std::cmp::Reverse(c.path.len()));
        Some(
            matching
                .iter()
                .map(|c| format!("{}={}", c.name, c.value))
                .collect::<Vec<_>>()
                .join("; "),
        )
    }

    /// Apply every `Set-Cookie` of a response from `url`.
    pub fn extract<'a>(&mut self, url: &Url, set_cookies: impl IntoIterator<Item = &'a str>) {
        let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
            return;
        };
        for raw in set_cookies {
            if let Some((cookie, expired)) = parse_set_cookie(raw, &host, url.path(), &self.tenant)
            {
                if expired {
                    self.cookies.retain(|c| {
                        !(c.name == cookie.name
                            && c.domain == cookie.domain
                            && c.path == cookie.path)
                    });
                } else {
                    self.store(cookie);
                }
            }
        }
    }
}

fn default_path(request_path: &str) -> String {
    if !request_path.starts_with('/') {
        return "/".into();
    }
    match request_path.rfind('/') {
        Some(0) | None => "/".into(),
        Some(i) => request_path[..i].to_string(),
    }
}

fn expires_in_past(value: &str) -> Option<bool> {
    let value = value.trim();
    for fmt in [
        "%a, %d %b %Y %H:%M:%S GMT",
        "%a, %d-%b-%Y %H:%M:%S GMT",
        "%A, %d-%b-%y %H:%M:%S GMT",
    ] {
        if let Ok(dt) = jiff::civil::DateTime::strptime(fmt, value) {
            let ts = dt.to_zoned(jiff::tz::TimeZone::UTC).ok()?.timestamp();
            return Some(ts <= jiff::Timestamp::now());
        }
    }
    None
}

/// `(cookie, expired)`, or `None` when the header is malformed or the domain is foreign.
fn parse_set_cookie(
    raw: &str,
    host: &str,
    request_path: &str,
    tenant: &str,
) -> Option<(Cookie, bool)> {
    let mut parts = raw.split(';');
    let (name, value) = parts.next()?.split_once('=')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let mut domain_attr: Option<String> = None;
    let mut path: Option<String> = None;
    let mut secure = false;
    let mut max_age: Option<i64> = None;
    let mut expires_past: Option<bool> = None;
    for attr in parts {
        let (k, v) = attr
            .split_once('=')
            .map_or((attr.trim(), ""), |(k, v)| (k.trim(), v.trim()));
        match k.to_ascii_lowercase().as_str() {
            "domain" => {
                let d = v.trim_start_matches('.').to_ascii_lowercase();
                if !d.is_empty() {
                    domain_attr = Some(d);
                }
            }
            "path" if v.starts_with('/') => path = Some(v.to_string()),
            "secure" => secure = true,
            "max-age" => max_age = v.parse().ok(),
            "expires" => expires_past = expires_in_past(v),
            _ => {}
        }
    }
    let (domain, host_only) = if host == tenant {
        (tenant.to_string(), true)
    } else if let Some(d) = domain_attr {
        if host != d && !host.ends_with(&format!(".{d}")) {
            return None;
        }
        (d, false)
    } else {
        (host.to_string(), true)
    };
    let expired = match max_age {
        Some(n) => n <= 0,
        None => expires_past.unwrap_or(false),
    };
    let cookie = Cookie {
        name: name.to_string(),
        value: value.trim().to_string(),
        domain,
        host_only,
        path: path.unwrap_or_else(|| default_path(request_path)),
        secure,
    };
    Some((cookie, expired))
}
