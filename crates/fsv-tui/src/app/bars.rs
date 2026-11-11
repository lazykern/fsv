//! Header, entity bar and status bar.

use super::*;

impl App {
    pub fn view_header(&self) -> String {
        let res = resource_for(self.entity);
        let view = if self.active_filters.is_empty() {
            self.view.get(res.name)
        } else {
            None
        };
        let name = match view {
            Some(v) => v.name.clone(),
            None if !self.active_filters.is_empty() => "Filtered".into(),
            None => format!("All {}", res.name),
        };
        let rows = self.items.len();
        let rng = if rows > 0 {
            format!("1-{rows}")
        } else {
            "0".into()
        };
        let of = match self.list_total {
            Some(t) => format!(" of {t}"),
            None => format!(" of {rows}{}", if self.has_more { "+" } else { "" }),
        };
        let sort = site::sort_label(&self.list_sort, self.schemas.get(res.name));
        format!(
            "[bold]{}[/][dim] · {rng}{of} · Sort by: {}[/]",
            escape(&name),
            escape(&sort)
        )
    }

    pub(super) fn count_text(&self) -> String {
        let rows = self.items.len() as i64;
        if self.total > rows {
            format!("{rows}/{}", self.total)
        } else {
            rows.to_string()
        }
    }

    /// `FsvApp._render_header`: entity tabs or search tabs with counts.
    pub fn header_markup(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.search_mode {
            parts.push(format!(
                "[bold cyan]fsv[/] :: [yellow bold]search[/] [yellow]{}[/]  ",
                escape(&self.search_term)
            ));
            parts.push(if self.search_entity.is_none() {
                "[bold reverse] ALL [/] ".into()
            } else {
                "ALL  ".into()
            });
            for tab in ENTITY_TABS {
                let label = tab.to_uppercase();
                let cnt = self
                    .search_totals
                    .iter()
                    .find(|(k, _)| k == tab)
                    .map_or(String::new(), |(_, c)| format!("({c})"));
                parts.push(if Some(tab) == self.search_entity {
                    format!("[bold reverse] {label}{cnt} [/] ")
                } else {
                    format!("{label}{cnt}  ")
                });
            }
            let sort = match self.search_sort {
                "created" => "date created",
                "modified" => "last modified",
                s => s,
            };
            parts.push(format!(
                "  [dim]{} shown  sort:{sort}  Esc exit[/]",
                self.count_text()
            ));
        } else {
            parts.push("[bold cyan]fsv[/] :: freshservice  ".into());
            for tab in ENTITY_TABS {
                let label = tab.to_uppercase();
                parts.push(if tab == self.entity {
                    format!("[bold reverse] {label} [/] ")
                } else {
                    format!("{label}  ")
                });
            }
            parts.push(format!("  {}", self.view_header()));
            parts.push("  [dim]v views  ? help[/]".into());
        }
        parts.concat()
    }

    /// `FsvApp._render_filter_bar`.
    pub fn filter_bar_markup(&self) -> String {
        let sel = self.selected.as_ref().map_or("-".into(), Item::display_id);
        let loading = if self.loading_more {
            "  [yellow]loading more…[/]"
        } else {
            ""
        };
        let part = if self.search_mode {
            format!(
                "[yellow]search: {}[/]  [dim]h/l narrow  s sort  Esc exit  f new search[/]",
                escape(&self.search_term)
            )
        } else if let Some(e) = &self.filter_error {
            format!("[red]filter error: {}[/]", escape(e))
        } else if !self.active_filters.is_empty() {
            format!(
                "[yellow]filter: {}[/]",
                escape(&self.active_filters.join(" "))
            )
        } else {
            "[dim]/ to filter[/]".into()
        };
        format!(
            "{part}  [dim]focus={}  sel={}  {} rows[/]{loading}",
            self.active_pane().as_str(),
            escape(&sel),
            self.count_text()
        )
    }

    /// `FsvApp._render_status`: `(label, dim suffix)`.
    pub fn status_parts(&self) -> (String, String) {
        let mut label = if self.search_mode {
            let mut l = format!("SEARCH: {}", self.search_term);
            if let Some(e) = self.search_entity {
                l += &format!(" > {}", e.to_uppercase());
            }
            l
        } else {
            self.entity.to_uppercase()
        };
        if let Some(sel) = &self.selected {
            label += &format!(
                " > {} > {}",
                sel.display_id(),
                self.current_tab_name(Some(sel.res))
            );
        }
        let suffix = if self.loading_more {
            "loading more…".to_string()
        } else {
            format!("{} rows", self.count_text())
        };
        (
            format!("{label}    "),
            format!("pane={} · {suffix}", self.active_pane().as_str()),
        )
    }

    /// `FsvApp._render_key_hints`.
    pub fn key_hints_markup(&self) -> String {
        let write = if self.read_only {
            ""
        } else {
            "[bold]s[/] status  [bold]n[/] note  [bold]N[/]/[bold]e[/] create/edit  [bold]a[/] actions  [bold]![/] failure  "
        };
        let rkey = if self.read_only {
            "[bold]r[/] reload  "
        } else {
            "[bold]r[/] reload/reply  "
        };
        format!(
            "[bold]Tab[/] pane  [bold]j/k[/] move or scroll  [bold]J/K[/] ×5  [bold]G/gg[/] ↕  [bold]h/l[/] entity/tab  [bold]gw/gt/gp/gc[/] entity  [bold]1-9[/] detail tab  [bold]v[/] views  [bold]\\[][/bold] resize  [bold]f[/] search  [bold]/[/] filter  [bold]o[/] browser  {write}{rkey}[bold]y[/] yank  [bold]q[/]/[bold]Ctrl+C[/] quit  [bold]?[/] help"
        )
    }
}
