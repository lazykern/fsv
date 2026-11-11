/// CPython `float.__repr__` (also what `json.dumps` and `str()` emit).
///
/// Shortest round-trip digits; fixed notation when `1e-4 <= |x| < 1e16`,
/// otherwise `d.ddde+XX` with at least two exponent digits. `json.dumps`
/// writes non-finite values as `NaN`, `Infinity`, `-Infinity`; `repr` uses
/// `nan`/`inf`, see [`float_repr_py`].
pub fn float_repr(value: f64) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    finite(value)
}

pub(crate) fn float_repr_py(value: f64) -> String {
    if value.is_nan() {
        return "nan".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "inf".into() } else { "-inf".into() };
    }
    finite(value)
}

fn finite(value: f64) -> String {
    if value == 0.0 {
        return if value.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    // Rust's `{:e}` prints the shortest round-trip digits, e.g. "1.5e-5".
    let sci = format!("{value:e}");
    let (mantissa, exp) = sci.split_once('e').expect("exponent");
    let exp: i32 = exp.parse().expect("exponent digits");
    let (neg, mantissa) = match mantissa.strip_prefix('-') {
        Some(m) => (true, m),
        None => (false, mantissa),
    };
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    if (-4..16).contains(&exp) {
        let n = digits.len() as i32;
        if exp < 0 {
            out.push_str("0.");
            for _ in 0..(-exp - 1) {
                out.push('0');
            }
            out.push_str(&digits);
        } else if exp + 1 >= n {
            out.push_str(&digits);
            for _ in 0..(exp + 1 - n) {
                out.push('0');
            }
            out.push_str(".0");
        } else {
            let split = (exp + 1) as usize;
            out.push_str(&digits[..split]);
            out.push('.');
            out.push_str(&digits[split..]);
        }
    } else {
        out.push_str(&digits[..1]);
        if digits.len() > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        out.push(if exp < 0 { '-' } else { '+' });
        let abs = exp.unsigned_abs();
        if abs < 10 {
            out.push('0');
        }
        out.push_str(&abs.to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_cpython() {
        let cases = [
            (1.0, "1.0"),
            (0.1, "0.1"),
            (-2.5, "-2.5"),
            (100.0, "100.0"),
            (1e16, "1e+16"),
            (1.5e16, "1.5e+16"),
            (9999999999999998.0, "9999999999999998.0"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (1.2345e-7, "1.2345e-07"),
            (123456.789, "123456.789"),
            (1e100, "1e+100"),
            (0.0, "0.0"),
            (-0.0, "-0.0"),
            (1747000000.123456, "1747000000.123456"),
        ];
        for (v, want) in cases {
            assert_eq!(float_repr(v), want, "{v}");
        }
        assert_eq!(float_repr(f64::NAN), "NaN");
        assert_eq!(float_repr_py(f64::INFINITY), "inf");
    }
}
