//! `decimal.Decimal` comparison of finite numeric strings.

use std::cmp::Ordering;

/// A finite `Decimal` as sign, significant digits (no leading zeros) and
/// the exponent of the last digit. Zero has no digits.
#[derive(Debug, PartialEq, Eq)]
struct Finite {
    negative: bool,
    digits: Vec<u8>,
    exp: i64,
}

/// `Decimal(text)` for finite values: surrounding whitespace, a sign, digits
/// with at most one `.`, and an optional exponent. ASCII digits only;
/// infinities, NaN and underscores are not accepted.
fn parse(text: &str) -> Option<Finite> {
    let s = text.trim();
    let (negative, s) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let (mantissa, exp) = match s.find(['e', 'E']) {
        Some(at) => {
            let e = &s[at + 1..];
            let body = e.strip_prefix(['+', '-']).unwrap_or(e);
            if body.is_empty() || !body.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            (&s[..at], e.parse::<i64>().ok()?)
        }
        None => (s, 0),
    };
    let (int, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if int.is_empty() && frac.is_empty()
        || !int.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let digits: Vec<u8> = int
        .bytes()
        .chain(frac.bytes())
        .map(|b| b - b'0')
        .skip_while(|d| *d == 0)
        .collect();
    let exp = exp.checked_sub(i64::try_from(frac.len()).ok()?)?;
    Some(Finite {
        negative: negative && !digits.is_empty(),
        digits,
        exp,
    })
}

fn magnitude(a: &Finite, b: &Finite) -> Ordering {
    match (a.digits.is_empty(), b.digits.is_empty()) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        _ => {}
    }
    let adjusted = |x: &Finite| x.exp + x.digits.len() as i64;
    adjusted(a).cmp(&adjusted(b)).then_with(|| {
        let n = a.digits.len().max(b.digits.len());
        let at = |x: &Finite, i: usize| x.digits.get(i).copied().unwrap_or(0);
        (0..n)
            .map(|i| at(a, i).cmp(&at(b, i)))
            .find(|o| o.is_ne())
            .unwrap_or(Ordering::Equal)
    })
}

/// `Decimal(a)` compared with `Decimal(b)`; `None` when either is not a
/// finite decimal string (`InvalidOperation`).
pub fn decimal_cmp(a: &str, b: &str) -> Option<Ordering> {
    let (a, b) = (parse(a)?, parse(b)?);
    Some(match (a.negative, b.negative) {
        (false, true) => Ordering::Greater,
        (true, false) => Ordering::Less,
        (false, false) => magnitude(&a, &b),
        (true, true) => magnitude(&b, &a),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_like_python_decimal() {
        for (a, b, want) in [
            ("4", "3", Ordering::Greater),
            ("2", "3", Ordering::Less),
            ("3.0", "3", Ordering::Equal),
            (" 3 ", "3", Ordering::Equal),
            ("-0", "0", Ordering::Equal),
            ("1e+20", "100000000000000000000", Ordering::Equal),
            ("0.5", "-12", Ordering::Greater),
            ("-12", "-3", Ordering::Less),
            ("007", "7", Ordering::Equal),
            (".5", "0.49", Ordering::Greater),
            ("4.5", "4", Ordering::Greater),
        ] {
            assert_eq!(decimal_cmp(a, b), Some(want), "{a} {b}");
        }
        for bad in ["", "abc", "1.2.3", "e5", "1e", "--1", "[1]", "."] {
            assert_eq!(decimal_cmp(bad, "1"), None, "{bad}");
        }
    }
}
