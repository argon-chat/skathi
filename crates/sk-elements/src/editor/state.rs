//! The per-node editor entity: focus, keyboard and mouse editing, IME, clipboard, caret blink and
//! the events they raise. The document itself is in [`EditorShared`], so JS reads and dispatches
//! go straight there even while this entity is being updated.

use std::ops::Range;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    px, App, Bounds, ClipboardItem, Context, EntityInputHandler, FocusHandle, Focusable,
    KeystrokeEvent, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Subscription,
    Task, TextInputConfiguration, UTF16Selection, Window,
};
use sk_editor::{Editor, EditorValue, Insert, Outcome, Selection, StepKind};

use super::{EditorEvent, EditorHandlers, EditorIntent, EditorShared, LineBreak, Paragraph};
use crate::text_input::boundaries;
use crate::text_input::{
    Backspace, ClearSelection, Copy, Cut, Delete, DeleteToEnd, DeleteToStart, DeleteWordLeft,
    DeleteWordRight, DocEnd, DocStart, Down, End, Home, Left, PageDown, PageUp, Paste, Redo, Right,
    SelectAll, SelectDocEnd, SelectDocStart, SelectDown, SelectEnd, SelectHome, SelectLeft,
    SelectPageDown, SelectPageUp, SelectRight, SelectUp, SelectWordLeft, SelectWordRight,
    ShowCharacterPalette, Undo, Up, WordLeft, WordRight,
};
use crate::{CompositionPhase, InputType};

const CARET_BLINK_MS: u64 = 500;

/// UTF-8 byte offset of a UTF-16 position in `text`.
fn to8(text: &str, pos: usize) -> usize {
    let mut units = 0;
    for (at, c) in text.char_indices() {
        if units >= pos {
            return at;
        }
        units += c.len_utf16();
    }
    text.len()
}

fn to16(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())]
        .chars()
        .map(char::len_utf16)
        .sum()
}

pub struct EditorState {
    focus_handle: FocusHandle,
    shared: Rc<EditorShared>,
    /// The IME pre-edit, UTF-16, and the document revision it was made in.
    marked: Option<(Range<usize>, u64)>,
    composition_open: bool,
    read_only: bool,
    handlers: EditorHandlers,
    input_config: TextInputConfiguration,
    /// A cancelled `keydown` that would have typed a character: refuse the text it still sends.
    suppress_text_input: bool,
    is_selecting: bool,
    /// Where a drag started, kept so a drag back over it keeps the start selected.
    drag_anchor: usize,
    /// The x a run of vertical moves keeps to, for the caret and revision it left.
    goal_x: Option<(usize, u64, Pixels)>,
    blink_anchor: Instant,
    blink_task: Option<Task<()>>,
    _key_interceptor: Subscription,
    _blur: Subscription,
}

impl EditorState {
    pub fn new(shared: Rc<EditorShared>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let weak = cx.weak_entity();
        let interceptor = cx.intercept_keystrokes(move |event, window, cx| {
            weak.update(cx, |this, cx| {
                this.on_intercept_keystroke(event, window, cx)
            })
            .ok();
        });
        let focus_handle = cx.focus_handle();
        let blur = cx.on_blur(&focus_handle, window, |this, window, cx| {
            this.shared.core.borrow_mut().seal_history();
            this.finish_composition(window, cx);
        });
        Self {
            focus_handle,
            shared,
            marked: None,
            composition_open: false,
            read_only: false,
            handlers: EditorHandlers::default(),
            input_config: TextInputConfiguration::default(),
            suppress_text_input: false,
            is_selecting: false,
            drag_anchor: 0,
            goal_x: None,
            blink_anchor: cx.background_executor().now(),
            blink_task: None,
            _key_interceptor: interceptor,
            _blur: blur,
        }
    }

    pub fn shared(&self) -> Rc<EditorShared> {
        self.shared.clone()
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus_handle.clone()
    }

