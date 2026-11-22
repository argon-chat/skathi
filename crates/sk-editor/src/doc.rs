//! The document: text, marks over it, atoms on U+FFFC characters, and a block type per hard line.

use std::ops::Range;

use crate::buffer::{utf16_len, SmallBuffer, TextBuffer};
use crate::value::{normalize_marks, Atom, Block, EditorValue, LineSpec, Mark, ATOM_CHAR};

/// Marks, atoms and line blocks: everything but the text, as history records it around a step.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Meta {
    pub marks: Vec<Mark>,
    pub atoms: Vec<Atom>,
    pub lines: Vec<LineSpec>,
}

#[derive(Clone, Debug)]
pub struct Document<B: TextBuffer = SmallBuffer> {
    buf: B,
    marks: Vec<Mark>,
    atoms: Vec<Atom>,
    blocks: Vec<Option<Block>>,
}

impl<B: TextBuffer> Default for Document<B> {
    fn default() -> Self {
        Self {
            buf: B::default(),
            marks: Vec::new(),
            atoms: Vec::new(),
            blocks: vec![None],
        }
    }
}

impl Document<SmallBuffer> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_value(value: EditorValue) -> Self {
        let mut doc = Self::new();
        doc.set_value(value);
        doc
    }
}

impl<B: TextBuffer> Document<B> {
    pub fn set_value(&mut self, value: EditorValue) {
        let v = value.normalized(true);
        self.buf = B::default();
        self.buf.replace(0..0, &v.text);
        self.marks = v.marks;
        self.atoms = v.atoms;
        self.blocks = vec![None; self.buf.line_count()];
        for spec in v.lines {
            self.blocks[spec.line] = Some(Block {
                kind: spec.kind,
                data: spec.data,
            });
        }
    }

    pub fn to_value(&self) -> EditorValue {
        let meta = self.meta();
        EditorValue {
            text: self.buf.text().to_owned(),
            marks: meta.marks,
            atoms: meta.atoms,
            lines: meta.lines,
        }
    }

    /// Whether `value`, once normalized, is this document.
    pub fn equals_value(&self, value: &EditorValue) -> bool {
        let v = value.clone().normalized(true);
        v.text == self.buf.text()
            && v.marks == self.marks
            && v.atoms == self.atoms
            && v.lines == self.meta().lines
    }

    pub fn buffer(&self) -> &B {
        &self.buf
    }

    pub fn text(&self) -> &str {
        self.buf.text()
    }

