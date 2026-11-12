//! `email.utils.parseaddr` (Python 3.12, `strict=True`) over
//! `email._parseaddr.AddrlistClass`.

const SPECIALS: &str = "()<>@,:;.\"[]";
const LWS: &str = " \t";
const CR: &str = "\r\n";

/// `email.utils.parseaddr(addr)`: `(realname, email)`, or `("", "")` when
/// the strict parser rejects the value or finds more than one address.
pub fn parseaddr(addr: &str) -> (String, String) {
    let addr = if check_parenthesis(addr) {
        addr.to_string()
    } else {
        "('', '')".to_string()
    };
    let addrs: Vec<(String, String)> = Addrlist::new(&addr)
        .addresslist()
        .into_iter()
        .map(|v| {
            if v.1.contains('[') {
                (String::new(), String::new())
            } else {
                v
            }
        })
        .collect();
    if addrs.len() != 1 {
        return (String::new(), String::new());
    }
    addrs.into_iter().next().unwrap_or_default()
}

/// `_iter_escaped_chars`: (position, char or `\` + char).
fn iter_escaped(addr: &[char]) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut escape = false;
    let mut pos = 0;
    for (i, &ch) in addr.iter().enumerate() {
        pos = i;
        if escape {
            out.push((i, format!("\\{ch}")));
            escape = false;
        } else if ch == '\\' {
            escape = true;
        } else {
            out.push((i, ch.to_string()));
        }
    }
    if escape {
        out.push((pos, "\\".into()));
    }
    out
}

/// `_strip_quoted_realnames`.
fn strip_quoted_realnames(addr: &str) -> String {
    if !addr.contains('"') {
        return addr.to_string();
    }
    let chars: Vec<char> = addr.chars().collect();
    let mut start = 0;
    let mut open: Option<usize> = None;
    let mut result = String::new();
    for (pos, ch) in iter_escaped(&chars) {
        if ch == "\"" {
            match open {
                None => open = Some(pos),
                Some(o) => {
                    if start != o {
                        result.extend(&chars[start..o]);
                    }
                    start = pos + 1;
                    open = None;
                }
            }
        }
    }
    if start < chars.len() {
        result.extend(&chars[start..]);
    }
    result
}

/// `_check_parenthesis`.
fn check_parenthesis(addr: &str) -> bool {
    let chars: Vec<char> = strip_quoted_realnames(addr).chars().collect();
    let mut opens = 0i64;
    for (_, ch) in iter_escaped(&chars) {
        if ch == "(" {
            opens += 1;
        } else if ch == ")" {
            opens -= 1;
            if opens < 0 {
                return false;
            }
        }
    }
    opens == 0
}

