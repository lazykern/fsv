//! `str()` of the Python exceptions a file read raises.

use crate::py_repr_str;

/// `str(OSError)` from `open(path)` / `read()`: `[Errno N] strerror: 'path'`.
pub fn os_error(e: &std::io::Error, path: &str) -> String {
    let Some(code) = e.raw_os_error() else {
        return e.to_string();
    };
    let text = e.to_string();
    let suffix = format!(" (os error {code})");
    let strerror = text.strip_suffix(&suffix).unwrap_or(&text);
    format!("[Errno {code}] {strerror}: {}", py_repr_str(path))
}

/// `str(UnicodeDecodeError)` from `bytes.decode("utf-8")`.
pub fn utf8_error(bytes: &[u8], e: &std::str::Utf8Error) -> String {
    let start = e.valid_up_to();
    let byte = bytes[start];
    let reason = if (0x80..=0xC1).contains(&byte) || byte >= 0xF5 {
        "invalid start byte"
    } else if e.error_len().is_some() {
        "invalid continuation byte"
    } else {
        "unexpected end of data"
    };
    let end = if e.error_len().is_none() {
        bytes.len()
    } else {
        start + 1
    };
    if end - start > 1 {
        format!(
            "'utf-8' codec can't decode bytes in position {start}-{}: {reason}",
            end - 1
        )
    } else {
        format!("'utf-8' codec can't decode byte 0x{byte:02x} in position {start}: {reason}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Texts from CPython 3.12 `bytes.decode("utf-8")` and `open()`.
    #[test]
    fn match_python() {
        for (b, want) in [
            (
                &b"\xff{}"[..],
                "'utf-8' codec can't decode byte 0xff in position 0: invalid start byte",
            ),
            (
                b"{}\x80",
                "'utf-8' codec can't decode byte 0x80 in position 2: invalid start byte",
            ),
            (
                b"{\"a\":\"\xe0\x80\"}",
                "'utf-8' codec can't decode byte 0xe0 in position 6: invalid continuation byte",
            ),
            (
                b"{\"a\":\"\xed\xa0\x80\"}",
                "'utf-8' codec can't decode byte 0xed in position 6: invalid continuation byte",
            ),
            (
                b"{}\xe2\x82",
                "'utf-8' codec can't decode bytes in position 2-3: unexpected end of data",
            ),
            (
                b"{}\xe2",
                "'utf-8' codec can't decode byte 0xe2 in position 2: unexpected end of data",
            ),
            (
                b"{}\xf0\x9f\x98",
                "'utf-8' codec can't decode bytes in position 2-4: unexpected end of data",
            ),
            (
                b"\xc0\xaf",
                "'utf-8' codec can't decode byte 0xc0 in position 0: invalid start byte",
            ),
            (
                b"{\"\xc3(",
                "'utf-8' codec can't decode byte 0xc3 in position 2: invalid continuation byte",
            ),
            (
                b"ab\xf4\x90\x80\x80",
                "'utf-8' codec can't decode byte 0xf4 in position 2: invalid continuation byte",
            ),
            (
                b"ab\xf0\x80",
                "'utf-8' codec can't decode byte 0xf0 in position 2: invalid continuation byte",
            ),
        ] {
            let e = std::str::from_utf8(b).unwrap_err();
            assert_eq!(utf8_error(b, &e), want, "{b:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn os_errors_match_python() {
        let dir = std::env::temp_dir();
        let e = std::fs::read(&dir).unwrap_err();
        let p = dir.to_string_lossy();
        assert_eq!(
            os_error(&e, &p),
            format!("[Errno 21] Is a directory: {}", py_repr_str(&p))
        );
        let e = std::fs::read("/nonexistent/x").unwrap_err();
        assert_eq!(
            os_error(&e, "/nonexistent/x"),
            "[Errno 2] No such file or directory: '/nonexistent/x'"
        );
    }
}
