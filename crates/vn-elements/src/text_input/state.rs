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
    App, Bounds, ClipboardItem, Context, EntityInputHandler, FocusHandle, Focusable, KeyDownEvent,
    KeystrokeEvent, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Subscription, Task,
    TextInputConfiguration, UTF16Selection, Window, WrappedLine, px,
};

use super::boundaries;
use super::{
    Backspace, ClearSelection, CompositionPhase, Copy, Cut, Delete, DeleteToEnd, DeleteToStart,
    DeleteWordLeft, ControlledValue, DeleteWordRight, Down, End, Home, InputIntent, InputType, Left,
    Newline, OnBeforeInput, OnChange, OnComposition, OnKeyDown, Paste, Redo, Right,
    SelectAll, SelectDown, SelectEnd, SelectHome, SelectLeft, SelectRight, SelectUp, SelectWordLeft,
    SelectWordRight, ShowCharacterPalette, TextInputHandlers, Undo, Up, WordLeft, WordRight,
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

/// Fold text into something a `<textarea>` can hold.
///
/// Line breaks SURVIVE — that is the whole difference from
/// [`single_line_text`] — normalised to `\n` the way a browser normalises a
/// textarea's value (the HTML "API value" uses LF, never CRLF, whatever the
/// platform sent). Tabs survive too. Every other control character is stripped,
/// because none of them can be shaped and one stray `\0` in a run of text makes
/// the whole line unshapeable.
///
/// A LONE `\r`/`\n` maps to `\n` rather than being dropped: on a platform that
/// routes Return through the IME text-input path instead of through the key
/// binding, that lone control character is the Return, and a textarea must
/// insert a newline for it.
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

/// The keystroke gpui synthesizes when one modifier is pressed and released
/// with nothing in between (zed#64537 hands it to interceptors): one of five
/// fixed names, no character, no modifiers held.
fn is_standalone_modifier(keystroke: &gpui::Keystroke) -> bool {
    keystroke.key_char.is_none()
        && keystroke.modifiers == gpui::Modifiers::default()
        && matches!(
            keystroke.key.as_str(),
            "shift" | "control" | "alt" | "platform" | "function"
        )
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
    on_before_input: Option<OnBeforeInput>,
    on_composition: Option<OnComposition>,
    on_key_down: Option<OnKeyDown>,
    /// A `compositionstart` went out and its `compositionend` is still owed.
    composition_open: bool,
    /// The text of a composition that a value assigned from outside cut short,
    /// whose `compositionend` the framework delivers
    /// ([`Self::flush_abandoned_composition`]): the assignment comes with no
    /// window to dispatch from.
    abandoned_composition: Option<String>,
    /// Ends a composition when the field loses focus ([`Self::finish_composition`]).
    _blur: Subscription,

    /// Set when `on_key_down` cancelled a keystroke that would have produced a
    /// character.
    ///
    /// Consuming the key event is usually enough — a platform that asks the app
    /// first (Windows re-sends `WM_KEYDOWN` as `WM_GPUI_KEYDOWN` and skips
    /// `TranslateMessage` when it is handled) then never produces the
    /// character. But not every path is gated on that: gpui's own
    /// `replay_pending_input` feeds `dispatch_input` straight to the input
    /// handler after a multi-stroke binding fails to complete. So the widget
    /// also refuses the text itself. Cleared by the next keystroke.
    suppress_text_input: bool,
    /// Keeps the app-wide keystroke interceptor alive for this input's
    /// lifetime; see [`Self::on_intercept_keystroke`].
    _key_interceptor: Option<Subscription>,

    /// Values we reported through `on_change`, newest last.
    pending_values: VecDeque<String>,
    /// The framework's write counter for the last value assignment we applied,
    /// so re-reading an untouched prop every frame is free. Keyed on the
    /// counter rather than the string: two assignments of the same value are
    /// two assignments, and the second one still has to land if the user typed
    /// in between (`v-model` clearing a field the user refilled).
    last_synced_write: Option<u32>,
    /// Where the controlled value is read again before a default action; see
    /// [`ControlledValue`].
    controlled_value: Option<ControlledValue>,

    undo_stack: Vec<EditSnapshot>,
    redo_stack: Vec<EditSnapshot>,

    is_selecting: bool,

    /// `<textarea>` rather than `<input>` — see [`super::TextInputOptions`].
    multiline: bool,

    /// What `text_input_configuration` hands gpui — see
    /// [`super::TextInputOptions::input_config`].
    input_config: TextInputConfiguration,
    /// The configuration gpui last actually TOOK, and how many times it has
    /// asked. gpui's own `Window::apply_text_input_configuration` is the only
    /// caller of the trait method, so this is evidence the configuration left
    /// this crate and reached the platform layer — not merely that it was
    /// computed. Recording the queried value (rather than reading
    /// `input_config` back later) keeps the two independent of where in the
    /// frame the reader sits.
    queried_config: Option<TextInputConfiguration>,
    config_queries: u64,
    /// The last configuration [`Self::newly_forwarded_configuration`] reported,
    /// so a focused field logs once per distinct value instead of once per
    /// frame.
    reported_config: Option<TextInputConfiguration>,

    // Written by the element during prepaint, read by hit testing, the caret
    // and the IME. One entry per HARD line (`shape_text` splits on `\n`); a
    // single-line input always has exactly one. The shaped lines LIVE here —
    // `WrappedLine` is not `Clone`, so the element hands them over and borrows
    // them back to paint.
    lines: Vec<LineBox>,
    last_bounds: Option<Bounds<Pixels>>,
    line_height: Pixels,
    content_width: Pixels,
    /// `line_height * total visual rows` — what the vertical scroll clamps to.
    content_height: Pixels,
    display_is_placeholder: bool,
    scroll_left: Pixels,
    /// Vertical scroll offset, always zero for a single-line input.
    scroll_top: Pixels,
    /// How far right the content sits inside the box because of `text-align`.
    ///
    /// Alignment is deliberately NOT applied where the text is painted, but
    /// here, in the one number that maps content space to window space
    /// ([`Self::content_left`]). Painting the glyphs somewhere the caret, the
    /// selection quads, the IME candidate window and click-to-position do not
    /// know about is how a centred field ends up with a caret that lands two
    /// characters away from where the user clicked.
    ///
    /// Zero unless the content is narrower than the box: once it overflows, the
    /// box scrolls and alignment has nothing left to distribute — which is also
    /// what a browser does.
    align_offset: Pixels,

    blink_anchor: Instant,
    blink_task: Option<Task<()>>,
}

/// One HARD line (a `\n`-delimited run) and where it sits.
///
/// Soft wrapping happens INSIDE a `WrappedLine`: one hard line can occupy
/// several visual rows, and `rows` is how many. `row0` is the visual row this
/// hard line starts on, counting from the top of the content.
pub(crate) struct LineBox {
    /// Byte offset of this line's first character in `content`.
    pub(crate) start: usize,
    pub(crate) row0: usize,
    pub(crate) rows: usize,
    pub(crate) line: WrappedLine,
}

impl LineBox {
    /// Byte offset one past this line's last character (excluding the `\n`).
    fn end(&self) -> usize {
        self.start + self.line.len()
    }
}

impl TextInputState {
    /// Create the state for one input node, in the window that shows it (its
    /// blur is that window's focus change).
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // A DOM `keydown` listener runs before the browser's default action for
        // the key. gpui resolves key *bindings* (our editing actions) before it
        // ever reaches element key listeners, so a plain `on_key_down` would
        // arrive after `backspace` had already deleted. The keystroke
        // interceptor is the one hook that runs first — `cx.stop_propagation()`
        // in it suppresses binding dispatch for the keystroke.
        let weak = cx.weak_entity();
        let interceptor = cx.intercept_keystrokes(move |event, window, cx| {
            weak.update(cx, |this, cx| this.on_intercept_keystroke(event, window, cx))
                .ok();
        });
        let focus_handle = cx.focus_handle();
        let blur = cx.on_blur(&focus_handle, window, |this, window, cx| this.finish_composition(window, cx));
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
            pending_values: VecDeque::new(),
            last_synced_write: None,
            controlled_value: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            is_selecting: false,
            multiline: false,
            input_config: TextInputConfiguration::default(),
            queried_config: None,
            config_queries: 0,
            reported_config: None,
            lines: Vec::new(),
            last_bounds: None,
            line_height: px(20.0),
            content_width: px(0.0),
            content_height: px(0.0),
            display_is_placeholder: true,
            align_offset: px(0.0),
            scroll_left: px(0.0),
            scroll_top: px(0.0),
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
    /// `write` identifies the *assignment*: the framework bumps it every time
    /// something writes the `value` prop, even when the string is unchanged.
    /// Idempotent in that re-passing the same `write` is a no-op, so this can
    /// be called unconditionally every frame; a fresh `write` always lands,
    /// which is what makes `v-model` able to clear a field twice in a row.
    ///
    /// A value that matches one this input itself reported is swallowed (echo
    /// ring); any other change is treated as external and hard-resets content,
    /// caret and undo history.
    pub fn sync_value(&mut self, value: &str, write: u32, cx: &mut Context<Self>) {
        if self.last_synced_write == Some(write) {
            return;
        }
        self.last_synced_write = Some(write);

        if take_echo(&mut self.pending_values, value) {
            return;
        }
        self.set_external_text(value, cx);
    }

    /// [`Self::sync_value`] from the framework's [`ControlledValue`], now
    /// rather than at the next frame: a hook that let a key or clipboard
    /// command through may have assigned the field, and the default action
    /// edits what it wrote.
    pub(crate) fn resync_controlled(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.controlled_value.clone() else {
            return;
        };
        if let Some((value, write)) = source(cx) {
            self.sync_value(&value, write, cx);
        }
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
    pub(crate) fn set_frame_params(
        &mut self,
        handlers: TextInputHandlers,
        read_only: bool,
        multiline: bool,
        input_config: TextInputConfiguration,
    ) {
        self.on_change = handlers.on_change;
        self.on_before_input = handlers.on_before_input;
        self.on_composition = handlers.on_composition;
        self.on_key_down = handlers.on_key_down;
        self.controlled_value = handlers.controlled_value;
        self.read_only = read_only;
        self.multiline = multiline;
        self.input_config = input_config;
    }

    /// `Some((configuration, queries))` the first time gpui has queried a
    /// configuration this field has not reported yet — i.e. edge-triggered on
    /// the VALUE, and only ever after gpui actually asked.
    ///
    /// The framework calls this once a frame and logs what it gets, which is
    /// the observable end of the chain `attributes → TextInputOptions →
    /// InputHandler → Window::apply_text_input_configuration → PlatformWindow`.
    /// Nothing else can distinguish "we built a configuration" from "gpui took
    /// it": on Windows the platform window's `set_text_input_configuration` is
    /// the default no-op, so the forwarding leaves no other trace.
    pub fn newly_forwarded_configuration(&mut self) -> Option<(TextInputConfiguration, u64)> {
        let queried = self.queried_config.as_ref()?;
        if self.reported_config.as_ref() == Some(queried) {
            return None;
        }
        self.reported_config = Some(queried.clone());
        Some((queried.clone(), self.config_queries))
    }

    /// Fold incoming text into something this field can hold: `single_line_text`
    /// for an `<input>`, newline normalisation for a `<textarea>`.
    fn sanitize(&self, text: &str) -> String {
        if self.multiline {
            multi_line_text(text)
        } else {
            single_line_text(text)
        }
    }

    /// DOM `tabindex`: `Some(n >= 0)` puts this input in the tab order at index
    /// `n`, `Some(n < 0)` keeps it focusable but skipped by `Tab`, `None` takes
    /// it out. gpui reads both values off the focus handle when the element is
    /// painted, so this rewrites the handle in place (same `FocusId` — focus is
    /// not disturbed).
    pub fn set_tab_index(&mut self, index: Option<isize>) {
        let tab_index = index.unwrap_or(0);
        let tab_stop = index.is_some_and(|n| n >= 0);
        if self.focus_handle.tab_index == tab_index && self.focus_handle.tab_stop == tab_stop {
            return;
        }
        self.focus_handle = self.focus_handle.clone().tab_index(tab_index).tab_stop(tab_stop);
    }

    /// The `keydown` hook, run for every keystroke the window sees, before gpui
    /// resolves key bindings. Only the focused input reacts.
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
        // Since zed#64537 interceptors also receive the keystroke gpui
        // synthesizes for a modifier pressed and released ALONE — on the
        // RELEASE, as `key: "shift"` with no modifiers. A browser fires that
        // key's `keydown` on the press, so turning this into one would be a
        // keydown at keyup time; it was never delivered before the merge, and
        // it is not now.
        if is_standalone_modifier(&event.keystroke) {
            return;
        }
        let Some(on_key_down) = self.on_key_down.clone() else {
            return;
        };
        let key_down = KeyDownEvent {
            keystroke: event.keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        };
        if on_key_down(&key_down, window, cx) {
            // The keydown's handlers may have assigned the field; the binding
            // or the character that follows edits what they wrote.
            self.resync_controlled(cx);
            self.flush_abandoned_composition(window, cx);
            return;
        }
        // preventDefault(): no binding action for this keystroke, and no
        // character insertion if it was going to produce one.
        self.suppress_text_input = event.keystroke.key_char.is_some();
        cx.stop_propagation();
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
        let value = self.sanitize(value);
        if self.content == value {
            return;
        }
        // The pre-edit's range means nothing in the new value: the composition
        // ends here, and its `compositionend` is owed.
        self.abandon_composition();
        self.content = value;
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

    /// Report the current value to the framework, with the edit that made it,
    /// and remember it in the echo ring. The callback runs deferred — after
    /// the current update completes — so it is safe for it to call back into
    /// this entity (`sync_value`).
    fn emit_change(&mut self, intent: InputIntent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(on_change) = self.on_change.clone() else {
            return;
        };
        remember_reported(&mut self.pending_values, self.content.clone());
        let value = self.content.clone();
        window.defer(cx, move |window, cx| on_change(&value, &intent, window, cx));
    }

    /// `beforeinput` for an edit about to be made: `false` when a handler
    /// cancelled it. Its handlers may have assigned the field, and the edit
    /// applies to what they wrote, so the value is read back first; the caller
    /// works out its range after this. The IME's edits cannot be cancelled,
    /// and the text in the middle of a composition is the IME's: nothing is
    /// read back then.
    fn before_input(&mut self, intent: &InputIntent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(hook) = self.on_before_input.clone() else {
            return true;
        };
        let proceed = hook(intent, window, cx);
        if intent.input_type.composing() {
            return true;
        }
        if proceed {
            self.resync_controlled(cx);
            self.flush_abandoned_composition(window, cx);
        }
        proceed
    }

    /// One stage of the IME composition for the framework: `Start` and
    /// `Update` now, `End` deferred, so that it follows the `input` the
    /// commit's edit queued. An `End` with no open composition is dropped, so
    /// each `Start` gets exactly one.
    fn composition(&mut self, phase: CompositionPhase, data: &str, window: &mut Window, cx: &mut Context<Self>) {
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
            window.defer(cx, move |window, cx| hook(CompositionPhase::End, &data, window, cx));
        } else {
            hook(phase, data, window, cx);
        }
    }

    /// Ends the composition in progress, keeping the text it has: the field
    /// lost focus (Chromium commits the composition then) or the platform
    /// unmarked it.
    pub(crate) fn finish_composition(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(range) = self.marked_range.take() else {
            return;
        };
        let text = self.content.get(range).unwrap_or_default().to_owned();
        cx.notify();
        self.composition(CompositionPhase::End, &text, window, cx);
    }

    /// The pre-edit is about to lose its meaning (a value assigned from
    /// outside): its `compositionend` is owed, with the text it had.
    fn abandon_composition(&mut self) {
        let Some(range) = self.marked_range.take() else {
            return;
        };
        if std::mem::take(&mut self.composition_open) {
            self.abandoned_composition = Some(self.content.get(range).unwrap_or_default().to_owned());
        }
    }

    /// Delivers the `compositionend` an assigned value owes
    /// ([`Self::abandon_composition`]), deferred like every `End`. The
    /// framework calls it after [`Self::sync_value`], where it has a window.
    pub fn flush_abandoned_composition(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = self.abandoned_composition.take() else {
            return;
        };
        let Some(hook) = self.on_composition.clone() else {
            return;
        };
        window.defer(cx, move |window, cx| hook(CompositionPhase::End, &text, window, cx));
    }

    fn restore(&mut self, snapshot: EditSnapshot, input_type: InputType, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_composition(window, cx);
        self.content = snapshot.content;
        self.selected_range = snapshot.selected_range;
        self.selection_reversed = snapshot.selection_reversed;
        self.reset_blink(cx);
        self.emit_change(InputIntent::bare(input_type), window, cx);
        cx.notify();
    }

    /// The one place content is mutated by the user. `range` is a byte range
    /// into the current content; the returned range covers the inserted text,
    /// or `None` if the edit was a no-op. `intent` is the edit as the `input`
    /// event reports it.
    fn edit(
        &mut self,
        range: Range<usize>,
        new_text: &str,
        intent: InputIntent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        // Never trust a range straight from the platform's IME.
        let start = boundaries::snap(&self.content, range.start);
        let end = boundaries::snap(&self.content, range.end).max(start);
        let range = start..end;

        let replacement = self.sanitize(new_text);
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
        self.emit_change(intent, window, cx);
        cx.notify();
        Some(inserted)
    }

    /// Replace the current selection (or the IME pre-edit) with `text`.
    fn replace_selection(&mut self, text: &str, intent: InputIntent, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let range = self
            .marked_range
            .clone()
            .unwrap_or_else(|| self.selected_range.clone());
        let Some(inserted) = self.edit(range, text, intent, window, cx) else {
            return;
        };
        self.selected_range = inserted.end..inserted.end;
        self.selection_reversed = false;
        // A paste in the middle of a composition replaced the pre-edit.
        if self.marked_range.take().is_some() {
            self.composition(CompositionPhase::End, text, window, cx);
        }
    }

    /// A deletion: the selection, or from the caret to `to(self)`. Nothing to
    /// delete (Backspace at the start of the value) is no edit and no
    /// `beforeinput`, as in Chromium.
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
        self.set_selection(0..self.content.len(), cx);
    }

    /// `Home` — the start of the VISUAL row in a textarea (what a browser
    /// does), the start of the value in a single-line input (where the two
    /// coincide anyway, since it has exactly one row).
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

    /// `ArrowUp`. In a textarea this moves one visual row up, keeping the
    /// caret's x; in a single-line input a browser jumps to the start of the
    /// value, which is what falling through to row -1 does here.
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

    /// Return in a `<textarea>`: insert a line break, never submit. Bound only
    /// in the textarea key context, so this is unreachable for an `<input>`;
    /// the guard is belt and braces.
    pub(crate) fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only || !self.multiline {
            return;
        }
        // Chromium's `insertLineBreak` for a textarea (a contenteditable's
        // Return is `insertParagraph`), with no data.
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
        self.delete_toward(InputType::DeleteSoftLineForward, |s| s.content.len(), window, cx);
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
        // Chromium writes the clipboard BEFORE it asks `beforeinput`
        // (`ClipboardCommands::ExecuteCut`): a cancelled `deleteByCut` leaves
        // the text both on the clipboard and in the field.
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
        // A plain-text field's `insertFromPaste` carries the clipboard's text
        // as `data` (and no DataTransfer), as Chromium's does.
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

    /// A right-button press: focus, and the caret moves to the press unless
    /// the press is inside the selection, which then stays for the context
    /// menu to act on (Chromium on Windows, in an editable field). No drag
    /// selection starts, and neither double- nor triple-click selects.
    pub(crate) fn on_secondary_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        let index = self.index_for_position(event.position);
        if !secondary_press_keeps(&self.selected_range, index) {
            self.move_to(index, cx);
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

    /// Called by the element every frame once the text has been shaped. The
    /// lines are MOVED in: `WrappedLine` is not `Clone`, so this is where they
    /// live for the rest of the frame and the element borrows them back to
    /// paint.
    pub(crate) fn set_layout(
        &mut self,
        bounds: Bounds<Pixels>,
        lines: Vec<LineBox>,
        line_height: Pixels,
        display_is_placeholder: bool,
        text_align: gpui::TextAlign,
    ) {
        self.content_width = lines
            .iter()
            .map(|l| l.line.width())
            .fold(px(0.0), Pixels::max);
        let rows: usize = lines.iter().map(|l| l.rows).sum();
        self.content_height = line_height * rows.max(1) as f32;
        self.lines = lines;
        self.last_bounds = Some(bounds);
        self.line_height = line_height;
        self.display_is_placeholder = display_is_placeholder;
        // Room to distribute, measured on the WIDEST line so a multi-line
        // textarea's block stays coherent. `CARET_WIDTH` is reserved for the
        // same reason `clamp_scroll` reserves it: a caret at the end of a
        // right-aligned field must still have somewhere to sit.
        let slack = (bounds.size.width - self.content_width - CARET_WIDTH).max(px(0.0));
        self.align_offset = match text_align {
            gpui::TextAlign::Left => px(0.0),
            gpui::TextAlign::Center => slack / 2.0,
            gpui::TextAlign::Right => slack,
        };
        self.clamp_scroll(bounds.size);
    }

    /// Window-space x of content-space x = 0 — the single place `text-align`
    /// and the horizontal scroll are turned into a position. Everything that
    /// crosses between the two spaces goes through it.
    pub(crate) fn content_left(&self, bounds_left: Pixels) -> Pixels {
        bounds_left - self.scroll_left + self.align_offset
    }

    /// Keep the caret inside the viewport, and the viewport inside the content.
    ///
    /// A textarea soft-wraps, so its content is never wider than the box and
    /// only the vertical axis moves; a single-line input is the mirror image.
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

    /// Wheel scrolling for a textarea, in CSS pixels (positive = down).
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

    /// Round a byte offset to the nearest grapheme boundary in the content, so
    /// a bisection over indices can never land inside a character.
    pub(crate) fn snap_index(&self, index: usize) -> usize {
        boundaries::snap(&self.content, index.min(self.content.len()))
    }

    pub(crate) fn line_height(&self) -> Pixels {
        self.line_height
    }

    /// The hard line containing `index`, and the index within it.
    fn line_at(&self, index: usize) -> Option<(&LineBox, usize)> {
        let boxes = &self.lines;
        // A `\n` sits BETWEEN two lines and belongs to neither, so an offset
        // pointing at one reports the end of the line before it — which is
        // where a browser draws the caret after pressing End.
        let found = boxes
            .iter()
            .rev()
            .find(|l| index >= l.start)
            .or_else(|| boxes.first())?;
        Some((found, (index - found.start.min(index)).min(found.line.len())))
    }

    /// Where the caret goes for a byte offset, relative to the top-left of the
    /// CONTENT (before scrolling). `(0, 0)` when nothing has been shaped yet.
    pub(crate) fn position_for_index(&self, index: usize) -> Point<Pixels> {
        if self.display_is_placeholder {
            return Point::default();
        }
        let Some((line_box, local)) = self.line_at(index) else {
            return Point::default();
        };
        let within = line_box
            .line
            .position_for_index(local, self.line_height)
            // `position_for_index` answers `None` past the last glyph; the end
            // of the last row is the right answer there.
            .unwrap_or_else(|| {
                Point::new(
                    line_box.line.width(),
                    self.line_height * (line_box.rows.saturating_sub(1)) as f32,
                )
            });
        Point::new(
            within.x,
            within.y + self.line_height * line_box.row0 as f32,
        )
    }

    /// Map a window-space position to a byte offset in the content.
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
        // Clicking above the first row or below the last one selects the near
        // end of the content, exactly as a browser does.
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
        if row >= line_box.row0 + line_box.rows && line_box.end() >= self.content.len() {
            return self.content.len();
        }
        let within = Point::new(
            local.x,
            local.y - self.line_height * line_box.row0 as f32,
        );
        let index = match line_box
            .line
            .closest_index_for_position(within, self.line_height)
        {
            Ok(i) | Err(i) => i,
        };
        boundaries::snap(&self.content, line_box.start + index.min(line_box.line.len()))
    }

    /// The start (`forward = false`) or end of the VISUAL row `index` is on.
    fn row_bound(&self, index: usize, forward: bool) -> usize {
        if self.lines.is_empty() {
            return if forward { self.content.len() } else { 0 };
        }
        let y = self.position_for_index(index).y;
        let x = if forward { px(1.0e6) } else { px(-1.0e6) };
        self.index_for_content_point(Point::new(x, y))
    }

    /// One visual row up (`delta = -1`) or down, keeping the caret's x. Off
    /// either end this saturates to the start / end of the content, which is
    /// both what a browser does in a textarea and what it does in a
    /// single-line input (where there is only ever one row).
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

    /// [`Self::index_for_position`] in CONTENT space (no bounds, no scroll).
    fn index_for_content_point(&self, point: Point<Pixels>) -> usize {
        let Some(bounds) = self.last_bounds else {
            return 0;
        };
        self.index_for_position(Point::new(
            point.x + self.content_left(bounds.left()),
            point.y + bounds.top() - self.scroll_top,
        ))
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
        // The keystroke that would have produced this text was cancelled by a
        // `keydown` handler; the platform still delivered the character.
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
            // The IME commits (or, with "", cancels) its composition. The
            // last `beforeinput` and `compositionupdate` carry the committed
            // text, then the edit and its `input`, then `compositionend`
            // (Chromium's `InsertTextDuringCompositionWithEvents`, then
            // `DispatchCompositionEndEvent`).
            let intent = InputIntent::with(InputType::InsertCompositionText, new_text);
            self.before_input(&intent, window, cx);
            self.composition(CompositionPhase::Update, new_text, window, cx);
            if let Some(inserted) = self.edit(range_of(self), new_text, intent, window, cx) {
                self.selected_range = inserted.end..inserted.end;
                self.selection_reversed = false;
            }
            if self.marked_range.take().is_some() {
                cx.notify();
            }
            self.composition(CompositionPhase::End, new_text, window, cx);
            return;
        }
        // Typed text. What the field cannot hold (Return in an `<input>`) is
        // no edit and no event, whatever is selected.
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
        // After the handlers: a value they assigned put the caret at its end.
        let Some(inserted) = self.edit(range_of(self), new_text, intent, window, cx) else {
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
                // An empty pre-edit outside a composition (some IMEs send
                // one): Chromium deletes the selection and starts nothing.
                if range.is_empty() {
                    return;
                }
                let intent = InputIntent::bare(InputType::DeleteContentBackward);
                if !self.before_input(&intent, window, cx) {
                    return;
                }
                let range = self.selected_range.clone();
                if let Some(inserted) = self.edit(range, "", intent, window, cx) {
                    self.selected_range = inserted.end..inserted.end;
                    self.selection_reversed = false;
                }
                return;
            }
            // The first pre-edit: `compositionstart`, whose data is what the
            // composition replaces (the selection, usually nothing).
            let replaced = self.content.get(range.clone()).unwrap_or_default().to_owned();
            self.composition(CompositionPhase::Start, &replaced, window, cx);
        }
        // Chromium's order for every pre-edit: `beforeinput` (not
        // cancelable), `compositionupdate`, the edit, `input`.
        let intent = InputIntent::with(InputType::InsertCompositionText, new_text);
        self.before_input(&intent, window, cx);
        self.composition(CompositionPhase::Update, new_text, window, cx);

        let Some(inserted) = self.edit(range, new_text, intent, window, cx) else {
            // The composition was cancelled with nothing to replace.
            if self.marked_range.take().is_some() {
                cx.notify();
            }
            self.composition(CompositionPhase::End, "", window, cx);
            return;
        };
        self.marked_range = (!inserted.is_empty()).then_some(inserted.clone());
        if self.marked_range.is_none() {
            // An empty pre-edit (the user deleted all of it, or Escape) ends
            // the composition.
            self.composition(CompositionPhase::End, "", window, cx);
        }

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
        let start = self.position_for_index(range.start);
        let end = self.position_for_index(range.end);
        // The candidate window hangs off the row the pre-edit ENDS on. When a
        // composition wraps or spans lines the two rows differ, and reporting
        // the start's row would put the IME popup a line too high.
        let top = element_bounds.top() + end.y - self.scroll_top;
        let left = self.content_left(element_bounds.left());
        let (x0, x1) = if start.y == end.y {
            (start.x, end.x.max(start.x + CARET_WIDTH))
        } else {
            (px(0.0), end.x.max(CARET_WIDTH))
        };
        Some(Bounds::from_corners(
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

    /// gpui asks the FOCUSED field for this once a frame and forwards it to the
    /// platform window only when it changed (zed#63264). The value is whatever
    /// the last frame's `<input>` attributes mapped to, so a reactive
    /// `:spellcheck` takes effect on the frame after it changes — which is what
    /// a browser does with the attribute too.
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

/// Does a right-button press at `index` keep `selection`? Only a press inside
/// a non-empty selection does (its edges included: a press there is on the
/// selected text's own glyph half).
fn secondary_press_keeps(selection: &Range<usize>, index: usize) -> bool {
    !selection.is_empty() && selection.start <= index && index <= selection.end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_right_press_keeps_only_the_selection_it_lands_in() {
        assert!(secondary_press_keeps(&(2..6), 4));
        assert!(secondary_press_keeps(&(2..6), 2));
        assert!(secondary_press_keeps(&(2..6), 6));
        assert!(!secondary_press_keeps(&(2..6), 7));
        assert!(!secondary_press_keeps(&(2..6), 0));
        // A caret is no selection: the press moves it.
        assert!(!secondary_press_keeps(&(3..3), 3));
    }

    #[test]
    fn ime_offsets_are_relative_to_the_replacement_text() {
        assert_eq!(utf16_offset_to_utf8("é🙂", 0), 0);
        assert_eq!(utf16_offset_to_utf8("é🙂", 1), "é".len());
        assert_eq!(utf16_offset_to_utf8("é🙂", 3), "é🙂".len());
    }

    /// Only gpui's synthesized lone-modifier stroke is filtered; a real key
    /// with a modifier held, or a modifier-named key carrying text, is not.
    #[test]
    fn a_released_lone_modifier_is_not_a_keydown() {
        let stroke = |key: &str, modifiers: gpui::Modifiers, key_char: Option<&str>| gpui::Keystroke {
            key: key.into(),
            modifiers,
            key_char: key_char.map(Into::into),
        };
        for key in ["shift", "control", "alt", "platform", "function"] {
            assert!(is_standalone_modifier(&stroke(key, gpui::Modifiers::default(), None)));
        }
        assert!(!is_standalone_modifier(&stroke("a", gpui::Modifiers::default(), Some("a"))));
        assert!(!is_standalone_modifier(&stroke("a", gpui::Modifiers::shift(), Some("A"))));
        assert!(!is_standalone_modifier(&stroke("shift", gpui::Modifiers::control(), None)));
        assert!(!is_standalone_modifier(&stroke("f12", gpui::Modifiers::default(), None)));
    }

    #[test]
    fn pasted_line_breaks_become_single_spaces() {
        assert_eq!(single_line_text("a\r\nb\nc\rd"), "a b c d");
        assert_eq!(single_line_text("plain"), "plain");
    }

    /// The whole difference between `<input>` and `<textarea>`, in one place.
    #[test]
    fn a_textarea_keeps_its_line_breaks() {
        // Every spelling normalises to LF — the HTML "API value" a browser
        // reports from `textarea.value`, whatever the platform sent.
        assert_eq!(multi_line_text("a\r\nb\nc\rd"), "a\nb\nc\nd");
        // …where the same paste into a single-line input folds into spaces.
        assert_eq!(single_line_text("a\r\nb\nc\rd"), "a b c d");

        // A LONE Return is a newline here and nothing at all there. That case
        // is the Return keypress on a platform that routes it through the text
        // input handler instead of through the key binding.
        assert_eq!(multi_line_text("\r"), "\n");
        assert_eq!(multi_line_text("\n"), "\n");
        assert_eq!(single_line_text("\r"), "");

        // Tabs survive; nothing else unshapeable does.
        assert_eq!(multi_line_text("a\tb\u{0}c\u{7}"), "a\tbc");
        assert_eq!(multi_line_text("plain"), "plain");
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