/// `email._parseaddr.quote`.
fn quote(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn is_space(s: &str) -> bool {
    fsv_core::pystr::strip(s).is_empty()
}

/// `AddrlistClass` state.
struct Addrlist {
    field: Vec<char>,
    pos: usize,
    commentlist: Vec<String>,
    atomends: String,
    phraseends: String,
}

impl Addrlist {
    fn new(field: &str) -> Self {
        let atomends = format!("{SPECIALS}{LWS}{CR}");
        let phraseends = atomends.replace('.', "");
        Self {
            field: field.chars().collect(),
            pos: 0,
            commentlist: Vec::new(),
            atomends,
            phraseends,
        }
    }

    fn more(&self) -> bool {
        self.pos < self.field.len()
    }

    fn cur(&self) -> char {
        self.field[self.pos]
    }

    fn at(&self, set: &str) -> bool {
        self.more() && set.contains(self.cur())
    }

    /// `AddressList(field).addresslist`.
    fn addresslist(mut self) -> Vec<(String, String)> {
        if self.field.is_empty() {
            return Vec::new();
        }
        let mut result = Vec::new();
        while self.more() {
            let ad = self.getaddress();
            if ad.is_empty() {
                result.push((String::new(), String::new()));
            } else {
                result.extend(ad);
            }
        }
        result
    }

    fn gotonext(&mut self) -> String {
        let mut ws = String::new();
        while self.more() {
            let c = self.cur();
            if LWS.contains(c) || c == '\n' || c == '\r' {
                if c != '\n' && c != '\r' {
                    ws.push(c);
                }
                self.pos += 1;
            } else if c == '(' {
                let c = self.getcomment();
                self.commentlist.push(c);
            } else {
                break;
            }
        }
        ws
    }

    fn getaddress(&mut self) -> Vec<(String, String)> {
        self.commentlist = Vec::new();
        self.gotonext();
        let oldpos = self.pos;
        let plist = self.getphraselist();
        self.gotonext();
        let mut returnlist = Vec::new();
        if !self.more() {
            if let Some(p) = plist.first() {
                returnlist = vec![(self.commentlist.join(" "), p.clone())];
            }
        } else if ".@".contains(self.cur()) {
            // Python restores `oldcl`, which aliases this same list: the
            // comments read so far stay and are read again.
            self.pos = oldpos;
            let addrspec = self.getaddrspec();
            returnlist = vec![(self.commentlist.join(" "), addrspec)];
        } else if self.cur() == ':' {
            let fieldlen = self.field.len();
            self.pos += 1;
            while self.more() {
                self.gotonext();
                if self.pos < fieldlen && self.cur() == ';' {
                    self.pos += 1;
                    break;
                }
                returnlist.extend(self.getaddress());
            }
        } else if self.cur() == '<' {
            let routeaddr = self.getrouteaddr();
            let name = if self.commentlist.is_empty() {
                plist.join(" ")
            } else {
                format!("{} ({})", plist.join(" "), self.commentlist.join(" "))
            };
            returnlist = vec![(name, routeaddr)];
        } else if let Some(p) = plist.first() {
            returnlist = vec![(self.commentlist.join(" "), p.clone())];
        } else if SPECIALS.contains(self.cur()) {
            self.pos += 1;
        }
        self.gotonext();
        if self.at(",") {
            self.pos += 1;
        }
        returnlist
    }

    fn getrouteaddr(&mut self) -> String {
        if self.cur() != '<' {
            return String::new();
        }
        let mut expectroute = false;
        self.pos += 1;
        self.gotonext();
        let mut adlist = String::new();
        while self.more() {
            if expectroute {
                self.getdomain();
                expectroute = false;
            } else if self.cur() == '>' {
                self.pos += 1;
                break;
            } else if self.cur() == '@' {
                self.pos += 1;
                expectroute = true;
            } else if self.cur() == ':' {
                self.pos += 1;
            } else {
                adlist = self.getaddrspec();
                self.pos += 1;
                break;
            }
            self.gotonext();
        }
        adlist
    }

    fn getaddrspec(&mut self) -> String {
        let mut aslist: Vec<String> = Vec::new();
        self.gotonext();
        while self.more() {
            let mut preserve_ws = true;
            let c = self.cur();
            if c == '.' {
                if aslist.last().is_some_and(|s| is_space(s)) {
                    aslist.pop();
                }
                aslist.push(".".into());
                self.pos += 1;
                preserve_ws = false;
            } else if c == '"' {
                let q = self.getquote();
                aslist.push(format!("\"{}\"", quote(&q)));
            } else if self.atomends.contains(c) {
                if aslist.last().is_some_and(|s| is_space(s)) {
                    aslist.pop();
                }
                break;
            } else {
                let a = self.getatom(None);
                aslist.push(a);
            }
            let ws = self.gotonext();
            if preserve_ws && !ws.is_empty() {
                aslist.push(ws);
            }
        }
        if !self.more() || self.cur() != '@' {
            return aslist.concat();
        }
        aslist.push("@".into());
        self.pos += 1;
        self.gotonext();
        let domain = self.getdomain();
        if domain.is_empty() {
            return String::new();
        }
        aslist.concat() + &domain
    }

    fn getdomain(&mut self) -> String {
        let mut sdlist = String::new();
        while self.more() {
            let c = self.cur();
            if LWS.contains(c) {
                self.pos += 1;
            } else if c == '(' {
                let c = self.getcomment();
                self.commentlist.push(c);
            } else if c == '[' {
                let d = self.getdomainliteral();
                sdlist.push_str(&d);
            } else if c == '.' {
                self.pos += 1;
                sdlist.push('.');
            } else if c == '@' {
                return String::new();
            } else if self.atomends.contains(c) {
                break;
            } else {
                let a = self.getatom(None);
                sdlist.push_str(&a);
            }
        }
        sdlist
    }

    fn getdelimited(&mut self, begin: char, endchars: &str, allowcomments: bool) -> String {
        if self.cur() != begin {
            return String::new();
        }
        let mut out = String::new();
        let mut quoted = false;
        self.pos += 1;
        while self.more() {
            let c = self.cur();
            if quoted {
                out.push(c);
                quoted = false;
            } else if endchars.contains(c) {
                self.pos += 1;
                break;
            } else if allowcomments && c == '(' {
                let c = self.getcomment();
                out.push_str(&c);
                continue;
            } else if c == '\\' {
                quoted = true;
            } else {
                out.push(c);
            }
            self.pos += 1;
        }
        out
    }

    fn getquote(&mut self) -> String {
        self.getdelimited('"', "\"\r", false)
    }

    fn getcomment(&mut self) -> String {
        self.getdelimited('(', ")\r", true)
    }

    fn getdomainliteral(&mut self) -> String {
        format!("[{}]", self.getdelimited('[', "]\r", false))
    }

    fn getatom(&mut self, atomends: Option<&str>) -> String {
        let ends = atomends.unwrap_or(&self.atomends).to_string();
        let mut out = String::new();
        while self.more() {
            if ends.contains(self.cur()) {
                break;
            }
            out.push(self.cur());
            self.pos += 1;
        }
        out
    }

    fn getphraselist(&mut self) -> Vec<String> {
        let mut plist = Vec::new();
        let fws = format!("{LWS}{CR}");
        while self.more() {
            let c = self.cur();
            if fws.contains(c) {
                self.pos += 1;
            } else if c == '"' {
                let q = self.getquote();
                plist.push(q);
            } else if c == '(' {
                let c = self.getcomment();
                self.commentlist.push(c);
            } else if self.phraseends.contains(c) {
                break;
            } else {
                let ends = self.phraseends.clone();
                let a = self.getatom(Some(&ends));
                plist.push(a);
            }
        }
        plist
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value as Json;

    /// `email.utils.parseaddr` over tricky inputs
    /// (`port/golden/gen_fsv_api_parseaddr.py`).
    #[test]
    fn parseaddr_matches_python() {
        let doc: Json =
            serde_json::from_str(include_str!("testdata/parseaddr_oracle.json")).unwrap();
        let cases = doc["cases"].as_array().unwrap();
        assert!(cases.len() > 40);
        for c in cases {
            let input = c["input"].as_str().unwrap();
            let (name, email) = parseaddr(input);
            assert_eq!(
                (name.as_str(), email.as_str()),
                (c["name"].as_str().unwrap(), c["email"].as_str().unwrap()),
                "{input:?}"
            );
        }
    }
}