    pub(crate) fn marked_range(&self) -> Option<Range<usize>> {
        self.marked.as_ref().map(|(r, _)| r.clone())
    }

    pub fn set_tab_index(&mut self, index: Option<isize>) {
        let tab_index = index.unwrap_or(0);
        let tab_stop = index.is_some_and(|n| n >= 0);
        if self.focus_handle.tab_index == tab_index && self.focus_handle.tab_stop == tab_stop {
            return;
        }
        self.focus_handle = self
            .focus_handle
            .clone()
            .tab_index(tab_index)
            .tab_stop(tab_stop);
    }

    pub(crate) fn set_frame_params(
        &mut self,
        handlers: EditorHandlers,
        read_only: bool,
        input_config: TextInputConfiguration,
    ) {
        self.handlers = handlers;
        self.read_only = read_only;
        self.input_config = input_config;
    }

    /// Drops a composition the document moved under (a `dispatch` or a `value` assignment).
    pub(crate) fn check_composition(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let revision = self.shared.core.borrow().revision();
        if matches!(&self.marked, Some((_, r)) if *r != revision) {
            self.marked = None;
            self.composition(CompositionPhase::End, "", window, cx);
        }
    }

    fn on_intercept_keystroke(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.suppress_text_input = false;
        if !self.focus_handle.is_focused(window) {
            return;
        }
        let (Some(key_down), Some(hook)) = (&event.key_down, self.handlers.on_key_down.clone())
        else {
            return;
        };
        if hook(key_down, window, cx) {
            return;
        }
        self.suppress_text_input = event.keystroke.key_char.is_some();
        cx.stop_propagation();
    }

    fn reset_blink(&mut self, cx: &Context<Self>) {
        self.blink_anchor = cx.background_executor().now();
    }

    pub(crate) fn caret_shown(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        if self.read_only || !self.focus_handle.is_focused(window) || !window.is_window_active() {
            self.blink_task = None;
            return false;
        }
        if self.blink_task.is_none() {
            self.reset_blink(cx);
            self.blink_task = Some(cx.spawn(async move |this, cx| loop {
                cx.background_executor()
                    .timer(Duration::from_millis(CARET_BLINK_MS))
                    .await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }));
        }
        (self.blink_anchor.elapsed().as_millis() as u64 / CARET_BLINK_MS) % 2 == 0
    }

    fn emit(&self, event: EditorEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(hook) = self.handlers.on_event.clone() {
            window.defer(cx, move |window, cx| hook(event, window, cx));
        }
    }

