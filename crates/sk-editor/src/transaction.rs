//! Transactions: the one way a document changes. Change positions are in the document before the
//! transaction; selections, marks and lines in the document after its changes.

use crate::buffer::TextBuffer;
use crate::doc::{Document, Meta};
use crate::value::{Block, EditorValue, Mark};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    pub fn caret(at: usize) -> Self {
        Self {
            anchor: at,
            head: at,
        }
    }

    pub fn from(&self) -> usize {
        self.anchor.min(self.head)
    }

    pub fn to(&self) -> usize {
        self.anchor.max(self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Insert {
    Text(String),
    Value(EditorValue),
}

impl Default for Insert {
    fn default() -> Self {
        Insert::Text(String::new())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChangeSpec {
    pub from: usize,
    pub to: Option<usize>,
    pub insert: Insert,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MarkRemoval {
    pub kind: Option<String>,
    pub from: usize,
    pub to: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetLines {
    pub from: usize,
    pub to: usize,
    pub kind: Option<String>,
    pub data: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transaction {
    pub changes: Vec<ChangeSpec>,
    pub selections: Option<Vec<Selection>>,
    pub add_marks: Vec<Mark>,
    pub remove_marks: Vec<MarkRemoval>,
    pub set_lines: Vec<SetLines>,
    pub history: bool,
    pub scroll_into_view: bool,
}

impl Default for Transaction {
    fn default() -> Self {
        Self {
            changes: Vec::new(),
            selections: None,
            add_marks: Vec::new(),
            remove_marks: Vec::new(),
            set_lines: Vec::new(),
            history: true,
            scroll_into_view: false,
        }
    }
}

/// One text replacement as history replays it: positions in the document it was applied to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextOp {
    pub at: usize,
    pub removed: String,
    pub inserted: String,
}

/// A change as the `change` event reports it: in the document before the transaction.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChangeDesc {
    pub from: usize,
    pub to: usize,
    pub insert: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    pub ops: Vec<TextOp>,
    pub changes: Vec<ChangeDesc>,
    pub before: Meta,
    pub after: Meta,
    pub doc_changed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TxError {
    Overlap { at: usize },
}

impl std::fmt::Display for TxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TxError::Overlap { at } => write!(
                f,
                "the transaction's changes overlap at {at}: give each change its own range of the \
                 document before the transaction"
            ),
        }
    }
}

/// Maps a position through `changes` (sorted, in the old document). A position at an insertion
/// point moves past the inserted text when `after`.
pub fn map_pos(changes: &[ChangeDesc], pos: usize, after: bool) -> usize {
    let mut delta: isize = 0;
    for c in changes {
        let ins = crate::buffer::utf16_len(&c.insert) as isize;
        if c.from == c.to {
            if pos < c.from || (pos == c.from && !after) {
                break;
            }
            delta += ins;
            continue;
        }
        if pos <= c.from {
            break;
        }
        if pos < c.to {
            return (c.from as isize + delta + if after { ins } else { 0 }) as usize;
        }
        delta += ins - (c.to - c.from) as isize;
    }
    (pos as isize + delta).max(0) as usize
}

impl<B: TextBuffer> Document<B> {
    /// Applies `tr`'s document part (not selections), recording what history needs.
    pub fn apply(&mut self, tr: &Transaction) -> Result<Applied, TxError> {
        let len = self.len();
        let mut specs: Vec<(usize, usize, &Insert)> = tr
            .changes
            .iter()
            .map(|c| {
                let from = c.from.min(len);
                let to = c.to.unwrap_or(c.from).clamp(from, len);
                (from, to, &c.insert)
            })
            .collect();
        specs.sort_by_key(|&(from, to, _)| (from, to));
        for pair in specs.windows(2) {
            if pair[1].0 < pair[0].1 {
                return Err(TxError::Overlap { at: pair[1].0 });
            }
        }
        let before = self.meta();
        let mut ops = Vec::with_capacity(specs.len());
        let mut changes = Vec::with_capacity(specs.len());
        for &(from, to, insert) in specs.iter().rev() {
            let (value, rich) = match insert {
                Insert::Text(t) => (EditorValue::plain(t).normalized(true), false),
                Insert::Value(v) => (v.clone().normalized(true), true),
            };
            if from == to && value.text.is_empty() {
                continue;
            }
            let removed = self.slice(from, to);
            self.replace(from, to, &value, rich);
            changes.push(ChangeDesc {
                from,
                to,
                insert: value.text.clone(),
            });
            ops.push(TextOp {
                at: from,
                removed,
                inserted: value.text,
            });
        }
        changes.reverse();
        for r in &tr.remove_marks {
            self.remove_marks(r.kind.as_deref(), r.from, r.to);
        }
        for m in &tr.add_marks {
            self.add_mark(m.clone());
        }
        for s in &tr.set_lines {
            let (l0, l1) = (self.line_at(s.from), self.line_at(s.to.max(s.from)));
            for line in l0..=l1 {
                let block = s.kind.as_ref().filter(|k| !k.is_empty()).map(|k| Block {
                    kind: k.clone(),
                    data: s.data.clone(),
                });
                self.set_block(line, block);
            }
        }
        let after = self.meta();
        let doc_changed = !ops.is_empty() || before != after;
        Ok(Applied {
            ops,
            changes,
            before,
            after,
            doc_changed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{Atom, LineSpec};

    fn doc(text: &str) -> Document {
        Document::from_value(EditorValue::plain(text))
    }

    fn change(from: usize, to: usize, text: &str) -> ChangeSpec {
        ChangeSpec {
            from,
            to: Some(to),
            insert: Insert::Text(text.into()),
        }
    }

    #[test]
    fn changes_are_positions_in_the_document_before_the_transaction() {
        let mut d = doc("hello world");
        let tr = Transaction {
            changes: vec![change(6, 11, "there"), change(0, 5, "Hi")],
            ..Default::default()
        };
        let applied = d.apply(&tr).unwrap();
        assert_eq!(d.text(), "Hi there");
        assert_eq!(
            applied.changes,
            vec![
                ChangeDesc {
                    from: 0,
                    to: 5,
                    insert: "Hi".into()
                },
                ChangeDesc {
                    from: 6,
                    to: 11,
                    insert: "there".into()
                }
            ]
        );
        let overlap = Transaction {
            changes: vec![change(0, 3, ""), change(2, 4, "")],
            ..Default::default()
        };
        assert_eq!(d.apply(&overlap), Err(TxError::Overlap { at: 2 }));
        assert_eq!(
            d.text(),
            "Hi there",
            "a rejected transaction changes nothing"
        );
    }

    #[test]
    fn marks_keep_to_their_text_and_do_not_grow_at_their_end() {
        let mut d = Document::from_value(EditorValue {
            text: "abcdef".into(),
            marks: vec![Mark::new("bold", 2, 4)],
            ..Default::default()
        });
        d.apply(&Transaction {
            changes: vec![change(4, 4, "X"), change(2, 2, "Y"), change(3, 3, "Z")],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(d.text(), "abYcZdXef");
        assert_eq!(
            d.marks(),
            &[Mark::new("bold", 3, 6)],
            "inside grows, edges do not"
        );
        d.apply(&Transaction {
            changes: vec![change(1, 4, "")],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(d.text(), "aZdXef");
        assert_eq!(d.marks(), &[Mark::new("bold", 1, 3)]);
    }

    #[test]
    fn adding_and_removing_marks_splits_and_replaces_by_type() {
        let mut d = doc("0123456789");
        d.apply(&Transaction {
            add_marks: vec![Mark::new("bold", 0, 10), Mark::new("italic", 2, 5)],
            remove_marks: vec![],
            ..Default::default()
        })
        .unwrap();
        d.apply(&Transaction {
            remove_marks: vec![MarkRemoval {
                kind: Some("bold".into()),
                from: 3,
                to: 6,
            }],
            ..Default::default()
        })
        .unwrap();
        let got: Vec<_> = d
            .marks()
            .iter()
            .map(|m| (m.kind.as_str(), m.from, m.to))
            .collect();
        assert_eq!(got, [("bold", 0, 3), ("italic", 2, 5), ("bold", 6, 10)]);
        d.apply(&Transaction {
            add_marks: vec![Mark {
                data: Some("1".into()),
                ..Mark::new("bold", 1, 8)
            }],
            ..Default::default()
        })
        .unwrap();
        let bolds: Vec<_> = d
            .marks()
            .iter()
            .filter(|m| m.kind == "bold")
            .map(|m| (m.from, m.to, m.data.clone()))
            .collect();
        assert_eq!(
            bolds,
            [(0, 1, None), (1, 8, Some("1".into())), (8, 10, None)],
            "a mark replaces its type over its range"
        );
    }

    #[test]
    fn atoms_move_with_text_and_go_with_their_character() {
        let atom = |id: &str, at| Atom {
            id: id.into(),
            at,
            kind: "mention".into(),
            text: Some("@x".into()),
            data: Some("{\"u\":1}".into()),
        };
        let mut d = Document::from_value(EditorValue {
            text: "a\u{FFFC}b\u{FFFC}".into(),
            atoms: vec![atom("one", 1), atom("two", 3)],
            ..Default::default()
        });
        d.apply(&Transaction {
            changes: vec![change(0, 0, "zz"), change(1, 2, "")],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(d.text(), "zzab\u{FFFC}");
        assert_eq!(d.atoms().len(), 1);
        assert_eq!((d.atoms()[0].id.as_str(), d.atoms()[0].at), ("two", 4));
        assert!(d.atoms_consistent());
        d.apply(&Transaction {
            changes: vec![change(0, 0, "\u{FFFC}")],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(d.text(), "zzab\u{FFFC}", "a bare U+FFFC is not an atom");
        let paste = EditorValue {
            text: "\u{FFFC}".into(),
            atoms: vec![atom("two", 0)],
            ..Default::default()
        };
        d.apply(&Transaction {
            changes: vec![ChangeSpec {
                from: 0,
                to: None,
                insert: Insert::Value(paste),
            }],
            ..Default::default()
        })
        .unwrap();
        let ids: Vec<_> = d.atoms().iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["two~2", "two"], "a pasted copy gets its own id");
        assert!(d.atoms_consistent());
    }

    #[test]
    fn line_blocks_follow_splits_and_joins() {
        let mut d = Document::from_value(EditorValue {
            text: "quote\nplain".into(),
            lines: vec![LineSpec {
                line: 0,
                kind: "quote".into(),
                data: None,
            }],
            ..Default::default()
        });
        d.apply(&Transaction {
            changes: vec![change(2, 2, "\n")],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(d.text(), "qu\note\nplain");
        let kinds = |d: &Document| {
            (0..d.line_count())
                .map(|l| d.block(l).map(|b| b.kind.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            kinds(&d),
            [Some("quote".into()), Some("quote".into()), None],
            "a split line's halves keep its block"
        );
        d.apply(&Transaction {
            changes: vec![change(6, 7, "")],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(d.text(), "qu\noteplain");
        assert_eq!(kinds(&d), [Some("quote".into()), Some("quote".into())]);
        d.apply(&Transaction {
            set_lines: vec![SetLines {
                from: 0,
                to: 4,
                kind: Some("code".into()),
                data: None,
            }],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(kinds(&d), [Some("code".into()), Some("code".into())]);
        let rich = EditorValue {
            text: "h\nx".into(),
            lines: vec![LineSpec {
                line: 0,
                kind: "h1".into(),
                data: None,
            }],
            ..Default::default()
        };
        d.apply(&Transaction {
            changes: vec![ChangeSpec {
                from: 3,
                to: None,
                insert: Insert::Value(rich),
            }],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(d.text(), "qu\nh\nxoteplain");
        assert_eq!(
            kinds(&d),
            [Some("code".into()), Some("h1".into()), None],
            "a rich paste at a line start brings its blocks"
        );
    }

    #[test]
    fn positions_map_through_changes() {
        let changes = [
            ChangeDesc {
                from: 2,
                to: 2,
                insert: "ab".into(),
            },
            ChangeDesc {
                from: 5,
                to: 8,
                insert: "".into(),
            },
        ];
        assert_eq!(map_pos(&changes, 1, true), 1);
        assert_eq!(map_pos(&changes, 2, true), 4);
        assert_eq!(map_pos(&changes, 2, false), 2);
        assert_eq!(
            map_pos(&changes, 6, true),
            7,
            "inside a deletion: its start"
        );
        assert_eq!(map_pos(&changes, 9, true), 8);
    }
}
