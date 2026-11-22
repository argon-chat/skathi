//! `EditorValue`, the document as the JS API and the clipboard carry it. Offsets are UTF-16 code
//! units; `data` is kept as JSON text and never interpreted.

use crate::buffer::utf16_len;

/// The object replacement character an atom stands on.
pub const ATOM_CHAR: char = '\u{FFFC}';

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mark {
    pub kind: String,
    pub from: usize,
    pub to: usize,
    pub data: Option<String>,
}

impl Mark {
    pub fn new(kind: &str, from: usize, to: usize) -> Self {
        Self {
            kind: kind.to_owned(),
            from,
            to,
            data: None,
        }
    }

    fn same_kind(&self, other: &Mark) -> bool {
        self.kind == other.kind && self.data == other.data
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Atom {
    pub id: String,
    pub at: usize,
    pub kind: String,
    /// What the atom is as plain text (the clipboard's `text/plain`).
    pub text: Option<String>,
    pub data: Option<String>,
}

/// A line's block type, kept per hard line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Block {
    pub kind: String,
    pub data: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineSpec {
    pub line: usize,
    pub kind: String,
    pub data: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditorValue {
    pub text: String,
    pub marks: Vec<Mark>,
    pub atoms: Vec<Atom>,
    pub lines: Vec<LineSpec>,
}

impl EditorValue {
    pub fn plain(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            ..Default::default()
        }
    }

    pub fn len_utf16(&self) -> usize {
        utf16_len(&self.text)
    }

    /// The plain-text reading: every atom replaced by its `text`.
    pub fn plain_text(&self) -> String {
        let mut out = String::with_capacity(self.text.len());
        let mut pos = 0usize;
        let mut atoms = self.atoms.iter().peekable();
        for c in self.text.chars() {
            if c == ATOM_CHAR {
                while atoms.peek().is_some_and(|a| a.at < pos) {
                    atoms.next();
                }
                if let Some(atom) = atoms.peek().filter(|a| a.at == pos) {
                    out.push_str(atom.text.as_deref().unwrap_or(""));
                }
            } else {
                out.push(c);
            }
            pos += c.len_utf16();
        }
        out
    }

    /// Makes the value well-formed: `\r\n` and `\r` become `\n` (single-line: a space), control
    /// characters other than `\n` and `\t` go, an atom must stand on a U+FFFC (one without an atom
    /// goes too), and marks are clipped, merged and sorted. Offsets follow the text.
    pub fn normalized(mut self, multiline: bool) -> Self {
        self.atoms.sort_by_key(|a| a.at);
        let mut map = Vec::with_capacity(self.text.len() + 1);
        let mut text = String::with_capacity(self.text.len());
        let (mut old, mut new) = (0usize, 0usize);
        let mut atoms = Vec::with_capacity(self.atoms.len());
        let mut pending = self.atoms.into_iter().peekable();
        let mut chars = self.text.chars().peekable();
        while let Some(c) = chars.next() {
            let units = c.len_utf16();
            for _ in 0..units {
                map.push(new);
            }
            let keep: Option<char> = match c {
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        None
                    } else {
                        Some(if multiline { '\n' } else { ' ' })
                    }
                }
                '\n' => Some(if multiline { '\n' } else { ' ' }),
                '\t' => Some('\t'),
                ATOM_CHAR => {
                    while pending.peek().is_some_and(|a| a.at < old) {
                        pending.next();
                    }
                    match pending.next_if(|a| a.at == old) {
                        Some(mut atom) => {
                            atom.at = new;
                            atoms.push(atom);
                            Some(ATOM_CHAR)
                        }
                        None => None,
                    }
                }
                c if c.is_control() => None,
                c => Some(c),
            };
            if let Some(k) = keep {
                text.push(k);
                new += k.len_utf16();
            }
            old += units;
        }
        map.push(new);
        let at = |p: usize| map[p.min(old)];
        let mut marks: Vec<Mark> = self
            .marks
            .into_iter()
            .filter_map(|mut m| {
                let (from, to) = (at(m.from), at(m.to.max(m.from)));
                (from < to && !m.kind.is_empty()).then(|| {
                    m.from = from;
                    m.to = to;
                    m
                })
            })
            .collect();
        normalize_marks(&mut marks);
        let lines = utf16_lines(&text);
        let mut specs: Vec<LineSpec> = self
            .lines
            .into_iter()
            .filter(|l| !l.kind.is_empty() && l.line < lines)
            .collect();
        specs.sort_by_key(|l| l.line);
        specs.dedup_by_key(|l| l.line);
        Self {
            text,
            marks,
            atoms,
            lines: specs,
        }
    }
}

