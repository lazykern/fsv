//! `rich.table.Table` with Rich's default layout rules.

use super::markup::{Style, Text, render};
use super::text::{Justify, Line, Overflow, measure, to_ansi, wrap};

/// Border style.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BoxStyle {
    /// Rich's default: heavy header, light body.
    #[default]
    HeavyHead,
    /// Light lines everywhere (what HEAVY_HEAD becomes without a header).
    Square,
}

struct BoxChars {
    top: [char; 4],
    head: [char; 4],
    head_row: [char; 4],
    mid: [char; 4],
    row: [char; 4],
    foot: [char; 4],
    bottom: [char; 4],
}

const HEAVY_HEAD: BoxChars = BoxChars {
    top: ['┏', '━', '┳', '┓'],
    head: ['┃', ' ', '┃', '┃'],
    head_row: ['┡', '━', '╇', '┩'],
    mid: ['│', ' ', '│', '│'],
    row: ['├', '─', '┼', '┤'],
    foot: ['│', ' ', '│', '│'],
    bottom: ['└', '─', '┴', '┘'],
};

const SQUARE: BoxChars = BoxChars {
    top: ['┌', '─', '┬', '┐'],
    head: ['│', ' ', '│', '│'],
    head_row: ['├', '─', '┼', '┤'],
    mid: ['│', ' ', '│', '│'],
    row: ['├', '─', '┼', '┤'],
    foot: ['│', ' ', '│', '│'],
    bottom: ['└', '─', '┴', '┘'],
};

/// A table column (`Table.add_column` options used by fsv).
#[derive(Clone, Debug, Default)]
pub struct Column {
    pub header: String,
    pub style: String,
    pub justify: Justify,
    pub no_wrap: bool,
    pub overflow: Overflow,
    /// Fixed content width.
    pub width: Option<usize>,
    pub min_width: Option<usize>,
    /// Flex ratio (only used when the table expands, like Rich).
    pub ratio: Option<usize>,
}

impl Column {
    pub fn new(header: impl Into<String>) -> Self {
        Column { header: header.into(), ..Column::default() }
    }
    pub fn style(mut self, style: impl Into<String>) -> Self {
        self.style = style.into();
        self
    }
    pub fn justify(mut self, j: Justify) -> Self {
        self.justify = j;
        self
    }
    pub fn no_wrap(mut self) -> Self {
        self.no_wrap = true;
        self
    }
    pub fn overflow(mut self, o: Overflow) -> Self {
        self.overflow = o;
        self
    }
    pub fn width(mut self, w: usize) -> Self {
        self.width = Some(w);
        self
    }
    pub fn min_width(mut self, w: usize) -> Self {
        self.min_width = Some(w);
        self
    }
    pub fn ratio(mut self, r: usize) -> Self {
        self.ratio = Some(r);
        self
    }
}

/// A Rich-style table. Cells are Rich markup.
#[derive(Clone, Debug)]
pub struct Table {
    pub title: Option<String>,
    pub show_header: bool,
    pub box_style: Option<BoxStyle>,
    pub pad_edge: bool,
    pub show_lines: bool,
    pub expand: bool,
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<String>>,
}

impl Default for Table {
    fn default() -> Self {
        Table {
            title: None,
            show_header: true,
            box_style: Some(BoxStyle::HeavyHead),
            pad_edge: true,
            show_lines: false,
            expand: false,
            columns: Vec::new(),
            rows: Vec::new(),
        }
    }
}

fn ratio_reduce(total: usize, ratios: &[usize], maximums: &[usize], values: &[usize]) -> Vec<usize> {
    let ratios: Vec<usize> = ratios.iter().zip(maximums).map(|(r, m)| if *m > 0 { *r } else { 0 }).collect();
    let mut total_ratio: usize = ratios.iter().sum();
    if total_ratio == 0 {
        return values.to_vec();
    }
    let mut remaining = total as f64;
    let mut out = Vec::with_capacity(values.len());
    for ((ratio, maximum), value) in ratios.iter().zip(maximums).zip(values) {
        if *ratio > 0 && total_ratio > 0 {
            let d = crate::pystr::round_half_even(*ratio as f64 * remaining / total_ratio as f64) as usize;
            let distributed = (*maximum).min(d);
            out.push(value.saturating_sub(distributed));
            remaining -= distributed as f64;
            total_ratio -= ratio;
        } else {
            out.push(*value);
        }
    }
    out
}

