//! `fsv auth login|logout|status`.

use std::io::Read;
use std::sync::Arc;

use fsv_core::config;
use fsv_core::session::{self, Backend};
use fsv_core::term::{self, Column, Justify, Table, err, out};
use fsv_core::{Client, Ctx, Error, Result};

use crate::cmd::cache;
use crate::generated::{AuthLoginArgs, AuthLogoutArgs};
use crate::util::{CmdResult, client, fail, fail_exc, no_input, stdin_tty, usage};

/// `_resolve_domain`: `true` when the answer looked like a URL.
pub fn resolve_domain(local_no_input: bool) -> Result<bool> {
    let current = config::domain();
    if !current.is_empty() && !stdin_tty() {
        err().print(&format!("Domain: [bold]{current}[/]"));
        return Ok(false);
    }
    if current.is_empty() && no_input(local_no_input) {
        return Err(usage(
            "no Freshservice domain; pass `fsv auth login --domain ...` when using --no-input",
        ));
    }
    let default = if current.is_empty() {
        "acme.freshservice.com".to_string()
    } else {
        current
    };
    let answer = fsv_core::pystr::strip(&term::prompt("Domain", Some(&default), true)?).to_string();
    let chosen = if answer.is_empty() { default } else { answer };
    let looks_like_url = chosen.contains("://");
    config::set_domain(&chosen).map_err(fail_exc)?;
    Ok(looks_like_url)
}

/// `_validate_new_cookies`.
fn validate_new_cookies(cookies: &session::Cookies) -> CmdResult {
    let domain = config::domain();
    let probe = Client::with_cookies(Arc::new(Ctx::from_process()), cookies.clone())
        .and_then(|c| c.check_session());
    match probe {
        Ok(_) => Ok(()),
        Err(Error::Session(_)) => Err(fail(
            &format!(
                "these cookies are expired or invalid for {domain}; nothing saved (use --force to save anyway)"
            ),
            3,
        )),
        Err(e) => Err(fail(
            &format!(
                "could not verify cookies against {domain}: {e}; nothing saved (use --force to save anyway)"
            ),
            1,
        )),
    }
}

/// `_choose_store`.
pub fn choose_store() -> Result<Backend> {
    let mut options: Vec<(&str, &str)> = Vec::new();
    if cfg!(target_os = "macos") {
        options.push(("keychain", "macOS Keychain; recommended"));
    }
    options.push(("argon", "encrypted file; asks passphrase when used"));
    options.push(("file", "plain JSON file; chmod 600"));
    let choices: Vec<&str> = options.iter().map(|(n, _)| *n).collect();
    let current = session::current_backend();
    let default_idx = choices
        .iter()
        .position(|c| *c == current.as_str())
        .unwrap_or(0)
        + 1;

    let mut t = Table::new().title("Store session");
    t.add_column(Column::new("#").justify(Justify::Right));
    t.add_column(Column::new("Backend"));
    t.add_column(Column::new("Notes"));
    for (i, (name, note)) in options.iter().enumerate() {
        let marker = if i + 1 == default_idx {
            " [green](default)[/]"
        } else {
            ""
        };
        t.add_row([
            (i + 1).to_string(),
            format!("{name}{marker}"),
            note.to_string(),
        ]);
    }
    out().print_table(&t);
    loop {
        let answer = fsv_core::pystr::strip(&term::prompt(
            "Choose store",
            Some(&default_idx.to_string()),
            true,
        )?)
        .to_lowercase();
        if let Some(idx) = fsv_core::pystr::parse_decimal(&answer)
            && idx >= 1
            && (idx as usize) <= options.len()
        {
            return Ok(Backend::parse(options[idx as usize - 1].0).unwrap_or(current));
        }
        if let Some(b) = choices
            .iter()
            .find(|c| **c == answer)
            .and_then(|c| Backend::parse(c))
        {
            return Ok(b);
        }
        err().print(&format!(
            "[red]error[/red]: choose 1-{} or one of: {}",
            options.len(),
            choices.join(", ")
        ));
    }
}

/// Print the identity after a successful save (shared with `setup`).
fn report_login() {
    let domain = config::domain();
    session::clear_cache();
    fsv_core::client::reset_client();
    let checked = fsv_core::get_client().and_then(|c| c.check_session());
    match checked {
        Ok(agent) => {
            let s = |k: &str| agent.get(k).map(fsv_py::py_str).unwrap_or_default();
            let name = format!("{} {}", s("first_name"), s("last_name"))
                .trim()
                .to_string();
            if agent.as_object().is_some_and(|m| !m.is_empty()) {
                out().print(&format!(
                    "logged in as: [bold]{name}[/bold] <{}>",
                    s("email")
                ));
            } else {
                out().print("logged in (identity unavailable: agent profile is not readable)");
            }
            err().print_plain("warming cache in background...");
            cache::refresh_background();
        }
        Err(e) => {
            err().print(&format!("[yellow]warning[/yellow]: saved cookies but could not verify login for {domain}: {e}"));
            err().print("If this is the wrong tenant, rerun with `fsv auth login --domain acme.freshservice.com`.");
        }
    }
}

