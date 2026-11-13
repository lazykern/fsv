//! Recipient address checks shared by notes, replies, ticket and service
//! request creates, and approvals (`ticket_notes._EMAIL` /
//! `_valid_recipient`; Python imports them from `ticket_notes` everywhere).

use std::sync::LazyLock;

use regex::Regex;

use crate::service::has_control;

/// Port of fsv.ticket_notes._EMAIL: one plain address (`fullmatch`).
pub static EMAIL: LazyLock<Regex> =
    // `\z`, not `$`: Python `fullmatch` + `\Z` rejects a trailing newline.
    LazyLock::new(|| Regex::new(r"^[^\s@<>,;]+@[^\s@<>,;]+\.[^\s@<>,;]+\z").expect("regex"));

/// Port of fsv.ticket_notes._FORMATTED_EMAIL: `Name <address>`.
static FORMATTED_EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[^<>\r\n]+ <([^\s@<>,;]+@[^\s@<>,;]+\.[^\s@<>,;]+)>\z").expect("regex")
});

/// Port of fsv.ticket_notes._valid_recipient: no C0 control or DEL, and a
/// plain or `Name <address>` email.
pub fn valid_recipient(value: &str) -> bool {
    !has_control(value, "") && (EMAIL.is_match(value) || FORMATTED_EMAIL.is_match(value))
}

/// `_valid_recipient(e) and _EMAIL.fullmatch(e)`: one plain address only
/// (ticket and service-request CCs, requested-for).
pub fn plain_email(value: &str) -> bool {
    valid_recipient(value) && EMAIL.is_match(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipient_shapes() {
        for (v, valid, plain) in [
            ("a@example.com", true, true),
            ("Name <a@example.com>", true, false),
            ("a@example", false, false),
            ("a@example.com\n", false, false),
            ("a@exa\u{7f}mple.com", false, false),
            ("Name <a@example.com>\r", false, false),
            ("a b@example.com", false, false),
            ("", false, false),
        ] {
            assert_eq!(
                (valid_recipient(v), plain_email(v)),
                (valid, plain),
                "{v:?}"
            );
        }
        assert!(!EMAIL.is_match("a@example.com\n"));
        assert!(!FORMATTED_EMAIL.is_match("Name <a@example.com>\n"));
    }
}
