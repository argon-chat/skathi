//! `EditorValue` as JSON, the editor's own clipboard type. `data` fields pass through as raw JSON.

use crate::editor::{Decoration, LineDecoration};
use crate::transaction::{
    ChangeDesc, ChangeSpec, Insert, MarkRemoval, Selection, SetLines, Transaction,
};
use crate::value::{Atom, EditorValue, LineSpec, Mark};

/// The clipboard type an editor writes its selection under, beside `text/plain`.
pub const MIME: &str = "application/x-skathi-editor";

fn string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => {
                out.push_str(&format!("\\u{:04x}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn data(out: &mut String, d: &Option<String>) {
    if let Some(d) = d {
        out.push_str(",\"data\":");
        out.push_str(d);
    }
}

pub fn to_json(v: &EditorValue) -> String {
    let mut out = String::with_capacity(v.text.len() + 64);
    out.push_str("{\"text\":");
    string(&mut out, &v.text);
    out.push_str(",\"marks\":[");
    for (i, m) in v.marks.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"type\":");
        string(&mut out, &m.kind);
        out.push_str(&format!(",\"from\":{},\"to\":{}", m.from, m.to));
        data(&mut out, &m.data);
        out.push('}');
    }
    out.push_str("],\"atoms\":[");
    for (i, a) in v.atoms.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"id\":");
        string(&mut out, &a.id);
        out.push_str(&format!(",\"at\":{},\"type\":", a.at));
        string(&mut out, &a.kind);
        if let Some(t) = &a.text {
            out.push_str(",\"text\":");
            string(&mut out, t);
        }
        data(&mut out, &a.data);
        out.push('}');
    }
    out.push_str("],\"lines\":[");
    for (i, l) in v.lines.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!("{{\"line\":{},\"type\":", l.line));
        string(&mut out, &l.kind);
        data(&mut out, &l.data);
        out.push('}');
    }
    out.push_str("]}");
    out
}

#[derive(Debug)]
enum Json<'a> {
    Null,
    Bool,
    Num(f64),
    Str(String),
    Arr(Vec<(Json<'a>, &'a str)>),
    Obj(Vec<(String, Json<'a>, &'a str)>),
}

struct Parser<'a> {
    s: &'a str,
    i: usize,
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.s[self.i..].starts_with([' ', '\n', '\r', '\t']) {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: char) -> Option<()> {
        self.ws();
        self.s[self.i..].starts_with(c).then(|| self.i += 1)
    }

    /// A value and the raw text it was parsed from.
    fn value(&mut self) -> Option<(Json<'a>, &'a str)> {
        self.ws();
        let start = self.i;
        let rest = &self.s[self.i..];
        let v = if rest.starts_with('{') {
            self.i += 1;
            let mut fields = Vec::new();
            if self.eat('}').is_none() {
                loop {
                    self.ws();
                    let Json::Str(k) = self.value()?.0 else {
                        return None;
                    };
                    self.eat(':')?;
                    let (v, raw) = self.value()?;
                    fields.push((k, v, raw));
                    if self.eat(',').is_none() {
                        self.eat('}')?;
                        break;
                    }
                }
            }
            Json::Obj(fields)
        } else if rest.starts_with('[') {
            self.i += 1;
            let mut items = Vec::new();
            if self.eat(']').is_none() {
                loop {
                    items.push(self.value()?);
                    if self.eat(',').is_none() {
                        self.eat(']')?;
                        break;
                    }
                }
            }
            Json::Arr(items)
        } else if rest.starts_with('"') {
            Json::Str(self.string()?)
        } else if rest.starts_with("true") || rest.starts_with("false") {
            self.i += if rest.starts_with('t') { 4 } else { 5 };
            Json::Bool
        } else if rest.starts_with("null") {
            self.i += 4;
            Json::Null
        } else {
            let len = rest
                .find(|c: char| !(c.is_ascii_digit() || "+-.eE".contains(c)))
                .unwrap_or(rest.len());
            let n = rest[..len].parse().ok()?;
            self.i += len;
            Json::Num(n)
        };
        Some((v, &self.s[start..self.i]))
    }

    fn string(&mut self) -> Option<String> {
        self.i += 1;
        let mut out = String::new();
        let mut chars = self.s[self.i..].char_indices();
        while let Some((at, c)) = chars.next() {
            match c {
                '"' => {
                    self.i += at + 1;
                    return Some(out);
                }
                '\\' => {
                    let (_, e) = chars.next()?;
                    match e {
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        'u' => {
                            fn unit(chars: &mut std::str::CharIndices) -> Option<u32> {
                                let hex: String = (0..4)
                                    .filter_map(|_| chars.next())
                                    .map(|(_, c)| c)
                                    .collect();
                                u32::from_str_radix(&hex, 16).ok()
                            }
                            let hi = unit(&mut chars)?;
                            let c = if (0xD800..0xDC00).contains(&hi) {
                                let mut ahead = chars.clone();
                                let pair = match (ahead.next(), ahead.next()) {
                                    (Some((_, '\\')), Some((_, 'u'))) => {
                                        unit(&mut ahead).filter(|lo| (0xDC00..0xE000).contains(lo))
                                    }
                                    _ => None,
                                };
                                match pair {
                                    Some(lo) => {
                                        chars = ahead;
                                        char::from_u32(
                                            0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00),
                                        )
                                    }
                                    None => None,
                                }
                            } else {
                                char::from_u32(hi)
                            };
                            out.push(c.unwrap_or('\u{FFFD}'));
                        }
                        other => out.push(other),
                    }
                }
                c => out.push(c),
            }
        }
        None
    }
}

