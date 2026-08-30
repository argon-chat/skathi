//! A small JSON reader/writer for the dev-loop control files
//! (`dist/manifest.json`, `dist/changes.json`) and the vn-devtools wire.
//!
//! Both file formats are written by our own `packages/vue-native/build.ts`, so
//! the reader only has to be correct, not fast or forgiving. The writer exists
//! for the same reason: `serde` + `serde_json` + derives is a lot of build for
//! one newline-delimited protocol that never leaves localhost, and the whole
//! thing is ~90 lines.

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_arr(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(a) => Some(a),
            _ => None,
        }
    }

    // -- writing ----------------------------------------------------------
    // Constructors short enough to build a response inline.

    pub fn s(v: impl Into<String>) -> Json {
        Json::Str(v.into())
    }
    pub fn n(v: impl Into<f64>) -> Json {
        Json::Num(v.into())
    }
    pub fn obj(entries: Vec<(&str, Json)>) -> Json {
        Json::Obj(entries.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }
    pub fn arr(items: Vec<Json>) -> Json {
        Json::Arr(items)
    }

    /// Compact (no whitespace) — the wire is newline-delimited, so a value may
    /// not contain a raw newline anywhere. `quote` escapes them; numbers that
    /// are not finite become `null`, exactly as `JSON.stringify` does.
    pub fn write(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) if n.is_finite() => {
                if *n == n.trunc() && n.abs() < 1e15 {
                    out.push_str(&format!("{}", *n as i64));
                } else {
                    out.push_str(&format!("{n}"));
                }
            }
            Json::Num(_) => out.push_str("null"),
            Json::Str(s) => out.push_str(&quote(s)),
            Json::Arr(items) => {
                out.push('[');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write(out);
                }
                out.push(']');
            }
            Json::Obj(entries) => {
                out.push('{');
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&quote(k));
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }

    pub fn to_json(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }
}

/// A value the host must pass through byte-for-byte because JS already
/// stringified it (`eval`'s result). Wrapping it in [`Json::Str`] would quote
/// it a second time.
#[derive(Debug, Clone)]
pub struct Raw(pub String);

impl From<Raw> for Json {
    /// Parses the pre-stringified text back, falling back to a string when it
    /// is not valid JSON — never emits a malformed line onto the wire.
    fn from(raw: Raw) -> Json {
        parse(&raw.0).unwrap_or(Json::Str(raw.0))
    }
}

pub fn parse(text: &str) -> Option<Json> {
    let mut p = Parser { b: text.as_bytes(), i: 0 };
    let v = p.value()?;
    p.ws();
    (p.i == p.b.len()).then_some(v)
}

