/// Python's `shlex.quote`.
pub fn shlex_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".into();
    }
    let safe = s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c));
    if safe {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests {
    use super::shlex_quote;

    #[test]
    fn quotes_like_python() {
        assert_eq!(shlex_quote(""), "''");
        assert_eq!(shlex_quote("--no-input"), "--no-input");
        assert_eq!(shlex_quote("Show topic help."), "'Show topic help.'");
        assert_eq!(shlex_quote("it's"), "'it'\"'\"'s'");
        assert_eq!(shlex_quote("é"), "'é'");
    }
}
