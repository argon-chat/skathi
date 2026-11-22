//! An editor's whole state apart from layout: the document, its history, the selections, the
//! marks the next typed character gets, and the app's decorations. The element and the JS API both
//! drive it, so every user edit and every `dispatch` lands here.

use std::collections::BTreeMap;

use crate::buffer::utf16_len;
use crate::doc::Document;
use crate::history::{History, StepKind};
use crate::transaction::{
    map_pos, Applied, ChangeDesc, ChangeSpec, Insert, Selection, Transaction, TxError,
};
use crate::value::{EditorValue, Mark};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoration {
    pub from: usize,
    pub to: usize,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineDecoration {
    pub line: usize,
    pub name: String,
    pub gutter: Option<String>,
}

/// What a change to the editor did, for the events it fires.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub changes: Vec<ChangeDesc>,
    pub doc_changed: bool,
    pub selection_changed: bool,
    pub scroll_into_view: bool,
}

#[derive(Debug, Default)]
pub struct Editor {
    doc: Document,
    history: History,
    selections: Vec<Selection>,
    /// `None`: the next character takes the marks around the caret ([`Document::marks_at`]).
    typing_marks: Option<Vec<String>>,
    exclusive_marks: Vec<String>,
    max_length: Option<usize>,
    decorations: BTreeMap<String, Vec<Decoration>>,
    line_decorations: BTreeMap<String, Vec<LineDecoration>>,
    /// Bumped on every document change; keys layout caches.
    revision: u64,
    decoration_revision: u64,
}

fn truncate16(s: &mut String, room: usize) {
    let mut units = 0;
    for (at, c) in s.char_indices() {
        units += c.len_utf16();
        if units > room {
            s.truncate(at);
            return;
        }
    }
}

impl Editor {
    pub fn new() -> Self {
        Self {
            doc: Document::new(),
            selections: vec![Selection::default()],
            ..Default::default()
        }
    }