/// A JSON string literal — also a valid JS string literal, so this is how we
/// splice a module id into an `eval`'d call.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn eat(&mut self, lit: &str) -> bool {
        if self.b[self.i..].starts_with(lit.as_bytes()) {
            self.i += lit.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self) -> Option<Json> {
        self.ws();
        match *self.b.get(self.i)? {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => self.string().map(Json::Str),
            b't' => self.eat("true").then_some(Json::Bool(true)),
            b'f' => self.eat("false").then_some(Json::Bool(false)),
            b'n' => self.eat("null").then_some(Json::Null),
            _ => self.number(),
        }
    }

    fn object(&mut self) -> Option<Json> {
        self.i += 1; // '{'
        let mut out = Vec::new();
        self.ws();
        if self.b.get(self.i) == Some(&b'}') {
            self.i += 1;
            return Some(Json::Obj(out));
        }
        loop {
            self.ws();
            let key = self.string()?;
            self.ws();
            if self.b.get(self.i)? != &b':' {
                return None;
            }
            self.i += 1;
            out.push((key, self.value()?));
            self.ws();
            match *self.b.get(self.i)? {
                b',' => self.i += 1,
                b'}' => {
                    self.i += 1;
                    return Some(Json::Obj(out));
                }
                _ => return None,
            }
        }
    }

    fn array(&mut self) -> Option<Json> {
        self.i += 1; // '['
        let mut out = Vec::new();
        self.ws();
        if self.b.get(self.i) == Some(&b']') {
            self.i += 1;
            return Some(Json::Arr(out));
        }
        loop {
            out.push(self.value()?);
            self.ws();
            match *self.b.get(self.i)? {
                b',' => self.i += 1,
                b']' => {
                    self.i += 1;
                    return Some(Json::Arr(out));
                }
                _ => return None,
            }
        }
    }

    fn string(&mut self) -> Option<String> {
        if self.b.get(self.i)? != &b'"' {
            return None;
        }
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = *self.b.get(self.i)?;
            self.i += 1;
            match c {
                b'"' => return String::from_utf8(out).ok(),
                b'\\' => {
                    let e = *self.b.get(self.i)?;
                    self.i += 1;
                    match e {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0c),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let mut cp = self.hex4()? as u32;
                            // surrogate pair
                            if (0xd800..0xdc00).contains(&cp)
                                && self.b.get(self.i) == Some(&b'\\')
                                && self.b.get(self.i + 1) == Some(&b'u')
                            {
                                self.i += 2;
                                let lo = self.hex4()? as u32;
                                cp = 0x10000 + ((cp - 0xd800) << 10) + (lo - 0xdc00);
                            }
                            let ch = char::from_u32(cp).unwrap_or('\u{fffd}');
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        _ => return None,
                    }
                }
                c => out.push(c), // raw UTF-8 passes straight through
            }
        }
    }

    fn hex4(&mut self) -> Option<u16> {
        let s = std::str::from_utf8(self.b.get(self.i..self.i + 4)?).ok()?;
        self.i += 4;
        u16::from_str_radix(s, 16).ok()
    }

    fn number(&mut self) -> Option<Json> {
        let start = self.i;
        while self.i < self.b.len()
            && matches!(self.b[self.i], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E')
        {
            self.i += 1;
        }
        std::str::from_utf8(&self.b[start..self.i]).ok()?.parse().ok().map(Json::Num)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_manifest() {
        let v = parse(r#"{"rev":3,"vendorRev":1,"entry":"a/b.js","modules":["a/b.js","a/C.vue"]}"#)
            .unwrap();
        assert_eq!(v.get("rev").and_then(Json::as_f64), Some(3.0));
        assert_eq!(v.get("entry").and_then(Json::as_str), Some("a/b.js"));
        assert_eq!(v.get("modules").and_then(Json::as_arr).unwrap().len(), 2);
    }

    #[test]
    fn reads_changes_and_escapes() {
        let v = parse("{\"changed\":[{\"id\":\"x\\u00e9/y.vue\",\"kind\":\"rerender\"}]}").unwrap();
        let a = v.get("changed").and_then(Json::as_arr).unwrap();
        assert_eq!(a[0].get("id").and_then(Json::as_str), Some("xé/y.vue"));
        assert_eq!(a[0].get("kind").and_then(Json::as_str), Some("rerender"));
    }

    #[test]
    fn rejects_trailing_garbage() {
        assert!(parse("{} x").is_none());
        assert!(parse("{\"a\":1,}").is_none());
    }

    #[test]
    fn writes_what_it_reads() {
        let text = r#"{"ok":true,"data":{"n":[1,2.5,-3],"s":"a\"b\n","z":null}}"#;
        let v = parse(text).unwrap();
        let round = v.to_json();
        assert_eq!(parse(&round).unwrap(), v);
        // Integers do not grow a `.0`, and control chars stay escaped so the
        // newline-delimited wire cannot be split by a payload.
        assert!(round.contains(r#""n":[1,2.5,-3]"#), "{round}");
        assert!(!round.contains('\n'), "{round}");
    }

    #[test]
    fn non_finite_numbers_become_null_like_json_stringify() {
        assert_eq!(Json::Num(f64::NAN).to_json(), "null");
        assert_eq!(Json::Num(f64::INFINITY).to_json(), "null");
    }

    #[test]
    fn raw_passes_pre_stringified_json_through_once() {
        assert_eq!(Json::from(Raw("{\"a\":1}".into())).to_json(), "{\"a\":1}");
        // …and degrades to a plain string rather than corrupting the line.
        assert_eq!(Json::from(Raw("not json".into())).to_json(), "\"not json\"");
    }
}
