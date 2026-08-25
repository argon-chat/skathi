//! The persistent editor state behind a single-line text input: content,
//! selection, IME composition, undo/redo, caret blink and the controlled-value
//! echo ring.
//!
//! Adapted from gpui-vue (`crates/gpui-vue-core/src/custom_elements/input.rs`,
//! Apache-2.0), which is itself derived from the native renderer in
//! [remorses/gpuix](https://github.com/remorses/gpuix) @
//! `b8bbff4ef0dc00dc24c3d3f3ca47818e1be15c69` (Apache-2.0) and from
//! [zeronsh/comet](https://github.com/zeronsh/comet) (MIT) — the caret blink is
//! ported from Comet's `crates/ui/src/composer.rs`.
//! See gpui-vue's `THIRD_PARTY_NOTICES.md`.
//!
//! Where gpui-vue was written against an older gpui, the API usage follows
//! gpui's own `crates/gpui/examples/input.rs` (Apache-2.0, zed-industries/zed).

use std::collections::VecDeque;
use std::ops::Range;
use std::time::{Duration, Instant};

use gpui::{
    App, Bounds, ClipboardItem, Context, EntityInputHandler, FocusHandle, Focusable, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, ShapedLine, Task, UTF16Selection, Window, px,
};

use super::boundaries;
use super::{
    Backspace, ClearSelection, Copy, Cut, Delete, DeleteToEnd, DeleteToStart, DeleteWordLeft,
    DeleteWordRight, End, Home, Left, OnChange, Paste, Redo, Right, SelectAll, SelectEnd,
    SelectHome, SelectLeft, SelectRight, SelectWordLeft, SelectWordRight, ShowCharacterPalette,
    Undo, WordLeft, WordRight,
};

/// How many self-reported values are remembered so the framework echoing them
/// back as a prop can be recognised and swallowed. Ported verbatim from
/// gpui-vue, where the ring is what keeps a `v-model`ed input from fighting the
/// user for the caret.
const ECHO_RING_CAPACITY: usize = 32;
/// Upper bound on the undo history (gpui-vue's is unbounded).
const UNDO_LIMIT: usize = 256;
const CARET_BLINK_MS: u64 = 500;
/// Caret width, also the slack kept visible to the right of the caret when
/// scrolling to follow it.
pub(crate) const CARET_WIDTH: Pixels = px(2.0);

fn caret_visible(ms_since_activity: u64) -> bool {
    (ms_since_activity / CARET_BLINK_MS) % 2 == 0
}

fn utf16_offset_to_utf8(text: &str, offset: usize) -> usize {
    let mut utf8_offset = 0;
    let mut utf16_count = 0;
    for character in text.chars() {
        if utf16_count >= offset {
            break;
        }
        utf16_count += character.len_utf16();
        utf8_offset += character.len_utf8();
    }
    utf8_offset
}

/// Fold text into something a single-line input can hold.
///
/// A lone control character is dropped entirely — that is the `\r`/`\n` a
/// Return keypress turns into on platforms that route it through the text input
/// handler, and a single-line input must ignore it. Longer runs of text (a
/// paste) keep their words separated by turning each line break into one space,
/// and other control characters are stripped.
///
/// gpui-vue turned *every* newline into a space, including a lone one; that
/// made Return insert a space here.
fn single_line_text(text: &str) -> String {
    let mut characters = text.chars();
    if let (Some(only), None) = (characters.next(), characters.next()) {
        if only.is_control() {
            return String::new();
        }
    }

    let mut out = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '\r' => {
                if characters.peek() == Some(&'\n') {
                    characters.next();
                }
                out.push(' ');
            }
            '\n' => out.push(' '),
            character if character.is_control() => {}
            character => out.push(character),
        }
    }
    out
}

/// Remember a value we just reported, evicting the oldest once the ring is
/// full.
fn remember_reported(pending: &mut VecDeque<String>, value: String) {
    pending.push_back(value);
    while pending.len() > ECHO_RING_CAPACITY {
        pending.pop_front();
    }
}