    /// One `selectionchange` per burst of moves, delivered after the move.
    fn queue_selection_event(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hook) = self.handlers.on_event.clone() else {
            return;
        };
        if self.shared.selection_event_queued.replace(true) {
            return;
        }
        let shared = self.shared.clone();
        window.defer(cx, move |window, cx| {
            shared.selection_event_queued.set(false);
            hook(EditorEvent::SelectionChange, window, cx)
        });
    }

    fn after(
        &mut self,
        outcome: Outcome,
        input: Option<EditorIntent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reset_blink(cx);
        if let Some(intent) = input {
            if outcome.doc_changed {
                self.emit(EditorEvent::Input(intent), window, cx);
            }
        }
        if outcome.doc_changed {
            self.emit(EditorEvent::Change(outcome.changes), window, cx);
        }
        if outcome.selection_changed {
            self.queue_selection_event(window, cx);
        }
        cx.notify();
    }

    fn before_input(
        &mut self,
        intent: &EditorIntent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(hook) = self.handlers.on_before_input.clone() else {
            return true;
        };
        let proceed = hook(intent, window, cx);
        proceed || intent.input_type.composing()
    }

    fn composition(
        &mut self,
        phase: CompositionPhase,
        data: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match phase {
            CompositionPhase::Start => self.composition_open = true,
            CompositionPhase::Update => {}
            CompositionPhase::End => {
                if !std::mem::take(&mut self.composition_open) {
                    return;
                }
            }
        }
        let Some(hook) = self.handlers.on_composition.clone() else {
            return;
        };
        if phase == CompositionPhase::End {
            let data = data.to_owned();
            window.defer(cx, move |window, cx| {
                hook(CompositionPhase::End, &data, window, cx)
            });
        } else {
            hook(phase, data, window, cx);
        }
    }

    pub(crate) fn finish_composition(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((range, _)) = self.marked.take() else {
            return;
        };
        let text = self
            .shared
            .core
            .borrow()
            .doc()
            .slice(range.start, range.end);
        cx.notify();
        self.composition(CompositionPhase::End, &text, window, cx);
    }

    fn selection(&self) -> Selection {
        self.shared.core.borrow().selection()
    }

    fn text(&self) -> String {
        self.shared.core.borrow().doc().text().to_owned()
    }

    /// A user edit: `beforeinput`, then the edit on whatever `target` says once the handlers
    /// ran (they may have moved the selection or changed the text), then `input`.
    fn edit(
        &mut self,
        input_type: InputType,
        data: Option<String>,
        insert: Insert,
        kind: StepKind,
        target: impl Fn(&Editor) -> (usize, usize),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        let (from, to) = target(&self.shared.core.borrow());
        let intent = EditorIntent {
            input_type,
            data,
            from,
            to,
        };
        if !self.before_input(&intent, window, cx) {
            return;
        }
        let outcome = {
            let mut core = self.shared.core.borrow_mut();
            let (from, to) = target(&core);
            core.user_replace(from, to, insert, kind, !input_type.composing())
        };
        if let Some(outcome) = outcome {
            self.after(outcome, Some(intent), window, cx);
        }
    }

    fn delete_toward(
        &mut self,
        input_type: InputType,
        kind: StepKind,
        to: fn(&str, usize) -> usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = move |core: &Editor| {
            let s = core.selection();
            if !s.is_empty() {
                return (s.from(), s.to());
            }
            let text = core.doc().text();
            let head = to8(text, s.head);
            let other = to16(text, to(text, head));
            (other.min(s.head), other.max(s.head))
        };
        let (from, to) = target(&self.shared.core.borrow());
        if from == to {
            return;
        }
        self.edit(
            input_type,
            None,
            Insert::Text(String::new()),
            kind,
            target,
            window,
            cx,
        );
    }

    fn move_to(&mut self, pos: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.select(Selection::caret(pos), window, cx);
    }

    fn select(&mut self, s: Selection, window: &mut Window, cx: &mut Context<Self>) {
        let changed = self.shared.core.borrow_mut().set_selection(s);
        self.reset_blink(cx);
        if changed {
            self.queue_selection_event(window, cx);
        }
        cx.notify();
    }

    fn extend_to(&mut self, pos: usize, window: &mut Window, cx: &mut Context<Self>) {
        let anchor = self.selection().anchor;
        self.select(Selection { anchor, head: pos }, window, cx);
    }

    fn by_text(&self, f: fn(&str, usize) -> usize) -> usize {
        let text = self.text();
        let head = to8(&text, self.selection().head);
        to16(&text, f(&text, head))
    }

    fn row_edge(&self, forward: bool) -> usize {
        let head = self.selection().head;
        let doc_edge = {
            let core = self.shared.core.borrow();
            let line = core.doc().line_range(core.doc().line_at(head));
            if forward {
                line.end
            } else {
                line.start
            }
        };
        self.shared
            .geometry
            .borrow()
            .row_edge(head, forward)
            .unwrap_or(doc_edge)
    }

    /// Moves or extends the caret a row, or a page with `page`, keeping the column of the first
    /// move in a run. A page also scrolls by the same distance.
    fn vertical(
        &mut self,
        down: bool,
        page: bool,
        extend: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let head = self.selection().head;
        let (len, revision) = {
            let core = self.shared.core.borrow();
            (core.doc().len(), core.revision())
        };
        let goal = self
            .goal_x
            .filter(|&(h, r, _)| h == head && r == revision)
            .map(|(.., x)| x);
        let (pos, x) = {
            let mut g = self.shared.geometry.borrow_mut();
            let mut by = px(0.0);
            if page {
                let step = g.page_height();
                by = (step - g.line_height).max(px(0.0));
                let top = g.scroll_top() + if down { step } else { -step };
                g.set_scroll(Some(top), None);
            }
            g.row_step(head, down, by, goal)
        };
        let pos = pos.unwrap_or(if down { len } else { 0 });
        if extend {
            self.extend_to(pos, window, cx);
        } else {
            self.move_to(pos, window, cx);
        }
        self.goal_x = x.map(|x| (pos, revision, x));
    }

    fn doc_edge(&mut self, end: bool, extend: bool, window: &mut Window, cx: &mut Context<Self>) {
        let pos = if end {
            self.shared.core.borrow().doc().len()
        } else {
            0
        };
        if extend {
            self.extend_to(pos, window, cx);
        } else {
            self.move_to(pos, window, cx);
        }
    }

    pub(crate) fn doc_start(&mut self, _: &DocStart, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_edge(false, false, window, cx);
    }

    pub(crate) fn doc_end(&mut self, _: &DocEnd, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_edge(true, false, window, cx);
    }

    pub(crate) fn select_doc_start(
        &mut self,
        _: &SelectDocStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.doc_edge(false, true, window, cx);
    }

    pub(crate) fn select_doc_end(
        &mut self,
        _: &SelectDocEnd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.doc_edge(true, true, window, cx);
    }

    pub(crate) fn page_up(&mut self, _: &PageUp, window: &mut Window, cx: &mut Context<Self>) {
        self.vertical(false, true, false, window, cx);
    }

    pub(crate) fn page_down(&mut self, _: &PageDown, window: &mut Window, cx: &mut Context<Self>) {
        self.vertical(true, true, false, window, cx);
    }

    pub(crate) fn select_page_up(
        &mut self,
        _: &SelectPageUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.vertical(false, true, true, window, cx);
    }

    pub(crate) fn select_page_down(
        &mut self,
        _: &SelectPageDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.vertical(true, true, true, window, cx);
    }

    pub(crate) fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        let kind = StepKind::DeleteBackward { at: 0 };
        self.delete_toward(
            InputType::DeleteContentBackward,
            kind,
            boundaries::previous_grapheme,
            window,
            cx,
        );
    }

    pub(crate) fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        let kind = StepKind::DeleteForward { at: 0 };
        self.delete_toward(
            InputType::DeleteContentForward,
            kind,
            boundaries::next_grapheme,
            window,
            cx,
        );
    }

    pub(crate) fn delete_word_left(
        &mut self,
        _: &DeleteWordLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_toward(
            InputType::DeleteWordBackward,
            StepKind::Other,
            boundaries::previous_word,
            window,
            cx,
        );
    }

    pub(crate) fn delete_word_right(
        &mut self,
        _: &DeleteWordRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_toward(
            InputType::DeleteWordForward,
            StepKind::Other,
            boundaries::next_word,
            window,
            cx,
        );
    }

    pub(crate) fn delete_to_start(
        &mut self,
        _: &DeleteToStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let start = self.row_edge(false);
        let head = self.selection().head;
        let target = move |core: &Editor| {
            let s = core.selection();
            if s.is_empty() {
                (start.min(head), head)
            } else {
                (s.from(), s.to())
            }
        };
        let (from, to) = target(&self.shared.core.borrow());
        if from < to {
            self.edit(
                InputType::DeleteSoftLineBackward,
                None,
                Insert::Text(String::new()),
                StepKind::Other,
                target,
                window,
                cx,
            );
        }
    }

    pub(crate) fn delete_to_end(
        &mut self,
        _: &DeleteToEnd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let end = self.row_edge(true);
        let head = self.selection().head;
        let target = move |core: &Editor| {
            let s = core.selection();
            if s.is_empty() {
                (head, end.max(head))
            } else {
                (s.from(), s.to())
            }
        };
        let (from, to) = target(&self.shared.core.borrow());
        if from < to {
            self.edit(
                InputType::DeleteSoftLineForward,
                None,
                Insert::Text(String::new()),
                StepKind::Other,
                target,
                window,
                cx,
            );
        }
    }

    pub(crate) fn left(&mut self, _: &Left, window: &mut Window, cx: &mut Context<Self>) {
        let s = self.selection();
        let pos = if s.is_empty() {
            self.by_text(boundaries::previous_grapheme)
        } else {
            s.from()
        };
        self.move_to(pos, window, cx);
    }

    pub(crate) fn right(&mut self, _: &Right, window: &mut Window, cx: &mut Context<Self>) {
        let s = self.selection();
        let pos = if s.is_empty() {
            self.by_text(boundaries::next_grapheme)
        } else {
            s.to()
        };
        self.move_to(pos, window, cx);
    }

    pub(crate) fn select_left(
        &mut self,
        _: &SelectLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pos = self.by_text(boundaries::previous_grapheme);
        self.extend_to(pos, window, cx);
    }

    pub(crate) fn select_right(
        &mut self,
        _: &SelectRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pos = self.by_text(boundaries::next_grapheme);
        self.extend_to(pos, window, cx);
    }

    pub(crate) fn word_left(&mut self, _: &WordLeft, window: &mut Window, cx: &mut Context<Self>) {
        let pos = self.by_text(boundaries::previous_word);
        self.move_to(pos, window, cx);
    }

    pub(crate) fn word_right(
        &mut self,
        _: &WordRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pos = self.by_text(boundaries::next_word);
        self.move_to(pos, window, cx);
    }

    pub(crate) fn select_word_left(
        &mut self,
        _: &SelectWordLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pos = self.by_text(boundaries::previous_word);
        self.extend_to(pos, window, cx);
    }

    pub(crate) fn select_word_right(
        &mut self,
        _: &SelectWordRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pos = self.by_text(boundaries::next_word);
        self.extend_to(pos, window, cx);
    }

    pub(crate) fn select_all(
        &mut self,
        _: &SelectAll,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let len = self.shared.core.borrow().doc().len();
        self.select(
            Selection {
                anchor: 0,
                head: len,
            },
            window,
            cx,
        );
    }

    pub(crate) fn home(&mut self, _: &Home, window: &mut Window, cx: &mut Context<Self>) {
        let pos = self.row_edge(false);
        self.move_to(pos, window, cx);
    }

    pub(crate) fn end(&mut self, _: &End, window: &mut Window, cx: &mut Context<Self>) {
        let pos = self.row_edge(true);
        self.move_to(pos, window, cx);
    }

    pub(crate) fn select_home(
        &mut self,
        _: &SelectHome,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pos = self.row_edge(false);
        self.extend_to(pos, window, cx);
    }

    pub(crate) fn select_end(
        &mut self,
        _: &SelectEnd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pos = self.row_edge(true);
        self.extend_to(pos, window, cx);
    }

    pub(crate) fn up(&mut self, _: &Up, window: &mut Window, cx: &mut Context<Self>) {
        self.vertical(false, false, false, window, cx);
    }

    pub(crate) fn down(&mut self, _: &Down, window: &mut Window, cx: &mut Context<Self>) {
        self.vertical(true, false, false, window, cx);
    }

    pub(crate) fn select_up(&mut self, _: &SelectUp, window: &mut Window, cx: &mut Context<Self>) {
        self.vertical(false, false, true, window, cx);
    }

    pub(crate) fn select_down(
        &mut self,
        _: &SelectDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.vertical(true, false, true, window, cx);
    }

    fn selection_target(core: &Editor) -> (usize, usize) {
        let s = core.selection();
        (s.from(), s.to())
    }

    pub(crate) fn paragraph(&mut self, _: &Paragraph, window: &mut Window, cx: &mut Context<Self>) {
        self.edit(
            InputType::InsertParagraph,
            None,
            Insert::Text("\n".into()),
            StepKind::Other,
            Self::selection_target,
            window,
            cx,
        );
    }

    pub(crate) fn line_break(
        &mut self,
        _: &LineBreak,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit(
            InputType::InsertLineBreak,
            None,
            Insert::Text("\n".into()),
            StepKind::Other,
            Self::selection_target,
            window,
            cx,
        );
    }

    /// The selection as plain text and as the editor's JSON; `None` when it is empty.
    fn selected_value(&self) -> Option<(String, String)> {
        let core = self.shared.core.borrow();
        let s = core.selection();
        if s.is_empty() {
            return None;
        }
        let value = core.doc().slice_value(s.from(), s.to());
        Some((value.plain_text(), sk_editor::json::to_json(&value)))
    }

    fn write_clipboard(&self, plain: &str, rich: &str, cx: &mut App) {
        match &self.handlers.write_clipboard {
            Some(write) => write(plain, rich, cx),
            None => cx.write_to_clipboard(ClipboardItem::new_string(plain.to_owned())),
        }
    }

    pub(crate) fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((plain, rich)) = self.selected_value() {
            self.write_clipboard(&plain, &rich, cx);
        }
    }

    pub(crate) fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let Some((plain, rich)) = self.selected_value() else {
            return;
        };
        self.write_clipboard(&plain, &rich, cx);
        self.edit(
            InputType::DeleteByCut,
            None,
            Insert::Text(String::new()),
            StepKind::Other,
            Self::selection_target,
            window,
            cx,
        );
    }

    pub(crate) fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let read = match &self.handlers.read_clipboard {
            Some(read) => read(cx),
            None => cx
                .read_from_clipboard()
                .and_then(|i| i.text())
                .map(|t| (t, None)),
        };
        let Some((plain, rich)) = read else {
            return;
        };
        let insert = match rich.as_deref().and_then(sk_editor::json::from_json) {
            Some(value) => Insert::Value(value),
            None => Insert::Text(plain.clone()),
        };
        self.edit(
            InputType::InsertFromPaste,
            Some(plain),
            insert,
            StepKind::Other,
            Self::selection_target,
            window,
            cx,
        );
    }

    fn history(&mut self, undo: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let possible = {
            let core = self.shared.core.borrow();
            if undo {
                core.can_undo()
            } else {
                core.can_redo()
            }
        };
        if !possible {
            return;
        }
        let input_type = if undo {
            InputType::HistoryUndo
        } else {
            InputType::HistoryRedo
        };
        let s = self.selection();
        let intent = EditorIntent {
            input_type,
            data: None,
            from: s.from(),
            to: s.to(),
        };
        if !self.before_input(&intent, window, cx) {
            return;
        }
        self.finish_composition(window, cx);
        let outcome = {
            let mut core = self.shared.core.borrow_mut();
            if undo {
                core.undo()
            } else {
                core.redo()
            }
        };
        if let Some(outcome) = outcome {
            self.after(outcome, Some(intent), window, cx);
        }
    }

    pub(crate) fn undo(&mut self, _: &Undo, window: &mut Window, cx: &mut Context<Self>) {
        self.history(true, window, cx);
    }

    pub(crate) fn redo(&mut self, _: &Redo, window: &mut Window, cx: &mut Context<Self>) {
        self.history(false, window, cx);
    }

    pub(crate) fn clear_selection(
        &mut self,
        _: &ClearSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let s = self.selection();
        if s.is_empty() {
            cx.propagate();
            return;
        }
        self.move_to(s.head, window, cx);
    }

    pub(crate) fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    fn index_for_window(&self, p: Point<Pixels>) -> Option<usize> {
        let geometry = self.shared.geometry.borrow();
        let local = geometry.untransform(p)?;
        Some(geometry.index_at(geometry.from_window(local)))
    }

    pub(crate) fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        let Some(pos) = self.index_for_window(event.position) else {
            self.is_selecting = false;
            return;
        };
        match event.click_count {
            0 | 1 => {
                self.is_selecting = true;
                if event.modifiers.shift {
                    self.extend_to(pos, window, cx);
                } else {
                    self.drag_anchor = pos;
                    self.move_to(pos, window, cx);
                }
            }
            2 => {
                let text = self.text();
                let r = boundaries::word_range_at(&text, to8(&text, pos));
                let (a, b) = (to16(&text, r.start), to16(&text, r.end));
                self.select(Selection { anchor: a, head: b }, window, cx);
            }
            _ => {
                let (a, b) = {
                    let core = self.shared.core.borrow();
                    let r = core.doc().line_range(core.doc().line_at(pos));
                    (r.start, r.end)
                };
                self.select(Selection { anchor: a, head: b }, window, cx);
            }
        }
    }

    pub(crate) fn on_secondary_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        let Some(pos) = self.index_for_window(event.position) else {
            return;
        };
        let s = self.selection();
        if s.is_empty() || pos < s.from() || pos > s.to() {
            self.move_to(pos, window, cx);
        }
    }

    pub(crate) fn on_mouse_drag(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_selecting {
            return;
        }
        let Some(pos) = self.index_for_window(event.position) else {
            self.is_selecting = false;
            return;
        };
        let anchor = self.drag_anchor;
        self.select(Selection { anchor, head: pos }, window, cx);
    }

    pub(crate) fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    pub fn scroll_by(&mut self, dx: Pixels, dy: Pixels, _: &mut Window, cx: &mut Context<Self>) {
        let mut g = self.shared.geometry.borrow_mut();
        let top = (g.scroll_top + dy).clamp(px(0.0), g.max_scroll_top());
        let left = (g.scroll_left + dx).clamp(px(0.0), g.max_scroll_left());
        if top != g.scroll_top || left != g.scroll_left {
            g.scroll_top = top;
            g.scroll_left = left;
            cx.notify();
        }
    }

    /// Replaces the whole document from outside (the `value` property); `false` if it was equal.
    pub fn set_value(
        &mut self,
        value: EditorValue,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let changed = self.shared.core.borrow_mut().set_value(value);
        if changed {
            self.check_composition(window, cx);
            cx.notify();
        }
        changed
    }

    pub(crate) fn emit_viewport_change(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.emit(EditorEvent::ViewportChange, window, cx);
    }

    fn range16(&self, range: &Range<usize>) -> Range<usize> {
        let len = self.shared.core.borrow().doc().len();
        range.start.min(len)..range.end.clamp(range.start.min(len), len)
    }
}