    pub fn doc(&self) -> &Document {
        &self.doc
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn decoration_revision(&self) -> u64 {
        self.decoration_revision
    }

    pub fn selection(&self) -> Selection {
        self.selections[0]
    }

    pub fn selections(&self) -> &[Selection] {
        &self.selections
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    pub fn set_max_length(&mut self, max: Option<usize>) {
        self.max_length = max;
    }

    pub fn max_length(&self) -> Option<usize> {
        self.max_length
    }

    pub fn set_exclusive_marks(&mut self, kinds: Vec<String>) {
        self.exclusive_marks = kinds;
    }

    pub fn exclusive_marks(&self) -> &[String] {
        &self.exclusive_marks
    }

    /// Ends the current run of typing for undo: a later edit is a step of its own.
    pub fn seal_history(&mut self) {
        self.history.seal();
    }

    fn clamp(&self, s: Selection) -> Selection {
        let len = self.doc.len();
        Selection {
            anchor: self.doc.snap(s.anchor.min(len)),
            head: self.doc.snap(s.head.min(len)),
        }
    }

    /// A caret move or selection by the user or the app. Resets the typing marks.
    pub fn set_selection(&mut self, s: Selection) -> bool {
        let s = self.clamp(s);
        self.typing_marks = None;
        self.history.seal();
        if self.selections == [s] {
            return false;
        }
        self.selections = vec![s];
        true
    }

    /// Moves the selection without ending a run of edits: an IME's caret inside its pre-edit.
    pub fn place_selection(&mut self, s: Selection) {
        self.selections = vec![self.clamp(s)];
    }

    pub fn typing_marks(&self) -> Vec<String> {
        match &self.typing_marks {
            Some(kinds) => kinds.clone(),
            None => self.marks_at(self.selection().head),
        }
    }

    pub fn set_typing_marks(&mut self, kinds: Option<Vec<String>>) {
        self.typing_marks = kinds.map(|mut k| {
            k.sort();
            k.dedup();
            k
        });
    }

    pub fn marks_at(&self, pos: usize) -> Vec<String> {
        let exclusive = |k: &str| self.exclusive_marks.iter().any(|e| e == k);
        let mut kinds: Vec<String> = self
            .doc
            .marks_at(pos, &exclusive)
            .into_iter()
            .map(|m| m.kind.clone())
            .collect();
        kinds.sort();
        kinds.dedup();
        kinds
    }

    /// The marks text replacing `[from, to)` takes: the typing marks at a caret, or those of the
    /// selection's first character, each with the data of the mark it continues.
    fn marks_for_insert(&self, from: usize, to: usize) -> Vec<Mark> {
        let source: Vec<&Mark> = if from < to {
            self.doc
                .marks()
                .iter()
                .filter(|m| m.from <= from && from < m.to)
                .collect()
        } else {
            let exclusive = |k: &str| self.exclusive_marks.iter().any(|e| e == k);
            self.doc.marks_at(from, &exclusive)
        };
        let kinds: Vec<String> = match (&self.typing_marks, from == to) {
            (Some(kinds), true) => kinds.clone(),
            _ => source.iter().map(|m| m.kind.clone()).collect(),
        };
        kinds
            .into_iter()
            .map(|kind| {
                let data = source
                    .iter()
                    .find(|m| m.kind == kind)
                    .and_then(|m| m.data.clone());
                Mark {
                    kind,
                    from: 0,
                    to: 0,
                    data,
                }
            })
            .collect()
    }

    fn bump(&mut self, applied: &Applied) {
        if applied.doc_changed {
            self.revision += 1;
            if !applied.changes.is_empty() {
                self.map_decorations(&applied.changes);
            }
        }
    }

    fn map_decorations(&mut self, changes: &[ChangeDesc]) {
        if self.decorations.is_empty() && self.line_decorations.is_empty() {
            return;
        }
        for list in self.decorations.values_mut() {
            for d in list.iter_mut() {
                d.from = map_pos(changes, d.from, true);
                d.to = map_pos(changes, d.to, false).max(d.from);
            }
            list.retain(|d| d.from < d.to);
        }
        self.decoration_revision += 1;
    }

    /// Line decorations name lines, so they are re-anchored by where their line started.
    fn map_line_decorations(&mut self, before: &Document, changes: &[ChangeDesc]) {
        for list in self.line_decorations.values_mut() {
            for d in list.iter_mut() {
                let start = before.line_range(d.line).start;
                d.line = self.doc.line_at(map_pos(changes, start, false));
            }
        }
    }

    pub fn dispatch(&mut self, tr: &Transaction) -> Result<Outcome, TxError> {
        let before_sel = self.selections.clone();
        let before_doc =
            (!self.line_decorations.is_empty() && !tr.changes.is_empty()).then(|| self.doc.clone());
        let applied = self.doc.apply(tr)?;
        if let Some(before) = &before_doc {
            self.map_line_decorations(before, &applied.changes);
        }
        self.bump(&applied);
        let selections = match &tr.selections {
            Some(list) if !list.is_empty() => list.iter().take(1).map(|s| self.clamp(*s)).collect(),
            _ => before_sel
                .iter()
                .map(|s| {
                    self.clamp(Selection {
                        anchor: map_pos(&applied.changes, s.anchor, true),
                        head: map_pos(&applied.changes, s.head, true),
                    })
                })
                .collect(),
        };
        let selection_changed = selections != before_sel;
        if selection_changed {
            self.typing_marks = None;
        }
        self.selections = selections;
        if applied.doc_changed {
            if tr.history {
                self.history.seal();
                self.history
                    .record(&applied, &before_sel, &self.selections, StepKind::Other);
                self.history.seal();
            } else if !applied.ops.is_empty() {
                // History replays text by position; an edit it did not see invalidates those.
                self.history.clear();
            }
        }
        Ok(Outcome {
            changes: applied.changes,
            doc_changed: applied.doc_changed,
            selection_changed,
            scroll_into_view: tr.scroll_into_view,
        })
    }

    /// The document assigned from outside (`value`): no history across it, caret at the end.
    pub fn set_value(&mut self, value: EditorValue) -> bool {
        if self.doc.equals_value(&value) {
            return false;
        }
        self.doc.set_value(value);
        self.revision += 1;
        self.history.clear();
        self.typing_marks = None;
        let end = self.doc.len();
        self.selections = vec![Selection::caret(end)];
        self.decorations.clear();
        self.line_decorations.clear();
        self.decoration_revision += 1;
        true
    }

    /// How many UTF-16 units `maxLength` leaves for text replacing `[from, to)`.
    pub fn room_for(&self, from: usize, to: usize) -> Option<usize> {
        let max = self.max_length?;
        let kept = self.doc.len() - (to.saturating_sub(from));
        Some(max.saturating_sub(kept))
    }

    /// A user edit replacing `[from, to)` (the selection, a deletion's reach, an IME range).
    /// Plain text takes the typing marks; `maxLength` caps what goes in. `None` if nothing changed.
    pub fn user_replace(
        &mut self,
        from: usize,
        to: usize,
        insert: Insert,
        kind: StepKind,
        limited: bool,
    ) -> Option<Outcome> {
        let len = self.doc.len();
        let from = self.doc.snap(from.min(len));
        let to = self.doc.snap(to.clamp(from, len));
        let room = self.room_for(from, to).filter(|_| limited);
        let (insert, typed) = match insert {
            Insert::Text(mut t) => {
                if let Some(room) = room {
                    truncate16(&mut t, room);
                }
                (Insert::Text(t), true)
            }
            Insert::Value(mut v) => {
                if let Some(room) = room {
                    truncate16(&mut v.text, room);
                }
                (Insert::Value(v), false)
            }
        };
        let empty = match &insert {
            Insert::Text(t) => t.is_empty(),
            Insert::Value(v) => v.text.is_empty(),
        };
        if empty && from == to {
            return None;
        }
        let marks = if typed {
            self.marks_for_insert(from, to)
        } else {
            Vec::new()
        };
        let before_sel = self.selections.clone();
        let applied = self
            .doc
            .apply(&Transaction {
                changes: vec![ChangeSpec {
                    from,
                    to: Some(to),
                    insert,
                }],
                ..Default::default()
            })
            .ok()?;
        let inserted = applied.ops.first().map_or(0, |op| utf16_len(&op.inserted));
        let end = from + inserted;
        let mut applied = applied;
        if inserted > 0 && typed {
            let had: Vec<String> = self
                .doc
                .marks()
                .iter()
                .filter(|m| m.from < end && m.to > from)
                .map(|m| m.kind.clone())
                .collect();
            for kind in had {
                if !marks.iter().any(|m| m.kind == kind) {
                    self.doc.remove_marks(Some(&kind), from, end);
                }
            }
            for m in marks {
                self.doc.add_mark(Mark { from, to: end, ..m });
            }
            applied.after = self.doc.meta();
        }
        self.bump(&applied);
        self.selections = vec![Selection::caret(end)];
        let kind = match kind {
            StepKind::Typing { .. } => StepKind::Typing { end },
            StepKind::DeleteBackward { .. } => StepKind::DeleteBackward { at: from },
            StepKind::DeleteForward { .. } => StepKind::DeleteForward { at: from },
            StepKind::Compose { .. } => StepKind::Compose { at: from },
            other => other,
        };
        self.history
            .record(&applied, &before_sel, &self.selections, kind);
        Some(Outcome {
            changes: applied.changes,
            doc_changed: applied.doc_changed,
            selection_changed: self.selections != before_sel,
            scroll_into_view: true,
        })
    }

    pub fn undo(&mut self) -> Option<Outcome> {
        self.restore(true)
    }

    pub fn redo(&mut self) -> Option<Outcome> {
        self.restore(false)
    }

    fn restore(&mut self, undo: bool) -> Option<Outcome> {
        let before_doc = (!self.line_decorations.is_empty()).then(|| self.doc.clone());
        let restored = if undo {
            self.history.undo(&mut self.doc)?
        } else {
            self.history.redo(&mut self.doc)?
        };
        let changes = restored.changes;
        if let Some(before) = &before_doc {
            self.map_line_decorations(before, &changes);
        }
        self.revision += 1;
        if !changes.is_empty() {
            self.map_decorations(&changes);
        }
        self.decoration_revision += 1;
        self.typing_marks = None;
        let before_sel = std::mem::take(&mut self.selections);
        self.selections = restored
            .selections
            .into_iter()
            .take(1)
            .map(|s| self.clamp(s))
            .collect();
        if self.selections.is_empty() {
            self.selections = vec![Selection::caret(self.doc.len())];
        }
        Some(Outcome {
            changes,
            doc_changed: true,
            selection_changed: self.selections != before_sel,
            scroll_into_view: true,
        })
    }

    pub fn set_decorations(&mut self, layer: &str, mut list: Vec<Decoration>) {
        let len = self.doc.len();
        list.retain_mut(|d| {
            d.from = d.from.min(len);
            d.to = d.to.min(len);
            d.from < d.to && !d.name.is_empty()
        });
        list.sort_by_key(|d| (d.from, d.to));
        if list.is_empty() {
            self.decorations.remove(layer);
        } else {
            self.decorations.insert(layer.to_owned(), list);
        }
        self.decoration_revision += 1;
    }

    pub fn set_line_decorations(&mut self, layer: &str, mut list: Vec<LineDecoration>) {
        let lines = self.doc.line_count();
        list.retain(|d| d.line < lines && !d.name.is_empty());
        if list.is_empty() {
            self.line_decorations.remove(layer);
        } else {
            self.line_decorations.insert(layer.to_owned(), list);
        }
        self.decoration_revision += 1;
    }

    /// Every layer's decorations, layers in name order.
    pub fn decorations(&self) -> impl Iterator<Item = &Decoration> {
        self.decorations.values().flatten()
    }

    pub fn line_decorations(&self) -> impl Iterator<Item = &LineDecoration> {
        self.line_decorations.values().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(e: &mut Editor, text: &str) -> Outcome {
        let s = e.selection();
        e.user_replace(
            s.from(),
            s.to(),
            Insert::Text(text.into()),
            StepKind::Typing { end: 0 },
            true,
        )
        .expect("an edit")
    }

    fn kinds_over(e: &Editor, from: usize, to: usize) -> Vec<&str> {
        e.doc()
            .marks()
            .iter()
            .filter(|m| m.from <= from && m.to >= to)
            .map(|m| m.kind.as_str())
            .collect()
    }

    #[test]
    fn typing_continues_the_marks_before_the_caret_unless_told_otherwise() {
        let mut e = Editor::new();
        e.set_value(EditorValue {
            text: "ab".into(),
            marks: vec![Mark::new("bold", 0, 2)],
            ..Default::default()
        });
        typed(&mut e, "c");
        assert_eq!(
            kinds_over(&e, 0, 3),
            ["bold"],
            "a mark grows when typing at its end"
        );
        e.set_exclusive_marks(vec!["bold".into()]);
        typed(&mut e, "d");
        assert_eq!(
            kinds_over(&e, 3, 4),
            Vec::<&str>::new(),
            "an exclusive one does not"
        );
        e.set_typing_marks(Some(vec!["italic".into()]));
        typed(&mut e, "e");
        assert_eq!(kinds_over(&e, 4, 5), ["italic"]);
        assert_eq!(e.typing_marks(), ["italic"], "typing keeps them");
        e.set_selection(Selection::caret(1));
        assert_eq!(
            e.typing_marks(),
            ["bold"],
            "a caret move resets them to the marks there"
        );
    }

    #[test]
    fn typing_over_a_selection_takes_its_first_characters_marks() {
        let mut e = Editor::new();
        e.set_value(EditorValue {
            text: "xyz".into(),
            marks: vec![Mark {
                data: Some("\"u\"".into()),
                ..Mark::new("link", 1, 3)
            }],
            ..Default::default()
        });
        e.set_selection(Selection { anchor: 1, head: 3 });
        typed(&mut e, "Q");
        assert_eq!(e.doc().text(), "xQ");
        let link = &e.doc().marks()[0];
        assert_eq!(
            (link.from, link.to, link.data.as_deref()),
            (1, 2, Some("\"u\""))
        );
    }

    #[test]
    fn max_length_caps_user_edits_but_not_dispatch() {
        let mut e = Editor::new();
        e.set_max_length(Some(4));
        typed(&mut e, "ab🙂z");
        assert_eq!(e.doc().text(), "ab🙂", "a pair is not split");
        assert!(e
            .user_replace(
                4,
                4,
                Insert::Text("q".into()),
                StepKind::Typing { end: 4 },
                true
            )
            .is_none());
        e.dispatch(&Transaction {
            changes: vec![ChangeSpec {
                from: 4,
                to: None,
                insert: Insert::Text("long".into()),
            }],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(e.doc().text(), "ab🙂long");
    }

    #[test]
    fn a_composition_is_one_undo_step() {
        let mut e = Editor::new();
        typed(&mut e, "a ");
        e.seal_history();
        let mut at = (2, 2);
        for pre in ["k", "ka", "かな"] {
            e.user_replace(
                at.0,
                at.1,
                Insert::Text(pre.into()),
                StepKind::Compose { at: 0 },
                false,
            );
            at = (2, 2 + utf16_len(pre));
        }
        assert_eq!(e.doc().text(), "a かな");
        e.undo();
        assert_eq!(e.doc().text(), "a ");
    }

    #[test]
    fn dispatch_maps_the_selection_and_decorations_unless_told() {
        let mut e = Editor::new();
        e.set_value(EditorValue::plain("hello"));
        e.set_selection(Selection::caret(3));
        e.set_decorations(
            "search",
            vec![Decoration {
                from: 3,
                to: 5,
                name: "hit".into(),
            }],
        );
        let out = e
            .dispatch(&Transaction {
                changes: vec![ChangeSpec {
                    from: 0,
                    to: None,
                    insert: Insert::Text(">>".into()),
                }],
                ..Default::default()
            })
            .unwrap();
        assert!(out.doc_changed && out.selection_changed);
        assert_eq!(e.selection(), Selection::caret(5));
        let d: Vec<_> = e.decorations().map(|d| (d.from, d.to)).collect();
        assert_eq!(d, [(5, 7)]);
        assert!(e.undo().is_some());
        assert_eq!(e.doc().text(), "hello");
        assert_eq!(e.selection(), Selection::caret(3));
    }

    fn apply_changes(text: &str, changes: &[ChangeDesc]) -> String {
        let mut units: Vec<u16> = text.encode_utf16().collect();
        for c in changes.iter().rev() {
            units.splice(c.from..c.to, c.insert.encode_utf16());
        }
        String::from_utf16(&units).expect("changes keep surrogate pairs whole")
    }

    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % n.max(1) as u64) as usize
        }
    }

    fn atom_value(id: usize) -> Insert {
        Insert::Value(EditorValue {
            text: "\u{FFFC}".into(),
            atoms: vec![crate::value::Atom {
                id: format!("a{id}"),
                at: 0,
                kind: "mention".into(),
                text: Some("@x".into()),
                data: None,
            }],
            ..Default::default()
        })
    }

    #[test]
    fn the_changes_of_every_edit_undo_and_redo_turn_the_old_text_into_the_new() {
        let pieces = ["a", "bc", "🙂", "\n", "xyz ", ""];
        for seed in 1..40u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let mut e = Editor::new();
            e.set_value(EditorValue {
                text: "one \u{FFFC} two\nthree".into(),
                marks: vec![Mark::new("bold", 0, 3)],
                atoms: vec![crate::value::Atom {
                    id: "start".into(),
                    at: 4,
                    kind: "mention".into(),
                    text: None,
                    data: None,
                }],
                ..Default::default()
            });
            let start = e.doc().to_value();
            let mut next_atom = 0;
            for _ in 0..60 {
                let before = e.doc().text().to_owned();
                let len = e.doc().len();
                let outcome = match rng.below(7) {
                    0 | 1 => {
                        let at = rng.below(len + 1);
                        let end = at + rng.below(3);
                        let text = pieces[rng.below(pieces.len())];
                        e.user_replace(
                            at,
                            end,
                            Insert::Text(text.into()),
                            StepKind::Typing { end: 0 },
                            true,
                        )
                    }
                    2 => {
                        let at = rng.below(len + 1);
                        next_atom += 1;
                        e.user_replace(at, at, atom_value(next_atom), StepKind::Other, true)
                    }
                    3 => {
                        let mut cuts: Vec<usize> = (0..4).map(|_| rng.below(len + 1)).collect();
                        cuts.sort_unstable();
                        let mut changes = Vec::new();
                        for pair in cuts.chunks(2) {
                            next_atom += 1;
                            let insert = if rng.below(3) == 0 {
                                atom_value(next_atom)
                            } else {
                                Insert::Text(pieces[rng.below(pieces.len())].into())
                            };
                            changes.push(ChangeSpec {
                                from: e.doc().snap(pair[0]),
                                to: Some(e.doc().snap(pair[1])),
                                insert,
                            });
                        }
                        changes.reverse();
                        e.dispatch(&Transaction {
                            changes,
                            ..Default::default()
                        })
                        .ok()
                    }
                    4 => e.undo(),
                    5 => e.redo(),
                    _ => {
                        e.seal_history();
                        None
                    }
                };
                if let Some(out) = outcome {
                    assert_eq!(
                        apply_changes(&before, &out.changes),
                        e.doc().text(),
                        "seed {seed}"
                    );
                    let mut sorted = true;
                    for w in out.changes.windows(2) {
                        sorted &= w[0].to <= w[1].from;
                    }
                    assert!(sorted, "seed {seed}: {:?}", out.changes);
                }
                assert!(e.doc().atoms_consistent(), "seed {seed}");
            }
            let end = e.doc().to_value();
            let mut undone = 0;
            while e.undo().is_some() {
                undone += 1;
            }
            assert_eq!(
                e.doc().to_value(),
                start,
                "seed {seed}: undo all the way back"
            );
            for _ in 0..undone {
                e.redo();
            }
            assert_eq!(
                e.doc().to_value(),
                end,
                "seed {seed}: redo all the way forward"
            );
        }
    }

    #[test]
    fn undo_reports_the_inverse_of_a_multi_range_transaction() {
        let mut e = Editor::new();
        e.set_value(EditorValue::plain("hello world"));
        let tr = Transaction {
            changes: vec![
                ChangeSpec {
                    from: 6,
                    to: Some(11),
                    insert: Insert::Text("there".into()),
                },
                ChangeSpec {
                    from: 0,
                    to: Some(5),
                    insert: Insert::Text("Hi".into()),
                },
            ],
            ..Default::default()
        };
        e.dispatch(&tr).unwrap();
        let undo = e.undo().unwrap();
        assert_eq!(e.doc().text(), "hello world");
        let ch = |from, to, insert: &str| ChangeDesc {
            from,
            to,
            insert: insert.into(),
        };
        assert_eq!(undo.changes, [ch(0, 2, "hello"), ch(3, 8, "world")]);
        let redo = e.redo().unwrap();
        assert_eq!(redo.changes, [ch(0, 5, "Hi"), ch(6, 11, "there")]);
    }

    #[test]
    fn an_edit_outside_history_drops_the_history_and_an_equal_value_is_no_change() {
        let mut e = Editor::new();
        typed(&mut e, "abc");
        e.dispatch(&Transaction {
            changes: vec![ChangeSpec {
                from: 0,
                to: None,
                insert: Insert::Text("x".into()),
            }],
            history: false,
            ..Default::default()
        })
        .unwrap();
        assert!(!e.can_undo());
        let same = e.doc().to_value();
        let rev = e.revision();
        assert!(!e.set_value(same));
        assert_eq!(e.revision(), rev);
        assert!(e.set_value(EditorValue::plain("new")));
        assert_eq!(e.selection(), Selection::caret(3));
    }
}