/// Decide whether an incoming controlled value is an echo of something this
/// input reported.
///
/// On a match every value up to and including it is retired — the framework
/// processed them, so anything older can never come back — and the caller
/// leaves the content alone. On a miss the ring is cleared and the value is
/// external.
fn take_echo(pending: &mut VecDeque<String>, value: &str) -> bool {
    match pending.iter().rposition(|reported| reported == value) {
        Some(index) => {
            pending.drain(..=index);
            true
        }
        None => {
            pending.clear();
            false
        }
    }
}

#[derive(Clone)]
struct EditSnapshot {
    content: String,
    selected_range: Range<usize>,
    selection_reversed: bool,
}

/// Persistent editor state — one [`gpui::Entity<TextInputState>`] per `<input>`
/// node, created lazily by the framework and kept across frames.
pub struct TextInputState {
    focus_handle: FocusHandle,
    content: String,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    read_only: bool,
    on_change: Option<OnChange>,

    /// Values we reported through `on_change`, newest last.
    pending_values: VecDeque<String>,
    /// The last value the framework pushed in, so re-syncing an unchanged prop
    /// every frame is free.
    last_synced_value: Option<String>,

    undo_stack: Vec<EditSnapshot>,
    redo_stack: Vec<EditSnapshot>,

    is_selecting: bool,

    // Written by the element during layout/prepaint, read by hit testing.
    last_line: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    line_height: Pixels,
    content_width: Pixels,
    display_is_placeholder: bool,
    scroll_left: Pixels,

    blink_anchor: Instant,
    blink_task: Option<Task<()>>,
}