fn utf16_lines(text: &str) -> usize {
    text.bytes().filter(|&b| b == b'\n').count() + 1
}

/// Sorts marks and merges overlapping or touching ranges of the same type and data.
pub fn normalize_marks(marks: &mut Vec<Mark>) {
    marks.retain(|m| m.from < m.to);
    marks.sort_by(|a, b| {
        (a.kind.as_str(), &a.data, a.from, a.to).cmp(&(b.kind.as_str(), &b.data, b.from, b.to))
    });
    let mut out: Vec<Mark> = Vec::with_capacity(marks.len());
    for m in marks.drain(..) {
        match out.last_mut() {
            Some(last) if last.same_kind(&m) && m.from <= last.to => last.to = last.to.max(m.to),
            _ => out.push(m),
        }
    }
    out.sort_by(|a, b| (a.from, a.to, a.kind.as_str()).cmp(&(b.from, b.to, b.kind.as_str())));
    *marks = out;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizing_drops_stray_characters_and_keeps_offsets_on_their_text() {
        let v = EditorValue {
            text: "a\r\nb\u{0}\u{FFFC}c\u{FFFC}".into(),
            marks: vec![Mark::new("bold", 3, 7)],
            atoms: vec![Atom {
                id: "m1".into(),
                at: 5,
                kind: "mention".into(),
                ..Default::default()
            }],
            lines: vec![LineSpec {
                line: 1,
                kind: "quote".into(),
                data: None,
            }],
        }
        .normalized(true);
        assert_eq!(v.text, "a\nb\u{FFFC}c");
        assert_eq!(v.atoms[0].at, 3);
        assert_eq!(v.marks, vec![Mark::new("bold", 2, 5)]);
        assert_eq!(v.lines.len(), 1);
        let single = EditorValue::plain("a\nb").normalized(false);
        assert_eq!(single.text, "a b");
    }

    #[test]
    fn marks_of_one_type_and_data_merge_and_others_stay_apart() {
        let mut marks = vec![
            Mark::new("bold", 4, 8),
            Mark::new("bold", 0, 4),
            Mark::new("italic", 2, 3),
            Mark {
                data: Some("\"x\"".into()),
                ..Mark::new("link", 0, 2)
            },
            Mark {
                data: Some("\"y\"".into()),
                ..Mark::new("link", 2, 4)
            },
            Mark::new("bold", 9, 9),
        ];
        normalize_marks(&mut marks);
        let kinds: Vec<_> = marks
            .iter()
            .map(|m| (m.kind.as_str(), m.from, m.to))
            .collect();
        assert_eq!(
            kinds,
            [
                ("link", 0, 2),
                ("bold", 0, 8),
                ("italic", 2, 3),
                ("link", 2, 4)
            ]
        );
    }

    #[test]
    fn plain_text_reads_atoms_as_their_text() {
        let v = EditorValue {
            text: "hi \u{FFFC}!".into(),
            atoms: vec![Atom {
                id: "a".into(),
                at: 3,
                kind: "mention".into(),
                text: Some("@Ann".into()),
                data: None,
            }],
            ..Default::default()
        };
        assert_eq!(v.plain_text(), "hi @Ann!");
    }
}