    pub fn len(&self) -> usize {
        self.buf.len_utf16()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn line_count(&self) -> usize {
        self.buf.line_count()
    }

    pub fn line_range(&self, n: usize) -> Range<usize> {
        self.buf.line_range(n)
    }

    pub fn line_at(&self, pos: usize) -> usize {
        self.buf.line_at(pos.min(self.len()))
    }

    pub fn slice(&self, from: usize, to: usize) -> String {
        let (from, to) = self.clamp(from, to);
        self.buf.slice(from..to)
    }

    pub fn marks(&self) -> &[Mark] {
        &self.marks
    }

    pub fn atoms(&self) -> &[Atom] {
        &self.atoms
    }

    pub fn atom_at(&self, pos: usize) -> Option<&Atom> {
        self.atoms
            .binary_search_by_key(&pos, |a| a.at)
            .ok()
            .map(|i| &self.atoms[i])
    }

    pub fn block(&self, line: usize) -> Option<&Block> {
        self.blocks.get(line).and_then(Option::as_ref)
    }

    fn clamp(&self, from: usize, to: usize) -> (usize, usize) {
        let len = self.len();
        let from = from.min(len);
        (from, to.clamp(from, len))
    }

    /// Rounds a position off the inside of a surrogate pair, toward the start.
    pub fn snap(&self, pos: usize) -> usize {
        self.buf.to_utf16(self.buf.to_utf8(pos))
    }

    pub fn meta(&self) -> Meta {
        Meta {
            marks: self.marks.clone(),
            atoms: self.atoms.clone(),
            lines: self
                .blocks
                .iter()
                .enumerate()
                .filter_map(|(line, b)| {
                    b.as_ref().map(|b| LineSpec {
                        line,
                        kind: b.kind.clone(),
                        data: b.data.clone(),
                    })
                })
                .collect(),
        }
    }

    pub fn restore_meta(&mut self, meta: &Meta) {
        self.marks = meta.marks.clone();
        self.atoms = meta.atoms.clone();
        self.blocks = vec![None; self.buf.line_count()];
        for spec in &meta.lines {
            if let Some(slot) = self.blocks.get_mut(spec.line) {
                *slot = Some(Block {
                    kind: spec.kind.clone(),
                    data: spec.data.clone(),
                });
            }
        }
    }

    /// Text only, for history: marks, atoms and blocks are put back with [`Self::restore_meta`].
    pub fn replace_text_raw(&mut self, from: usize, to: usize, text: &str) {
        let (from, to) = self.clamp(from, to);
        self.buf.replace(from..to, text);
        self.blocks.resize(self.buf.line_count(), None);
    }

    /// `[from, to)` with its marks, atoms and line blocks, offsets relative to `from`.
    pub fn slice_value(&self, from: usize, to: usize) -> EditorValue {
        let (from, to) = self.clamp(from, to);
        let marks = self
            .marks
            .iter()
            .filter(|m| m.from < to && m.to > from)
            .map(|m| Mark {
                from: m.from.max(from) - from,
                to: m.to.min(to) - from,
                ..m.clone()
            })
            .collect();
        let atoms = self
            .atoms
            .iter()
            .filter(|a| a.at >= from && a.at < to)
            .map(|a| Atom {
                at: a.at - from,
                ..a.clone()
            })
            .collect();
        let (l0, l1) = (self.line_at(from), self.line_at(to));
        let lines = (l0..=l1)
            .filter_map(|l| {
                self.block(l).map(|b| LineSpec {
                    line: l - l0,
                    kind: b.kind.clone(),
                    data: b.data.clone(),
                })
            })
            .collect();
        EditorValue {
            text: self.buf.slice(from..to),
            marks,
            atoms,
            lines,
        }
    }

    /// Replaces `[from, to)` with a normalized `insert`. Marks around the range keep to their
    /// text (one ending at `from` does not grow, one spanning it does), atoms inside it go, and the
    /// inserted lines take `insert`'s blocks when `rich`, else the block of the line they split.
    pub fn replace(&mut self, from: usize, to: usize, insert: &EditorValue, rich: bool) {
        let (from, to) = self.clamp(from, to);
        let (from, to) = (self.snap(from), self.snap(to));
        let ins = insert.len_utf16();
        let removed = to - from;
        let (l0, l1) = (self.buf.line_at(from), self.buf.line_at(to));
        let at_line_start = self.buf.line_range(l0).start == from;
        let kept = self.blocks.get(l0).cloned().flatten();
        let spec = |line: usize| {
            insert.lines.iter().find(|s| s.line == line).map(|s| Block {
                kind: s.kind.clone(),
                data: s.data.clone(),
            })
        };
        let added_lines = insert.text.bytes().filter(|&b| b == b'\n').count();
        let mut new_blocks = Vec::with_capacity(added_lines + 1);
        new_blocks.push(if rich && at_line_start && spec(0).is_some() {
            spec(0)
        } else {
            kept.clone()
        });
        for i in 1..=added_lines {
            new_blocks.push(if rich { spec(i) } else { kept.clone() });
        }

        self.buf.replace(from..to, &insert.text);
        let end = l1.min(self.blocks.len().saturating_sub(1));
        self.blocks.splice(l0..=end, new_blocks);
        self.blocks.resize(self.buf.line_count(), None);

        let map_start = |p: usize| {
            if p < from {
                p
            } else if p >= to {
                p - removed + ins
            } else {
                from + ins
            }
        };
        let map_end = |p: usize| {
            if p <= from {
                p
            } else if p > to {
                p - removed + ins
            } else {
                from
            }
        };
        for m in &mut self.marks {
            m.from = map_start(m.from);
            m.to = map_end(m.to);
        }
        self.marks.extend(insert.marks.iter().map(|m| Mark {
            from: m.from + from,
            to: m.to + from,
            ..m.clone()
        }));
        normalize_marks(&mut self.marks);

        self.atoms.retain(|a| a.at < from || a.at >= to);
        for a in &mut self.atoms {
            if a.at >= to {
                a.at = a.at - removed + ins;
            }
        }
        for a in &insert.atoms {
            let mut atom = Atom {
                at: a.at + from,
                ..a.clone()
            };
            if self.atoms.iter().any(|x| x.id == atom.id) {
                let base = atom.id.clone();
                let mut n = 2;
                while self.atoms.iter().any(|x| x.id == atom.id) {
                    atom.id = format!("{base}~{n}");
                    n += 1;
                }
            }
            self.atoms.push(atom);
        }
        self.atoms.sort_by_key(|a| a.at);
    }

    /// Puts `mark` on its range, replacing marks of its type there.
    pub fn add_mark(&mut self, mark: Mark) {
        let (from, to) = self.clamp(mark.from, mark.to);
        if from >= to || mark.kind.is_empty() {
            return;
        }
        self.remove_marks(Some(&mark.kind), from, to);
        self.marks.push(Mark { from, to, ..mark });
        normalize_marks(&mut self.marks);
    }

    /// Takes marks (of `kind`, or every kind) off `[from, to)`, splitting those that reach past it.
    pub fn remove_marks(&mut self, kind: Option<&str>, from: usize, to: usize) {
        let (from, to) = self.clamp(from, to);
        if from >= to {
            return;
        }
        let mut out = Vec::with_capacity(self.marks.len());
        for m in self.marks.drain(..) {
            let hit = kind.is_none_or(|k| k == m.kind) && m.from < to && m.to > from;
            if !hit {
                out.push(m);
                continue;
            }
            if m.from < from {
                out.push(Mark {
                    to: from,
                    ..m.clone()
                });
            }
            if m.to > to {
                out.push(Mark { from: to, ..m });
            }
        }
        self.marks = out;
        normalize_marks(&mut self.marks);
    }

    pub fn set_block(&mut self, line: usize, block: Option<Block>) {
        if let Some(slot) = self.blocks.get_mut(line) {
            *slot = block;
        }
    }

    /// The marks a character typed at `pos` continues: those covering the character before it
    /// (at the start of a line, the one after it). A mark of an `exclusive` type does not grow
    /// past its end.
    pub fn marks_at(&self, pos: usize, exclusive: &dyn Fn(&str) -> bool) -> Vec<&Mark> {
        let pos = pos.min(self.len());
        let line_start = self.buf.line_range(self.buf.line_at(pos)).start == pos;
        self.marks
            .iter()
            .filter(|m| {
                if line_start {
                    m.from <= pos && pos < m.to
                } else {
                    m.from < pos && pos <= m.to && !(pos == m.to && exclusive(&m.kind))
                }
            })
            .collect()
    }

    /// Whether every atom stands on a U+FFFC and every U+FFFC has an atom.
    pub fn atoms_consistent(&self) -> bool {
        let text = self.buf.text();
        let mut pos = 0usize;
        let mut found = Vec::new();
        for c in text.chars() {
            if c == ATOM_CHAR {
                found.push(pos);
            }
            pos += c.len_utf16();
        }
        found.len() == self.atoms.len() && found.iter().zip(&self.atoms).all(|(p, a)| *p == a.at)
    }
}

/// Units of `s` in UTF-16, re-exported for callers that build positions.
pub fn len16(s: &str) -> usize {
    utf16_len(s)
}
