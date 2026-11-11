//! `fsv journal ls|tail|path`.

use serde_json::Value;

use fsv_core::journal;
use fsv_core::term::{Column, Table, out};

use crate::generated::{JournalLsArgs, JournalTailArgs};
use crate::util::{CmdResult, head};

fn cell(r: &Value, key: &str, dash: bool) -> String {
    match r.get(key) {
        Some(v) if fsv_py::truthy(v) || !dash => fsv_py::py_str(v),
        _ if dash => "-".into(),
        _ => String::new(),
    }
}

/// `_journal_rows`.
fn rows_out(rows: &[Value], json_out: bool) {
    if json_out {
        for r in rows {
            println!("{}", fsv_py::Dumps::default().ensure_ascii(false).dump(r));
        }
        return;
    }
    if rows.is_empty() {
        out().print("[dim]journal is empty[/dim]");
        return;
    }
    let mut t = Table::new();
    for c in [
        "time", "command", "kind", "target", "status", "created", "plan",
    ] {
        t.add_column(Column::new(c));
    }
    for r in rows {
        let ok = r.get("ok").is_some_and(fsv_py::truthy);
        let status = r
            .get("status")
            .filter(|v| fsv_py::truthy(v))
            .map(fsv_py::py_str)
            .unwrap_or_default();
        t.add_row([
            cell(r, "ts", false),
            cell(r, "command", true),
            cell(r, "kind", false),
            cell(r, "target", true),
            format!("{}{status}", if ok { "ok " } else { "FAIL " }),
            cell(r, "created_id", true),
            head(&cell(r, "plan_hash", true), 12),
        ]);
    }
    out().print_table(&t);
}

/// Python `rows[-n:]` for any integer `n` (0 handled by the caller).
fn tail_slice(rows: Vec<Value>, n: i64) -> Vec<Value> {
    let len = rows.len() as i64;
    let start = if n > 0 {
        (len - n).max(0)
    } else {
        (-n).min(len)
    };
    rows.into_iter().skip(start as usize).collect()
}

/// `fsv journal ls`.
pub fn ls(a: JournalLsArgs) -> CmdResult {
    let rows: Vec<Value> = journal::read(None)
        .into_iter()
        .filter(|r| !a.failed || !r.get("ok").is_some_and(fsv_py::truthy))
        .collect();
    let rows = if a.limit != 0 {
        tail_slice(rows, a.limit)
    } else {
        rows
    };
    rows_out(&rows, a.json_out);
    Ok(())
}

/// `fsv journal tail`.
pub fn tail(a: JournalTailArgs) -> CmdResult {
    let rows = journal::read(None);
    let rows = if a.limit > 0 {
        tail_slice(rows, a.limit)
    } else {
        rows
    };
    rows_out(&rows, a.json_out);
    Ok(())
}

/// `fsv journal path`.
pub fn path() -> CmdResult {
    println!("{}", journal::journal_path().display());
    Ok(())
}