impl Focusable for EditorState {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EntityInputHandler for EditorState {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range16(&range);
        actual.replace(range.clone());
        Some(
            self.shared
                .core
                .borrow()
                .doc()
                .slice(range.start, range.end),
        )
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let s = self.selection();
        Some(UTF16Selection {
            range: s.from()..s.to(),
            reversed: s.head < s.anchor,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range()
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_composition(window, cx);
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        if std::mem::take(&mut self.suppress_text_input) && range.is_none() {
            return;
        }
        self.check_composition(window, cx);
        if let Some((marked, _)) = self.marked.clone() {
            let target = range.map(|r| self.range16(&r)).unwrap_or(marked.clone());
            let intent = EditorIntent {
                input_type: InputType::InsertCompositionText,
                data: Some(text.to_owned()),
                from: target.start,
                to: target.end,
            };
            self.before_input(&intent, window, cx);
            self.composition(CompositionPhase::Update, text, window, cx);
            let outcome = self.shared.core.borrow_mut().user_replace(
                target.start,
                target.end,
                Insert::Text(text.to_owned()),
                StepKind::Compose { at: 0 },
                true,
            );
            self.marked = None;
            if let Some(outcome) = outcome {
                self.after(outcome, Some(intent), window, cx);
            }
            self.shared.core.borrow_mut().seal_history();
            self.composition(CompositionPhase::End, text, window, cx);
            cx.notify();
            return;
        }
        // A lone `\r` / `\n` is Return routed through the text path on some platforms.
        if text == "\n" || text == "\r" {
            self.paragraph(&Paragraph, window, cx);
            return;
        }
        if text.chars().all(|c| c.is_control() && c != '\t') {
            return;
        }
        let explicit = range.map(|r| self.range16(&r));
        let target = move |core: &Editor| {
            explicit.clone().map_or_else(
                || {
                    let s = core.selection();
                    (s.from(), s.to())
                },
                |r| (r.start, r.end),
            )
        };
        self.edit(
            InputType::InsertText,
            Some(text.to_owned()),
            Insert::Text(text.to_owned()),
            StepKind::Typing { end: 0 },
            target,
            window,
            cx,
        );
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        self.check_composition(window, cx);
        let target = range
            .map(|r| self.range16(&r))
            .or_else(|| self.marked_range())
            .unwrap_or_else(|| {
                let s = self.selection();
                s.from()..s.to()
            });
        if self.marked.is_none() {
            if text.is_empty() {
                return;
            }
            let replaced = self
                .shared
                .core
                .borrow()
                .doc()
                .slice(target.start, target.end);
            self.composition(CompositionPhase::Start, &replaced, window, cx);
            self.shared.core.borrow_mut().seal_history();
        }
        let intent = EditorIntent {
            input_type: InputType::InsertCompositionText,
            data: Some(text.to_owned()),
            from: target.start,
            to: target.end,
        };
        self.before_input(&intent, window, cx);
        self.composition(CompositionPhase::Update, text, window, cx);
        let outcome = self.shared.core.borrow_mut().user_replace(
            target.start,
            target.end,
            Insert::Text(text.to_owned()),
            StepKind::Compose { at: 0 },
            false,
        );
        let inserted = target.start..target.start + sk_editor::utf16_len(text);
        let revision = self.shared.core.borrow().revision();
        self.marked = (!inserted.is_empty()).then(|| (inserted.clone(), revision));
        if let Some(sel) = selected {
            let s = Selection {
                anchor: inserted.start + sel.start,
                head: inserted.start + sel.end,
            };
            self.shared.core.borrow_mut().place_selection(s);
        }
        if let Some(outcome) = outcome {
            self.after(outcome, Some(intent), window, cx);
        }
        if self.marked.is_none() {
            self.composition(CompositionPhase::End, "", window, cx);
        }
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let g = self.shared.geometry.borrow();
        let (start, _, _) = g.caret_at(range.start)?;
        let (end, height, _) = g.caret_at(range.end)?;
        let (x0, x1) = if start.y == end.y {
            (start.x, end.x.max(start.x + px(2.0)))
        } else {
            (end.x, end.x + px(2.0))
        };
        let a = g.to_window(gpui::point(x0, end.y));
        let b = g.to_window(gpui::point(x1, end.y + height));
        g.transform_bounds(Bounds::from_corners(a, b))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        self.index_for_window(point)
    }

    fn set_selected_text_range(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let r = self.range16(&range);
        self.select(
            Selection {
                anchor: r.start,
                head: r.end,
            },
            window,
            cx,
        );
    }

    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.shared.core.borrow().doc().len())
    }

    fn accepts_text_input(&self, _: &mut Window, _: &mut Context<Self>) -> bool {
        !self.read_only
    }

    fn text_input_configuration(
        &mut self,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> TextInputConfiguration {
        self.input_config.clone()
    }
}