fn ratio_distribute(total: usize, ratios: &[usize], minimums: Option<&[usize]>) -> Vec<usize> {
    let ratios: Vec<usize> = match minimums {
        Some(m) => ratios.iter().zip(m).map(|(r, mn)| if *mn > 0 { *r } else { 0 }).collect(),
        None => ratios.to_vec(),
    };
    let mut total_ratio: usize = ratios.iter().sum();
    let mut remaining = total as i64;
    let mut out = Vec::new();
    for (i, ratio) in ratios.iter().enumerate() {
        let minimum = minimums.map_or(0, |m| m[i]) as i64;
        let distributed = if total_ratio > 0 {
            minimum.max((*ratio as f64 * remaining as f64 / total_ratio as f64).ceil() as i64)
        } else {
            remaining
        };
        out.push(distributed.max(0) as usize);
        total_ratio -= ratio;
        remaining -= distributed;
    }
    out
}

impl Table {
    pub fn new() -> Self {
        Table::default()
    }
    /// `Table("A", "B")`: columns from headers.
    pub fn with_headers(headers: &[&str]) -> Self {
        let mut t = Table::new();
        for h in headers {
            t.add_column(Column::new(*h));
        }
        t
    }
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }
    pub fn show_header(mut self, yes: bool) -> Self {
        self.show_header = yes;
        self
    }
    /// `box=None`.
    pub fn no_box(mut self) -> Self {
        self.box_style = None;
        self
    }
    pub fn pad_edge(mut self, yes: bool) -> Self {
        self.pad_edge = yes;
        self
    }
    pub fn show_lines(mut self, yes: bool) -> Self {
        self.show_lines = yes;
        self
    }
    pub fn expand(mut self, yes: bool) -> Self {
        self.expand = yes;
        self
    }
    pub fn add_column(&mut self, column: Column) -> &mut Self {
        self.columns.push(column);
        self
    }
    /// Add a row (missing cells are empty).
    pub fn add_row<S: Into<String>>(&mut self, cells: impl IntoIterator<Item = S>) -> &mut Self {
        self.rows.push(cells.into_iter().map(Into::into).collect());
        self
    }

    fn padding(&self, index: usize) -> (usize, usize) {
        let first = index == 0;
        let last = index + 1 == self.columns.len();
        let left = if !self.pad_edge && first { 0 } else { 1 };
        let right = if !self.pad_edge && last { 0 } else { 1 };
        (left, right)
    }

    fn extra_width(&self) -> usize {
        match self.box_style {
            Some(_) => 2 + self.columns.len().saturating_sub(1),
            None => 0,
        }
    }

    fn cells(&self, index: usize) -> Vec<Text> {
        let mut out = Vec::new();
        if self.show_header {
            out.push(render(&self.columns[index].header));
        }
        for row in &self.rows {
            out.push(render(row.get(index).map_or("", String::as_str)));
        }
        out
    }

    fn measure_column(&self, index: usize, max_width: usize) -> (usize, usize) {
        let col = &self.columns[index];
        let (l, r) = self.padding(index);
        let pad = l + r;
        if let Some(w) = col.width {
            let w = (w + pad).min(max_width);
            return (w, w);
        }
        let cells = self.cells(index);
        let (mut min, mut max) = if cells.is_empty() {
            (1, max_width)
        } else {
            cells.iter().map(|t| measure(&t.plain)).fold((0, 0), |(a, b), (mn, mx)| (a.max(mn + pad), b.max(mx + pad)))
        };
        max = max.min(max_width);
        min = min.min(max);
        if let Some(mw) = col.min_width {
            let floor = mw + pad;
            min = min.max(floor);
            max = max.max(floor);
        }
        (min, max)
    }

    fn collapse_widths(widths: &[usize], wrapable: &[bool], max_width: usize) -> Vec<usize> {
        let mut widths = widths.to_vec();
        let mut total: usize = widths.iter().sum();
        if wrapable.iter().any(|w| *w) {
            while total > 0 && total > max_width {
                let excess = total - max_width;
                let max_column =
                    widths.iter().zip(wrapable).filter(|(_, w)| **w).map(|(v, _)| *v).max().unwrap_or(0);
                let second = widths
                    .iter()
                    .zip(wrapable)
                    .map(|(v, w)| if *w && *v != max_column { *v } else { 0 })
                    .max()
                    .unwrap_or(0);
                let diff = max_column - second;
                let ratios: Vec<usize> =
                    widths.iter().zip(wrapable).map(|(v, w)| usize::from(*v == max_column && *w)).collect();
                if ratios.iter().all(|r| *r == 0) || diff == 0 {
                    break;
                }
                let max_reduce = vec![excess.min(diff); widths.len()];
                widths = ratio_reduce(excess, &ratios, &max_reduce, &widths);
                total = widths.iter().sum();
            }
        }
        widths
    }

    fn column_widths(&self, max_width: usize) -> Vec<usize> {
        let n = self.columns.len();
        let ranges: Vec<(usize, usize)> = (0..n).map(|i| self.measure_column(i, max_width)).collect();
        let mut widths: Vec<usize> = ranges.iter().map(|(_, mx)| if *mx == 0 { 1 } else { *mx }).collect();
        if self.expand {
            let ratios: Vec<usize> = self.columns.iter().filter_map(|c| c.ratio).collect();
            if ratios.iter().any(|r| *r > 0) {
                let fixed: Vec<usize> =
                    self.columns.iter().zip(&ranges).map(|(c, r)| if c.ratio.is_some() { 0 } else { r.1 }).collect();
                let flex_min: Vec<usize> = (0..n)
                    .filter(|i| self.columns[*i].ratio.is_some())
                    .map(|i| {
                        let (l, r) = self.padding(i);
                        self.columns[i].width.unwrap_or(1) + l + r
                    })
                    .collect();
                let flexible = max_width.saturating_sub(fixed.iter().sum());
                let flex = ratio_distribute(flexible, &ratios, Some(&flex_min));
                let mut it = flex.into_iter();
                for i in 0..n {
                    if self.columns[i].ratio.is_some() {
                        widths[i] = fixed[i] + it.next().unwrap_or(0);
                    }
                }
            }
        }
        let mut table_width: usize = widths.iter().sum();
        if table_width > max_width {
            let wrapable: Vec<bool> = self.columns.iter().map(|c| c.width.is_none() && !c.no_wrap).collect();
            widths = Self::collapse_widths(&widths, &wrapable, max_width);
            table_width = widths.iter().sum();
            if table_width > max_width {
                let excess = table_width - max_width;
                widths = ratio_reduce(excess, &vec![1; n], &widths, &widths);
            }
            widths = widths
                .iter()
                .enumerate()
                .map(|(i, w)| {
                    let (_, mx) = self.measure_column(i, *w);
                    mx
                })
                .collect();
            table_width = widths.iter().sum();
        }
        if table_width < max_width && self.expand {
            let pads = ratio_distribute(max_width - table_width, &widths, None);
            widths = widths.iter().zip(pads).map(|(w, p)| w + p).collect();
        }
        widths
    }

    /// Render to lines (no trailing newline) for a terminal `width` wide.
    pub fn render_lines(&self, width: usize, color: bool) -> Vec<String> {
        if self.columns.is_empty() {
            return Vec::new();
        }
        let max_width = width.saturating_sub(self.extra_width());
        let widths = self.column_widths(max_width);
        let table_width: usize = widths.iter().sum::<usize>() + self.extra_width();
        let mut out = Vec::new();

        if let Some(title) = self.title.as_deref().filter(|t| !t.is_empty()) {
            let italic = Style { italic: Some(true), ..Style::default() };
            for line in wrap(&render(title), italic, table_width, Justify::Center, Overflow::Fold, false) {
                out.push(to_ansi(&line, color));
            }
        }

        let bx = match self.box_style {
            Some(BoxStyle::HeavyHead) if self.show_header => Some(&HEAVY_HEAD),
            Some(_) => Some(&SQUARE),
            None => None,
        };
        let edge = |chars: &[char; 4]| -> String {
            let mut s = String::new();
            s.push(chars[0]);
            for (i, w) in widths.iter().enumerate() {
                if i > 0 {
                    s.push(chars[2]);
                }
                s.extend(std::iter::repeat_n(chars[1], *w));
            }
            s.push(chars[3]);
            s
        };
        if let Some(b) = bx {
            out.push(edge(&b.top));
        }

        let header_style = Style { bold: Some(true), ..Style::default() };
        let mut all_rows: Vec<(bool, Vec<Text>)> = Vec::new();
        if self.show_header {
            all_rows.push((true, self.columns.iter().map(|c| render(&c.header)).collect()));
        }
        for row in &self.rows {
            all_rows.push((false, (0..self.columns.len()).map(|i| render(row.get(i).map_or("", String::as_str))).collect()));
        }
        let count = all_rows.len();
        for (index, (is_header, cells)) in all_rows.iter().enumerate() {
            let first = index == 0;
            let last = index + 1 == count;
            let mut rendered: Vec<Vec<Line>> = Vec::new();
            for (i, cell) in cells.iter().enumerate() {
                let col = &self.columns[i];
                let base = if *is_header { header_style } else { Style::parse(&col.style) };
                let (l, r) = self.padding(i);
                let inner = widths[i].saturating_sub(l + r);
                let lines = wrap(cell, base, inner, col.justify, col.overflow, col.no_wrap);
                let lines: Vec<Line> = lines
                    .into_iter()
                    .map(|ln| {
                        let mut v: Line = std::iter::repeat_n((' ', base), l).collect();
                        v.extend(ln);
                        v.extend(std::iter::repeat_n((' ', base), r));
                        v
                    })
                    .collect();
                rendered.push(lines);
            }
            let height = rendered.iter().map(Vec::len).max().unwrap_or(1).max(1);
            for (i, lines) in rendered.iter_mut().enumerate() {
                let base = if *is_header { header_style } else { Style::parse(&self.columns[i].style) };
                let blank: Line = std::iter::repeat_n((' ', base), widths[i]).collect();
                while lines.len() < height {
                    if *is_header {
                        lines.insert(0, blank.clone());
                    } else {
                        lines.push(blank.clone());
                    }
                }
            }
            let sides = bx.map(|b| if first { b.head } else if last { b.foot } else { b.mid });
            for line_no in 0..height {
                let mut s = String::new();
                if let Some(sd) = sides {
                    s.push(sd[0]);
                }
                for (i, lines) in rendered.iter().enumerate() {
                    if i > 0 {
                        if let Some(sd) = sides {
                            s.push(sd[2]);
                        }
                    }
                    s.push_str(&to_ansi(&lines[line_no], color));
                }
                if let Some(sd) = sides {
                    s.push(sd[3]);
                }
                out.push(s);
            }
            if let Some(b) = bx {
                if first && self.show_header {
                    out.push(edge(&b.head_row));
                } else if self.show_lines && !last {
                    out.push(edge(&b.row));
                }
            }
        }
        if let Some(b) = bx {
            out.push(edge(&b.bottom));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heavy_head_layout() {
        let mut t = Table::new();
        t.add_column(Column::new("ID").style("cyan"));
        t.add_column(Column::new("Subject"));
        t.add_row(["CHN-1", "Patch servers"]);
        let lines = t.render_lines(80, false);
        assert_eq!(
            lines,
            vec![
                "┏━━━━━━━┳━━━━━━━━━━━━━━━┓",
                "┃ ID    ┃ Subject       ┃",
                "┡━━━━━━━╇━━━━━━━━━━━━━━━┩",
                "│ CHN-1 │ Patch servers │",
                "└───────┴───────────────┘",
            ]
        );
    }

    #[test]
    fn boxless_without_edge_padding() {
        let mut t = Table::new().show_header(false).no_box().pad_edge(false);
        t.add_column(Column::new("").style("cyan").no_wrap());
        t.add_column(Column::new("").overflow(Overflow::Fold));
        t.add_row(["Subject", "x"]);
        t.add_row(["Priority", "High"]);
        assert_eq!(t.render_lines(80, false), vec!["Subject   x   ", "Priority  High"]);
    }

    #[test]
    fn wraps_to_width() {
        let mut t = Table::new();
        t.add_column(Column::new("ID"));
        t.add_column(Column::new("Subject"));
        t.add_row(["1", "aaaa bbbb cccc dddd"]);
        let lines = t.render_lines(20, false);
        assert!(lines.iter().all(|l| super::super::text::cell_len(l) <= 20), "{lines:?}");
        assert!(lines.len() > 5);
    }
}
