//! Click's `INT` / `IntRange` conversion: Python `int(value)` on the argument
//! text, then the range check. clap prints the message after
//! `invalid value '<text>' for '<arg>': `.

use fsv_core::pystr;
use fsv_py::py_repr_str;

type Parser = fn(&str) -> Result<i64, String>;

/// Click `int` (`IntParamType`).
pub const INT: Parser = |s| parse(s, None, None);

/// Click `IntRange(min=1)`.
pub const POSITIVE: Parser = |s| parse(s, Some(1), None);

/// Click `IntRange(1, 100)`.
pub const PER_PAGE: Parser = |s| parse(s, Some(1), Some(100));

fn parse(s: &str, min: Option<i64>, max: Option<i64>) -> Result<i64, String> {
    let ranged = min.is_some() || max.is_some();
    let Some(text) = pystr::py_int(s) else {
        let name = if ranged { "integer range" } else { "integer" };
        return Err(format!("{} is not a valid {name}.", py_repr_str(s)));
    };
    let range = || {
        let desc = match (min, max) {
            (Some(lo), Some(hi)) => format!("{lo}<=x<={hi}"),
            (Some(lo), None) => format!("x>={lo}"),
            (None, Some(hi)) => format!("x<={hi}"),
            (None, None) => String::new(),
        };
        format!("{text} is not in the range {desc}.")
    };
    match text.parse::<i64>() {
        Ok(n) if min.is_some_and(|lo| n < lo) || max.is_some_and(|hi| n > hi) => Err(range()),
        Ok(n) => Ok(n),
        Err(_) if text.starts_with('-') && min.is_some() => Err(range()),
        Err(_) if !text.starts_with('-') && max.is_some() => Err(range()),
        Err(_) => Err(format!("{text} does not fit a 64-bit integer.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Messages from Click 8 (`typer._click`) `INT` and `IntRange` run on
    /// the same text.
    #[test]
    fn matches_click() {
        assert_eq!(INT(" 12 "), Ok(12));
        assert_eq!(INT("1_000"), Ok(1000));
        assert_eq!(INT("\u{661}\u{662}"), Ok(12));
        assert_eq!(INT("-0"), Ok(0));
        assert_eq!(INT("x"), Err("'x' is not a valid integer.".into()));
        assert_eq!(INT("\u{b2}"), Err("'²' is not a valid integer.".into()));
        assert_eq!(
            POSITIVE("1__0"),
            Err("'1__0' is not a valid integer range.".into())
        );
        assert_eq!(POSITIVE("0"), Err("0 is not in the range x>=1.".into()));
        assert_eq!(
            PER_PAGE("-5"),
            Err("-5 is not in the range 1<=x<=100.".into())
        );
        assert_eq!(
            PER_PAGE("200"),
            Err("200 is not in the range 1<=x<=100.".into())
        );
        assert_eq!(PER_PAGE("\u{a0}1_00\n"), Ok(100));
        assert_eq!(
            POSITIVE("-99999999999999999999"),
            Err("-99999999999999999999 is not in the range x>=1.".into())
        );
        assert_eq!(
            POSITIVE("99999999999999999999"),
            Err("99999999999999999999 does not fit a 64-bit integer.".into())
        );
    }
}