impl TextInputState {
    /// Create the state for one input node. `window` is unused today but kept
    /// in the signature so the framework's construction site does not have to
    /// change when it becomes needed.
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: String::new(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            read_only: false,
            on_change: None,
            pending_values: VecDeque::new(),
            last_synced_value: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            is_selecting: false,
            last_line: None,
            last_bounds: None,
            line_height: px(20.0),
            content_width: px(0.0),
            display_is_placeholder: true,
            scroll_left: px(0.0),
            blink_anchor: cx.background_executor().now(),
            blink_task: None,
        }
    }

    /// The current value, always free of line breaks.
    pub fn text(&self) -> &str {
        &self.content
    }

    /// Controlled-value sync from the framework (Vue `v-model`).
    ///
    /// Idempotent: passing the same value again is a no-op, so this can be
    /// called unconditionally every frame. A value that matches one this input
    /// itself reported is swallowed (echo ring); any other change is treated as
    /// external and hard-resets content, caret and undo history.
    pub fn sync_value(&mut self, value: &str, cx: &mut Context<Self>) {
        if self.last_synced_value.as_deref() == Some(value) {
            return;
        }
        self.last_synced_value = Some(value.to_owned());

        if take_echo(&mut self.pending_values, value) {
            return;
        }
        self.set_external_text(value, cx);
    }

    /// The focus handle owned by this input. The framework needs it to focus
    /// the input programmatically (autofocus, tab order) and to observe
    /// focus/blur.
    pub fn focus_handle(&self) -> FocusHandle {
        self.focus_handle.clone()
    }

    /// Whether this input currently holds keyboard focus.
    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    /// Move keyboard focus to this input.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus_handle, cx);
    }

    /// The selection, as byte offsets into [`Self::text`].
    pub fn selected_range(&self) -> Range<usize> {
        self.selected_range.clone()
    }

    /// The IME pre-edit range, if a composition is in progress.
    pub fn marked_range(&self) -> Option<Range<usize>> {
        self.marked_range.clone()
    }

    /// Per-frame parameters pushed in by [`super::text_input`].
    pub(crate) fn set_frame_params(&mut self, on_change: OnChange, read_only: bool) {
        self.on_change = Some(on_change);
        self.read_only = read_only;
    }

    // ---------------------------------------------------------------- editing

    fn snapshot(&self) -> EditSnapshot {
        EditSnapshot {
            content: self.content.clone(),
            selected_range: self.selected_range.clone(),
            selection_reversed: self.selection_reversed,
        }
    }

    fn push_undo(&mut self) {
        self.undo_stack.push(self.snapshot());
        if self.undo_stack.len() > UNDO_LIMIT {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    fn set_external_text(&mut self, value: &str, cx: &mut Context<Self>) {
        let value = single_line_text(value);
        if self.content == value {
            return;
        }
        self.content = value;
        let end = self.content.len();
        self.selected_range = end..end;
        self.selection_reversed = false;
        self.marked_range = None;
        self.scroll_left = px(0.0);
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.reset_blink(cx);
        cx.notify();
    }

    /// Report the current value to the framework and remember it in the echo
    /// ring. The callback runs deferred — after the current update completes —
    /// so it is safe for it to call back into this entity (`sync_value`).
    fn emit_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(on_change) = self.on_change.clone() else {
            return;
        };
        remember_reported(&mut self.pending_values, self.content.clone());
        let value = self.content.clone();
        window.defer(cx, move |window, cx| on_change(&value, window, cx));
    }

    fn restore(&mut self, snapshot: EditSnapshot, window: &mut Window, cx: &mut Context<Self>) {
        self.content = snapshot.content;
        self.selected_range = snapshot.selected_range;
        self.selection_reversed = snapshot.selection_reversed;
        self.marked_range = None;
        self.reset_blink(cx);
        self.emit_change(window, cx);
        cx.notify();
    }

    /// The one place content is mutated by the user. `range` is a byte range
    /// into the current content; the returned range covers the inserted text,
    /// or `None` if the edit was a no-op.
    fn edit(
        &mut self,
        range: Range<usize>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        // Never trust a range straight from the platform's IME.
        let start = boundaries::snap(&self.content, range.start);
        let end = boundaries::snap(&self.content, range.end).max(start);
        let range = start..end;

        let replacement = single_line_text(new_text);
        if replacement.is_empty() && range.is_empty() {
            // Nothing to do — e.g. Return, which reaches a single-line input
            // as a control character and is dropped. Leave the selection alone.
            return None;
        }
        // An IME composition is one undo step: only its first edit (the one
        // that has no pre-edit range yet) pushes a snapshot.
        if self.marked_range.is_none() {
            self.push_undo();
        }
        self.content =
            self.content[..range.start].to_owned() + &replacement + &self.content[range.end..];
        let inserted = range.start..range.start + replacement.len();
        self.reset_blink(cx);
        self.emit_change(window, cx);
        cx.notify();
        Some(inserted)
    }

    /// Replace the current selection (or the IME pre-edit) with `text`.
    fn replace_selection(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let range = self
            .marked_range
            .clone()
            .unwrap_or_else(|| self.selected_range.clone());
        let Some(inserted) = self.edit(range, text, window, cx) else {
            return;
        };
        self.selected_range = inserted.end..inserted.end;
        self.selection_reversed = false;
        self.marked_range = None;
    }

    // -------------------------------------------------------------- selection

    /// The moving end of the selection — where the caret is drawn.
    pub(crate) fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = boundaries::snap(&self.content, offset);
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        self.reset_blink(cx);
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = boundaries::snap(&self.content, offset);
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        self.reset_blink(cx);
        cx.notify();
    }

    fn set_selection(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        let start = boundaries::snap(&self.content, range.start);
        let end = boundaries::snap(&self.content, range.end);
        self.selected_range = start.min(end)..start.max(end);
        self.selection_reversed = false;
        self.reset_blink(cx);
        cx.notify();
    }

    fn selected_text(&self) -> &str {
        &self.content[self.selected_range.clone()]
    }

    // ------------------------------------------------------------- key actions

    pub(crate) fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        if self.selected_range.is_empty() {
            let previous = boundaries::previous_grapheme(&self.content, self.cursor_offset());
            if previous == self.cursor_offset() {
                return;
            }
            self.select_to(previous, cx);
        }
        self.replace_selection("", window, cx);
    }

    pub(crate) fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        if self.selected_range.is_empty() {
            let next = boundaries::next_grapheme(&self.content, self.cursor_offset());
            if next == self.cursor_offset() {
                return;
            }
            self.select_to(next, cx);
        }
        self.replace_selection("", window, cx);
    }

    pub(crate) fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        let offset = if self.selected_range.is_empty() {
            boundaries::previous_grapheme(&self.content, self.cursor_offset())
        } else {
            self.selected_range.start
        };
        self.move_to(offset, cx);
    }

    pub(crate) fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        let offset = if self.selected_range.is_empty() {
            boundaries::next_grapheme(&self.content, self.cursor_offset())
        } else {
            self.selected_range.end
        };
        self.move_to(offset, cx);
    }

    pub(crate) fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        let offset = boundaries::previous_grapheme(&self.content, self.cursor_offset());
        self.select_to(offset, cx);
    }

    pub(crate) fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        let offset = boundaries::next_grapheme(&self.content, self.cursor_offset());
        self.select_to(offset, cx);
    }

    pub(crate) fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.set_selection(0..self.content.len(), cx);
    }

    pub(crate) fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    pub(crate) fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    pub(crate) fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(0, cx);
    }

    pub(crate) fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.content.len(), cx);
    }

    pub(crate) fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let offset = boundaries::previous_word(&self.content, self.cursor_offset());
        self.move_to(offset, cx);
    }

    pub(crate) fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        let offset = boundaries::next_word(&self.content, self.cursor_offset());
        self.move_to(offset, cx);
    }

    pub(crate) fn select_word_left(
        &mut self,
        _: &SelectWordLeft,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let offset = boundaries::previous_word(&self.content, self.cursor_offset());
        self.select_to(offset, cx);
    }

    pub(crate) fn select_word_right(
        &mut self,
        _: &SelectWordRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let offset = boundaries::next_word(&self.content, self.cursor_offset());
        self.select_to(offset, cx);
    }

    pub(crate) fn delete_word_left(
        &mut self,
        _: &DeleteWordLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        if self.selected_range.is_empty() {
            let offset = boundaries::previous_word(&self.content, self.cursor_offset());
            self.select_to(offset, cx);
        }
        self.replace_selection("", window, cx);
    }

    pub(crate) fn delete_word_right(
        &mut self,
        _: &DeleteWordRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        if self.selected_range.is_empty() {
            let offset = boundaries::next_word(&self.content, self.cursor_offset());
            self.select_to(offset, cx);
        }
        self.replace_selection("", window, cx);
    }

    pub(crate) fn delete_to_start(
        &mut self,
        _: &DeleteToStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        if self.selected_range.is_empty() {
            if self.cursor_offset() == 0 {
                return;
            }
            self.select_to(0, cx);
        }
        self.replace_selection("", window, cx);
    }

    pub(crate) fn delete_to_end(
        &mut self,
        _: &DeleteToEnd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        if self.selected_range.is_empty() {
            if self.cursor_offset() == self.content.len() {
                return;
            }
            self.select_to(self.content.len(), cx);
        }
        self.replace_selection("", window, cx);
    }

    pub(crate) fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(self.selected_text().to_owned()));
    }

    pub(crate) fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only || self.selected_range.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(self.selected_text().to_owned()));
        self.replace_selection("", window, cx);
    }

    pub(crate) fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_selection(&text, window, cx);
        }
    }

    pub(crate) fn undo(&mut self, _: &Undo, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        if let Some(previous) = self.undo_stack.pop() {
            self.redo_stack.push(self.snapshot());
            self.restore(previous, window, cx);
        }
    }

    pub(crate) fn redo(&mut self, _: &Redo, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        if let Some(next) = self.redo_stack.pop() {
            self.undo_stack.push(self.snapshot());
            self.restore(next, window, cx);
        }
    }

    /// Escape collapses the selection; with nothing selected the key is left to
    /// the rest of the app (closing a dialog, clearing a filter, ...).
    pub(crate) fn clear_selection(
        &mut self,
        _: &ClearSelection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            cx.propagate();
            return;
        }
        let offset = self.cursor_offset();
        self.move_to(offset, cx);
    }

    pub(crate) fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    // ------------------------------------------------------------------ mouse

    pub(crate) fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        let index = self.index_for_position(event.position);
        match event.click_count {
            0 | 1 => {
                self.is_selecting = true;
                if event.modifiers.shift {
                    self.select_to(index, cx);
                } else {
                    self.move_to(index, cx);
                }
            }
            2 => {
                let range = boundaries::word_range_at(&self.content, index);
                self.set_selection(range, cx);
            }
            _ => self.set_selection(0..self.content.len(), cx),
        }
    }

    /// Registered window-wide during paint, so a drag that leaves the input
    /// keeps extending the selection.
    pub(crate) fn on_mouse_drag(
        &mut self,
        event: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_selecting {
            return;
        }
        let index = self.index_for_position(event.position);
        self.select_to(index, cx);
    }

    pub(crate) fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    // ----------------------------------------------------------- layout hooks

    /// Called by the element every frame once the line has been shaped.
    pub(crate) fn set_layout(
        &mut self,
        bounds: Bounds<Pixels>,
        line: ShapedLine,
        line_height: Pixels,
        display_is_placeholder: bool,
    ) {
        self.content_width = line.width();
        self.last_line = Some(line);
        self.last_bounds = Some(bounds);
        self.line_height = line_height;
        self.display_is_placeholder = display_is_placeholder;
        self.clamp_scroll(bounds.size.width);
    }

    /// Keep the caret inside the viewport, and the viewport inside the content.
    fn clamp_scroll(&mut self, viewport_width: Pixels) {
        let caret = self.x_for_index(self.cursor_offset());
        if caret < self.scroll_left {
            self.scroll_left = caret;
        } else if caret + CARET_WIDTH > self.scroll_left + viewport_width {
            self.scroll_left = caret + CARET_WIDTH - viewport_width;
        }
        let max_scroll = (self.content_width + CARET_WIDTH - viewport_width).max(px(0.0));
        self.scroll_left = self.scroll_left.clamp(px(0.0), max_scroll);
    }

    pub(crate) fn scroll_left(&self) -> Pixels {
        self.scroll_left
    }

    pub(crate) fn x_for_index(&self, index: usize) -> Pixels {
        if self.display_is_placeholder {
            return px(0.0);
        }
        match self.last_line.as_ref() {
            Some(line) => line.x_for_index(index.min(line.len())),
            None => px(0.0),
        }
    }

    /// Map a window-space position to a byte offset in the content.
    pub(crate) fn index_for_position(&self, position: Point<Pixels>) -> usize {
        if self.display_is_placeholder || self.content.is_empty() {
            return 0;
        }
        let (Some(bounds), Some(line)) = (self.last_bounds, self.last_line.as_ref()) else {
            return 0;
        };
        let x = position.x - bounds.left() + self.scroll_left;
        let index = line.closest_index_for_x(x);
        boundaries::snap(&self.content, index)
    }

    // ------------------------------------------------------------------ blink

    fn reset_blink(&mut self, cx: &Context<Self>) {
        self.blink_anchor = cx.background_executor().now();
    }

    /// Whether the caret is in its "on" phase, starting/stopping the blink
    /// timer as focus comes and goes. Ported from Comet via gpui-vue.
    pub(crate) fn caret_shown(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        if self.read_only || !self.focus_handle.is_focused(window) || !window.is_window_active() {
            self.blink_task = None;
            return false;
        }
        if self.blink_task.is_none() {
            self.reset_blink(cx);
            self.blink_task = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(CARET_BLINK_MS))
                        .await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            }));
        }
        caret_visible(self.blink_anchor.elapsed().as_millis() as u64)
    }

    // ------------------------------------------------------------------ utf16

    fn offset_from_utf16(&self, offset: usize) -> usize {
        utf16_offset_to_utf8(&self.content, offset)
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;
        for character in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += character.len_utf8();
            utf16_offset += character.len_utf16();
        }
        utf16_offset
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }
}