fn field<'b, 'a>(
    fields: &'b [(String, Json<'a>, &'a str)],
    name: &str,
) -> Option<&'b (String, Json<'a>, &'a str)> {
    fields.iter().find(|(k, _, _)| k == name)
}

fn num(fields: &[(String, Json, &str)], name: &str) -> Option<usize> {
    match field(fields, name)?.1 {
        Json::Num(n) if n >= 0.0 && n.is_finite() => Some(n as usize),
        _ => None,
    }
}

fn text(fields: &[(String, Json, &str)], name: &str) -> Option<String> {
    match &field(fields, name)?.1 {
        Json::Str(s) => Some(s.clone()),
        _ => None,
    }
}

fn raw(fields: &[(String, Json, &str)], name: &str) -> Option<String> {
    field(fields, name).and_then(|(_, v, raw)| (!matches!(v, Json::Null)).then(|| raw.to_string()))
}

fn objects<'b, 'a>(
    fields: &'b [(String, Json<'a>, &'a str)],
    name: &str,
) -> Vec<&'b [(String, Json<'a>, &'a str)]> {
    match field(fields, name).map(|f| &f.1) {
        Some(Json::Arr(items)) => items
            .iter()
            .filter_map(|(v, _)| match v {
                Json::Obj(f) => Some(f.as_slice()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn parse(s: &str) -> Option<Vec<(String, Json<'_>, &str)>> {
    let mut p = Parser { s, i: 0 };
    match p.value()? {
        (Json::Obj(fields), _) => Some(fields),
        _ => None,
    }
}

fn marks_of(fields: &[(String, Json, &str)], name: &str) -> Vec<Mark> {
    objects(fields, name)
        .into_iter()
        .filter_map(|f| {
            Some(Mark {
                kind: text(f, "type")?,
                from: num(f, "from")?,
                to: num(f, "to")?,
                data: raw(f, "data"),
            })
        })
        .collect()
}

/// Reads [`to_json`]'s form back; `None` for anything else. Not normalized.
pub fn from_json(s: &str) -> Option<EditorValue> {
    value_of(&parse(s)?)
}

/// A JS `Transaction` as `JSON.stringify` wrote it; malformed entries are skipped.
pub fn transaction_from_json(s: &str) -> Option<Transaction> {
    let fields = parse(s)?;
    let changes = objects(&fields, "changes")
        .into_iter()
        .filter_map(|f| {
            let insert = match field(f, "insert").map(|x| &x.1) {
                Some(Json::Str(s)) => Insert::Text(s.clone()),
                Some(Json::Obj(v)) => Insert::Value(value_of(v)?),
                _ => Insert::Text(String::new()),
            };
            Some(ChangeSpec {
                from: num(f, "from")?,
                to: num(f, "to"),
                insert,
            })
        })
        .collect();
    let selections =
        matches!(field(&fields, "selections"), Some((_, Json::Arr(_), _))).then(|| {
            objects(&fields, "selections")
                .into_iter()
                .filter_map(|f| {
                    let anchor = num(f, "anchor")?;
                    Some(Selection {
                        anchor,
                        head: num(f, "head").unwrap_or(anchor),
                    })
                })
                .collect()
        });
    let remove_marks = objects(&fields, "removeMarks")
        .into_iter()
        .filter_map(|f| {
            Some(MarkRemoval {
                kind: text(f, "type"),
                from: num(f, "from")?,
                to: num(f, "to")?,
            })
        })
        .collect();
    let set_lines = objects(&fields, "setLines")
        .into_iter()
        .filter_map(|f| {
            Some(SetLines {
                from: num(f, "from")?,
                to: num(f, "to")?,
                kind: text(f, "type"),
                data: raw(f, "data"),
            })
        })
        .collect();
    let flag = |name: &str, default: bool| match field(&fields, name).map(|x| &x.1) {
        Some(Json::Bool) => field(&fields, name).is_some_and(|x| x.2 == "true"),
        _ => default,
    };
    Some(Transaction {
        changes,
        selections,
        add_marks: marks_of(&fields, "addMarks"),
        remove_marks,
        set_lines,
        history: flag("history", true),
        scroll_into_view: flag("scrollIntoView", false),
    })
}

/// `[{from, to, name}]` for `setDecorations`.
pub fn decorations_from_json(s: &str) -> Vec<Decoration> {
    let wrapped = format!("{{\"d\":{s}}}");
    let Some(fields) = parse(&wrapped) else {
        return Vec::new();
    };
    objects(&fields, "d")
        .into_iter()
        .filter_map(|f| {
            Some(Decoration {
                from: num(f, "from")?,
                to: num(f, "to")?,
                name: text(f, "name")?,
            })
        })
        .collect()
}

/// `[{line, name, gutter?}]` for `setLineDecorations`.
pub fn line_decorations_from_json(s: &str) -> Vec<LineDecoration> {
    let wrapped = format!("{{\"d\":{s}}}");
    let Some(fields) = parse(&wrapped) else {
        return Vec::new();
    };
    objects(&fields, "d")
        .into_iter()
        .filter_map(|f| {
            Some(LineDecoration {
                line: num(f, "line")?,
                name: text(f, "name")?,
                gutter: text(f, "gutter"),
            })
        })
        .collect()
}

/// A string array (`typingMarks`, `exclusiveMarks`).
pub fn strings_from_json(s: &str) -> Vec<String> {
    let wrapped = format!("{{\"d\":{s}}}");
    match parse(&wrapped)
        .as_deref()
        .and_then(|f| field(f, "d"))
        .map(|x| &x.1)
    {
        Some(Json::Arr(items)) => items
            .iter()
            .filter_map(|(v, _)| match v {
                Json::Str(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The `change` event's list: `[{from, to, insert}]` in the document before the change.
pub fn changes_to_json(changes: &[ChangeDesc]) -> String {
    let mut out = String::from("[");
    for (i, c) in changes.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"from\":{},\"to\":{},\"insert\":",
            c.from, c.to
        ));
        string(&mut out, &c.insert);
        out.push('}');
    }
    out.push(']');
    out
}

pub fn strings_to_json(items: &[String]) -> String {
    let mut out = String::from("[");
    for (i, s) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        string(&mut out, s);
    }
    out.push(']');
    out
}

fn value_of(fields: &[(String, Json, &str)]) -> Option<EditorValue> {
    let marks = marks_of(fields, "marks");
    let atoms = objects(fields, "atoms")
        .into_iter()
        .filter_map(|f| {
            Some(Atom {
                id: text(f, "id")?,
                at: num(f, "at")?,
                kind: text(f, "type")?,
                text: text(f, "text"),
                data: raw(f, "data"),
            })
        })
        .collect();
    let lines = objects(fields, "lines")
        .into_iter()
        .filter_map(|f| {
            Some(LineSpec {
                line: num(f, "line")?,
                kind: text(f, "type")?,
                data: raw(f, "data"),
            })
        })
        .collect();
    Some(EditorValue {
        text: text(fields, "text")?,
        marks,
        atoms,
        lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_survives_the_clipboard_form_with_its_data_verbatim() {
        let v = EditorValue {
            text: "say \"hi\"\n\u{FFFC}🙂\\".into(),
            marks: vec![Mark {
                data: Some(r#"{"href":"https://x.y/?a=1&b=\"2\"","n":[1,2.5,null,true]}"#.into()),
                ..Mark::new("link", 0, 3)
            }],
            atoms: vec![Atom {
                id: "m\"1".into(),
                at: 9,
                kind: "mention".into(),
                text: Some("@Аня".into()),
                data: Some("42".into()),
            }],
            lines: vec![LineSpec {
                line: 1,
                kind: "quote".into(),
                data: None,
            }],
        };
        assert_eq!(from_json(&to_json(&v)), Some(v));
    }

    #[test]
    fn a_js_transaction_reads_with_its_defaults() {
        let tr = transaction_from_json(
            r#"{"changes":[{"from":1,"insert":"x"},{"from":3,"to":5,"insert":{"text":"y","marks":[{"type":"b","from":0,"to":1}]}}],
                "selections":[{"anchor":2}],"history":false,"setLines":[{"from":0,"to":0,"type":null}],
                "removeMarks":[{"from":0,"to":9}]}"#,
        )
        .unwrap();
        assert_eq!(tr.changes[0].to, None);
        let Insert::Value(v) = &tr.changes[1].insert else {
            panic!()
        };
        assert_eq!(v.marks[0].kind, "b");
        assert_eq!(tr.selections, Some(vec![Selection { anchor: 2, head: 2 }]));
        assert!(!tr.history && !tr.scroll_into_view);
        assert_eq!(tr.set_lines[0].kind, None);
        assert_eq!(tr.remove_marks[0].kind, None);
        assert_eq!(transaction_from_json("{}").unwrap().selections, None);
        assert_eq!(
            from_json(r#"{"text":"a\ud800b"}"#).unwrap().text,
            "a\u{FFFD}b"
        );
    }

    #[test]
    fn escapes_and_foreign_json_are_read_or_refused() {
        let v =
            from_json(r#" {"text":"A🙂\n","marks":[{"type":"b","from":0,"to":1,"data":null}]} "#)
                .unwrap();
        assert_eq!(v.text, "A🙂\n");
        assert_eq!(v.marks[0].data, None);
        assert_eq!(from_json("[1,2]"), None);
        assert_eq!(from_json(r#"{"marks":[]}"#), None);
        assert_eq!(from_json(r#"{"text":"unterminated"#), None);
    }
}
