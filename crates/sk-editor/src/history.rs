//! Undo and redo as a log of steps: each step keeps the text it replaced, not a copy of the
//! document, plus the marks, atoms and line blocks around it. Typing and single deletions in a
//! row merge into one step.

use crate::buffer::{utf16_len, TextBuffer};
use crate::doc::{Document, Meta};
use crate::transaction::{compose, Applied, ChangeDesc, Selection, TextOp};

const LIMIT: usize = 256;

/// What a step was, for merging with the next one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepKind {
    /// Text typed at a caret; the caret ended at `end`.
    Typing {
        end: usize,
    },
    /// One character removed before the caret, which ended at `at`.
    DeleteBackward {
        at: usize,
    },
    /// One character removed after the caret at `at`.
    DeleteForward {
        at: usize,
    },
    /// An IME pre-edit or commit replacing the composition that started at `at`.
    Compose {
        at: usize,
    },
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    ops: Vec<TextOp>,
    before: Meta,
    after: Meta,
    sel_before: Vec<Selection>,
    sel_after: Vec<Selection>,
    kind: StepKind,
}

#[derive(Clone, Debug, Default)]
pub struct History {
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// Set by anything that ends a run of typing: a caret move, a different edit, a focus change.
    sealed: bool,
}

fn continues(prev: StepKind, next: StepKind, next_ops: &[TextOp]) -> bool {
    let [op] = next_ops else { return false };
    match (prev, next) {
        (StepKind::Typing { end }, StepKind::Typing { .. }) => {
            op.removed.is_empty() && op.at == end
        }
        (StepKind::DeleteBackward { at }, StepKind::DeleteBackward { .. }) => {
            op.inserted.is_empty() && op.at + utf16_len(&op.removed) == at
        }
        (StepKind::DeleteForward { at }, StepKind::DeleteForward { .. }) => {
            op.inserted.is_empty() && op.at == at
        }
        (StepKind::Compose { at }, StepKind::Compose { .. }) => op.at == at,
        _ => false,
    }
}

impl History {
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn seal(&mut self) {
        self.sealed = true;
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.sealed = true;
    }

    pub fn record(
        &mut self,
        applied: &Applied,
        sel_before: &[Selection],
        sel_after: &[Selection],
        kind: StepKind,
    ) {
        if !applied.doc_changed {
            return;
        }
        self.redo.clear();
        if !self.sealed {
            if let Some(last) = self.undo.last_mut() {
                if continues(last.kind, kind, &applied.ops) {
                    last.ops.extend(applied.ops.iter().cloned());
                    last.after = applied.after.clone();
                    last.sel_after = sel_after.to_vec();
                    last.kind = kind;
                    return;
                }
            }
        }
        self.sealed = false;
        self.undo.push(Step {
            ops: applied.ops.clone(),
            before: applied.before.clone(),
            after: applied.after.clone(),
            sel_before: sel_before.to_vec(),
            sel_after: sel_after.to_vec(),
            kind,
        });
        if self.undo.len() > LIMIT {
            self.undo.remove(0);
        }
    }

    /// Reverts the last step; answers the selections to restore and the text changes it made.
    pub fn undo<B: TextBuffer>(&mut self, doc: &mut Document<B>) -> Option<Restored> {
        let step = self.undo.pop()?;
        let len = doc.len();
        let inverse = step
            .ops
            .iter()
            .rev()
            .map(|op| (op.at, utf16_len(&op.inserted), op.removed.as_str()));
        let changes = replay(doc, len, inverse);
        doc.restore_meta(&step.before);
        let selections = step.sel_before.clone();
        self.redo.push(step);
        self.sealed = true;
        Some(Restored {
            selections,
            changes,
        })
    }

    pub fn redo<B: TextBuffer>(&mut self, doc: &mut Document<B>) -> Option<Restored> {
        let step = self.redo.pop()?;
        let len = doc.len();
        let ops = step
            .ops
            .iter()
            .map(|op| (op.at, utf16_len(&op.removed), op.inserted.as_str()));
        let changes = replay(doc, len, ops);
        doc.restore_meta(&step.after);
        let selections = step.sel_after.clone();
        self.undo.push(step);
        self.sealed = true;
        Some(Restored {
            selections,
            changes,
        })
    }
}

/// What an undo or redo did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Restored {
    pub selections: Vec<Selection>,
    pub changes: Vec<ChangeDesc>,
}

