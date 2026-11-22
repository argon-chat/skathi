//! The persistent editor state behind a text input: content, selection, IME composition,
//! undo/redo, caret blink and the controlled-value echo ring.
//!
//! Adapted from gpui-vue (`crates/gpui-vue-core/src/custom_elements/input.rs`, Apache-2.0), itself
//! derived from [remorses/gpuix](https://github.com/remorses/gpuix) @
//! `b8bbff4ef0dc00dc24c3d3f3ca47818e1be15c69` (Apache-2.0) and [zeronsh/comet](https://github.com/zeronsh/comet)
//! (MIT); the caret blink is ported from Comet's `crates/ui/src/composer.rs`. API usage follows
//! gpui's own `crates/gpui/examples/input.rs` (Apache-2.0, zed-industries/zed).

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ops::Range;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    px, App, Bounds, ClipboardItem, Context, EntityInputHandler, FocusHandle, Focusable,
    KeystrokeEvent, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, SharedString,
    Subscription, Task, TextInputConfiguration, TransformationMatrix, UTF16Selection, Window,
    WrappedLine,
};

use super::element::ShapeKey;

/// GPUI paints affine matrices in device pixels while pointer events and IME queries use logical
/// window pixels, so the untransformed layout bounds alone cannot map either way.
#[derive(Clone, Copy)]
struct InputTransform {
    matrix: TransformationMatrix,
    scale_factor: f32,
}

impl Default for InputTransform {
    fn default() -> Self {
        Self {
            matrix: TransformationMatrix::unit(),
            scale_factor: 1.0,
        }
    }
}

impl InputTransform {
    fn map(&self, matrix: TransformationMatrix, point: Point<Pixels>) -> Point<Pixels> {
        let scale = self.scale_factor;
        let device = matrix.apply(Point::new(
            px(f32::from(point.x) * scale),
            px(f32::from(point.y) * scale),
        ));
        Point::new(
            px(f32::from(device.x) / scale),
            px(f32::from(device.y) / scale),
        )
    }

    fn layout_point(&self, window_point: Point<Pixels>) -> Option<Point<Pixels>> {
        let local = self.map(self.matrix.inverse()?, window_point);
        (f32::from(local.x).is_finite() && f32::from(local.y).is_finite()).then_some(local)
    }

    /// IME candidate windows need an axis-aligned rectangle in window space: transform every corner.
    fn window_bounds(&self, local: Bounds<Pixels>) -> Option<Bounds<Pixels>> {
        self.matrix.inverse()?; // A collapsed input has no visible caret.
        let corners = [
            local.origin,
            Point::new(local.right(), local.top()),
            Point::new(local.left(), local.bottom()),
            local.bottom_right(),
        ];
        let mut min = Point::new(px(f32::INFINITY), px(f32::INFINITY));
        let mut max = Point::new(px(f32::NEG_INFINITY), px(f32::NEG_INFINITY));
        for corner in corners {
            let p = self.map(self.matrix, corner);
            if !f32::from(p.x).is_finite() || !f32::from(p.y).is_finite() {
                return None;
            }
            min.x = min.x.min(p.x);
            min.y = min.y.min(p.y);
            max.x = max.x.max(p.x);
            max.y = max.y.max(p.y);
        }
        Some(Bounds::from_corners(min, max))
    }
}

use super::boundaries;
use super::{
    Backspace, ClearSelection, CompositionPhase, ControlledValue, Copy, Cut, Delete, DeleteToEnd,
    DeleteToStart, DeleteWordLeft, DeleteWordRight, Down, End, Home, InputIntent, InputType, Left,
    Newline, OnBeforeInput, OnChange, OnComposition, OnKeyDown, Paste, Redo, Right, SelectAll,
    SelectDown, SelectEnd, SelectHome, SelectLeft, SelectRight, SelectUp, SelectWordLeft,
    SelectWordRight, ShowCharacterPalette, TextInputHandlers, Undo, Up, WordLeft, WordRight,
};

/// Write counters of framework echoes not yet synced (see [`echo_write`]).
const ECHO_RING_CAPACITY: usize = 32;
const UNDO_LIMIT: usize = 256;
const CARET_BLINK_MS: u64 = 500;
/// Caret width, also the slack kept visible right of the caret when scrolling to follow it.
pub(crate) const CARET_WIDTH: Pixels = px(2.0);
pub(crate) const MASK: char = '\u{2022}';

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

/// Fold text into something a single-line input can hold. A lone control character is dropped (the
/// `\r`/`\n` of a Return routed through the text input handler); longer runs turn each line break
/// into one space and lose other control characters.
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

/// Fold text into something a `<textarea>` can hold. Line breaks become `\n` (the HTML API value),
/// tabs survive, other control characters go (one stray `\0` makes the line unshapeable). A lone
/// `\r`/`\n` is the Return of a platform that routes it through the IME text path.
fn multi_line_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '\r' => {
                if characters.peek() == Some(&'\n') {
                    characters.next();
                }
                out.push('\n');
            }
            '\n' | '\t' => out.push(character),
            character if character.is_control() => {}
            character => out.push(character),
        }
    }
    out
}

/// The write that echoes a reported value: one made while script handled that report (`before` is
/// the counter when the dispatch began) and holding exactly the reported string. Matching an older
/// report is not enough, since a script may assign any value the user once typed.
fn echo_write(reported: &str, before: Option<u32>, after: Option<(&str, u32)>) -> Option<u32> {
    let (value, write) = after?;
    (before != Some(write) && value == reported).then_some(write)
}

fn remember_echo(echoes: &mut VecDeque<u32>, write: u32) {
    echoes.push_back(write);
    while echoes.len() > ECHO_RING_CAPACITY {
        echoes.pop_front();
    }
}

/// Whether `write` is a recorded echo; it and the echoes before it are retired.
fn take_echo(echoes: &mut VecDeque<u32>, write: u32) -> bool {
    match echoes.iter().position(|&echo| echo == write) {
        Some(index) => {
            echoes.drain(..=index);
            true
        }
        None => false,
    }
}

fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// Cuts `text` to at most `room` UTF-16 code units, never inside a character.
fn truncate_utf16(text: &mut String, room: usize) {
    let mut units = 0;
    for (at, character) in text.char_indices() {
        units += character.len_utf16();
        if units > room {
            text.truncate(at);
            return;
        }
    }
}

/// The run of edits the last one belongs to, by where the caret ended up (a byte offset).
/// Consecutive typing, or consecutive single-grapheme deletes in one direction, is one undo step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UndoGroup {
    Typing(usize),
    Backward(usize),
    Forward(usize),
}

