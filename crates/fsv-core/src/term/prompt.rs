//! `typer.prompt` / `typer.confirm` (click formatting and abort semantics).
//!
//! Prompts go to stdout like click. EOF, `--no-input` and a refused
//! `confirm(abort=True)` fail with [`Error::Aborted`] (click prints
//! `Aborted!` and exits 1).

use std::io::{BufRead, Write};

use crate::error::{Error, Result};

fn read_line(prompt: &str) -> Result<String> {
    if crate::runtime::no_input() {
        return Err(Error::Aborted);
    }
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(prompt.as_bytes());
    let _ = out.flush();
    drop(out);
    let mut line = String::new();
    match std::io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => Err(Error::Aborted),
        Ok(_) => {
            if line.ends_with('\n') {
                line.pop();
                if line.ends_with('\r') {
                    line.pop();
                }
            }
            Ok(line)
        }
    }
}

fn echo(msg: &str) {
    println!("{msg}");
}

/// Prompt text as click builds it: `Text [default]: `.
pub fn prompt_text(text: &str, default: Option<&str>, show_default: bool) -> String {
    match default {
        Some(d) if show_default => format!("{text} [{d}]: "),
        _ => format!("{text}: "),
    }
}

/// `typer.prompt(text, default=..., show_default=...)`; empty input takes the
/// default, or re-prompts without one.
pub fn prompt(text: &str, default: Option<&str>, show_default: bool) -> Result<String> {
    let p = prompt_text(text, default, show_default);
    loop {
        let value = read_line(&p)?;
        if !value.is_empty() {
            return Ok(value);
        }
        if let Some(d) = default {
            return Ok(d.to_string());
        }
    }
}

/// `typer.prompt(text, default=<int>)`: re-prompts on non-integers.
pub fn prompt_int(text: &str, default: Option<i64>, show_default: bool) -> Result<i64> {
    let d = default.map(|d| d.to_string());
    let p = prompt_text(text, d.as_deref(), show_default);
    loop {
        let mut value = read_line(&p)?;
        if value.is_empty() {
            match &d {
                Some(d) => value = d.clone(),
                None => continue,
            }
        }
        let cleaned = crate::pystr::strip(&value).replace('_', "");
        match cleaned.parse::<i64>() {
            Ok(n) => return Ok(n),
            Err(_) => echo(&format!(
                "Error: {} is not a valid integer.",
                fsv_py::py_repr_str(&value)
            )),
        }
    }
}

/// `typer.confirm(text, default=..., abort=...)`: `Text [y/N]: `.
pub fn confirm(text: &str, default: Option<bool>, abort: bool) -> Result<bool> {
    let suffix = match default {
        None => "y/n",
        Some(true) => "Y/n",
        Some(false) => "y/N",
    };
    let p = format!("{text} [{suffix}]: ");
    let rv = loop {
        let value = crate::pystr::strip(&read_line(&p)?).to_lowercase();
        match value.as_str() {
            "y" | "yes" => break true,
            "n" | "no" => break false,
            "" if default.is_some() => break default.unwrap_or(false),
            _ => echo("Error: invalid input"),
        }
    };
    if abort && !rv {
        return Err(Error::Aborted);
    }
    Ok(rv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_text_format() {
        assert_eq!(prompt_text("Domain", Some("acme"), true), "Domain [acme]: ");
        assert_eq!(prompt_text("Pick", Some("n"), false), "Pick: ");
        assert_eq!(prompt_text("Pick", None, true), "Pick: ");
    }
}