impl Focusable for TextInputState {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EntityInputHandler for TextInputState {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        self.content.get(range).map(str::to_owned)
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked_range = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        let Some(inserted) = self.edit(range, new_text, window, cx) else {
            return;
        };
        self.selected_range = inserted.end..inserted.end;
        self.selection_reversed = false;
        self.marked_range = None;
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());

        let Some(inserted) = self.edit(range, new_text, window, cx) else {
            // The composition was cancelled with nothing to replace.
            if self.marked_range.take().is_some() {
                cx.notify();
            }
            return;
        };
        self.marked_range = (!inserted.is_empty()).then_some(inserted.clone());

        // `new_selected_range_utf16` is relative to the inserted text.
        let selected_range = match new_selected_range_utf16.as_ref() {
            Some(selected) => {
                let replacement = &self.content[inserted.clone()];
                let start = inserted.start + utf16_offset_to_utf8(replacement, selected.start);
                let end = inserted.start + utf16_offset_to_utf8(replacement, selected.end);
                start..end
            }
            None => inserted.end..inserted.end,
        };
        self.selected_range = selected_range;
        self.selection_reversed = false;
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.range_from_utf16(&range_utf16);
        let start = self.x_for_index(range.start) - self.scroll_left;
        let end = self.x_for_index(range.end) - self.scroll_left;
        Some(Bounds::from_corners(
            gpui::point(element_bounds.left() + start, element_bounds.top()),
            gpui::point(
                element_bounds.left() + end.max(start + CARET_WIDTH),
                element_bounds.top() + self.line_height,
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.offset_to_utf16(self.index_for_position(point)))
    }

    fn set_selected_text_range(
        &mut self,
        range_utf16: Range<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.range_from_utf16(&range_utf16);
        self.set_selection(range, cx);
    }

    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.content.encode_utf16().count())
    }

    fn accepts_text_input(&self, _: &mut Window, _: &mut Context<Self>) -> bool {
        !self.read_only
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ime_offsets_are_relative_to_the_replacement_text() {
        assert_eq!(utf16_offset_to_utf8("é🙂", 0), 0);
        assert_eq!(utf16_offset_to_utf8("é🙂", 1), "é".len());
        assert_eq!(utf16_offset_to_utf8("é🙂", 3), "é🙂".len());
    }

    #[test]
    fn pasted_line_breaks_become_single_spaces() {
        assert_eq!(single_line_text("a\r\nb\nc\rd"), "a b c d");
        assert_eq!(single_line_text("plain"), "plain");
    }

    #[test]
    fn a_lone_control_character_is_dropped() {
        // Return / Tab reaching the input handler must not insert anything.
        assert_eq!(single_line_text("\r"), "");
        assert_eq!(single_line_text("\n"), "");
        assert_eq!(single_line_text("\t"), "");
        assert_eq!(single_line_text("\r\n"), " ");
    }

    fn ring(values: &[&str]) -> VecDeque<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn our_own_value_coming_back_is_swallowed() {
        let mut pending = ring(&["a", "ab"]);
        assert!(take_echo(&mut pending, "ab"));
        assert!(pending.is_empty());
    }

    #[test]
    fn an_echo_retires_everything_older_than_it() {
        // The framework is a frame behind: it acknowledges "a" while we have
        // already reported "ab" and "abc".
        let mut pending = ring(&["a", "ab", "abc"]);
        assert!(take_echo(&mut pending, "a"));
        assert_eq!(pending, ring(&["ab", "abc"]));
        assert!(take_echo(&mut pending, "abc"));
        assert!(pending.is_empty());
    }

    #[test]
    fn a_genuinely_external_value_clears_the_ring() {
        let mut pending = ring(&["a", "ab"]);
        assert!(!take_echo(&mut pending, "reset from js"));
        assert!(pending.is_empty());
    }

    #[test]
    fn the_ring_never_grows_past_its_capacity() {
        let mut pending = VecDeque::new();
        for index in 0..ECHO_RING_CAPACITY * 2 {
            remember_reported(&mut pending, index.to_string());
        }
        assert_eq!(pending.len(), ECHO_RING_CAPACITY);
        assert_eq!(pending.front().map(String::as_str), Some("32"));
        // The oldest values are gone, so a very stale echo reads as external.
        assert!(!take_echo(&mut pending, "0"));
    }

    #[test]
    fn caret_blink_phase() {
        assert!(caret_visible(0));
        assert!(caret_visible(CARET_BLINK_MS - 1));
        assert!(!caret_visible(CARET_BLINK_MS));
        assert!(!caret_visible(2 * CARET_BLINK_MS - 1));
        assert!(caret_visible(2 * CARET_BLINK_MS));
    }
}
