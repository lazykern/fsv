//! Type-to-filter matching for dropdowns, checklists and pickers
//! (`fsv.formui.fuzzy`).
//!
//! Case-insensitive and accent-insensitive (`"anderung"` finds
//! `"Änderung"`), over the label and the choice description. Ranking, best
//! first: label starts with the query, a word of the label starts with it,
//! the label contains it, the description contains it, the label contains
//! the query's letters in order (`"stg"` -> `"Staging"`). Ties keep the
//! form's order.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::canonical_combining_class;

use crate::pytext::{casefold, split_ws};

/// Casefolded, NFKD with combining marks dropped, whitespace collapsed.
pub fn fold(text: &str) -> String {
    let stripped: String = text
        .nfkd()
        .filter(|c| canonical_combining_class(*c) == 0)
        .collect();
    split_ws(&casefold(&stripped)).collect::<Vec<_>>().join(" ")
}

fn subsequence(needle: &str, hay: &str) -> bool {
    let mut it = hay.chars();
    needle.chars().all(|ch| it.by_ref().any(|h| h == ch))
}

/// Rank of a match (lower is better) or `None` for no match.
pub fn score(query: &str, label: &str, description: &str) -> Option<u8> {
    let q = fold(query);
    if q.is_empty() {
        return Some(0);
    }
    let lab = fold(label);
    if lab.starts_with(&q) {
        return Some(0);
    }
    let spaced = lab.replace(['-', '/', '('], " ");
    if split_ws(&spaced).any(|w| w.starts_with(&q)) {
        return Some(1);
    }
    if lab.contains(&q) {
        return Some(2);
    }
    if !description.is_empty() && fold(description).contains(&q) {
        return Some(3);
    }
    let compact = q.replace(' ', "");
    if !compact.is_empty() && subsequence(&compact, &lab.replace(' ', "")) {
        return Some(4);
    }
    None
}

/// Indexes of `items` (label, description) matching `query`, best first.
pub fn fuzzy_filter<L: AsRef<str>, D: AsRef<str>>(query: &str, items: &[(L, D)]) -> Vec<usize> {
    let mut ranked: Vec<(u8, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, (label, desc))| {
            score(query, label.as_ref(), desc.as_ref()).map(|s| (s, i))
        })
        .collect();
    ranked.sort();
    ranked.into_iter().map(|(_, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_ranks_match_python() {
        let cases: [(&str, &str, &str, Option<u8>); 11] = [
            ("anderung", "Änderung", "", Some(0)),
            ("stg", "Staging", "", Some(4)),
            ("prod", "Production", "", Some(0)),
            ("duct", "Production", "", Some(2)),
            ("win", "Change (Window)", "", Some(1)),
            ("db", "Other", "Database work", None),
            ("xyz", "Other", "", None),
            ("", "x", "", Some(0)),
            ("STRASSE", "Straße", "", Some(0)),
            ("ion", "Production", "", Some(2)),
            ("pd", "Prod", "", Some(4)),
        ];
        for (q, label, desc, want) in cases {
            assert_eq!(score(q, label, desc), want, "{q:?} vs {label:?}");
        }
        assert_eq!(score("data", "Other", "Database work"), Some(3));
    }

    #[test]
    fn filter_orders_by_rank_then_index() {
        let items = [
            ("Approve", ""),
            ("Production", ""),
            ("Other", "pr desc"),
            ("Prod-2", ""),
            ("Spare", ""),
            ("Xyz", ""),
        ];
        assert_eq!(fuzzy_filter("pr", &items), vec![1, 3, 0, 2, 4]);
    }

    #[test]
    fn fold_drops_marks_and_collapses_space() {
        assert_eq!(fold("  Ärger\t ÉTÉ  "), "arger ete");
        assert_eq!(fold("ﬁle"), "file");
    }
}

#[cfg(test)]
mod py_tests;