impl UndoGroup {
    fn continued_by(
        prev: Option<Self>,
        content: &str,
        range: &Range<usize>,
        ty: InputType,
    ) -> bool {
        match (prev, ty) {
            (Some(Self::Typing(at)), InputType::InsertText) => {
                range.is_empty() && range.start == at
            }
            (Some(Self::Backward(at)), InputType::DeleteContentBackward) => {
                range.end == at && boundaries::previous_grapheme(content, at) == range.start
            }
            (Some(Self::Forward(at)), InputType::DeleteContentForward) => {
                range.start == at && boundaries::next_grapheme(content, at) == range.end
            }
            _ => false,
        }
    }

    fn after(range: &Range<usize>, inserted: usize, ty: InputType) -> Option<Self> {
        match ty {
            InputType::InsertText => Some(Self::Typing(range.start + inserted)),
            InputType::DeleteContentBackward => Some(Self::Backward(range.start)),
            InputType::DeleteContentForward => Some(Self::Forward(range.start)),
            _ => None,
        }
    }
}

/// The selection in UTF-16 code units, as the DOM's `selectionStart` / `selectionEnd` report it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaretUtf16 {
    pub start: usize,
    pub end: usize,
    pub backward: bool,
    /// The framework write the content corresponds to ([`TextInputState::sync_value`]).
    pub synced_write: Option<u32>,
}

/// The caret as it stood when a hook was called from inside an update of the state, readable while
/// the entity is leased. `None` outside such a hook.
pub type CaretMirror = Rc<Cell<Option<CaretUtf16>>>;

/// A size-to-content field's last measurement: its key, text width and row count.
pub(crate) type MeasureCache = Rc<RefCell<Option<(ShapeKey, Pixels, usize)>>>;

struct ExposedCaret(CaretMirror);

impl Drop for ExposedCaret {
    fn drop(&mut self) {
        self.0.set(None);
    }
}

#[derive(Clone)]
struct EditSnapshot {
    content: String,
    selected_range: Range<usize>,
    selection_reversed: bool,
}

/// Persistent editor state: one `Entity<TextInputState>` per input node, kept across frames.
pub struct TextInputState {
    focus_handle: FocusHandle,
    content: String,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    read_only: bool,
    on_change: Option<OnChange>,
    on_before_input: Option<OnBeforeInput>,
    on_composition: Option<OnComposition>,
    on_key_down: Option<OnKeyDown>,
    composition_open: bool,
    /// The text of a composition cut short by an outside value assignment, whose `compositionend`
    /// the framework delivers ([`Self::flush_abandoned_composition`]); the assignment has no window.
    abandoned_composition: Option<String>,
    _blur: Subscription,

    /// Set when `on_key_down` cancelled a keystroke that would have produced a character: gpui's
    /// `replay_pending_input` feeds the input handler directly after a multi-stroke binding fails,
    /// so the widget must refuse the text too. Cleared by the next keystroke.
    suppress_text_input: bool,
    _key_interceptor: Option<Subscription>,

    echo_writes: VecDeque<u32>,
    /// The framework's write counter for the last assignment applied. Keyed on the counter, not the
    /// string: a second assignment of the same value must land if the user typed in between.
    last_synced_write: Option<u32>,
    controlled_value: Option<ControlledValue>,

    undo_stack: Vec<EditSnapshot>,
    redo_stack: Vec<EditSnapshot>,
    undo_group: Option<UndoGroup>,
    /// HTML `maxlength` in UTF-16 code units: caps what the user inserts, not assigned values.
    max_length: Option<usize>,
    /// Bumped on every content change; keys the shaped-layout cache.
    revision: u64,
    caret_mirror: CaretMirror,

    is_selecting: bool,

    multiline: bool,

    input_config: TextInputConfiguration,
    /// The configuration gpui last took, and how many times it has asked.
    queried_config: Option<TextInputConfiguration>,
    config_queries: u64,
    reported_config: Option<TextInputConfiguration>,

    // Written by the element during prepaint; one entry per hard line (`WrappedLine` is not `Clone`).
    lines: Vec<LineBox>,
    /// What `lines` were shaped from; the same key next frame reuses them.
    shaped: Option<ShapeKey>,
    display: Option<(u64, bool, SharedString)>,
    measured: MeasureCache,
    last_bounds: Option<Bounds<Pixels>>,
    input_transform: InputTransform,
    line_height: Pixels,
    content_width: Pixels,
    content_height: Pixels,
    display_is_placeholder: bool,
    /// `type="password"`: the shaped lines hold one [`MASK`] per character; offsets cross through
    /// [`Self::display_index`] and [`Self::content_index`].
    masked: bool,
    scroll_left: Pixels,
    scroll_top: Pixels,
    /// How far right the content sits because of `text-align`. Applied in the one number mapping
    /// content space to window space ([`Self::content_left`]) so caret, selection, IME and clicks
    /// agree. Zero once the content overflows.
    align_offset: Pixels,

    blink_anchor: Instant,
    blink_task: Option<Task<()>>,
}

/// One hard (`\n`-delimited) line; soft wrapping happens inside a `WrappedLine`.
pub(crate) struct LineBox {
    pub(crate) start: usize,
    pub(crate) row0: usize,
    pub(crate) rows: usize,
    pub(crate) line: WrappedLine,
}

impl LineBox {
    fn end(&self) -> usize {
        self.start + self.line.len()
    }
}

impl TextInputState {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // gpui resolves key bindings before element key listeners, so a plain `on_key_down` would
        // run after `backspace` had already deleted. The keystroke interceptor runs first, and
        // `cx.stop_propagation()` in it suppresses binding dispatch.
        let weak = cx.weak_entity();
        let interceptor = cx.intercept_keystrokes(move |event, window, cx| {
            weak.update(cx, |this, cx| {
                this.on_intercept_keystroke(event, window, cx)
            })
            .ok();
        });
        let focus_handle = cx.focus_handle();
        let blur = cx.on_blur(&focus_handle, window, |this, window, cx| {
            this.finish_composition(window, cx)
        });
        Self {
            focus_handle,
            content: String::new(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            read_only: false,
            on_change: None,
            on_before_input: None,
            on_composition: None,
            on_key_down: None,
            composition_open: false,
            abandoned_composition: None,
            _blur: blur,
            suppress_text_input: false,
            _key_interceptor: Some(interceptor),
            echo_writes: VecDeque::new(),
            last_synced_write: None,
            controlled_value: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            undo_group: None,
            max_length: None,
            revision: 0,
            caret_mirror: Rc::default(),
            is_selecting: false,
            multiline: false,
            input_config: TextInputConfiguration::default(),
            queried_config: None,
            config_queries: 0,
            reported_config: None,
            lines: Vec::new(),
            shaped: None,
            display: None,
            measured: Rc::default(),
            last_bounds: None,
            input_transform: InputTransform::default(),
            line_height: px(20.0),
            content_width: px(0.0),
            content_height: px(0.0),
            display_is_placeholder: true,
            masked: false,
            align_offset: px(0.0),
            scroll_left: px(0.0),
            scroll_top: px(0.0),
            blink_anchor: cx.background_executor().now(),
            blink_task: None,
        }
    }

