//! Ticket templates (read-only)
//! (`templates_cli.py`).

use serde_json::{Value, json};

use fsv_api::templates;

use crate::cmd::inventory::guard;
use crate::generated::*;
use crate::util::{CmdResult, client};

/// `json.dumps(value, indent=2, ensure_ascii=False)`.
fn dumps(v: &Value) -> String {
    fsv_py::Dumps::default()
        .indent(2)
        .ensure_ascii(false)
        .dump(v)
}

fn text(v: Option<&Value>) -> String {
    fsv_py::py_str(v.unwrap_or(&Value::Null))
}

/// `fsv templates list`.
pub fn templates_list(a: TemplatesListArgs) -> CmdResult {
    let c = client()?;
    guard((|| {
        let rows = templates::list_templates(c.as_ref())?;
        if a.json_out {
            println!("{}", dumps(&Value::Array(rows)));
            return Ok(());
        }
        for r in &rows {
            let kind = r.get("type").filter(|v| fsv_py::truthy(v));
            println!(
                "{}\t{}\t{}",
                text(r.get("id")),
                text(r.get("name")),
                kind.map(fsv_py::py_str).unwrap_or_default()
            );
        }
        if rows.is_empty() {
            println!("No ticket templates.");
        }
        Ok(())
    })())
}

/// `fsv templates show NAME_OR_ID`.
pub fn templates_show(a: TemplatesShowArgs) -> CmdResult {
    let c = client()?;
    guard((|| {
        let row = templates::show(c.as_ref(), &a.name)?;
        let p = templates::to_draft(&row);
        let doc = json!({"id": row["id"], "name": row.get("name").cloned().unwrap_or(Value::Null),
                         "prefills": p.applied, "not_applied": p.ignored,
                         "template_data": row["template_data"]});
        println!("{}", dumps(&doc));
        Ok(())
    })())
}