fn replay<'a, B: TextBuffer>(
    doc: &mut Document<B>,
    len: usize,
    ops: impl Iterator<Item = (usize, usize, &'a str)> + Clone,
) -> Vec<ChangeDesc> {
    for (at, removed, inserted) in ops.clone() {
        doc.replace_text_raw(at, at + removed, inserted);
    }
    compose(len, ops)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transaction::{ChangeSpec, Insert, Transaction};
    use crate::value::{Atom, EditorValue, Mark};

    fn type_at(d: &mut Document, h: &mut History, at: usize, text: &str) {
        let tr = Transaction {
            changes: vec![ChangeSpec {
                from: at,
                to: None,
                insert: Insert::Text(text.into()),
            }],
            ..Default::default()
        };
        let applied = d.apply(&tr).unwrap();
        let end = at + utf16_len(text);
        h.record(
            &applied,
            &[Selection::caret(at)],
            &[Selection::caret(end)],
            StepKind::Typing { end },
        );
    }

    #[test]
    fn typing_in_a_row_is_one_step_and_a_caret_move_seals_it() {
        let (mut d, mut h) = (Document::new(), History::default());
        for (i, c) in "abc".chars().enumerate() {
            type_at(&mut d, &mut h, i, &c.to_string());
        }
        h.seal();
        type_at(&mut d, &mut h, 3, "d");
        assert_eq!(d.text(), "abcd");
        assert_eq!(
            h.undo(&mut d).map(|r| r.selections),
            Some(vec![Selection::caret(3)])
        );
        assert_eq!(d.text(), "abc");
        assert_eq!(
            h.undo(&mut d).map(|r| r.selections),
            Some(vec![Selection::caret(0)])
        );
        assert_eq!(d.text(), "");
        assert!(h.undo(&mut d).is_none());
        assert_eq!(
            h.redo(&mut d).map(|r| r.selections),
            Some(vec![Selection::caret(3)])
        );
        assert_eq!(d.text(), "abc");
        assert_eq!(
            h.redo(&mut d).map(|r| r.selections),
            Some(vec![Selection::caret(4)])
        );
        assert_eq!(d.text(), "abcd");
    }

    #[test]
    fn typing_elsewhere_starts_a_new_step() {
        let (mut d, mut h) = (Document::new(), History::default());
        type_at(&mut d, &mut h, 0, "ab");
        type_at(&mut d, &mut h, 0, "X");
        h.undo(&mut d);
        assert_eq!(d.text(), "ab");
    }

    #[test]
    fn undo_restores_marks_atoms_and_text_exactly() {
        let mut d = Document::from_value(EditorValue {
            text: "hi \u{FFFC} there".into(),
            marks: vec![Mark::new("bold", 0, 2)],
            atoms: vec![Atom {
                id: "a".into(),
                at: 3,
                kind: "mention".into(),
                text: Some("@a".into()),
                data: None,
            }],
            ..Default::default()
        });
        let start = d.to_value();
        let mut h = History::default();
        let tr = Transaction {
            changes: vec![ChangeSpec {
                from: 1,
                to: Some(5),
                insert: Insert::Text("ey".into()),
            }],
            add_marks: vec![Mark::new("italic", 0, 3)],
            ..Default::default()
        };
        let applied = d.apply(&tr).unwrap();
        h.record(
            &applied,
            &[Selection::caret(1)],
            &[Selection::caret(3)],
            StepKind::Other,
        );
        assert!(d.atoms().is_empty());
        let edited = d.to_value();
        h.undo(&mut d);
        assert_eq!(d.to_value(), start);
        h.redo(&mut d);
        assert_eq!(d.to_value(), edited);
    }

    #[test]
    fn backspaces_in_a_row_merge() {
        let mut d = Document::from_value(EditorValue::plain("abcd"));
        let mut h = History::default();
        for at in [4usize, 3, 2] {
            let tr = Transaction {
                changes: vec![ChangeSpec {
                    from: at - 1,
                    to: Some(at),
                    insert: Insert::Text(String::new()),
                }],
                ..Default::default()
            };
            let applied = d.apply(&tr).unwrap();
            h.record(
                &applied,
                &[Selection::caret(at)],
                &[Selection::caret(at - 1)],
                StepKind::DeleteBackward { at: at - 1 },
            );
        }
        assert_eq!(d.text(), "a");
        assert_eq!(
            h.undo(&mut d).map(|r| r.selections),
            Some(vec![Selection::caret(4)])
        );
        assert_eq!(d.text(), "abcd");
    }
}