/// `fsv auth login`.
pub fn login(a: AuthLoginArgs) -> CmdResult {
    let no_input = no_input(a.no_input);
    if let Some(s) = &a.store
        && !matches!(s.as_str(), "file" | "argon" | "keychain")
    {
        return Err(usage("--store must be 'file', 'argon', or 'keychain'"));
    }
    if a.browser && a.header.is_some() {
        return Err(usage("use either --browser or --header, not both"));
    }
    if a.browser && no_input {
        return Err(usage("--browser is interactive; do not use --no-input"));
    }
    if a.header.is_none() && !a.browser && no_input {
        return Err(usage("pass --header when using --no-input"));
    }
    let explicit = a.domain.is_some();
    let mut url_pasted = false;
    if let Some(d) = &a.domain {
        config::set_domain(d).map_err(fail_exc)?;
    } else if config::domain().is_empty() || (!no_input && stdin_tty()) {
        url_pasted = resolve_domain(no_input)?;
    }
    if !explicit && stdin_tty() && url_pasted && !no_input {
        term::confirm(
            &format!("Extracted {} — login?", config::domain()),
            Some(true),
            true,
        )?;
    }
    let cookies = if a.browser {
        session::login_browser()
    } else if let Some(header) = &a.header {
        let header = if header == "-" {
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s)?;
            fsv_core::pystr::strip(&s).to_string()
        } else {
            header.clone()
        };
        let cookies = session::parse_cookie_header(&header);
        session::validate(&cookies).map(|_| cookies)
    } else {
        session::login_interactive()
    };
    let cookies = match cookies {
        Ok(c) => c,
        Err(e @ Error::Session(_)) => return Err(fail_exc(e)),
        Err(e) => return Err(e),
    };
    if !a.force {
        validate_new_cookies(&cookies)?;
    }
    let mut store = a.store.as_deref().and_then(Backend::parse);
    if store.is_none() && stdin_tty() && !no_input {
        store = Some(choose_store()?);
    }
    let backend = session::save_cookies(&cookies, store)?;
    let target = if backend == Backend::Keychain {
        "Keychain".to_string()
    } else {
        config::display_path(&config::session_file())
    };
    out().print(&format!("saved {} cookies to {target}", cookies.len()));
    report_login();
    Ok(())
}

/// `fsv auth logout`.
pub fn logout(a: AuthLogoutArgs) -> CmdResult {
    if !a.yes {
        let d = config::domain();
        let domain = if d.is_empty() {
            "Freshservice".to_string()
        } else {
            d
        };
        if stdin_tty() && !no_input(a.no_input) {
            term::confirm(&format!("Wipe session for {domain}?"), Some(false), true)?;
        } else {
            return Err(usage("pass --yes to confirm non-interactive logout"));
        }
    }
    session::logout();
    out().print("session cleared");
    Ok(())
}

/// `fsv auth status`.
pub fn status() -> CmdResult {
    let Some(age) = session::session_age_hours() else {
        return Err(usage("no session; run `fsv auth login`"));
    };
    let c = client()?;
    let agent = c.check_session().map_err(fail_exc)?;
    let (rem, tot) = c.rate_limit_remaining().map_err(fail_exc)?;
    let s = |k: &str| agent.get(k).map(fsv_py::py_str).unwrap_or_default();
    let empty = agent.as_object().is_none_or(|m| m.is_empty());
    let (name, email) = if empty {
        ("identity unavailable (403)".to_string(), String::new())
    } else {
        (
            format!("{} {}", s("first_name"), s("last_name"))
                .trim()
                .to_string(),
            s("email"),
        )
    };
    let dept = agent
        .get("department_names")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().map(fsv_py::py_str).collect::<Vec<_>>().join(", "))
        .unwrap_or_default();
    let team = agent
        .get("custom_fields")
        .and_then(|cf| cf.get("team"))
        .filter(|v| fsv_py::truthy(v))
        .map(fsv_py::py_str)
        .unwrap_or_default();
    let dept_display = match (dept.is_empty(), team.is_empty()) {
        (false, false) => format!("{dept}  ({team})"),
        (false, true) => dept,
        _ => team,
    };
    if email.is_empty() {
        out().print(&format!("[bold]{name}[/bold]"));
    } else {
        out().print(&format!("[bold]{name}[/bold]  <{email}>"));
    }
    out().print("session:     valid");
    if !dept_display.is_empty() {
        out().print(&format!("department:  {dept_display}"));
    }
    let mut age_str = format!("{age:.1}h");
    if age > 8.0 {
        age_str.push_str("  [yellow]⚠ session may be stale[/yellow]");
    }
    out().print(&format!("session age: {age_str}"));
    let opt = |v: Option<i64>| v.map_or("None".to_string(), |n| n.to_string());
    out().print(&format!("rate limit:  {}/{}", opt(rem), opt(tot)));
    Ok(())
}