    pub fn text(&self) -> &str {
        &self.content
    }

    /// Controlled-value sync from the framework (Vue `v-model`). The same `write` is a no-op, so
    /// this can be called every frame. A write recorded as the echo of a reported value is swallowed;
    /// any other is an assignment and hard-resets content, caret and undo.
    pub fn sync_value(&mut self, value: &str, write: u32, cx: &mut Context<Self>) {
        if self.last_synced_write == Some(write) {
            return;
        }
        self.last_synced_write = Some(write);

        if take_echo(&mut self.echo_writes, write) {
            return;
        }
        self.set_external_text(value, cx);
    }

    /// [`Self::sync_value`] now rather than at the next frame: a hook may have assigned the field.
    pub(crate) fn resync_controlled(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.controlled_value.clone() else {
            return;
        };
        if let Some((value, write)) = source(cx) {
            self.sync_value(&value, write, cx);
        }
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus_handle.clone()
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus_handle, cx);
    }

    /// Sets the field mode before the initial controlled value is sanitized, so call it before `sync_value`.
    pub fn set_multiline(&mut self, multiline: bool) {
        self.multiline = multiline;
    }

    pub fn select_all_text(&mut self, cx: &mut Context<Self>) {
        self.set_selection(0..self.content.len(), cx);
    }

    pub fn set_selection_range_utf16(
        &mut self,
        range: Range<usize>,
        backward: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let end = self.offset_from_utf16(range.end);
        let start = if range.start >= range.end {
            end
        } else {
            self.offset_from_utf16(range.start)
        };
        let range = start..end;
        if self.selected_range == range && self.selection_reversed == backward {
            return false;
        }
        self.set_selection(range, cx);
        self.selection_reversed = backward;
        true
    }

    pub fn selected_range(&self) -> Range<usize> {
        self.selected_range.clone()
    }

    pub fn caret_utf16(&self) -> CaretUtf16 {
        CaretUtf16 {
            start: self.offset_to_utf16(self.selected_range.start),
            end: self.offset_to_utf16(self.selected_range.end),
            backward: self.selection_reversed,
            synced_write: self.last_synced_write,
        }
    }

    pub fn caret_mirror(&self) -> CaretMirror {
        self.caret_mirror.clone()
    }

    /// Publishes the caret to [`Self::caret_mirror`] until the guard drops, around a hook that runs
    /// JS while this entity is being updated.
    fn expose_caret(&self) -> ExposedCaret {
        self.caret_mirror.set(Some(self.caret_utf16()));
        ExposedCaret(self.caret_mirror.clone())
    }

    pub fn marked_range(&self) -> Option<Range<usize>> {
        self.marked_range.clone()
    }

    pub(crate) fn set_frame_params(
        &mut self,
        handlers: TextInputHandlers,
        read_only: bool,
        multiline: bool,
        input_config: TextInputConfiguration,
        max_length: Option<usize>,
    ) {
        self.on_change = handlers.on_change;
        self.on_before_input = handlers.on_before_input;
        self.on_composition = handlers.on_composition;
        self.on_key_down = handlers.on_key_down;
        self.controlled_value = handlers.controlled_value;
        self.read_only = read_only;
        self.multiline = multiline;
        self.input_config = input_config;
        self.max_length = max_length;
    }

    /// How many UTF-16 code units `maxlength` leaves for text replacing `range`; `None` unlimited.
    /// As in Chromium, the cut happens after `beforeinput`: a full field fires it and then no `input`.
    fn room_for(&self, range: &Range<usize>) -> Option<usize> {
        let max = self.max_length?;
        let kept = utf16_len(&self.content) - utf16_len(self.content.get(range.clone())?);
        Some(max.saturating_sub(kept))
    }

    /// `Some((configuration, queries))` the first time gpui has queried a configuration not yet
    /// reported. The Windows platform window's `set_text_input_configuration` is a no-op, so this is
    /// the only evidence gpui took it.
    pub fn newly_forwarded_configuration(&mut self) -> Option<(TextInputConfiguration, u64)> {
        let queried = self.queried_config.as_ref()?;
        if self.reported_config.as_ref() == Some(queried) {
            return None;
        }
        self.reported_config = Some(queried.clone());
        Some((queried.clone(), self.config_queries))
    }

    fn sanitize(&self, text: &str) -> String {
        if self.multiline {
            multi_line_text(text)
        } else {
            single_line_text(text)
        }
    }

    /// DOM `tabindex`: `Some(n >= 0)` is a tab stop at `n`, `Some(n < 0)` focusable but skipped by
    /// `Tab`, `None` removed. Rewrites the focus handle in place (same `FocusId`, focus undisturbed).
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
        // A shortcut synthesized on modifier release has no physical key down.
        let (Some(key_down), Some(on_key_down)) = (&event.key_down, self.on_key_down.clone())
        else {
            return;
        };
        let proceed = {
            let _caret = self.expose_caret();
            on_key_down(key_down, window, cx)
        };
        if proceed {
            self.resync_controlled(cx);
            self.flush_abandoned_composition(window, cx);
            return;
        }
        self.suppress_text_input = event.keystroke.key_char.is_some();
        cx.stop_propagation();
    }

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
        let value = self.sanitize(value);
        if self.content == value {
            return;
        }
        self.abandon_composition();
        self.content = value;
        self.revision += 1;
        self.undo_group = None;
        let end = self.content.len();
        self.selected_range = end..end;
        self.selection_reversed = false;
        self.marked_range = None;
        self.scroll_left = px(0.0);
        self.scroll_top = px(0.0);
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.reset_blink(cx);
        cx.notify();
    }

    /// Reports the current value and its edit to the framework, recording the framework's write-back
    /// of it as an echo. Deferred, so the callback may call back into this entity.
    fn emit_change(&mut self, intent: InputIntent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(on_change) = self.on_change.clone() else {
            return;
        };
        let value = self.content.clone();
        let source = self.controlled_value.clone();
        let this = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            let read = |cx: &App| source.as_ref().and_then(|source| source(cx));
            let before = read(cx).map(|(_, write)| write);
            // The dispatch runs script and its microtasks to completion, so a framework write-back
            // has landed by the time it returns.
            on_change(&value, &intent, window, cx);
            let after = read(cx);
            let after = after.as_ref().map(|(text, write)| (text.as_str(), *write));
            if let Some(write) = echo_write(&value, before, after) {
                this.update(cx, |this, _| remember_echo(&mut this.echo_writes, write))
                    .ok();
            }
        });
    }

    /// `beforeinput` for an edit about to be made: `false` when a handler cancelled it. IME edits
    /// cannot be cancelled and nothing is read back mid-composition.
    fn before_input(
        &mut self,
        intent: &InputIntent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(hook) = self.on_before_input.clone() else {
            return true;
        };
        let proceed = {
            let _caret = self.expose_caret();
            hook(intent, window, cx)
        };
        if intent.input_type.composing() {
            return true;
        }
        if proceed {
            self.resync_controlled(cx);
            self.flush_abandoned_composition(window, cx);
        }
        proceed
    }

    /// One IME composition stage; `End` is deferred so it follows the commit's `input`.
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
        let Some(hook) = self.on_composition.clone() else {
            return;
        };
        if phase == CompositionPhase::End {
            let data = data.to_owned();
            window.defer(cx, move |window, cx| {
                hook(CompositionPhase::End, &data, window, cx)
            });
        } else {
            let _caret = self.expose_caret();
            hook(phase, data, window, cx);
        }
    }

    pub(crate) fn finish_composition(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(range) = self.marked_range.take() else {
            return;
        };
        let text = self.content.get(range).unwrap_or_default().to_owned();
        cx.notify();
        self.composition(CompositionPhase::End, &text, window, cx);
    }

    fn abandon_composition(&mut self) {
        let Some(range) = self.marked_range.take() else {
            return;
        };
        if std::mem::take(&mut self.composition_open) {
            self.abandoned_composition =
                Some(self.content.get(range).unwrap_or_default().to_owned());
        }
    }

    /// Delivers the `compositionend` an assigned value owes; called after [`Self::sync_value`].
    pub fn flush_abandoned_composition(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = self.abandoned_composition.take() else {
            return;
        };
        let Some(hook) = self.on_composition.clone() else {
            return;
        };
        window.defer(cx, move |window, cx| {
            hook(CompositionPhase::End, &text, window, cx)
        });
    }

    fn restore(
        &mut self,
        snapshot: EditSnapshot,
        input_type: InputType,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.finish_composition(window, cx);
        self.content = snapshot.content;
        self.revision += 1;
        self.undo_group = None;
        self.selected_range = snapshot.selected_range;
        self.selection_reversed = snapshot.selection_reversed;
        self.reset_blink(cx);
        self.emit_change(InputIntent::bare(input_type), window, cx);
        cx.notify();
    }

    /// The one place content is mutated by the user. `range` is a byte range into the content;
    /// `limited` applies `maxlength` (not to an IME pre-edit, which is capped when it commits).
    fn edit(
        &mut self,
        range: Range<usize>,
        new_text: &str,
        intent: InputIntent,
        limited: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        let start = boundaries::snap(&self.content, range.start);
        let end = boundaries::snap(&self.content, range.end).max(start);
        let range = start..end;

        let mut replacement = self.sanitize(new_text);
        if let Some(room) = self.room_for(&range).filter(|_| limited) {
            truncate_utf16(&mut replacement, room);
        }
        if replacement.is_empty() && range.is_empty() {
            return None;
        }
        // An IME composition is one undo step: only its first edit pushes a snapshot.
        let ty = intent.input_type;
        if self.marked_range.is_none() {
            if !UndoGroup::continued_by(self.undo_group, &self.content, &range, ty) {
                self.push_undo();
            }
            self.undo_group = UndoGroup::after(&range, replacement.len(), ty);
        } else {
            self.undo_group = None;
        }
        self.content =
            self.content[..range.start].to_owned() + &replacement + &self.content[range.end..];
        self.revision += 1;
        let inserted = range.start..range.start + replacement.len();
        self.reset_blink(cx);
        self.emit_change(intent, window, cx);
        cx.notify();
        Some(inserted)
    }

    fn replace_selection(
        &mut self,
        text: &str,
        intent: InputIntent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        let range = self
            .marked_range
            .clone()
            .unwrap_or_else(|| self.selected_range.clone());
        let Some(inserted) = self.edit(range, text, intent, true, window, cx) else {
            return;
        };
        self.selected_range = inserted.end..inserted.end;
        self.selection_reversed = false;
        if self.marked_range.take().is_some() {
            self.composition(CompositionPhase::End, text, window, cx);
        }
    }

    /// A deletion: the selection, or from the caret to `to(self)`. Nothing to delete is no edit.
    fn delete_toward(
        &mut self,
        input_type: InputType,
        to: fn(&Self) -> usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only || (self.selected_range.is_empty() && to(self) == self.cursor_offset()) {
            return;
        }
        let intent = InputIntent::bare(input_type);
        if !self.before_input(&intent, window, cx) {
            return;
        }
        if self.selected_range.is_empty() {
            let offset = to(self);
            if offset == self.cursor_offset() {
                return;
            }
            self.select_to(offset, cx);
        }
        self.replace_selection("", intent, window, cx);
    }

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
        self.undo_group = None;
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
        self.undo_group = None;
        self.reset_blink(cx);
        cx.notify();
    }

    fn selected_text(&self) -> &str {
        &self.content[self.selected_range.clone()]
    }

    pub(crate) fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        let to = |s: &Self| boundaries::previous_grapheme(&s.content, s.cursor_offset());
        self.delete_toward(InputType::DeleteContentBackward, to, window, cx);
    }

    pub(crate) fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        let to = |s: &Self| boundaries::next_grapheme(&s.content, s.cursor_offset());
        self.delete_toward(InputType::DeleteContentForward, to, window, cx);
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
        self.select_all_text(cx);
    }

    pub(crate) fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.row_bound(self.cursor_offset(), false);
        self.move_to(offset, cx);
    }

    pub(crate) fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.row_bound(self.cursor_offset(), true);
        self.move_to(offset, cx);
    }

    pub(crate) fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.row_bound(self.cursor_offset(), false);
        self.select_to(offset, cx);
    }

    pub(crate) fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.row_bound(self.cursor_offset(), true);
        self.select_to(offset, cx);
    }

    pub(crate) fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.offset_one_row(self.cursor_offset(), -1);
        self.move_to(offset, cx);
    }

    pub(crate) fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.offset_one_row(self.cursor_offset(), 1);
        self.move_to(offset, cx);
    }

    pub(crate) fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.offset_one_row(self.cursor_offset(), -1);
        self.select_to(offset, cx);
    }

    pub(crate) fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.offset_one_row(self.cursor_offset(), 1);
        self.select_to(offset, cx);
    }

    pub(crate) fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only || !self.multiline {
            return;
        }
        let intent = InputIntent::bare(InputType::InsertLineBreak);
        if !self.before_input(&intent, window, cx) {
            return;
        }
        self.replace_selection("\n", intent, window, cx);
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
        let to = |s: &Self| boundaries::previous_word(&s.content, s.cursor_offset());
        self.delete_toward(InputType::DeleteWordBackward, to, window, cx);
    }

    pub(crate) fn delete_word_right(
        &mut self,
        _: &DeleteWordRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let to = |s: &Self| boundaries::next_word(&s.content, s.cursor_offset());
        self.delete_toward(InputType::DeleteWordForward, to, window, cx);
    }

    pub(crate) fn delete_to_start(
        &mut self,
        _: &DeleteToStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_toward(InputType::DeleteSoftLineBackward, |_| 0, window, cx);
    }

    pub(crate) fn delete_to_end(
        &mut self,
        _: &DeleteToEnd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_toward(
            InputType::DeleteSoftLineForward,
            |s| s.content.len(),
            window,
            cx,
        );
    }

    pub(crate) fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if self.masked || self.selected_range.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(self.selected_text().to_owned()));
    }

    pub(crate) fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if self.masked || self.read_only || self.selected_range.is_empty() {
            return;
        }
        // Chromium writes the clipboard before asking `beforeinput`: a cancelled `deleteByCut`
        // leaves the text both on the clipboard and in the field.
        cx.write_to_clipboard(ClipboardItem::new_string(self.selected_text().to_owned()));
        let intent = InputIntent::bare(InputType::DeleteByCut);
        if !self.before_input(&intent, window, cx) || self.selected_range.is_empty() {
            return;
        }
        self.replace_selection("", intent, window, cx);
    }

    pub(crate) fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let intent = InputIntent::with(InputType::InsertFromPaste, &text);
        if !self.before_input(&intent, window, cx) {
            return;
        }
        self.replace_selection(&text, intent, window, cx);
    }

    pub(crate) fn undo(&mut self, _: &Undo, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only || self.undo_stack.is_empty() {
            return;
        }
        if !self.before_input(&InputIntent::bare(InputType::HistoryUndo), window, cx) {
            return;
        }
        if let Some(previous) = self.undo_stack.pop() {
            self.redo_stack.push(self.snapshot());
            self.restore(previous, InputType::HistoryUndo, window, cx);
        }
    }

    pub(crate) fn redo(&mut self, _: &Redo, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only || self.redo_stack.is_empty() {
            return;
        }
        if !self.before_input(&InputIntent::bare(InputType::HistoryRedo), window, cx) {
            return;
        }
        if let Some(next) = self.redo_stack.pop() {
            self.undo_stack.push(self.snapshot());
            self.restore(next, InputType::HistoryRedo, window, cx);
        }
    }

    /// Escape collapses the selection; with none, the key is left to the app (closing a dialog, ...).
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

    pub(crate) fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        let Some(index) = self.index_for_window_position(event.position) else {
            self.is_selecting = false;
            return;
        };
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

    /// A right-button press: focus, and the caret moves unless the press is inside the selection,
    /// which stays for the context menu (Chromium on Windows).
    pub(crate) fn on_secondary_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        let Some(index) = self.index_for_window_position(event.position) else {
            self.is_selecting = false;
            return;
        };
        if !secondary_press_keeps(&self.selected_range, index) {
            self.move_to(index, cx);
        }
    }

    /// Registered window-wide during paint so a drag that leaves the input keeps extending the selection.
    pub(crate) fn on_mouse_drag(
        &mut self,
        event: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_selecting {
            return;
        }
        let Some(index) = self.index_for_window_position(event.position) else {
            self.is_selecting = false;
            return;
        };
        self.select_to(index, cx);
    }

    pub(crate) fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    /// The composed matrix in force while this field is drawn; captured in prepaint and paint since
    /// cached layout can be reused under a wrapper whose transform changed.
    pub(crate) fn set_input_transform(&mut self, matrix: TransformationMatrix, scale_factor: f32) {
        self.input_transform = InputTransform {
            matrix,
            scale_factor,
        };
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// What is drawn for the content (masked for a password), kept until the content changes.
    pub(crate) fn display_shared(&mut self) -> SharedString {
        match &self.display {
            Some((revision, masked, text))
                if *revision == self.revision && *masked == self.masked =>
            {
                text.clone()
            }
            _ => {
                let text = SharedString::from(self.display_text());
                self.display = Some((self.revision, self.masked, text.clone()));
                text
            }
        }
    }

    pub(crate) fn shaped_key(&self) -> Option<&ShapeKey> {
        self.shaped.as_ref()
    }

    pub(crate) fn measure_cache(&self) -> MeasureCache {
        self.measured.clone()
    }

    /// Called by the element every frame; `shaped` is `None` when the lines from the last frame
    /// still match ([`Self::shaped_key`]).
    pub(crate) fn set_layout(
        &mut self,
        bounds: Bounds<Pixels>,
        shaped: Option<(Vec<LineBox>, ShapeKey)>,
        line_height: Pixels,
        display_is_placeholder: bool,
        text_align: gpui::TextAlign,
    ) {
        if let Some((lines, key)) = shaped {
            self.lines = lines;
            self.shaped = Some(key);
        }
        self.content_width = self
            .lines
            .iter()
            .map(|l| l.line.width())
            .fold(px(0.0), Pixels::max);
        let rows: usize = self.lines.iter().map(|l| l.rows).sum();
        self.content_height = line_height * rows.max(1) as f32;
        self.last_bounds = Some(bounds);
        self.line_height = line_height;
        self.display_is_placeholder = display_is_placeholder;
        // `CARET_WIDTH` is reserved as in `clamp_scroll` so a caret at the end of a right-aligned field fits.
        let slack = (bounds.size.width - self.content_width - CARET_WIDTH).max(px(0.0));
        self.align_offset = match text_align {
            gpui::TextAlign::Left => px(0.0),
            gpui::TextAlign::Center => slack / 2.0,
            gpui::TextAlign::Right => slack,
        };
        self.clamp_scroll(bounds.size);
    }

    pub(crate) fn content_left(&self, bounds_left: Pixels) -> Pixels {
        bounds_left - self.scroll_left + self.align_offset
    }

    fn clamp_scroll(&mut self, viewport: gpui::Size<Pixels>) {
        let caret = self.position_for_index(self.cursor_offset());
        if self.multiline {
            self.scroll_left = px(0.0);
            if caret.y < self.scroll_top {
                self.scroll_top = caret.y;
            } else if caret.y + self.line_height > self.scroll_top + viewport.height {
                self.scroll_top = caret.y + self.line_height - viewport.height;
            }
            let max_scroll = (self.content_height - viewport.height).max(px(0.0));
            self.scroll_top = self.scroll_top.clamp(px(0.0), max_scroll);
            return;
        }
        self.scroll_top = px(0.0);
        if caret.x < self.scroll_left {
            self.scroll_left = caret.x;
        } else if caret.x + CARET_WIDTH > self.scroll_left + viewport.width {
            self.scroll_left = caret.x + CARET_WIDTH - viewport.width;
        }
        let max_scroll = (self.content_width + CARET_WIDTH - viewport.width).max(px(0.0));
        self.scroll_left = self.scroll_left.clamp(px(0.0), max_scroll);
    }

    pub fn scroll_by(&mut self, dy: Pixels, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.multiline {
            return;
        }
        let viewport = self.last_bounds.map(|b| b.size.height).unwrap_or(px(0.0));
        let max_scroll = (self.content_height - viewport).max(px(0.0));
        let next = (self.scroll_top + dy).clamp(px(0.0), max_scroll);
        if next != self.scroll_top {
            self.scroll_top = next;
            cx.notify();
        }
    }

    pub fn scroll_top(&self) -> Pixels {
        self.scroll_top
    }

    pub(crate) fn content_height(&self) -> Pixels {
        self.content_height
    }

    pub(crate) fn lines(&self) -> &[LineBox] {
        &self.lines
    }

    pub(crate) fn set_masked(&mut self, masked: bool) {
        self.masked = masked;
    }

    pub(crate) fn display_text(&self) -> String {
        if self.masked {
            std::iter::repeat_n(MASK, self.content.chars().count()).collect()
        } else {
            self.content.clone()
        }
    }

    pub(crate) fn display_index(&self, index: usize) -> usize {
        if self.masked {
            masked_offset(&self.content, index)
        } else {
            index
        }
    }

    fn content_index(&self, display: usize) -> usize {
        if self.masked {
            unmasked_offset(&self.content, display)
        } else {
            display
        }
    }

    pub(crate) fn snap_index(&self, index: usize) -> usize {
        boundaries::snap(&self.content, index.min(self.content.len()))
    }

    pub(crate) fn next_index(&self, index: usize) -> usize {
        boundaries::next_grapheme(&self.content, index)
    }

    pub(crate) fn line_height(&self) -> Pixels {
        self.line_height
    }

    fn line_at(&self, index: usize) -> Option<(&LineBox, usize)> {
        let boxes = &self.lines;
        // A `\n` belongs to neither line; an offset at one reports the end of the line before it
        // (where a browser draws the caret after End).
        let found = boxes
            .iter()
            .rev()
            .find(|l| index >= l.start)
            .or_else(|| boxes.first())?;
        Some((
            found,
            (index - found.start.min(index)).min(found.line.len()),
        ))
    }

    pub(crate) fn position_for_index(&self, index: usize) -> Point<Pixels> {
        if self.display_is_placeholder {
            return Point::default();
        }
        let index = self.display_index(index);
        let Some((line_box, local)) = self.line_at(index) else {
            return Point::default();
        };
        let within = line_box
            .line
            .position_for_index(local, self.line_height)
            // `position_for_index` is `None` past the last glyph; the end of the last row is right.
            .unwrap_or_else(|| {
                Point::new(
                    line_box.line.width(),
                    self.line_height * (line_box.rows.saturating_sub(1)) as f32,
                )
            });
        Point::new(within.x, within.y + self.line_height * line_box.row0 as f32)
    }

    fn index_for_window_position(&self, position: Point<Pixels>) -> Option<usize> {
        Some(self.index_for_position(self.input_transform.layout_point(position)?))
    }

    pub(crate) fn index_for_position(&self, position: Point<Pixels>) -> usize {
        if self.display_is_placeholder || self.content.is_empty() {
            return 0;
        }
        let Some(bounds) = self.last_bounds else {
            return 0;
        };
        let local = Point::new(
            position.x - self.content_left(bounds.left()),
            position.y - bounds.top() + self.scroll_top,
        );
        let row = if self.line_height > px(0.0) {
            (f32::from(local.y) / f32::from(self.line_height)).floor()
        } else {
            0.0
        };
        if row < 0.0 {
            return 0;
        }
        let row = row as usize;
        let Some(line_box) = self
            .lines
            .iter()
            .rev()
            .find(|l| row >= l.row0)
            .or_else(|| self.lines.first())
        else {
            return 0;
        };
        if row >= line_box.row0 + line_box.rows
            && line_box.end() >= self.display_index(self.content.len())
        {
            return self.content.len();
        }
        let within = Point::new(local.x, local.y - self.line_height * line_box.row0 as f32);
        let index = match line_box
            .line
            .closest_index_for_position(within, self.line_height)
        {
            Ok(i) | Err(i) => i,
        };
        let index = self.content_index(line_box.start + index.min(line_box.line.len()));
        boundaries::snap(&self.content, index)
    }

    fn row_bound(&self, index: usize, forward: bool) -> usize {
        if self.lines.is_empty() {
            return if forward { self.content.len() } else { 0 };
        }
        let y = self.position_for_index(index).y;
        let x = if forward { px(1.0e6) } else { px(-1.0e6) };
        self.index_for_content_point(Point::new(x, y))
    }

    fn offset_one_row(&self, index: usize, delta: i32) -> usize {
        let caret = self.position_for_index(index);
        let y = caret.y + self.line_height * delta as f32;
        if y < px(0.0) {
            return 0;
        }
        if y >= self.content_height {
            return self.content.len();
        }
        self.index_for_content_point(Point::new(caret.x, y))
    }

    fn index_for_content_point(&self, point: Point<Pixels>) -> usize {
        let Some(bounds) = self.last_bounds else {
            return 0;
        };
        self.index_for_position(Point::new(
            point.x + self.content_left(bounds.left()),
            point.y + bounds.top() - self.scroll_top,
        ))
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
        caret_visible(self.blink_anchor.elapsed().as_millis() as u64)
    }

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

    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_composition(window, cx);
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
        if std::mem::take(&mut self.suppress_text_input) && range_utf16.is_none() {
            return;
        }
        let range_of = |this: &Self| {
            range_utf16
                .as_ref()
                .map(|range| this.range_from_utf16(range))
                .or_else(|| this.marked_range.clone())
                .unwrap_or_else(|| this.selected_range.clone())
        };
        if self.marked_range.is_some() {
            // The IME commits (or, with "", cancels): the last `beforeinput` and `compositionupdate`
            // carry the committed text, then the edit and its `input`, then `compositionend`.
            let intent = InputIntent::with(InputType::InsertCompositionText, new_text);
            self.before_input(&intent, window, cx);
            self.composition(CompositionPhase::Update, new_text, window, cx);
            if let Some(inserted) = self.edit(range_of(self), new_text, intent, true, window, cx) {
                self.selected_range = inserted.end..inserted.end;
                self.selection_reversed = false;
            }
            if self.marked_range.take().is_some() {
                cx.notify();
            }
            self.composition(CompositionPhase::End, new_text, window, cx);
            return;
        }
        let replacement = self.sanitize(new_text);
        if replacement.is_empty() && (!new_text.is_empty() || range_of(self).is_empty()) {
            return;
        }
        let intent = if replacement == "\n" {
            InputIntent::bare(InputType::InsertLineBreak)
        } else {
            InputIntent::with(InputType::InsertText, new_text)
        };
        if !self.before_input(&intent, window, cx) {
            return;
        }
        let Some(inserted) = self.edit(range_of(self), new_text, intent, true, window, cx) else {
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

        if self.marked_range.is_none() {
            if new_text.is_empty() {
                // An empty pre-edit outside a composition (some IMEs send one): delete the selection, start nothing.
                if range.is_empty() {
                    return;
                }
                let intent = InputIntent::bare(InputType::DeleteContentBackward);
                if !self.before_input(&intent, window, cx) {
                    return;
                }
                let range = self.selected_range.clone();
                if let Some(inserted) = self.edit(range, "", intent, false, window, cx) {
                    self.selected_range = inserted.end..inserted.end;
                    self.selection_reversed = false;
                }
                return;
            }
            let replaced = self
                .content
                .get(range.clone())
                .unwrap_or_default()
                .to_owned();
            self.composition(CompositionPhase::Start, &replaced, window, cx);
        }
        // Chromium's order for every pre-edit: `beforeinput` (not cancelable), `compositionupdate`, the edit, `input`.
        let intent = InputIntent::with(InputType::InsertCompositionText, new_text);
        self.before_input(&intent, window, cx);
        self.composition(CompositionPhase::Update, new_text, window, cx);

        let Some(inserted) = self.edit(range, new_text, intent, false, window, cx) else {
            if self.marked_range.take().is_some() {
                cx.notify();
            }
            self.composition(CompositionPhase::End, "", window, cx);
            return;
        };
        self.marked_range = (!inserted.is_empty()).then_some(inserted.clone());
        if self.marked_range.is_none() {
            self.composition(CompositionPhase::End, "", window, cx);
        }

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
        let start = self.position_for_index(range.start);
        let end = self.position_for_index(range.end);
        // The candidate window hangs off the row the pre-edit ENDS on; the start's row would put it a line too high.
        let top = element_bounds.top() + end.y - self.scroll_top;
        let left = self.content_left(element_bounds.left());
        let (x0, x1) = if start.y == end.y {
            (start.x, end.x.max(start.x + CARET_WIDTH))
        } else {
            (px(0.0), end.x.max(CARET_WIDTH))
        };
        self.input_transform.window_bounds(Bounds::from_corners(
            gpui::point(left + x0, top),
            gpui::point(left + x1, top + self.line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.offset_to_utf16(self.index_for_window_position(point)?))
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

    /// gpui asks the focused field for this once a frame and forwards it only on change (zed#63264),
    /// so a reactive `:spellcheck` takes effect on the next frame.
    fn text_input_configuration(
        &mut self,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> TextInputConfiguration {
        self.config_queries = self.config_queries.saturating_add(1);
        self.queried_config = Some(self.input_config.clone());
        self.input_config.clone()
    }
}

fn secondary_press_keeps(selection: &Range<usize>, index: usize) -> bool {
    !selection.is_empty() && selection.start <= index && index <= selection.end
}

fn masked_offset(content: &str, index: usize) -> usize {
    content
        .char_indices()
        .take_while(|(at, _)| *at < index)
        .count()
        * MASK.len_utf8()
}

fn unmasked_offset(content: &str, masked: usize) -> usize {
    content
        .char_indices()
        .nth(masked / MASK.len_utf8())
        .map_or(content.len(), |(at, _)| at)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_at_the_caret_is_one_undo_step_until_the_caret_jumps() {
        let ty = InputType::InsertText;
        let after_a = UndoGroup::after(&(0..0), 1, ty);
        assert_eq!(after_a, Some(UndoGroup::Typing(1)));
        assert!(UndoGroup::continued_by(after_a, "a", &(1..1), ty));
        assert!(
            !UndoGroup::continued_by(after_a, "ab", &(0..0), ty),
            "elsewhere"
        );
        assert!(
            !UndoGroup::continued_by(after_a, "ab", &(1..2), ty),
            "over a selection"
        );
        assert!(!UndoGroup::continued_by(None, "a", &(1..1), ty));
        assert!(!UndoGroup::continued_by(
            after_a,
            "a",
            &(1..1),
            InputType::InsertFromPaste
        ));
        assert_eq!(
            UndoGroup::after(&(1..1), 1, InputType::InsertLineBreak),
            None
        );
    }

    #[test]
    fn single_deletes_group_per_direction_and_per_grapheme() {
        let text = "aé🙂b";
        let back = InputType::DeleteContentBackward;
        let at = text.len() - 1;
        let group = UndoGroup::after(&(at..text.len()), 0, back);
        let emoji = at - "🙂".len()..at;
        assert!(UndoGroup::continued_by(group, text, &emoji, back));
        assert!(
            !UndoGroup::continued_by(group, text, &(1..at), back),
            "a word, not one grapheme"
        );
        let fwd = InputType::DeleteContentForward;
        assert!(!UndoGroup::continued_by(group, text, &emoji, fwd));
        let group = UndoGroup::after(&(1..1 + "é".len()), 0, fwd);
        assert!(UndoGroup::continued_by(
            group,
            "a🙂b",
            &(1..1 + "🙂".len()),
            fwd
        ));
    }

    #[test]
    fn maxlength_truncation_counts_utf16_units_and_keeps_characters_whole() {
        let cut = |s: &str, room| {
            let mut s = s.to_owned();
            truncate_utf16(&mut s, room);
            s
        };
        assert_eq!(cut("abc", 2), "ab");
        assert_eq!(cut("a🙂b", 2), "a", "the emoji takes two units");
        assert_eq!(cut("a🙂b", 3), "a🙂");
        assert_eq!(cut("abc", 0), "");
        assert_eq!(cut("abc", 9), "abc");
        assert_eq!(utf16_len("я🙂"), 3);
    }

    #[test]
    fn masked_offsets_round_trip_across_multibyte_characters() {
        let content = "aж😀b";
        let boundaries: Vec<usize> = content
            .char_indices()
            .map(|(at, _)| at)
            .chain([content.len()])
            .collect();
        for (n, &at) in boundaries.iter().enumerate() {
            assert_eq!(masked_offset(content, at), n * MASK.len_utf8());
            assert_eq!(unmasked_offset(content, n * MASK.len_utf8()), at);
        }
        assert_eq!(unmasked_offset(content, 4), 1);
        assert_eq!(unmasked_offset(content, 99), content.len());
        assert_eq!(masked_offset("", 0), 0);
    }

    #[test]
    fn input_pointer_coordinates_invert_rotation_scale_and_shear_at_every_dpi() {
        for dpi in [1.0, 1.25, 1.5, 2.0] {
            let transform = InputTransform {
                matrix: TransformationMatrix {
                    rotation_scale: [[0.0, -2.0], [3.0, 0.5]],
                    translation: [120.0 * dpi, -45.0 * dpi],
                },
                scale_factor: dpi,
            };
            let local = transform
                .layout_point(gpui::point(px(52.0), px(8.0)))
                .unwrap();
            assert!(
                (f32::from(local.x) - 12.0).abs() < 0.0001,
                "dpi {dpi}: {local:?}"
            );
            assert!(
                (f32::from(local.y) - 34.0).abs() < 0.0001,
                "dpi {dpi}: {local:?}"
            );
            let reflected = InputTransform {
                matrix: TransformationMatrix {
                    rotation_scale: [[-1.0, 0.5], [0.0, 2.0]],
                    translation: [200.0 * dpi, 50.0 * dpi],
                },
                scale_factor: dpi,
            };
            let local = reflected
                .layout_point(gpui::point(px(205.0), px(118.0)))
                .unwrap();
            assert!((f32::from(local.x) - 12.0).abs() < 0.0001);
            assert!((f32::from(local.y) - 34.0).abs() < 0.0001);
        }
    }

    #[test]
    fn ime_anchor_bounds_follow_all_transformed_corners_at_fractional_dpi() {
        for dpi in [1.0, 1.25, 1.5, 2.0] {
            let transform = InputTransform {
                matrix: TransformationMatrix {
                    rotation_scale: [[0.0, -2.0], [3.0, 0.5]],
                    translation: [120.0 * dpi, -45.0 * dpi],
                },
                scale_factor: dpi,
            };
            let local = Bounds::from_corners(
                gpui::point(px(10.0), px(20.0)),
                gpui::point(px(14.0), px(28.0)),
            );
            let window = transform.window_bounds(local).unwrap();
            assert_eq!(
                window,
                Bounds::from_corners(
                    gpui::point(px(64.0), px(-5.0)),
                    gpui::point(px(80.0), px(11.0))
                ),
                "dpi {dpi}"
            );
        }
    }

    #[test]
    fn collapsed_input_transforms_cannot_accept_pointer_or_ime_geometry() {
        let transform = InputTransform {
            matrix: TransformationMatrix {
                rotation_scale: [[0.0, 0.0], [0.0, 1.0]],
                translation: [10.0, 20.0],
            },
            scale_factor: 1.5,
        };
        assert!(transform
            .layout_point(gpui::point(px(15.0), px(25.0)))
            .is_none());
        assert!(transform
            .window_bounds(Bounds::from_corners(
                gpui::point(px(1.0), px(2.0)),
                gpui::point(px(3.0), px(4.0))
            ))
            .is_none());
    }

    #[test]
    fn a_right_press_keeps_only_the_selection_it_lands_in() {
        assert!(secondary_press_keeps(&(2..6), 4));
        assert!(secondary_press_keeps(&(2..6), 2));
        assert!(secondary_press_keeps(&(2..6), 6));
        assert!(!secondary_press_keeps(&(2..6), 7));
        assert!(!secondary_press_keeps(&(2..6), 0));
        assert!(!secondary_press_keeps(&(3..3), 3));
    }

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
    fn a_textarea_keeps_its_line_breaks() {
        assert_eq!(multi_line_text("a\r\nb\nc\rd"), "a\nb\nc\nd");
        assert_eq!(single_line_text("a\r\nb\nc\rd"), "a b c d");

        assert_eq!(multi_line_text("\r"), "\n");
        assert_eq!(multi_line_text("\n"), "\n");
        assert_eq!(single_line_text("\r"), "");

        assert_eq!(multi_line_text("a\tb\u{0}c\u{7}"), "a\tbc");
        assert_eq!(multi_line_text("plain"), "plain");
    }

    #[test]
    fn a_lone_control_character_is_dropped() {
        assert_eq!(single_line_text("\r"), "");
        assert_eq!(single_line_text("\n"), "");
        assert_eq!(single_line_text("\t"), "");
        assert_eq!(single_line_text("\r\n"), " ");
    }

    /// Reports each prefix of `typed`; `framework` answers one (the value seen, the counter before)
    /// with the field's controlled value after it, as `(value, write)`.
    fn type_reports(
        typed: &str,
        mut framework: impl FnMut(&str, Option<u32>) -> Option<(String, u32)>,
    ) -> VecDeque<u32> {
        let mut echoes = VecDeque::new();
        let mut current: Option<(String, u32)> = None;
        for end in 1..=typed.len() {
            let reported = &typed[..end];
            let before = current.as_ref().map(|(_, write)| *write);
            current = framework(reported, before).or(current);
            let after = current
                .as_ref()
                .map(|(value, write)| (value.as_str(), *write));
            if let Some(write) = echo_write(reported, before, after) {
                remember_echo(&mut echoes, write);
            }
        }
        echoes
    }

    #[test]
    fn an_earlier_typed_value_assigned_by_script_is_not_an_echo() {
        let mut echoes = type_reports("abcde", |_, _| None);
        assert!(echoes.is_empty());
        assert!(!take_echo(&mut echoes, 1));
        assert_eq!(echo_write("ab", None, Some(("ab", 1))), Some(1));
        assert_eq!(echo_write("abcde", Some(0), Some(("ab", 1))), None);
    }

    #[test]
    fn write_backs_are_echoes_even_when_synced_late_or_out_of_order() {
        let mut write = 0;
        let mut echoes = type_reports("abcd", |reported, _| {
            write += 1;
            Some((reported.to_owned(), write))
        });
        assert_eq!(echoes, VecDeque::from([1, 2, 3, 4]));
        assert!(take_echo(&mut echoes, 2));
        assert!(take_echo(&mut echoes, 4));
        assert!(echoes.is_empty());
    }

    #[test]
    fn a_handler_rewriting_the_value_is_an_assignment() {
        let echoes = type_reports("ab", |reported, before| {
            Some((reported.to_uppercase(), before.unwrap_or(0) + 1))
        });
        assert!(echoes.is_empty());
    }
}
