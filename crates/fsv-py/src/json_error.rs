//! `str(json.JSONDecodeError)` for text CPython 3.12's `json.loads` refuses:
//! the C scanner's first error and its position.

struct Fail(&'static str, usize);

/// Scanner outcome: `Stop(i)` is the scanner's `StopIteration` ("Expecting
/// value" at `i`); `Err` a `raise_errmsg`.
enum Scan {
    Stop(usize),
    Err(Fail),
}

impl From<Fail> for Scan {
    fn from(f: Fail) -> Scan {
        Scan::Err(f)
    }
}

struct Doc {
    s: Vec<char>,
}

fn is_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

impl Doc {
    fn at(&self, i: usize) -> Option<char> {
        self.s.get(i).copied()
    }

    fn skip_ws(&self, mut i: usize) -> usize {
        while self.at(i).is_some_and(is_ws) {
            i += 1;
        }
        i
    }

    fn starts(&self, i: usize, word: &str) -> bool {
        word.chars()
            .enumerate()
            .all(|(k, c)| self.at(i + k) == Some(c))
    }

    fn scan(&self, i: usize) -> Result<usize, Scan> {
        let Some(c) = self.at(i) else {
            return Err(Scan::Stop(i));
        };
        match c {
            '"' => self.string(i + 1).map_err(Scan::from),
            '{' => self.object(i + 1),
            '[' => self.array(i + 1),
            'n' if self.starts(i, "null") => Ok(i + 4),
            't' if self.starts(i, "true") => Ok(i + 4),
            'f' if self.starts(i, "false") => Ok(i + 5),
            'N' if self.starts(i, "NaN") => Ok(i + 3),
            'I' if self.starts(i, "Infinity") => Ok(i + 8),
            '-' if self.starts(i, "-Infinity") => Ok(i + 9),
            _ => self.number(i).ok_or(Scan::Stop(i)),
        }
    }

    fn number(&self, start: usize) -> Option<usize> {
        let digit = |i: usize| self.at(i).is_some_and(|c| c.is_ascii_digit());
        let mut i = start;
        if self.at(i) == Some('-') {
            i += 1;
        }
        match self.at(i) {
            Some('1'..='9') => {
                while digit(i) {
                    i += 1;
                }
            }
            Some('0') => i += 1,
            _ => return None,
        }
        if self.at(i) == Some('.') && digit(i + 1) {
            i += 1;
            while digit(i) {
                i += 1;
            }
        }
        if matches!(self.at(i), Some('e' | 'E')) {
            let mut j = i + 1;
            if matches!(self.at(j), Some('-' | '+')) {
                j += 1;
            }
            if digit(j) {
                while digit(j) {
                    j += 1;
                }
                i = j;
            }
        }
        Some(i)
    }

    /// `scanstring`, `end` just past the opening quote.
    fn string(&self, mut end: usize) -> Result<usize, Fail> {
        let begin = end - 1;
        let len = self.s.len();
        loop {
            let mut next = end;
            let mut c = '\0';
            while next < len {
                c = self.s[next];
                if c == '"' || c == '\\' {
                    break;
                }
                if (c as u32) <= 0x1f {
                    return Err(Fail("Invalid control character at", next));
                }
                next += 1;
            }
            if next >= len || !(c == '"' || c == '\\') {
                return Err(Fail("Unterminated string starting at", begin));
            }
            if c == '"' {
                return Ok(next + 1);
            }
            next += 1;
            if next == len {
                return Err(Fail("Unterminated string starting at", begin));
            }
            let esc = self.s[next];
            if esc != 'u' {
                end = next + 1;
                if !matches!(esc, '"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't') {
                    return Err(Fail("Invalid \\escape", end - 2));
                }
                continue;
            }
            next += 1;
            end = next + 4;
            if end >= len {
                return Err(Fail("Invalid \\uXXXX escape", next - 1));
            }
            let mut code = 0u32;
            for k in next..end {
                let d = self.s[k]
                    .to_digit(16)
                    .ok_or(Fail("Invalid \\uXXXX escape", end - 5))?;
                code = code << 4 | d;
            }
            next = end;
            if (0xD800..=0xDBFF).contains(&code)
                && end + 6 < len
                && self.s[next] == '\\'
                && self.s[next + 1] == 'u'
            {
                end += 6;
                let mut low = 0u32;
                for k in next + 2..end {
                    let d = self.s[k]
                        .to_digit(16)
                        .ok_or(Fail("Invalid \\uXXXX escape", end - 5))?;
                    low = low << 4 | d;
                }
                if !(0xDC00..=0xDFFF).contains(&low) {
                    end -= 6;
                }
            }
        }
    }

    fn object(&self, i: usize) -> Result<usize, Scan> {
        let mut i = self.skip_ws(i);
        if self.at(i) == Some('}') {
            return Ok(i + 1);
        }
        loop {
            if self.at(i) != Some('"') {
                return Err(Fail("Expecting property name enclosed in double quotes", i).into());
            }
            i = self.skip_ws(self.string(i + 1)?);
            if self.at(i) != Some(':') {
                return Err(Fail("Expecting ':' delimiter", i).into());
            }
            i = self.skip_ws(i + 1);
            i = self.skip_ws(self.scan(i)?);
            match self.at(i) {
                Some('}') => return Ok(i + 1),
                Some(',') => i = self.skip_ws(i + 1),
                _ => return Err(Fail("Expecting ',' delimiter", i).into()),
            }
        }
    }

    fn array(&self, i: usize) -> Result<usize, Scan> {
        let mut i = self.skip_ws(i);
        if self.at(i) == Some(']') {
            return Ok(i + 1);
        }
        loop {
            i = self.skip_ws(self.scan(i)?);
            match self.at(i) {
                Some(']') => return Ok(i + 1),
                Some(',') => i = self.skip_ws(i + 1),
                _ => return Err(Fail("Expecting ',' delimiter", i).into()),
            }
        }
    }
}

/// `str(e)` of the `json.JSONDecodeError` that `json.loads(text)` raises, or
/// `None` when CPython accepts the text (including `NaN` / `Infinity`).
pub fn json_decode_error(text: &str) -> Option<String> {
    let doc = Doc {
        s: text.chars().collect(),
    };
    let fail = if doc.at(0) == Some('\u{feff}') {
        Fail("Unexpected UTF-8 BOM (decode using utf-8-sig)", 0)
    } else {
        let start = doc.skip_ws(0);
        match doc.scan(start) {
            Ok(end) => {
                let end = doc.skip_ws(end);
                if end == doc.s.len() {
                    return None;
                }
                Fail("Extra data", end)
            }
            Err(Scan::Stop(i)) => Fail("Expecting value", i),
            Err(Scan::Err(f)) => f,
        }
    };
    let Fail(msg, pos) = fail;
    let before = &doc.s[..pos.min(doc.s.len())];
    let line = before.iter().filter(|&&c| c == '\n').count() + 1;
    let col = match before.iter().rposition(|&c| c == '\n') {
        Some(nl) => pos - nl,
        None => pos + 1,
    };
    Some(format!("{msg}: line {line} column {col} (char {pos})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `str(e)` from CPython 3.12 `json.loads(text)` (`None`: accepted).
    #[test]
    fn matches_python_json_loads() {
        let cases: &[(&str, Option<&str>)] = &[
            ("", Some("Expecting value: line 1 column 1 (char 0)")),
            (" ", Some("Expecting value: line 1 column 2 (char 1)")),
            (
                "{",
                Some("Expecting property name enclosed in double quotes: line 1 column 2 (char 1)"),
            ),
            ("}", Some("Expecting value: line 1 column 1 (char 0)")),
            ("[", Some("Expecting value: line 1 column 2 (char 1)")),
            ("]", Some("Expecting value: line 1 column 1 (char 0)")),
            (
                "{bad",
                Some("Expecting property name enclosed in double quotes: line 1 column 2 (char 1)"),
            ),
            (
                "[{",
                Some("Expecting property name enclosed in double quotes: line 1 column 3 (char 2)"),
            ),
            (
                "{\"a\"",
                Some("Expecting ':' delimiter: line 1 column 5 (char 4)"),
            ),
            ("{\"a\":", Some("Expecting value: line 1 column 6 (char 5)")),
            (
                "{\"a\":1",
                Some("Expecting ',' delimiter: line 1 column 7 (char 6)"),
            ),
            (
                "{\"a\":1,",
                Some("Expecting property name enclosed in double quotes: line 1 column 8 (char 7)"),
            ),
            (
                "{\"a\":1,}",
                Some("Expecting property name enclosed in double quotes: line 1 column 8 (char 7)"),
            ),
            ("[1,]", Some("Expecting value: line 1 column 4 (char 3)")),
            (
                "[1",
                Some("Expecting ',' delimiter: line 1 column 3 (char 2)"),
            ),
            (
                "[1 2]",
                Some("Expecting ',' delimiter: line 1 column 4 (char 3)"),
            ),
            (
                "{\"a\" 1}",
                Some("Expecting ':' delimiter: line 1 column 6 (char 5)"),
            ),
            (
                "{1:2}",
                Some("Expecting property name enclosed in double quotes: line 1 column 2 (char 1)"),
            ),
            (
                "\"abc",
                Some("Unterminated string starting at: line 1 column 1 (char 0)"),
            ),
            (
                "\"a\\",
                Some("Unterminated string starting at: line 1 column 1 (char 0)"),
            ),
            (
                "\"a\\x\"",
                Some("Invalid \\escape: line 1 column 3 (char 2)"),
            ),
            (
                "\"\\u12\"",
                Some("Invalid \\uXXXX escape: line 1 column 3 (char 2)"),
            ),
            (
                "\"\\u12",
                Some("Invalid \\uXXXX escape: line 1 column 3 (char 2)"),
            ),
            ("\"\\u12345\"", None),
            (
                "\"\\uZZZZ\"",
                Some("Invalid \\uXXXX escape: line 1 column 3 (char 2)"),
            ),
            (
                "\"\\ud800\\uZZZZ\"",
                Some("Invalid \\uXXXX escape: line 1 column 9 (char 8)"),
            ),
            (
                "\"\\ud800\\ud800\\uZZZZ\"",
                Some("Invalid \\uXXXX escape: line 1 column 15 (char 14)"),
            ),
            (
                "\"\\ud800\\u12\"",
                Some("Invalid \\uXXXX escape: line 1 column 9 (char 8)"),
            ),
            (
                "\"a\u{1}b\"",
                Some("Invalid control character at: line 1 column 3 (char 2)"),
            ),
            ("nul", Some("Expecting value: line 1 column 1 (char 0)")),
            ("tru", Some("Expecting value: line 1 column 1 (char 0)")),
            ("nulll", Some("Extra data: line 1 column 5 (char 4)")),
            ("NaN", None),
            ("-Infinity", None),
            ("Infinity", None),
            ("-", Some("Expecting value: line 1 column 1 (char 0)")),
            ("-a", Some("Expecting value: line 1 column 1 (char 0)")),
            ("01", Some("Extra data: line 1 column 2 (char 1)")),
            ("1.", Some("Extra data: line 1 column 2 (char 1)")),
            ("1.e5", Some("Extra data: line 1 column 2 (char 1)")),
            ("1e", Some("Extra data: line 1 column 2 (char 1)")),
            ("1e+", Some("Extra data: line 1 column 2 (char 1)")),
            (".5", Some("Expecting value: line 1 column 1 (char 0)")),
            ("1 2", Some("Extra data: line 1 column 3 (char 2)")),
            ("[1]x", Some("Extra data: line 1 column 4 (char 3)")),
            (
                "\u{feff}[]",
                Some("Unexpected UTF-8 BOM (decode using utf-8-sig): line 1 column 1 (char 0)"),
            ),
            (
                "[\n1,\n2\n",
                Some("Expecting ',' delimiter: line 4 column 1 (char 7)"),
            ),
            (
                "{\n\"a\":\n}",
                Some("Expecting value: line 3 column 1 (char 7)"),
            ),
            ("\n\n  x", Some("Expecting value: line 3 column 3 (char 4)")),
            ("[] \n []", Some("Extra data: line 2 column 2 (char 5)")),
            (
                "{\"a\":[1,{\"b\":tru}]}",
                Some("Expecting value: line 1 column 14 (char 13)"),
            ),
            (
                "\"\u{e9}\u{e9}",
                Some("Unterminated string starting at: line 1 column 1 (char 0)"),
            ),
            (
                "[\"\u{e9}\", x]",
                Some("Expecting value: line 1 column 7 (char 6)"),
            ),
            (
                "[true, false, null, nul]",
                Some("Expecting value: line 1 column 21 (char 20)"),
            ),
            (
                "{\"a\":1 \"b\":2}",
                Some("Expecting ',' delimiter: line 1 column 8 (char 7)"),
            ),
            ("[-]", Some("Expecting value: line 1 column 2 (char 1)")),
            ("[- 1]", Some("Expecting value: line 1 column 2 (char 1)")),
            (
                "{\"\\u00e9\":1,\"b\"}",
                Some("Expecting ':' delimiter: line 1 column 16 (char 15)"),
            ),
            ("\"ok\"", None),
            ("{}", None),
            ("[]", None),
            ("1.5e3", None),
            ("{\"a\": NaN}", None),
            ("\t[\r1\r]\tz", Some("Extra data: line 1 column 8 (char 7)")),
        ];
        for (text, want) in cases {
            assert_eq!(json_decode_error(text).as_deref(), *want, "{text:?}");
        }
    }
}
